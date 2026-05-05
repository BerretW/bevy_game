//! HTTP file server (Axum) pro Phase 2 resource file sync.
//!
//! Klient v handshake fázi dostane od serveru `ServerHello` digest přes
//! `lightyear`, pak chybějící soubory stáhne přímo přes HTTP. HTTP server
//! je intencionálně oddělený od game-trafficku — drobné soubory si můžeme
//! pohodlně cachovat reverse-proxy / CDN, aniž by to zatěžovalo herní
//! socket.
//!
//! API:
//! * `GET /health` → `200 ok`
//! * `GET /resources/<id>/<rel_path>` → file bytes
//!
//! Path traversal: `tokio::fs::canonicalize` po join + ověření, že výsledek
//! zůstal pod `vfs_root`. Symlinky tím pádem nemohou utéct ven.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::{
    extract::{Path as AxumPath, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bevy::prelude::*;

use crate::TokioRuntime;

const DEFAULT_HTTP_PORT: u16 = 8081;

/// Bevy resource — vkládá se z `main.rs` před přidáním pluginu.
#[derive(Resource, Clone)]
pub struct HttpServerConfig {
    pub bind: SocketAddr,
    pub vfs_root: PathBuf,
}

impl HttpServerConfig {
    pub fn new(vfs_root: impl Into<PathBuf>) -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], DEFAULT_HTTP_PORT)),
            vfs_root: vfs_root.into(),
        }
    }
}

#[derive(Clone)]
struct AppState {
    /// Kanonizovaný `vfs_root` — všechna requestovaná cesta se musí po
    /// canonicalize stále nacházet pod ním, jinak 403.
    vfs_root: PathBuf,
}

pub struct HttpFileServerPlugin;

impl Plugin for HttpFileServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_http_server);
    }
}

fn spawn_http_server(runtime: Res<TokioRuntime>, config: Res<HttpServerConfig>) {
    let canonical_root = match std::fs::canonicalize(&config.vfs_root) {
        Ok(p) => p,
        Err(e) => {
            error!(
                "[host_server::http] failed to canonicalize vfs_root {:?}: {}",
                config.vfs_root, e
            );
            return;
        }
    };

    let state = AppState {
        vfs_root: canonical_root,
    };

    let router = Router::new()
        .route("/health", get(health))
        .route("/resources/*path", get(serve_resource_file))
        .with_state(state);

    let bind = config.bind;
    let handle = runtime.handle();
    handle.spawn(async move {
        let listener = match tokio::net::TcpListener::bind(bind).await {
            Ok(l) => l,
            Err(e) => {
                bevy::log::error!("[host_server::http] failed to bind {}: {}", bind, e);
                return;
            }
        };
        bevy::log::info!(
            "[host_server::http] HTTP file server listening on http://{}",
            bind
        );
        if let Err(e) = axum::serve(listener, router).await {
            bevy::log::error!("[host_server::http] axum serve error: {}", e);
        }
    });
}

async fn health() -> &'static str {
    "ok"
}

async fn serve_resource_file(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    // Klient posílá cesty s forward-slashy (z digestu). Na Windows musíme
    // nahradit za platformní separátor, jinak `join` vyrobí jednu dlouhou
    // komponentu.
    let native = path.replace('/', std::path::MAIN_SEPARATOR_STR);
    let joined = state.vfs_root.join(&native);

    let canonical = match tokio::fs::canonicalize(&joined).await {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };

    if !canonical.starts_with(&state.vfs_root) {
        warn!(
            "[host_server::http] path traversal blocked: {:?} -> {:?}",
            joined, canonical
        );
        return (StatusCode::FORBIDDEN, "outside vfs root").into_response();
    }

    let bytes = match tokio::fs::read(&canonical).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };

    let content_type = guess_content_type(&canonical);
    ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
}

fn guess_content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|s| s.to_str()) {
        Some("lua") | Some("wgsl") | Some("txt") | Some("toml") => {
            "text/plain; charset=utf-8"
        }
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("gltf") => "model/gltf+json",
        Some("glb") => "model/gltf-binary",
        _ => "application/octet-stream",
    }
}
