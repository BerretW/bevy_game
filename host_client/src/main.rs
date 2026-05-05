//! `host_client` — herní klient s oknem a renderováním.
//!
//! Drží `WinitPlugin` + `RenderPlugin` (přes `DefaultPlugins`).
//! Veškerá herní logika a UI bude jednou žít v Lua resources;
//! tento crate je jen "host shell" — okno, asset server, network klient.

use bevy::prelude::*;
use core_shared::SharedPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_game — host_client".into(),
                resolution: (1280.0, 720.0).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((SharedPlugin, ClientCorePlugin))
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
