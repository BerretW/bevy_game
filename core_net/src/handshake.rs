//! Phase 2 handshake: digest delivery + file download + ready signal.
//!
//! Sekvence:
//!
//! ```text
//!   Client                          Server
//!     |  --- lightyear connect --->   |
//!     |                               |  Add<Connected>
//!     |  <-- ServerHello (digest) --  |  send_hello_on_connect
//!     |                               |
//!     |  HTTP GET /resources/...      |  axum HTTP file server
//!     |  <-- bytes ----------------   |
//!     |  (write to cache)             |
//!     |                               |
//!     |  -- ClientReady ----------->  |  log_client_ready
//!     |                               |
//! ```
//!
//! Po `ClientReady` server uvolní klienta do "in-game" stavu (Phase 3+).
//!
//! Download běží v `bevy::tasks::IoTaskPool` jako blokující reqwest call —
//! `reqwest::blocking::Client` si interně drží `current_thread` Tokio
//! runtime, takže nepotřebujeme spravovat svůj.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use crossbeam_channel::{unbounded, Receiver};
use lightyear::prelude::*;

use core_resources::{ResourceId, ResourcesDirty, ServerResourceAllowlist};

use crate::auth::ServerAuthConfig;
use crate::digest::ResourceDigest;
use crate::digest_cache::ResourceDigestCache;
use crate::net_plugin::HandshakeChannel;
use crate::protocol::{ClientReady, ServerHello, PROTOCOL_VERSION};

// ---------------------------------------------------------------------------
// Server side
// ---------------------------------------------------------------------------

/// Konfigurace pro server-side handshake. `http_base_url` musí ukazovat
/// na HTTP file server, který klient použije pro download.
#[derive(Resource, Clone, Debug)]
pub struct ServerHandshakeConfig {
    pub http_base_url: String,
}

impl Default for ServerHandshakeConfig {
    fn default() -> Self {
        Self {
            // Dev default: HTTP server na stejném hostu jako lightyear UDP.
            // Produkce by tu měla externí IP / hostname.
            http_base_url: "http://127.0.0.1:8081".to_string(),
        }
    }
}

/// Marker component added to peer entity when `ClientReady` is received.
/// Other systems (e.g. `ServerAuthPlugin`) observe `Add<ClientHandshakeComplete>`
/// instead of consuming `MessageReceiver<ClientReady>` a second time.
#[derive(Component)]
pub struct ClientHandshakeComplete;

pub struct ServerHandshakePlugin;

impl Plugin for ServerHandshakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerHandshakeConfig>();
        app.init_resource::<ServerAuthConfig>();
        app.add_observer(send_hello_on_connect);
        app.add_systems(Update, mark_client_ready);
    }
}

fn send_hello_on_connect(
    trigger: On<Add, Connected>,
    config: Res<ServerHandshakeConfig>,
    auth_config: Res<ServerAuthConfig>,
    cache: Res<ResourceDigestCache>,
    mut senders: Query<&mut MessageSender<ServerHello>>,
) {
    let entity = trigger.entity;
    let Ok(mut sender) = senders.get_mut(entity) else {
        warn!(
            "[handshake/server] entity {:?} has no MessageSender<ServerHello> yet",
            entity
        );
        return;
    };

    if !cache.is_ready() {
        warn!(
            "[handshake/server] digest cache not ready yet — dropping ServerHello to {:?}",
            entity
        );
        return;
    }

    let hello = ServerHello {
        protocol_version: PROTOCOL_VERSION,
        http_base_url: config.http_base_url.clone(),
        manifests: cache.set().manifests.clone(),
        auth_required: auth_config.require_auth,
    };
    let count = hello.manifests.len();
    sender.send::<HandshakeChannel>(hello);
    info!(
        "[handshake/server] sent ServerHello to {:?} ({} manifest(s), http_base={}, auth_required={})",
        entity, count, config.http_base_url, auth_config.require_auth
    );
}

/// When ClientReady is received: log it and mark the peer entity with
/// `ClientHandshakeComplete` so other systems (auth) can observe it
/// without consuming the message a second time.
fn mark_client_ready(
    mut commands: Commands,
    mut receivers: Query<(Entity, &mut MessageReceiver<ClientReady>, &RemoteId)>,
) {
    for (entity, mut rx, remote_id) in receivers.iter_mut() {
        for ready in rx.receive() {
            let client_id = match remote_id.0 {
                PeerId::Netcode(id) => id,
                _ => 0,
            };
            info!(
                "[handshake/server] client {} ready (protocol v{})",
                client_id, ready.protocol_version
            );
            commands.entity(entity).insert(ClientHandshakeComplete);
        }
    }
}

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

#[derive(Resource, Clone, Debug)]
pub struct ClientHandshakeConfig {
    /// Lokální cache root, kam stahujeme files. Default: `cache/resources/`
    /// relativně k CWD. Produkce by sem měla dát platform cache dir
    /// (`%LOCALAPPDATA%/.../cache`, `~/.cache/...`).
    pub cache_root: PathBuf,
    /// Pokud server pošle vlastní `http_base_url`, klient ho použije.
    /// Když je `None`, klient z digestu padá na default (užitečné při
    /// debugu, kde server zapomene config).
    pub http_base_url_override: Option<String>,
}

