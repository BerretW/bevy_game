use std::fs;
use std::path::{Path, PathBuf};

use bevy::prelude::*;

use core_drawable::MapManifest;
use core_resources::{EntityHandle, LocalObjectMarker, LuaWorldState};

use crate::AppState;

pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadedMapFiles>()
            .add_systems(OnEnter(AppState::InGame), load_maps_on_enter)
            .add_systems(OnExit(AppState::InGame), cleanup_maps_on_exit)
            .add_systems(Update, enforce_navmesh_only_hidden.run_if(in_state(AppState::InGame)));
    }
}

#[derive(Resource, Default)]
struct LoadedMapFiles {
    loaded: Vec<PathBuf>,
}

#[allow(dead_code)]
#[derive(Component, Debug, Clone)]
pub struct MapObjectInstance {
    pub map_file: String,
    pub entry_index: usize,
    pub id: String,
    pub navmesh_only: bool,
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
) {
    let maps_root = asset_root().join("maps");
    let files = collect_map_files(&maps_root);
    if files.is_empty() {
        info!("[map_loader] no map TOML found in {:?}", maps_root);
        return;
    }

    let mut spawned_total = 0usize;
    for map_path in files {
        if loaded_maps.loaded.iter().any(|p| p == &map_path) {
            continue;
        }

        let Some(map_text) = fs::read_to_string(&map_path).ok() else {
            warn!("[map_loader] failed to read {:?}", map_path);
            continue;
        };
        let manifest = match MapManifest::from_toml_str(&map_text) {
            Ok(v) => v,
            Err(err) => {
                warn!("[map_loader] invalid map TOML {:?}: {}", map_path, err);
                continue;
            }
        };

        let file_name = map_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("map.toml")
            .to_string();

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
                LocalObjectMarker {
                    model: entry.model.clone(),
                },
                MapObjectInstance {
                    map_file: file_name.clone(),
                    entry_index,
                    id,
                    navmesh_only: entry.navmesh_only,
                },
            ));

            if entry.navmesh_only {
                entity_cmds.insert(Visibility::Hidden);
            }

            // Přiřaď EntityHandle aby byl objekt viditelný pro Lua crosshair/API.
            let entity_id = entity_cmds.id();
            let handle = entity_id.to_bits();
            entity_cmds.insert(EntityHandle(handle));
            world_state.register(handle, entity_id);

            spawned_total += 1;
        }

        loaded_maps.loaded.push(map_path.clone());
        info!(
            "[map_loader] loaded '{}' with {} instance(s)",
            file_name,
            manifest.instances.len()
        );
    }

    info!("[map_loader] spawned {} map instance(s)", spawned_total);
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
    query: Query<Entity, With<MapObjectInstance>>,
) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
    loaded_maps.loaded.clear();
    info!("[map_loader] cleaned map instances and reset loaded map cache");
}

fn enforce_navmesh_only_hidden(
    mut query: Query<(&MapObjectInstance, &mut Visibility)>,
) {
    for (instance, mut visibility) in &mut query {
        if instance.navmesh_only {
            *visibility = Visibility::Hidden;
        }
    }
}
