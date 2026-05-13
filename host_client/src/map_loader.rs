use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::Deserialize;

use core_drawable::{HLODTile, HLODState, MapManifest, NavmeshRegistry, TileGraph, TilePathDef};
use core_net::TileStreamingCommand;
use core_resources::{
    EntityHandle, LocalObjectMarker, LuaWorldState, NpcAgent, NpcMoveGoal, NpcOwner,
    NpcPathWaypoint,
};
use core_shared::PlayerMarker;

use crate::gameplay::LocalClientId;
use crate::AppState;

const DEFAULT_TILE_LOAD_RADIUS: f32 = 1200.0;
const TILE_STREAM_UPDATE_SECS: f32 = 1.0;

pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadedMapFiles>()
            .init_resource::<NavmeshRegistry>()
            .init_resource::<WorldTileIndex>()
            .init_resource::<ServerTileStreamingState>()
            .add_systems(OnEnter(AppState::InGame), load_maps_on_enter)
            .add_systems(OnExit(AppState::InGame), cleanup_maps_on_exit)
            .add_systems(
                Update,
                (
                    receive_tile_streaming_commands.run_if(in_state(AppState::InGame)),
                    stream_world_tiles.run_if(in_state(AppState::InGame)),
                    update_npc_nav_paths.run_if(in_state(AppState::InGame)),
                    enforce_navmesh_only_hidden.run_if(in_state(AppState::InGame)),
                ),
            );
    }
}

#[derive(Debug, Clone)]
struct LoadedMapRecord {
    map_key: String,
    entities: Vec<Entity>,
    handles: Vec<u64>,
}

#[derive(Resource, Default)]
struct LoadedMapFiles {
    loaded: HashMap<PathBuf, LoadedMapRecord>,
}

#[derive(Resource, Debug, Default)]
struct WorldTileIndex {
    maps_root: PathBuf,
    tiles: Vec<WorldTileDef>,
    active: bool,
}

#[derive(Resource, Debug, Default)]
struct ServerTileStreamingState {
    authoritative_tiles: HashSet<String>,
    last_update_secs: f64,
    has_server_override: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct WorldTileIndexManifest {
    #[serde(default)]
    version: String,
    #[serde(default)]
    tiles: Vec<WorldTileDef>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorldTileDef {
    #[serde(default)]
    id: String,
    map: String,
    center: [f32; 3],
    #[serde(default = "default_tile_load_radius")]
    load_radius: f32,
    #[serde(default)]
    always_loaded: bool,
}

impl WorldTileDef {
    fn runtime_id(&self) -> String {
        if self.id.trim().is_empty() {
            self.map.clone()
        } else {
            self.id.clone()
        }
    }
}

#[allow(dead_code)]
#[derive(Component, Debug, Clone)]
pub struct MapObjectInstance {
    pub map_file: String,
    pub entry_index: usize,
    pub id: String,
    pub navmesh_only: bool,
}

fn default_tile_load_radius() -> f32 {
    DEFAULT_TILE_LOAD_RADIUS
}

fn asset_root() -> PathBuf {
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        return PathBuf::from(manifest).join("assets");
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("assets")))
        .unwrap_or_else(|| PathBuf::from("assets"))
}

fn load_maps_on_enter(
    mut commands: Commands,
    mut loaded_maps: ResMut<LoadedMapFiles>,
    mut world_state: ResMut<LuaWorldState>,
    mut navmesh_registry: ResMut<NavmeshRegistry>,
    mut tile_index: ResMut<WorldTileIndex>,
    mut tile_graph: ResMut<TileGraph>,
) {
    let maps_root = asset_root().join("maps");
    tile_index.maps_root = maps_root.clone();

    if let Some(index) = load_world_tile_index(&maps_root) {
        tile_index.active = true;
        tile_index.tiles = index.tiles.clone();
        
        // Build tile graph from index
        tile_graph.tiles.clear();
        for tile_def in &index.tiles {
            let tile_id = tile_def.runtime_id();
            tile_graph.tiles.insert(
                tile_id.clone(),
                TilePathDef {
                    id: tile_id,
                    center: Vec3::new(tile_def.center[0], tile_def.center[1], tile_def.center[2]),
                    load_radius: tile_def.load_radius,
                    traversal_cost: 1.0, // Default cost; can be parameterized per tile
                },
            );
        }
        tile_graph.build_adjacency(2500.0); // Max distance threshold for adjacency
        
        info!(
            "[map_loader] tile streaming index enabled ({} tiles, {} edges)",
            tile_graph.tiles.len(),
            tile_graph.adjacency.values().map(|v| v.len()).sum::<usize>()
        );
        return;
    }

    tile_index.active = false;
    tile_index.tiles.clear();

    let files = collect_map_files(&maps_root);
    if files.is_empty() {
        info!("[map_loader] no map TOML found in {:?}", maps_root);
        return;
    }

    let mut spawned_total = 0usize;
    for map_path in files {
        if let Some(loaded) = load_single_map_file(
            &mut commands,
            &mut loaded_maps,
            &mut world_state,
            &mut navmesh_registry,
            &maps_root,
            &map_path,
        ) {
            spawned_total += loaded;
        }
    }

    info!("[map_loader] spawned {} map instance(s)", spawned_total);
}

