//! `host_client` — herní klient s oknem a renderováním.
//!
//! Drží `WinitPlugin` + `RenderPlugin` (přes `DefaultPlugins`).
//! Veškerá herní logika a UI bude jednou žít v Lua resources;
//! tento crate je jen "host shell" — okno, asset server, network klient.
//!
//! Konfigurace přichází z `client.toml` v platform-specific config dir
//! (`%APPDATA%\bevy_game\client.toml` na Windows). Detaily v [`config`].

mod auth_ui;
mod config;
mod console;
mod drawable;
mod gameplay;
mod gui_render;
mod lobby;
mod map_loader;
mod native_assets;
mod physics;

use std::backtrace::Backtrace;
use std::fs::OpenOptions;
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};

use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::asset::{AssetPlugin, UnapprovedPathMode};
use bevy::render::settings::{Backends, PowerPreference, RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::window::{
    MonitorSelection, PresentMode, PrimaryWindow, VideoModeSelection, WindowMode,
    WindowResolution,
};
use bevy_framepace::{FramepacePlugin, FramepaceSettings, Limiter};

use core_net::{
    ClientAuthPlugin, ClientHandshakeConfig, ClientHandshakePlugin, ClientHandshakeState,
    ClientLuaRpcPlugin, ClientNetConfig, ClientNetPlugin, HandshakeStatus, FIXED_TIMESTEP_HZ,
};
use core_resources::{ResourcesPlugin, Side};
use core_shared::SharedPlugin;

use crate::config::{
    ClientConfig, ClientConfigResource, GraphicsBackend, GpuPriority, PresentModeConfig,
    WindowModeConfig,
};

/// Stav aplikace — lobby je výchozí, InGame přechází po stisknutí Connect.
#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub(crate) enum AppState {
    #[default]
    Lobby,
    InGame,
}

const LUA_SAFE_CLIENT_ID_MAX: u64 = i64::MAX as u64;

#[derive(Resource, Debug, Clone)]
struct ClientRuntimePaths {
    config_path: PathBuf,
    cache_root: PathBuf,
    log_dir: PathBuf,
}