impl Default for ClientHandshakeConfig {
    fn default() -> Self {
        Self {
            cache_root: PathBuf::from("cache/resources"),
            http_base_url_override: None,
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeStatus {
    /// Nebyl ještě přijatý `ServerHello`.
    #[default]
    AwaitingHello,
    /// Server requires auth — waiting for the user to log in via native UI.
    /// Resource download starts only after auth succeeds.
    AwaitingAuth,
    /// Stahujeme soubory přes HTTP.
    Downloading,
    /// Files staženy a `ClientReady` odeslán.
    Ready,
    /// Něco selhalo (mismatch protocol version, HTTP error, ...).
    Failed,
}

/// Stav klientského handshake — sledujeme ho z ECS systémů, abychom
/// nezačali stahovat dvakrát a abychom věděli, kdy poslat `ClientReady`.
#[derive(Resource, Default)]
pub struct ClientHandshakeState {
    pub status: HandshakeStatus,
    pub manifests: Vec<ResourceDigest>,
    /// Stored while AwaitingAuth; used to kick off download after auth.
    pub pending_base: Option<String>,
    download_rx: Option<Receiver<Result<(), String>>>,
}

pub struct ClientHandshakePlugin;

impl Plugin for ClientHandshakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientHandshakeConfig>();
        app.init_resource::<ClientHandshakeState>();
        app.add_systems(
            Update,
            (receive_hello, check_auth_then_download, poll_download).chain(),
        );
    }
}

fn receive_hello(
    mut receivers: Query<&mut MessageReceiver<ServerHello>>,
    config: Res<ClientHandshakeConfig>,
    mut state: ResMut<ClientHandshakeState>,
    mut allowlist: ResMut<ServerResourceAllowlist>,
    bridges: Res<core_resources::GameBridges>,
) {
    if state.status != HandshakeStatus::AwaitingHello {
        for mut rx in receivers.iter_mut() {
            rx.receive().for_each(|_| {});
        }
        return;
    }

    for mut rx in receivers.iter_mut() {
        for hello in rx.receive() {
            if hello.protocol_version != PROTOCOL_VERSION {
                error!(
                    "[handshake/client] protocol mismatch: server v{} != client v{}",
                    hello.protocol_version, PROTOCOL_VERSION
                );
                state.status = HandshakeStatus::Failed;
                return;
            }

            bridges.auth.set_required(hello.auth_required);

            allowlist.ids = Some(
                hello.manifests.iter()
                    .map(|m| ResourceId::new(&m.id))
                    .collect()
            );
            allowlist.file_hashes = hello.manifests.iter()
                .map(|m| {
                    let id = ResourceId::new(&m.id);
                    let hashes = m.blob.iter()
                        .map(|f| (f.rel_path.clone(), f.blake3))
                        .collect::<std::collections::HashMap<_, _>>();
                    (id, hashes)
                })
                .collect();

            let base = config
                .http_base_url_override
                .clone()
                .unwrap_or_else(|| hello.http_base_url.clone());
            let manifests = hello.manifests.clone();

            info!(
                "[handshake/client] received ServerHello — {} manifest(s), http_base={}, auth_required={}",
                manifests.len(), base, hello.auth_required,
            );

            state.manifests = manifests;

            if hello.auth_required {
                // Pause here — native login UI shows; check_auth_then_download
                // will kick off the download once the user authenticates.
                state.pending_base = Some(base);
                state.status = HandshakeStatus::AwaitingAuth;
                info!("[handshake/client] auth required — waiting for login");
            } else {
                start_download(&mut state, base, config.cache_root.clone());
            }
            return;
        }
    }
}

/// Polls auth bridge; once authenticated, starts the resource download.
fn check_auth_then_download(
    mut state: ResMut<ClientHandshakeState>,
    config: Res<ClientHandshakeConfig>,
    bridges: Res<core_resources::GameBridges>,
) {
    if state.status != HandshakeStatus::AwaitingAuth {
        return;
    }
    if !bridges.auth.is_client_authenticated() {
        return;
    }
    let base = match state.pending_base.take() {
        Some(b) => b,
        None => {
            error!("[handshake/client] auth ok but pending_base was not set — download will use empty base URL");
            String::new()
        }
    };
    info!("[handshake/client] auth ok — starting resource download");
    start_download(&mut state, base, config.cache_root.clone());
}

fn start_download(
    state: &mut ClientHandshakeState,
    base: String,
    cache_root: std::path::PathBuf,
) {
    let manifests = state.manifests.clone();
    let (tx, rx_recv) = unbounded::<Result<(), String>>();
    let manifests_for_task = manifests.clone();
    IoTaskPool::get()
        .spawn(async move {
            let result = download_all_blocking(&base, &manifests_for_task, &cache_root)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        })
        .detach();
    state.status = HandshakeStatus::Downloading;
    state.download_rx = Some(rx_recv);
}

fn poll_download(
    mut state: ResMut<ClientHandshakeState>,
    mut senders: Query<&mut MessageSender<ClientReady>>,
    mut dirty_writer: MessageWriter<ResourcesDirty>,
) {
    if state.status != HandshakeStatus::Downloading {
        return;
    }

    let Some(rx) = state.download_rx.as_ref() else {
        return;
    };

    let result = match rx.try_recv() {
        Ok(r) => r,
        Err(crossbeam_channel::TryRecvError::Empty) => return,
        Err(crossbeam_channel::TryRecvError::Disconnected) => {
            error!("[handshake/client] download task channel disconnected");
            state.status = HandshakeStatus::Failed;
            state.download_rx = None;
            return;
        }
    };

    state.download_rx = None;

    match result {
        Ok(()) => {
            info!(
                "[handshake/client] download complete ({} manifest(s) staged) — \
                 sending ClientReady",
                state.manifests.len()
            );

            // Pošli ClientReady na všech sender komponentách klientského
            // entity. Reálně bude existovat jediná, ale Query nás
            // ochrání před edge-case více klient entit.
            let mut sent_any = false;
            for mut sender in senders.iter_mut() {
                sender.send::<HandshakeChannel>(ClientReady {
                    protocol_version: PROTOCOL_VERSION,
                });
                sent_any = true;
            }
            if !sent_any {
                warn!(
                    "[handshake/client] no MessageSender<ClientReady> found — \
                     ClientReady not sent. Server will eventually time out."
                );
                state.status = HandshakeStatus::Failed;
                return;
            }
            state.status = HandshakeStatus::Ready;
            // Vynutit reload i když žádné soubory nebylo třeba stáhnout
            // (cache hit) — watcher by jinak nevyslal ResourcesDirty.
            dirty_writer.write(ResourcesDirty);
        }
        Err(e) => {
            error!("[handshake/client] download failed: {}", e);
            state.status = HandshakeStatus::Failed;
        }
    }
}

fn download_all_blocking(
    base: &str,
    manifests: &[ResourceDigest],
    cache_root: &Path,
) -> Result<(), DownloadError> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| DownloadError::Build(e.to_string()))?;

    std::fs::create_dir_all(cache_root).map_err(|e| DownloadError::Io {
        path: cache_root.to_path_buf(),
        message: e.to_string(),
    })?;

    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let total: usize = manifests.iter().map(|m| m.blob.len()).sum();

    for manifest in manifests {
        for digest in &manifest.blob {
            // Klient ukládá soubory do `<cache_root>/<resource_id>/<rel_path>`,
            // což odpovídá rozložení `/resources/` na serveru. Pak může
            // ResourcesPlugin v "network mode" prostě ukázat na cache_root
            // a všechno se nascanuje.
            let rel_native = digest.rel_path.replace('/', std::path::MAIN_SEPARATOR_STR);
            let local = cache_root
                .join(manifest.id.replace('/', std::path::MAIN_SEPARATOR_STR))
                .join(&rel_native);

            // Cache hit: file exists with matching blake3 hash.
            if let Ok(bytes) = std::fs::read(&local) {
                if blake3::hash(&bytes).as_bytes() == &digest.blake3 {
                    skipped += 1;
                    continue;
                }
            }

            let url = format!("{}/resources/{}/{}", base, manifest.id, digest.rel_path);
            let resp = client
                .get(&url)
                .send()
                .map_err(|e| DownloadError::Http(format!("{url}: {e}")))?;
            if !resp.status().is_success() {
                return Err(DownloadError::Http(format!("{url} -> {}", resp.status())));
            }
            let bytes = resp
                .bytes()
                .map_err(|e| DownloadError::Http(format!("{url}: read body {e}")))?;

            // Sanity check — server mohl mít desync mezi digestem a soubory
            // (např. rozjetá hot-reload race). Když nesedí hash, fail-fast.
            let actual = blake3::hash(&bytes);
            if actual.as_bytes() != &digest.blake3 {
                return Err(DownloadError::HashMismatch {
                    url,
                    expected: digest.blake3,
                    actual: *actual.as_bytes(),
                });
            }

            if let Some(parent) = local.parent() {
                std::fs::create_dir_all(parent).map_err(|e| DownloadError::Io {
                    path: parent.to_path_buf(),
                    message: e.to_string(),
                })?;
            }
            std::fs::write(&local, &bytes).map_err(|e| DownloadError::Io {
                path: local.clone(),
                message: e.to_string(),
            })?;
            downloaded += 1;
        }
    }

    info!(
        "[handshake/client] download summary: total={total}, downloaded={downloaded}, cached={skipped}"
    );
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum DownloadError {
    #[error("failed to build HTTP client: {0}")]
    Build(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("io error at {path:?}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("hash mismatch at {url}: expected {expected:?}, got {actual:?}")]
    HashMismatch {
        url: String,
        expected: [u8; 32],
        actual: [u8; 32],
    },
}