fn stream_world_tiles(
    time: Res<Time>,
    local_client_id: Option<Res<LocalClientId>>,
    players: Query<(&Transform, &PlayerMarker)>,
    mut ticker: Local<f32>,
    mut commands: Commands,
    mut loaded_maps: ResMut<LoadedMapFiles>,
    mut world_state: ResMut<LuaWorldState>,
    mut navmesh_registry: ResMut<NavmeshRegistry>,
    tile_index: Res<WorldTileIndex>,
    server_streaming: Res<ServerTileStreamingState>,
) {
    if !tile_index.active {
        return;
    }

    *ticker += time.delta_secs();
    if *ticker < TILE_STREAM_UPDATE_SECS {
        return;
    }
    *ticker = 0.0;

    let focus = resolve_stream_focus(local_client_id.as_deref(), &players);

    let mut wanted: HashSet<PathBuf> = HashSet::new();

    let use_server_override = server_streaming.has_server_override
        && (time.elapsed_secs_f64() - server_streaming.last_update_secs) <= 5.0;

    for tile in &tile_index.tiles {
        if use_server_override {
            if server_streaming.authoritative_tiles.contains(&tile.runtime_id()) {
                wanted.insert(tile_index.maps_root.join(&tile.map));
            }
        } else {
            let center = Vec3::new(tile.center[0], tile.center[1], tile.center[2]);
            let radius = tile.load_radius.max(1.0);
            if tile.always_loaded || focus.distance(center) <= radius {
                wanted.insert(tile_index.maps_root.join(&tile.map));
            }
        }
    }

    for path in &wanted {
        if loaded_maps.loaded.contains_key(path) {
            continue;
        }
        load_single_map_file(
            &mut commands,
            &mut loaded_maps,
            &mut world_state,
            &mut navmesh_registry,
            &tile_index.maps_root,
            path,
        );
    }

    let currently_loaded: Vec<PathBuf> = loaded_maps.loaded.keys().cloned().collect();
    for path in currently_loaded {
        if wanted.contains(&path) {
            continue;
        }
        unload_single_map_file(
            &mut commands,
            &mut loaded_maps,
            &mut world_state,
            &mut navmesh_registry,
            &path,
        );
    }
}

fn receive_tile_streaming_commands(
    time: Res<Time>,
    mut receivers: Query<&mut MessageReceiver<TileStreamingCommand>>,
    mut server_streaming: ResMut<ServerTileStreamingState>,
) {
    let mut saw_message = false;

    for mut rx in &mut receivers {
        for msg in rx.receive() {
            saw_message = true;
            match msg.action.as_str() {
                "load" => {
                    server_streaming.authoritative_tiles.insert(msg.tile_id);
                }
                "unload" => {
                    server_streaming.authoritative_tiles.remove(&msg.tile_id);
                }
                other => {
                    warn!("[map_loader] unknown TileStreamingCommand action '{}'", other);
                }
            }
        }
    }

    if saw_message {
        server_streaming.last_update_secs = time.elapsed_secs_f64();
        server_streaming.has_server_override = true;
    }
}

fn resolve_stream_focus(
    local_client_id: Option<&LocalClientId>,
    players: &Query<(&Transform, &PlayerMarker)>,
) -> Vec3 {
    let Some(local_id) = local_client_id.map(|v| v.0) else {
        return Vec3::ZERO;
    };

    for (tf, marker) in players.iter() {
        if marker.client_id == local_id {
            return tf.translation;
        }
    }

    Vec3::ZERO
}

