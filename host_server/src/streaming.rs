use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::Deserialize;

use core_net::{ClientHandshakeComplete, TileStreamingChannel, TileStreamingCommand};
use core_shared::{NetTransform, PlayerMarker};

const TILE_STREAM_SEND_INTERVAL_SECS: f32 = 1.0;

pub struct ServerTileStreamingPlugin;

impl Plugin for ServerTileStreamingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerTileIndex>()
            .init_resource::<LastAuthoritativeTileSets>()
            .add_systems(Startup, load_server_tile_index)
            .add_systems(Update, send_tile_streaming_updates);
    }
}

#[derive(Resource, Debug, Default)]
struct ServerTileIndex {
    tiles: Vec<ServerTileDef>,
    active: bool,
}

#[derive(Resource, Debug, Default)]
struct LastAuthoritativeTileSets {
    per_client: HashMap<u64, HashSet<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorldTileIndexManifest {
    #[serde(default)]
    tiles: Vec<ServerTileDef>,
}

#[derive(Debug, Clone, Deserialize)]
struct ServerTileDef {
    #[serde(default)]
    id: String,
    map: String,
    center: [f32; 3],
    load_radius: f32,
    #[serde(default)]
    always_loaded: bool,
}

impl ServerTileDef {
    fn runtime_id(&self) -> String {
        if self.id.trim().is_empty() {
            self.map.clone()
        } else {
            self.id.clone()
        }
    }
}

fn load_server_tile_index(mut tile_index: ResMut<ServerTileIndex>) {
    for path in tile_index_candidates() {
        if !path.exists() {
            continue;
        }

        let Ok(text) = fs::read_to_string(&path) else {
            warn!("[streaming/server] failed reading {:?}", path);
            continue;
        };

        let Ok(parsed) = toml::from_str::<WorldTileIndexManifest>(&text) else {
            warn!("[streaming/server] invalid tile index {:?}", path);
            continue;
        };

        if parsed.tiles.is_empty() {
            continue;
        }

        tile_index.tiles = parsed.tiles;
        tile_index.active = true;
        info!(
            "[streaming/server] loaded authoritative tile index {:?} ({} tile(s))",
            path,
            tile_index.tiles.len()
        );
        return;
    }

    info!("[streaming/server] no authoritative tile index found; server tile authority disabled");
}

fn send_tile_streaming_updates(
    time: Res<Time>,
    tile_index: Res<ServerTileIndex>,
    players: Query<(&PlayerMarker, &NetTransform)>,
    mut senders: Query<(
        &RemoteId,
        &mut MessageSender<TileStreamingCommand>,
        Option<&ClientHandshakeComplete>,
    )>,
    mut last_sets: ResMut<LastAuthoritativeTileSets>,
    mut ticker: Local<f32>,
) {
    if !tile_index.active {
        return;
    }

    *ticker += time.delta_secs();
    if *ticker < TILE_STREAM_SEND_INTERVAL_SECS {
        return;
    }
    *ticker = 0.0;

    for (remote_id, mut sender, handshake_complete) in &mut senders {
        if handshake_complete.is_none() {
            continue;
        }

        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => continue,
        };

        let Some((_, player_tf)) = players.iter().find(|(marker, _)| marker.client_id == client_id) else {
            continue;
        };

        let wanted = wanted_tiles_for_position(&tile_index.tiles, player_tf.translation);
        let previous = last_sets.per_client.entry(client_id).or_default();

        for tile_id in wanted.difference(previous) {
            sender.send::<TileStreamingChannel>(TileStreamingCommand {
                tile_id: tile_id.clone(),
                action: "load".to_string(),
            });
        }

        for tile_id in previous.difference(&wanted).cloned().collect::<Vec<_>>() {
            sender.send::<TileStreamingChannel>(TileStreamingCommand {
                tile_id: tile_id.clone(),
                action: "unload".to_string(),
            });
        }

        *previous = wanted;
    }
}

fn wanted_tiles_for_position(tiles: &[ServerTileDef], position: Vec3) -> HashSet<String> {
    let mut wanted = HashSet::new();

    for tile in tiles {
        let center = Vec3::new(tile.center[0], tile.center[1], tile.center[2]);
        let radius = tile.load_radius.max(1.0);
        if tile.always_loaded || center.distance(position) <= radius {
            wanted.insert(tile.runtime_id());
        }
    }

    wanted
}

fn tile_index_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let crate_dir = PathBuf::from(manifest_dir);
        if let Some(workspace_root) = crate_dir.parent() {
            candidates.push(workspace_root.join("host_client/assets/maps/world.index.toml"));
            candidates.push(workspace_root.join("host_client/assets/maps/map.index.toml"));
        }
        candidates.push(crate_dir.join("assets/maps/world.index.toml"));
        candidates.push(crate_dir.join("assets/maps/map.index.toml"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join("assets/maps/world.index.toml"));
            candidates.push(exe_dir.join("assets/maps/map.index.toml"));
        }
    }

    candidates
}