fn main() {
    // 1. Resolve config path: CLI arg #1 → BEVY_GAME_CLIENT_CONFIG → AppData default.
    let config_path = match ClientConfig::resolve_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[host_client] FATAL: {e}");
            std::process::exit(1);
        }
    };

    // 2. Load (nebo vygeneruj výchozí) config.
    let cfg = ClientConfig::load_or_create(&config_path);
    let log_dir = resolve_log_dir(&cfg, &config_path);
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "[host_client] failed to create log dir {}: {}",
            log_dir.display(),
            e
        );
    }
    install_client_panic_hook(log_dir.clone());
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    // 3. Cache dir — kam handshake stahuje soubory. Watcher na něm pak
    //    spustí hot-reload, jakmile se objeví obsah.
    let cache_root = cfg.paths.resolve_cache_dir();
    if let Err(e) = std::fs::create_dir_all(&cache_root) {
        eprintln!(
            "[host_client] failed to create cache dir {}: {}",
            cache_root.display(),
            e
        );
    }

    // 4. Network — server addr / local bind / client_id z configu.
    let server_addr = cfg.network.default_server.parse().unwrap_or_else(|e| {
        panic!(
            "[host_client] invalid network.default_server {:?}: {}",
            cfg.network.default_server, e
        )
    });
    let local_addr = cfg.network.local_bind.parse().unwrap_or_else(|e| {
        panic!(
            "[host_client] invalid network.local_bind {:?}: {}",
            cfg.network.local_bind, e
        )
    });

    let raw_client_id = if cfg.player.saved_client_id == 0 {
        // 0 = generuj náhodný ID. Phase 4 by tu měl být persistent token.
        nanorand_u64()
    } else {
        cfg.player.saved_client_id
    };
    let client_id = clamp_client_id(raw_client_id);

    let client_net_config = ClientNetConfig {
        server: server_addr,
        local: local_addr,
        client_id,
        tick_duration: std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        protocol_id: core_net::NETCODE_PROTOCOL_ID,
        private_key: [0u8; 32],
    };

    let handshake_config = ClientHandshakeConfig {
        cache_root: cache_root.clone(),
        http_base_url_override: None,
    };

    // 5. Banner — admin/dev hned vidí, na co se klient připojuje.
    eprintln!(
        "[host_client] {} ({}x{}, backend={:?}, server={}, cache={})",
        cfg.player.name,
        cfg.graphics.resolution_width,
        cfg.graphics.resolution_height,
        cfg.graphics.backend,
        server_addr,
        cache_root.display(),
    );

    // 6. Postav Bevy app — RenderPlugin a WindowPlugin se musí konfigurovat
    //    při buildu, ne jako ECS resource. Ostatní (audio / input / quality)
    //    běží jako Bevy resources, které gameplay systémy konzumují.
    let wgpu_settings = WgpuSettings {
        backends: Some(backends_from_config(cfg.graphics.backend)),
        power_preference: power_preference(cfg.graphics.gpu_priority),
        ..default()
    };

    let primary_window = Window {
        title: cfg.graphics.window_title.clone(),
        resolution: WindowResolution::new(
            cfg.graphics.resolution_width,
            cfg.graphics.resolution_height,
        ),
        mode: window_mode(cfg.graphics.window_mode, cfg.graphics.monitor_index),
        present_mode: present_mode(cfg.graphics.present_mode),
        ..default()
    };

    // Frame pacing: fps_cap=0 → auto (monitor refresh), >0 → hard cap.
    let framepace_limiter = if cfg.graphics.fps_cap > 0 {
        Limiter::from_framerate(cfg.graphics.fps_cap as f64)
    } else {
        Limiter::Auto
    };

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.1, 0.3, 0.1))) // Tmavě zelená
        .insert_resource(client_net_config)
        .insert_resource(handshake_config)
        .insert_resource(gameplay::LocalClientId(client_id))
        .insert_resource(ClientConfigResource(cfg.clone()))
        .insert_resource(ClientRuntimePaths {
            config_path,
            cache_root: cache_root.clone(),
            log_dir,
        })
        .add_plugins(
            DefaultPlugins
                .set(LogPlugin {
                    level: cfg.advanced.log_level.to_bevy(),
                    filter: cfg.advanced.log_filter.clone(),
                    custom_layer: console::tracing_layer,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(primary_window),
                    ..default()
                })
                .set(AssetPlugin {
                    // Resource stream modely se nacitaji z absolutnich cest v klientskem cache dir.
                    unapproved_path_mode: UnapprovedPathMode::Allow,
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(wgpu_settings),
                    ..default()
                }),
        )
        .init_state::<AppState>()
        .add_plugins((
            // Lobby — zobrazí se před připojením.
            lobby::LobbyPlugin,
            // Phase 3 — klientský renderer replikovaných hráčů + WASD vstup.
            gameplay::ClientGameplayPlugin,
            console::ConsolePlugin,
            SharedPlugin,
            ResourcesPlugin::new(cache_root.clone(), Side::Client),
            ClientNetPlugin,
            ClientHandshakePlugin,
            // Phase 4 — username/password auth: receives AuthChallenge/AuthResult,
            // emits local events for the core/auth Lua resource, sends credentials.
            ClientAuthPlugin,
            ClientLuaRpcPlugin,
            // Phase 4 — native login/register panel (shown before resource download).
            auth_ui::AuthUiPlugin,
            // Phase 4 — GUI overlay (immediate-mode Lua drawing API).
            gui_render::GuiRenderPlugin,
            // Native assets (fonts, models) from `assets/` directory.
            native_assets::NativeAssetsPlugin,
            // Drawable system: .drawable manifests, material swap, vertex color sanitization.
            drawable::DrawablePlugin,
            ClientCorePlugin,
            FramepacePlugin,
        ))
        .add_plugins(map_loader::ClientMapPlugin)
        .add_plugins(physics::ClientPhysicsPlugin)
        .insert_resource(FramepaceSettings { limiter: framepace_limiter })
        .run();
}

/// Client-specifická logika. Phase 2 = log online; Phase 4 přidá GUI framework, audio mixer atd.
pub struct ClientCorePlugin;

impl Plugin for ClientCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, on_client_start).add_systems(
            Update,
            (
                log_app_state_transitions,
                log_handshake_status_transitions,
                log_primary_window_state,
            ),
        );
    }
}

fn on_client_start(cfg: Res<ClientConfigResource>, paths: Res<ClientRuntimePaths>) {
    info!(
        "[host_client] online — render + winit + asset server ready (player=\"{}\", lang={}, backend={:?}, gpu_validation={}, config={}, cache={}, logs={})",
        cfg.0.player.name,
        cfg.0.ui.language,
        cfg.0.graphics.backend,
        cfg.0.advanced.gpu_validation,
        paths.config_path.display(),
        paths.cache_root.display(),
        paths.log_dir.display(),
    );
}

fn log_app_state_transitions(
    state: Res<State<AppState>>,
    mut last: Local<Option<AppState>>,
) {
    if last.as_ref() == Some(state.get()) {
        return;
    }

    info!("[host_client] app state -> {:?}", state.get());
    *last = Some(state.get().clone());
}