fn load_single_map_file(
    commands: &mut Commands,
    loaded_maps: &mut LoadedMapFiles,
    world_state: &mut LuaWorldState,
    navmesh_registry: &mut NavmeshRegistry,
    maps_root: &Path,
    map_path: &Path,
) -> Option<usize> {
    if loaded_maps.loaded.contains_key(map_path) {
        return Some(0);
    }

    let map_text = match fs::read_to_string(map_path) {
        Ok(v) => v,
        Err(err) => {
            warn!("[map_loader] failed to read {:?}: {}", map_path, err);
            return None;
        }
    };

    let manifest = match MapManifest::from_toml_str(&map_text) {
        Ok(v) => v,
        Err(err) => {
            warn!("[map_loader] invalid map TOML {:?}: {}", map_path, err);
            return None;
        }
    };

    let map_key = map_key_for_path(maps_root, map_path);
    let mut record = LoadedMapRecord {
        map_key: map_key.clone(),
        entities: Vec::new(),
        handles: Vec::new(),
    };

    for (entry_index, entry) in manifest.instances.iter().enumerate() {
        let transform = Transform::from_translation(Vec3::new(
            entry.position[0],
            entry.position[1],
            entry.position[2],
        ))
        .with_rotation(Quat::from_euler(
            EulerRot::XYZ,
            entry.rotation_deg[0].to_radians(),
            entry.rotation_deg[1].to_radians(),
            entry.rotation_deg[2].to_radians(),
        ))
        .with_scale(Vec3::new(entry.scale[0], entry.scale[1], entry.scale[2]));

        let id = if entry.id.is_empty() {
            format!("{}_{}", entry.model, entry_index)
        } else {
            entry.id.clone()
        };

        let mut entity_cmds = commands.spawn((
            Name::new(format!("map:{}", id)),
            transform,
            HLODState::default(),
            HLODTile::new(map_key.clone()),
            LocalObjectMarker {
                model: entry.model.clone(),
            },
            MapObjectInstance {
                map_file: map_key.clone(),
                entry_index,
                id,
                navmesh_only: entry.navmesh_only,
            },
        ));

        if entry.navmesh_only {
            entity_cmds.insert(Visibility::Hidden);
        }

        let entity_id = entity_cmds.id();
        let handle = entity_id.to_bits();
        entity_cmds.insert(EntityHandle(handle));

        world_state.register(handle, entity_id);
        record.entities.push(entity_id);
        record.handles.push(handle);
    }

    if let Some((navmesh_path, navmesh_text)) = read_first_navmesh_sidecar(map_path) {
        match navmesh_registry.load_navmesh(map_key.clone(), &navmesh_text) {
            Ok(_) => info!("[map_loader] loaded navmesh '{}' for {}", navmesh_path.display(), map_key),
            Err(err) => warn!("[map_loader] failed to parse navmesh {:?}: {}", navmesh_path, err),
        }
    }

    let count = manifest.instances.len();
    loaded_maps.loaded.insert(map_path.to_path_buf(), record);
    info!("[map_loader] loaded '{}' with {} instance(s)", map_key, count);

    Some(count)
}

fn unload_single_map_file(
    commands: &mut Commands,
    loaded_maps: &mut LoadedMapFiles,
    world_state: &mut LuaWorldState,
    navmesh_registry: &mut NavmeshRegistry,
    map_path: &Path,
) {
    let Some(record) = loaded_maps.loaded.remove(map_path) else {
        return;
    };

    for handle in &record.handles {
        world_state.remove(*handle);
    }

    for entity in &record.entities {
        commands.entity(*entity).despawn();
    }

    navmesh_registry.unload_navmesh(&record.map_key);
    info!("[map_loader] unloaded '{}'", record.map_key);
}

fn map_key_for_path(root: &Path, map_path: &Path) -> String {
    map_path
        .strip_prefix(root)
        .unwrap_or(map_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn read_first_navmesh_sidecar(map_path: &Path) -> Option<(PathBuf, String)> {
    for candidate in navmesh_candidates(map_path) {
        if !candidate.exists() {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&candidate) {
            return Some((candidate, text));
        }
    }
    None
}

fn navmesh_candidates(map_path: &Path) -> Vec<PathBuf> {
    let mut out = vec![map_path.with_extension("navmesh.toml")];

    if let Some(name) = map_path.file_name().and_then(|s| s.to_str()) {
        if let Some(base) = name.strip_suffix(".map.toml") {
            out.push(map_path.with_file_name(format!("{}.navmesh.toml", base)));
        }
    }

    out
}

fn load_world_tile_index(maps_root: &Path) -> Option<WorldTileIndexManifest> {
    let candidates = [
        maps_root.join("world.index.toml"),
        maps_root.join("map.index.toml"),
    ];

    for path in candidates {
        if !path.exists() {
            continue;
        }

        let text = match fs::read_to_string(&path) {
            Ok(v) => v,
            Err(err) => {
                warn!("[map_loader] failed reading tile index {:?}: {}", path, err);
                continue;
            }
        };

        let parsed = match toml::from_str::<WorldTileIndexManifest>(&text) {
            Ok(v) => v,
            Err(err) => {
                warn!("[map_loader] invalid tile index {:?}: {}", path, err);
                continue;
            }
        };

        if parsed.tiles.is_empty() {
            warn!("[map_loader] tile index {:?} has no tiles", path);
            continue;
        }

        info!(
            "[map_loader] loaded tile index {:?} (version={}, tiles={})",
            path,
            parsed.version,
            parsed.tiles.len()
        );
        return Some(parsed);
    }

    None
}

fn collect_map_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(v) => v,
        Err(_) => return out,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.ends_with(".map.toml") || name.ends_with(".map") || name == "map.toml" {
            out.push(path);
        }
    }

    out.sort();
    out
}

