//! `host_client` — herní klient s oknem a renderováním.
//!
//! Drží `WinitPlugin` + `RenderPlugin` (přes `DefaultPlugins`).
//! Veškerá herní logika a UI bude jednou žít v Lua resources;
//! tento crate je jen "host shell" — okno, asset server, network klient.
//!
//! Konfigurace přichází z `client.toml` v platform-specific config dir
//! (`%APPDATA%\bevy_game\client.toml` na Windows). Detaily v [`config`].

mod config;
mod console;
mod gameplay;

use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::render::settings::{Backends, PowerPreference, RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::window::{
    MonitorSelection, PresentMode, VideoModeSelection, WindowMode, WindowResolution,
};
use bevy_framepace::{FramepacePlugin, FramepaceSettings, Limiter};

use core_net::{
    ClientHandshakeConfig, ClientHandshakePlugin, ClientLuaRpcPlugin, ClientNetConfig,
    ClientNetPlugin, FIXED_TIMESTEP_HZ,
};
use core_resources::{ResourcesPlugin, Side};
use core_shared::SharedPlugin;

use crate::config::{
    ClientConfig, ClientConfigResource, GraphicsBackend, GpuPriority, PresentModeConfig,
    WindowModeConfig,
};

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

    let client_id = if cfg.player.saved_client_id == 0 {
        // 0 = generuj náhodný ID. Phase 4 by tu měl být persistent token.
        nanorand_u64()
    } else {
        cfg.player.saved_client_id
    };

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
        .insert_resource(ClientConfigResource(cfg.clone()))
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
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(wgpu_settings),
                    ..default()
                }),
        )
        .add_plugins((
            // Phase 3 — klientský renderer replikovaných hráčů + WASD vstup.
            gameplay::ClientGameplayPlugin,
            console::ConsolePlugin,
            SharedPlugin,
            ResourcesPlugin::new(cache_root, Side::Client),
            ClientNetPlugin,
            ClientHandshakePlugin,
            ClientLuaRpcPlugin,
            ClientCorePlugin,
            FramepacePlugin,
        ))
        .insert_resource(FramepaceSettings { limiter: framepace_limiter })
        .run();
}

/// Client-specifická logika. Phase 2 = log online; Phase 4 přidá NUI
/// (Dioxus / WebView), audio mixer napojený na `audio.*`, atd.
pub struct ClientCorePlugin;

impl Plugin for ClientCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, on_client_start);
    }
}

fn on_client_start(cfg: Res<ClientConfigResource>) {
    info!(
        "[host_client] online — render + winit + asset server ready (player=\"{}\", lang={})",
        cfg.0.player.name, cfg.0.ui.language
    );
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
    (now.as_nanos() as u64).wrapping_mul(1_000_003).wrapping_add(pid)
}
