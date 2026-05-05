//! `host_client` — herní klient s oknem a renderováním.
//!
//! Drží `WinitPlugin` + `RenderPlugin` (přes `DefaultPlugins`).
//! Veškerá herní logika a UI bude jednou žít v Lua resources;
//! tento crate je jen "host shell" — okno, asset server, network klient.

use bevy::prelude::*;
use bevy::window::WindowResolution;
use core_net::{ClientHandshakePlugin, ClientLuaRpcPlugin, ClientNetPlugin};
use core_resources::{ResourcesPlugin, Side};
use core_shared::SharedPlugin;

/// V Phase 2 klient nečte přímo `/resources/` — přepneme na lokální cache,
/// kterou plní handshake downloader. Resources se objeví až po úspěšném
/// dokončení `ServerHello → download → ClientReady` sekvence.
const CACHE_ROOT: &str = "cache/resources";

fn main() {
    // Cache adresář musí existovat předtím, než ho `notify` watcher začne
    // sledovat — jinak by se watch pokus odmítl. Nezáleží, jestli je prázdný:
    // hot-reload pak zachytí soubory zapsané downloaderem.
    if let Err(e) = std::fs::create_dir_all(CACHE_ROOT) {
        eprintln!(
            "[host_client] failed to create cache root {:?}: {}",
            CACHE_ROOT, e
        );
    }

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_game — host_client".into(),
                resolution: WindowResolution::new(1280, 720),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            SharedPlugin,
            // V Phase 2 klient resources stahuje od serveru přes HTTP
            // a ukládá je do `cache/resources/`. ResourcesPlugin watch
            // detekuje zápisy a hot-reload spustí sandboxy.
            ResourcesPlugin::new(CACHE_ROOT, Side::Client),
            // Lightyear client (UDP netcode) — ProtocolPlugin se přidá
            // dovnitř, a tak je registrace messages/channelů identická
            // s `ServerNetPlugin`.
            ClientNetPlugin,
            // Handshake state machine — přijme ServerHello, spustí
            // download na IoTaskPool, po úspěchu pošle ClientReady.
            ClientHandshakePlugin,
            // Lua RPC bridge — protějšek ServerLuaRpcPlugin.
            ClientLuaRpcPlugin,
            ClientCorePlugin,
        ))
        .run();
}

/// Client-specifická logika. Phase 2 sem přidá `lightyear::ClientPlugin`
/// + handshake / asset download, Phase 4 NUI (Dioxus / WebView).
pub struct ClientCorePlugin;

impl Plugin for ClientCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, on_client_start);
    }
}

fn on_client_start() {
    info!("[host_client] online — render + winit + asset server ready");
}
