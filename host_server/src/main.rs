//! `host_server` — dedicated headless server.
//!
//! Žádný window, render ani winit. ECS běží na fixním tickrate
//! přes `ScheduleRunnerPlugin`. Vedle ECS držíme samostatný
//! Tokio runtime pro asynchronní operace (sqlx queries, VFS scan,
//! resource HTTP serving), abychom **nikdy neblokovali ECS loop**.
//!
//! Konfigurace přichází z [`server.toml`](../../server.toml) v CWD;
//! detaily v [`config`].

mod config;
mod http_server;
mod log_filter;

use std::path::PathBuf;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;

use core_net::{
    DigestPlugin, ServerHandshakeConfig, ServerHandshakePlugin, ServerLuaRpcPlugin,
    ServerNetConfig, ServerNetPlugin,
};
use core_resources::{ResourcesPlugin, Side};
use core_shared::SharedPlugin;

use crate::config::{
    resolve_default_config_path, resolve_path_relative_to_exe, ConfigError, ServerConfig,
};
use crate::http_server::{HttpFileServerPlugin, HttpServerConfig};

fn main() {
    // 1. Konfigurační soubor — defaultně `<exe_dir>/server.toml` (s fallbackem
    //    na CWD pro `cargo run`). Override: první positional CLI argument.
    let cfg_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(resolve_default_config_path);
    let cfg = ServerConfig::load_or_default(&cfg_path);

    // 2. Validace network/key vstupů — když je tu chyba, raději spadneme
    //    s pochopitelnou hláškou než až runtime panic v lightyearu.
    let udp_bind = cfg
        .net
        .udp_bind_addr()
        .unwrap_or_else(|e| panic!("[host_server] {}", e));
    let http_bind = cfg
        .net
        .http_bind_addr()
        .unwrap_or_else(|e| panic!("[host_server] {}", e));
    let private_key = cfg
        .load_private_key()
        .unwrap_or_else(|e: ConfigError| panic!("[host_server] {}", e))
        .unwrap_or([0u8; 32]);

    // Resources root — když je v configu relativní cesta (např. `"resources"`),
    // přednostně hledáme vedle `.exe`, jinak fallback na CWD. Distribuce s
    // rozložením `host_server.exe + server.toml + resources/` tak funguje
    // out-of-the-box bez ohledu na to, odkud admin server spouští.
    let resources_root = resolve_path_relative_to_exe(&cfg.resources.root);

    // Banner — ať admin v logu hned vidí, co serveru běží.
    eprintln!(
        "[host_server] {} — \"{}\" (max_players={}, udp={}, http={}, gamemode={}, resources={})",
        cfg.server.name,
        cfg.server.description.lines().next().unwrap_or(""),
        cfg.gameplay.max_players,
        udp_bind,
        http_bind,
        cfg.gameplay.gamemode.as_deref().unwrap_or("<none>"),
        resources_root.display(),
    );

    // 3. Tokio runtime žije celou dobu života procesu jako Bevy Resource.
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("host-server-tokio")
        .build()
        .expect("failed to build Tokio runtime for host_server");

    let tick_duration = cfg.net.tick_duration();

    let server_net_config = ServerNetConfig {
        bind: udp_bind,
        tick_duration,
        protocol_id: cfg.net.protocol_id,
        private_key,
        client_timeout_sec: cfg.net.connection_timeout_sec,
    };

    let http_config = HttpServerConfig {
        bind: http_bind,
        vfs_root: resources_root.clone(),
    };

    let handshake_config = ServerHandshakeConfig {
        http_base_url: cfg.net.http_public_url.clone(),
    };

    App::new()
        .insert_resource(TokioRuntime(tokio_runtime))
        .insert_resource(http_config)
        .insert_resource(server_net_config)
        .insert_resource(handshake_config)
        .insert_resource(ServerInfoResource(cfg.server.clone()))
        .insert_resource(GameplayResource(cfg.gameplay.clone()))
        .insert_resource(DevResource(cfg.dev.clone()))
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(tick_duration)),
            LogPlugin {
                level: cfg.logging.level.to_bevy(),
                filter: cfg.logging.filter.clone(),
                // Windows-specific: tlumí WSAECONNRESET spam po klientském
                // odpojení (viz log_filter.rs). Na ostatních OS no-op.
                custom_layer: |_app| Some(Box::new(log_filter::LightyearUdpNoiseFilter)),
                ..default()
            },
            SharedPlugin,
            ResourcesPlugin::new(resources_root.clone(), Side::Server)
                .with_watch(cfg.resources.hot_reload),
            // Phase 2 — server část: digest cache (předpočítané hashe pro
            // `ServerHello`) a HTTP file server (klientský download endpoint).
            DigestPlugin,
            HttpFileServerPlugin,
            // Lightyear server (UDP netcode) + sdílená protocol registrace.
            ServerNetPlugin,
            // Handshake — observer na Add<Connected>, posílá ServerHello
            // s digestem manifestů. Konzumuje ResourceDigestCache.
            ServerHandshakePlugin,
            // Lua RPC bridge — drainuje outgoing TriggerClientEvent ze
            // serverových sandboxů a posílá je klientům; routuje příchozí
            // TriggerServerEvent k handlerům RegisterEvent v sandboxech.
            ServerLuaRpcPlugin,
            ServerCorePlugin,
        ))
        .run();
}

/// Wrapper kolem Tokio runtime, aby šel uložit jako Bevy `Resource`.
#[derive(Resource)]
pub struct TokioRuntime(pub tokio::runtime::Runtime);

impl TokioRuntime {
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.0.handle().clone()
    }
}

/// Read-only resource exposing parsed `[server]` sekci configu — gameplay
/// systémy si můžou přečíst název, MOTD atd. pro broadcast / UI.
#[derive(Resource, Clone, Debug)]
pub struct ServerInfoResource(pub config::ServerInfo);

/// Read-only resource exposing parsed `[gameplay]` sekci.
#[derive(Resource, Clone, Debug)]
pub struct GameplayResource(pub config::GameplayConfig);

/// Read-only resource exposing parsed `[dev]` sekci.
#[derive(Resource, Clone, Debug)]
pub struct DevResource(pub config::DevConfig);

/// Server-specifická logika. Phase 3 přinese ECS bridge, Phase 4 sqlx pool.
pub struct ServerCorePlugin;

impl Plugin for ServerCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, on_server_start);
    }
}

fn on_server_start(
    _runtime: Res<TokioRuntime>,
    info: Res<ServerInfoResource>,
    gameplay: Res<GameplayResource>,
    net: Res<ServerNetConfig>,
) {
    let tick_hz = if net.tick_duration.as_secs_f64() > 0.0 {
        1.0 / net.tick_duration.as_secs_f64()
    } else {
        0.0
    };
    info!(
        "[host_server] online — \"{}\", tick {:.0} Hz, slots {} (queue {})",
        info.0.name, tick_hz, gameplay.0.max_players, gameplay.0.queue_max
    );
    if !info.0.tags.is_empty() {
        info!("[host_server] tags: {:?}", info.0.tags);
    }
}