fn cleanup_maps_on_exit(
    mut commands: Commands,
    mut loaded_maps: ResMut<LoadedMapFiles>,
    mut world_state: ResMut<LuaWorldState>,
    mut navmesh_registry: ResMut<NavmeshRegistry>,
    mut tile_index: ResMut<WorldTileIndex>,
    mut server_streaming: ResMut<ServerTileStreamingState>,
) {
    let paths: Vec<PathBuf> = loaded_maps.loaded.keys().cloned().collect();
    for path in paths {
        unload_single_map_file(
            &mut commands,
            &mut loaded_maps,
            &mut world_state,
            &mut navmesh_registry,
            &path,
        );
    }

    tile_index.active = false;
    tile_index.tiles.clear();
    tile_index.maps_root = PathBuf::new();
    server_streaming.authoritative_tiles.clear();
    server_streaming.has_server_override = false;
    server_streaming.last_update_secs = 0.0;
    info!("[map_loader] cleaned map instances and reset tile streaming state");
}

fn enforce_navmesh_only_hidden(mut query: Query<(&MapObjectInstance, &mut Visibility)>) {
    for (instance, mut visibility) in &mut query {
        if instance.navmesh_only {
            *visibility = Visibility::Hidden;
        }
    }
}

fn update_npc_nav_paths(
    local_client_id: Option<Res<LocalClientId>>,
    world_state: Res<LuaWorldState>,
    tile_index: Res<WorldTileIndex>,
    tile_graph: Res<TileGraph>,
    navmesh_registry: Res<NavmeshRegistry>,
    globals: Query<&GlobalTransform>,
    mut npcs: Query<(&Transform, &mut NpcAgent, Option<&NpcOwner>)>,
) {
    if !tile_index.active || tile_graph.tiles.is_empty() {
        return;
    }

    let local_client_id = local_client_id.as_ref().map(|v| v.0);

    for (transform, mut agent, owner) in &mut npcs {
        if let (Some(local_id), Some(owner)) = (local_client_id, owner) {
            if owner.0 != Some(local_id) {
                continue;
            }
        }

        if !agent.current_path.is_empty() && agent.waypoint_index < agent.current_path.len() {
            continue;
        }

        let target = match &agent.goal {
            NpcMoveGoal::GoToCoord { target, .. } => Some(*target),
            NpcMoveGoal::GoToEntity { target_handle, .. } => {
                world_state
                    .entity_for(*target_handle)
                    .and_then(|entity| globals.get(entity).ok())
                    .map(|tf| tf.translation())
            }
            _ => None,
        };

        let Some(target) = target else {
            continue;
        };

        let Some(start_tile_id) = tile_id_for_position(&tile_index, transform.translation) else {
            continue;
        };
        let Some(goal_tile_id) = tile_id_for_position(&tile_index, target) else {
            continue;
        };

        let Some(path) = navmesh_registry.find_hierarchical_path(
            &tile_graph,
            &start_tile_id,
            &goal_tile_id,
            transform.translation,
            target,
            0.35,
        ) else {
            continue;
        };

        agent.map_id = start_tile_id;
        agent.current_path = path
            .into_iter()
            .map(|segment| NpcPathWaypoint { target: segment.target })
            .collect();
        agent.waypoint_index = 0;
    }
}

fn tile_id_for_position(tile_index: &WorldTileIndex, position: Vec3) -> Option<String> {
    let mut best: Option<(f32, String)> = None;

    for tile in &tile_index.tiles {
        let center = Vec3::new(tile.center[0], tile.center[1], tile.center[2]);
        let radius = tile.load_radius.max(1.0);
        let distance = center.distance(position);
        if distance > radius {
            continue;
        }

        match &best {
            Some((best_distance, _)) if *best_distance <= distance => {}
            _ => best = Some((distance, tile.runtime_id())),
        }
    }

    best.map(|(_, tile_id)| tile_id)
}