fn log_handshake_status_transitions(
    handshake: Res<ClientHandshakeState>,
    mut last: Local<Option<HandshakeStatus>>,
) {
    let pending_base = handshake.pending_base.as_deref().unwrap_or("<none>");
    let manifests = handshake.manifests.len();
    let current = handshake.status;

    if last.as_ref() == Some(&current) {
        return;
    }

    match current {
        HandshakeStatus::AwaitingHello => info!(
            "[host_client] handshake -> AwaitingHello (waiting for ServerHello)"
        ),
        HandshakeStatus::AwaitingAuth => info!(
            "[host_client] handshake -> AwaitingAuth (manifests={}, pending_base={})",
            manifests,
            pending_base,
        ),
        HandshakeStatus::Downloading => info!(
            "[host_client] handshake -> Downloading (manifests={}, cache warmup in progress)",
            manifests,
        ),
        HandshakeStatus::Ready => info!(
            "[host_client] handshake -> Ready (manifests={}, resources should hot-reload next)",
            manifests,
        ),
        HandshakeStatus::Failed => error!(
            "[host_client] handshake -> Failed (manifests={}, pending_base={})",
            manifests,
            pending_base,
        ),
    }

    *last = Some(current);
}

fn log_primary_window_state(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut logged: Local<bool>,
) {
    if *logged {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };

    info!(
        "[host_client] primary window ready (title=\"{}\", size={}x{}, present_mode={:?}, mode={:?})",
        window.title,
        window.resolution.physical_width(),
        window.resolution.physical_height(),
        window.present_mode,
        window.mode,
    );
    *logged = true;
}

fn resolve_log_dir(cfg: &ClientConfig, config_path: &Path) -> PathBuf {
    if let Some(path) = &cfg.paths.log_dir {
        return path.clone();
    }

    config_path
        .parent()
        .map(|dir| dir.join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

fn install_client_panic_hook(log_dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log_panic_details(&log_dir, info);
        previous(info);
    }));
}

fn log_panic_details(log_dir: &Path, info: &PanicHookInfo<'_>) {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let location = info
        .location()
        .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
        .unwrap_or_else(|| "<unknown>".to_string());
    let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    };
    let backtrace = Backtrace::force_capture();
    let report = format!(
        "[host_client] PANIC thread='{thread_name}' location={location}\nmessage: {payload}\nbacktrace:\n{backtrace}\n"
    );

    console::ring_push(report.trim_end().to_string());
    eprintln!("{report}");

    let path = log_dir.join("latest_panic.log");
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            let _ = writeln!(file, "{report}");
        }
        Err(err) => {
            eprintln!(
                "[host_client] failed to append panic log {}: {}",
                path.display(),
                err
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Config → Bevy mapping helpers
// ---------------------------------------------------------------------------

fn backends_from_config(b: GraphicsBackend) -> Backends {
    match b {
        GraphicsBackend::Auto => Backends::all(),
        GraphicsBackend::Vulkan => Backends::VULKAN,
        GraphicsBackend::Dx12 => Backends::DX12,
        GraphicsBackend::Metal => Backends::METAL,
        GraphicsBackend::Gl => Backends::GL,
        GraphicsBackend::BrowserWebgpu => Backends::BROWSER_WEBGPU,
    }
}

fn power_preference(p: GpuPriority) -> PowerPreference {
    match p {
        GpuPriority::HighPerformance => PowerPreference::HighPerformance,
        GpuPriority::LowPower => PowerPreference::LowPower,
    }
}

fn window_mode(mode: WindowModeConfig, monitor_index: u32) -> WindowMode {
    let monitor = if monitor_index == 0 {
        MonitorSelection::Primary
    } else {
        MonitorSelection::Index(monitor_index as usize)
    };
    match mode {
        WindowModeConfig::Windowed => WindowMode::Windowed,
        WindowModeConfig::BorderlessFullscreen => WindowMode::BorderlessFullscreen(monitor),
        WindowModeConfig::Fullscreen => {
            WindowMode::Fullscreen(monitor, VideoModeSelection::Current)
        }
    }
}

fn present_mode(p: PresentModeConfig) -> PresentMode {
    match p {
        PresentModeConfig::Auto => PresentMode::AutoVsync,
        PresentModeConfig::Fifo => PresentMode::Fifo,
        PresentModeConfig::Mailbox => PresentMode::Mailbox,
        PresentModeConfig::Immediate => PresentMode::Immediate,
    }
}

/// Triviální `u64` rng — generujeme client_id z monotonic clock + PID,
/// protože netcode klient potřebuje unikátní ID, ale Phase 2 ještě nemá
/// auth backend. Phase 4 bude perzistovat ID v `[player].saved_client_id`.
fn nanorand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let pid = std::process::id() as u64;
    let raw = (now.as_nanos() as u64).wrapping_mul(1_000_003).wrapping_add(pid);
    clamp_client_id(raw)
}

fn clamp_client_id(id: u64) -> u64 {
    if id <= LUA_SAFE_CLIENT_ID_MAX {
        return id;
    }
    // Drzime client_id v i64 rozsahu kvuli robustnimu prenosu do Lua.
    let clamped = id & LUA_SAFE_CLIENT_ID_MAX;
    if clamped == 0 {
        1
    } else {
        clamped
    }
}
