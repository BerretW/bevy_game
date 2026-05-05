//! `host_server` — dedicated headless server.
//!
//! Žádný window, render ani winit. ECS běží na fixním tickrate
//! přes `ScheduleRunnerPlugin`. Vedle ECS držíme samostatný
//! Tokio runtime pro asynchronní operace (sqlx queries, VFS scan,
//! resource HTTP serving), abychom **nikdy neblokovali ECS loop**.

mod http_server;

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;

use core_net::{DigestPlugin, ServerHandshakePlugin, ServerLuaRpcPlugin, ServerNetPlugin};
use core_resources::{ResourcesPlugin, Side};
use core_shared::SharedPlugin;

use crate::http_server::{HttpFileServerPlugin, HttpServerConfig};

const SERVER_TICK_HZ: f64 = 60.0;
const RESOURCES_ROOT: &str = "resources";

fn main() {
    // Tokio runtime žije celou dobu života procesu jako Bevy Resource.
    // ECS systémy ho používají přes `runtime.spawn(...)` (fire-and-forget)
    // nebo přes `bevy::tasks::IoTaskPool`, který umí přemostit budoucnost
    // do Bevy `AsyncComputeTaskPool` výsledků.
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("host-server-tokio")
        .build()
        .expect("failed to build Tokio runtime for host_server");

    App::new()
        .insert_resource(TokioRuntime(tokio_runtime))
        .insert_resource(HttpServerConfig::new(RESOURCES_ROOT))
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / SERVER_TICK_HZ,
            ))),
            LogPlugin {
                level: Level::INFO,
                filter: "wgpu=warn,naga=warn".into(),
                ..default()
            },
            SharedPlugin,
            ResourcesPlugin::new(RESOURCES_ROOT, Side::Server),
            // Phase 2 — server část: digest cache (předpočítané hashe pro
            // `ServerHello`) a HTTP file server (klientský download endpoint).
            DigestPlugin,
            HttpFileServerPlugin,
            // Lightyear server (UDP netcode) + sdílená protocol registrace.
            // Pořadí matters: ServerNetPlugin dovnitř přidá ServerPlugins
            // a ProtocolPlugin v jednom kroku.
            ServerNetPlugin,
            // Handshake — observer na Add<Connected>, posílá ServerHello
            // s digestem manifestů. Konzumuje ResourceDigestCache.
            ServerHandshakePlugin,
            // Lua RPC bridge — drainuje outgoing TriggerClientEvent z
            // serverových sandboxů a posílá je klientům; routuje příchozí
            // TriggerServerEvent k handlerům RegisterEvent v sandboxech.
            ServerLuaRpcPlugin,
            ServerCorePlugin,
        ))
        .run();
}

/// Wrapper kolem Tokio runtime, aby šel uložit jako Bevy `Resource`.
///
/// Runtime je `!Sync` na okrajích, ale `tokio::runtime::Runtime` samotný
/// `Send + Sync` je — můžeme ho z systémů sahat přes `Res<TokioRuntime>`.
#[derive(Resource)]
pub struct TokioRuntime(pub tokio::runtime::Runtime);

impl TokioRuntime {
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.0.handle().clone()
    }
}

/// Server-specifická logika. Phase 2 sem přidá `lightyear::ServerPlugin`,
/// Phase 3 universal ECS API, Phase 4 sqlx pool jako další Resource.
pub struct ServerCorePlugin;

impl Plugin for ServerCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, on_server_start);
    }
}

fn on_server_start(_runtime: Res<TokioRuntime>) {
    info!(
        "[host_server] online — headless mode, tick {} Hz, tokio runtime ready",
        SERVER_TICK_HZ
    );
}
