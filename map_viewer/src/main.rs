use std::fs;
use std::path::{Path, PathBuf};

use bevy::asset::{LoadState, UnapprovedPathMode};
use bevy::camera::primitives::Aabb;
use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use bevy::render::batching::NoAutomaticBatching;
use bevy::window::WindowResolution;
use serde::Deserialize;

use core_drawable::{
    AdmScene, AdmSceneRoot, CollisionShape, DrawableCollision, DrawableManifest,
    DrawableManifestRegistry, DrawablePlugin, GltfHandleCache, MapManifest, NavmeshRegistry,
};
use core_resources::{ModelName, ModelRegistry};

mod camera;

const DEFAULT_WORLD_INDEX: &str = "maps/world.index.toml";
const DEFAULT_MAP_FILE: &str = "maps/example.map.toml";

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .insert_resource(ViewerSettings::default())
        .insert_resource(InputTarget(parse_input_target()))
        .init_resource::<DebugLog>()
        .init_resource::<TileOverlayIndex>()
        .insert_resource(SceneBounds::default())
        .init_resource::<camera::OrbitState>()
        .init_resource::<ModelRegistry>()
        .init_resource::<GltfHandleCache>()
        .init_resource::<DrawableManifestRegistry>()
        .init_resource::<NavmeshRegistry>()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Map Viewer".to_string(),
                        resolution: WindowResolution::new(1600, 900),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: viewer_asset_root(),
                    unapproved_path_mode: UnapprovedPathMode::Allow,
                    ..default()
                }),
        )
        .add_plugins(DrawablePlugin)
        .add_systems(Startup, (setup_scene, register_native_assets, load_requested_input).chain())
        .add_systems(
            Update,
            (
                disable_automatic_batching_on_meshes,
                camera::orbit_camera,
                handle_keyboard,
                update_overlay,
                update_debug_overlay,
                draw_grid,
                draw_tile_overlays,
                draw_navmeshes,
                draw_colliders,
            ),
        )
        .run();
}

#[derive(Resource)]
struct InputTarget(PathBuf);

#[derive(Resource)]
struct ViewerSettings {
    grid_visible: bool,
    colliders_visible: bool,
    navmesh_visible: bool,
    tile_overlay_visible: bool,
    meshes_visible: bool,
    overlay_visible: bool,
    debug_visible: bool,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            grid_visible: true,
            colliders_visible: true,
            navmesh_visible: true,
            tile_overlay_visible: true,
            meshes_visible: true,
            overlay_visible: true,
            debug_visible: false,
        }
    }
}

#[derive(Resource, Default)]
struct SceneBounds {
    min: Option<Vec3>,
    max: Option<Vec3>,
}

impl SceneBounds {
    fn include_point(&mut self, point: Vec3) {
        match (&mut self.min, &mut self.max) {
            (Some(min), Some(max)) => {
                *min = min.min(point);
                *max = max.max(point);
            }
            _ => {
                self.min = Some(point);
                self.max = Some(point);
            }
        }
    }

    fn include_transform(&mut self, translation: Vec3, scale: Vec3) {
        let half = Vec3::new(scale.x.abs(), scale.y.abs(), scale.z.abs()) * 0.5;
        self.include_point(translation - half);
        self.include_point(translation + half);
    }

    fn center(&self) -> Vec3 {
        match (self.min, self.max) {
            (Some(min), Some(max)) => (min + max) * 0.5,
            _ => Vec3::ZERO,
        }
    }

    fn radius(&self) -> f32 {
        match (self.min, self.max) {
            (Some(min), Some(max)) => (max - min).length().max(10.0),
            _ => 20.0,
        }
    }
}

#[derive(Component)]
struct MapMeshRoot;

#[derive(Component)]
struct ViewerOverlay;

#[derive(Component)]
struct ViewerDebugOverlay;

#[derive(Component, Clone)]
struct MapAssetDebugInfo {
    reason: String,
    source_map: String,
    instance_id: String,
    tile_id: Option<String>,
    model: String,
    asset_path: String,
    drawable_path: Option<String>,
}

#[derive(Resource, Default)]
struct DebugLog {
    entries: Vec<String>,
}

impl DebugLog {
    fn push(&mut self, message: impl Into<String>) {
        let message = message.into();
        info!("[map_viewer/debug] {}", message);
        self.entries.push(message);
        if self.entries.len() > 80 {
            let overflow = self.entries.len() - 80;
            self.entries.drain(0..overflow);
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct WorldTileIndexManifest {
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
}

#[derive(Resource, Default)]
struct TileOverlayIndex {
    tiles: Vec<TileOverlayDef>,
}

#[derive(Clone)]
struct TileOverlayDef {
    id: String,
    map: String,
    center: Vec3,
    load_radius: f32,
    color: Color,
}

#[derive(Default)]
struct BoundsAccumulator {
    min: Option<Vec3>,
    max: Option<Vec3>,
}

impl BoundsAccumulator {
    fn include_aabb(&mut self, center: Vec3, half_extents: Vec3) {
        let min = center - half_extents;
        let max = center + half_extents;
        match (&mut self.min, &mut self.max) {
            (Some(current_min), Some(current_max)) => {
                *current_min = current_min.min(min);
                *current_max = current_max.max(max);
            }
            _ => {
                self.min = Some(min);
                self.max = Some(max);
            }
        }
    }

    fn center(&self) -> Option<Vec3> {
        Some((self.min? + self.max?) * 0.5)
    }

    fn half_extents(&self) -> Option<Vec3> {
        Some((self.max? - self.min?) * 0.5)
    }
}

fn default_tile_load_radius() -> f32 {
    1200.0
}

fn parse_input_target() -> PathBuf {
    let cli_path = std::env::args().skip(1).find(|arg| !arg.starts_with('-'));
    match cli_path {
        Some(path) => PathBuf::from(path),
        None => {
            let assets = PathBuf::from(viewer_asset_root());
            let world_index = assets.join(DEFAULT_WORLD_INDEX);
            if world_index.exists() {
                world_index
            } else {
                assets.join(DEFAULT_MAP_FILE)
            }
        }
    }
}

fn viewer_asset_root() -> String {
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let workspace = PathBuf::from(&dir)
            .parent()
            .map(|p| p.to_owned())
            .unwrap_or_else(|| PathBuf::from(&dir));
        let client_assets = workspace.join("host_client").join("assets");
        if client_assets.exists() {
            return client_assets.to_string_lossy().into_owned();
        }
        let local = PathBuf::from(&dir).join("assets");
        if local.exists() {
            return local.to_string_lossy().into_owned();
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("assets")))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "assets".to_string())
}

fn asset_root_path() -> PathBuf {
    PathBuf::from(viewer_asset_root())
}

fn tile_color(seed: &str) -> Color {
    let mut hash = 0u32;
    for byte in seed.bytes() {
        hash = hash.wrapping_mul(16777619) ^ byte as u32;
    }
    let hue = (hash % 360) as f32;
    Color::hsl(hue, 0.85, 0.58)
}

fn setup_scene(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(Vec3::new(0.0, 18.0, 28.0)).looking_at(Vec3::ZERO, Vec3::Y),
        camera::OrbitCamera,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 30_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.6, 0.0)),
    ));

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 13.0,
            ..default()
        },
        TextColor(Color::srgba(0.96, 0.96, 0.98, 0.95)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            max_width: Val::Px(560.0),
            ..default()
        },
        ViewerOverlay,
    ));

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 12.0,
            ..default()
        },
        TextColor(Color::srgba(0.95, 0.95, 0.97, 0.92)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(8.0),
            max_width: Val::Px(760.0),
            ..default()
        },
        Visibility::Hidden,
        ViewerDebugOverlay,
    ));
}

fn register_native_assets(
    asset_server: Res<AssetServer>,
    model_registry: ResMut<ModelRegistry>,
    mut gltf_cache: ResMut<GltfHandleCache>,
    mut drawable_registry: ResMut<DrawableManifestRegistry>,
    mut debug_log: ResMut<DebugLog>,
) {
    let models_dir = asset_root_path().join("models");
    let Ok(entries) = fs::read_dir(&models_dir) else {
        warn!("[map_viewer] cannot scan {:?}", models_dir);
        debug_log.push(format!("models scan failed: {}", models_dir.display()));
        return;
    };

    debug_log.push(format!("scanning models dir: {}", models_dir.display()));

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };

        match ext {
            "drawable" => {
                let rel = format!("models/{}.drawable", stem);
                let handle: Handle<DrawableManifest> = asset_server.load(rel);
                drawable_registry.0.insert(stem.to_string(), handle);
                debug_log.push(format!("drawable registered: {}", stem));
            }
            "adm" => {
                let rel = format!("models/{}.adm", stem);
                model_registry.register_native(stem.to_string(), rel.clone());
                let handle: Handle<AdmScene> = asset_server.load(rel);
                gltf_cache.0.remove(stem);
                let _ = handle;
                debug_log.push(format!("adm model registered: {}", stem));
            }
            "glb" | "gltf" => {
                let adm_path = path.with_extension("adm");
                if adm_path.exists() {
                    continue;
                }
                let rel = format!("models/{}.{}", stem, ext);
                model_registry.register_native(stem.to_string(), rel.clone());
                let handle: Handle<bevy::gltf::Gltf> = asset_server.load_with_settings(
                    rel.clone(),
                    |settings: &mut bevy::gltf::GltfLoaderSettings| {
                        settings.include_source = true;
                    },
                );
                gltf_cache.0.insert(stem.to_string(), (handle, rel));
                debug_log.push(format!("gltf model registered: {}", stem));
            }
            _ => {}
        }
    }
}

fn load_requested_input(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    input: Res<InputTarget>,
    model_registry: Res<ModelRegistry>,
    gltf_cache: Res<GltfHandleCache>,
    mut navmesh_registry: ResMut<NavmeshRegistry>,
    mut bounds: ResMut<SceneBounds>,
    mut orbit: ResMut<camera::OrbitState>,
    mut debug_log: ResMut<DebugLog>,
    mut tile_index: ResMut<TileOverlayIndex>,
) {
    let path = if input.0.is_absolute() {
        input.0.clone()
    } else {
        asset_root_path().join(&input.0)
    };
    debug_log.push(format!("input target resolved: {}", path.display()));

    let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
    match ext {
        "toml" if path.file_name().and_then(|value| value.to_str()) == Some("world.index.toml")
            || path.file_name().and_then(|value| value.to_str()) == Some("map.index.toml") =>
        {
            load_world_index(
                &mut commands,
                &asset_server,
                &path,
                &model_registry,
                &gltf_cache,
                &mut navmesh_registry,
                &mut bounds,
                &mut debug_log,
                &mut tile_index,
            );
        }
        "toml" => {
            tile_index.tiles.clear();
            load_single_map(
                &mut commands,
                &asset_server,
                &path,
                &model_registry,
                &gltf_cache,
                &mut navmesh_registry,
                &mut bounds,
                &mut debug_log,
                None,
            );
        }
        other => {
            warn!("[map_viewer] unsupported input {:?} ({})", path, other);
            debug_log.push(format!("unsupported input: {} ({})", path.display(), other));
        }
    }

    orbit.focus = bounds.center();
    orbit.distance = bounds.radius() * 0.85;
}

fn load_world_index(
    commands: &mut Commands,
    asset_server: &AssetServer,
    path: &Path,
    model_registry: &ModelRegistry,
    gltf_cache: &GltfHandleCache,
    navmesh_registry: &mut NavmeshRegistry,
    bounds: &mut SceneBounds,
    debug_log: &mut DebugLog,
    tile_index: &mut TileOverlayIndex,
) {
    let Ok(raw) = fs::read_to_string(path) else {
        warn!("[map_viewer] cannot read {:?}", path);
        debug_log.push(format!("failed to read world index: {}", path.display()));
        return;
    };
    let Ok(index) = toml::from_str::<WorldTileIndexManifest>(&raw) else {
        warn!("[map_viewer] invalid world index {:?}", path);
        debug_log.push(format!("invalid world index TOML: {}", path.display()));
        return;
    };
    debug_log.push(format!("world index loaded: {} ({} tile(s))", path.display(), index.tiles.len()));
    tile_index.tiles.clear();

    let maps_root = path.parent().unwrap_or_else(|| Path::new("."));
    for tile in index.tiles {
        let map_path = maps_root.join(&tile.map);
        let tile_id = if tile.id.trim().is_empty() {
            tile.map.clone()
        } else {
            tile.id.clone()
        };
        tile_index.tiles.push(TileOverlayDef {
            id: tile_id.clone(),
            map: map_path.display().to_string(),
            center: Vec3::new(tile.center[0], tile.center[1], tile.center[2]),
            load_radius: tile.load_radius,
            color: tile_color(&tile_id),
        });
        let reason = if tile.id.trim().is_empty() {
            format!("world index entry -> {}", tile.map)
        } else {
            format!("world index tile '{}'", tile.id)
        };
        load_single_map(
            commands,
            asset_server,
            &map_path,
            model_registry,
            gltf_cache,
            navmesh_registry,
            bounds,
            debug_log,
            Some(reason.clone()),
        );
        bounds.include_point(Vec3::new(tile.center[0], tile.center[1], tile.center[2]));
        if !tile.id.trim().is_empty() {
            info!("[map_viewer] loaded tile '{}' -> {:?}", tile.id, map_path);
        }
        debug_log.push(format!("tile scheduled: {} -> {}", reason, map_path.display()));
    }
}

fn load_single_map(
    commands: &mut Commands,
    asset_server: &AssetServer,
    path: &Path,
    model_registry: &ModelRegistry,
    gltf_cache: &GltfHandleCache,
    navmesh_registry: &mut NavmeshRegistry,
    bounds: &mut SceneBounds,
    debug_log: &mut DebugLog,
    reason: Option<String>,
) {
    let Ok(raw) = fs::read_to_string(path) else {
        warn!("[map_viewer] cannot read map {:?}", path);
        debug_log.push(format!("failed to read map: {}", path.display()));
        return;
    };
    let Ok(manifest) = MapManifest::from_toml_str(&raw) else {
        warn!("[map_viewer] invalid map manifest {:?}", path);
        debug_log.push(format!("invalid map TOML: {}", path.display()));
        return;
    };
    let source_map = path.display().to_string();
    let reason = reason.unwrap_or_else(|| format!("direct map load '{}'", source_map));
    debug_log.push(format!("map loaded: {} ({} instance(s))", source_map, manifest.instances.len()));

    for instance in manifest.instances {
        let translation = Vec3::new(instance.position[0], instance.position[1], instance.position[2]);
        let rotation = Quat::from_euler(
            EulerRot::XYZ,
            instance.rotation_deg[0].to_radians(),
            instance.rotation_deg[1].to_radians(),
            instance.rotation_deg[2].to_radians(),
        );
        let scale = Vec3::new(instance.scale[0], instance.scale[1], instance.scale[2]);
        spawn_map_instance(
            commands,
            asset_server,
            model_registry,
            gltf_cache,
            &instance.model,
            Transform {
                translation,
                rotation,
                scale,
            },
            instance.id,
            &source_map,
            &reason,
            reason_to_tile_id(&reason),
            debug_log,
        );
        bounds.include_transform(translation, scale.max(Vec3::ONE));
    }

    if let Some(navmesh_path) = navmesh_sidecar_path(path) {
        if let Ok(raw_navmesh) = fs::read_to_string(&navmesh_path) {
            if let Err(err) = navmesh_registry.load_navmesh(path.to_string_lossy().to_string(), &raw_navmesh) {
                warn!("[map_viewer] failed to load navmesh {:?}: {}", navmesh_path, err);
                debug_log.push(format!("navmesh load failed: {} ({})", navmesh_path.display(), err));
            } else {
                debug_log.push(format!("navmesh loaded: {}", navmesh_path.display()));
            }
        } else {
            debug_log.push(format!("navmesh missing or unreadable: {}", navmesh_path.display()));
        }
    } else {
        debug_log.push(format!("no navmesh sidecar for map: {}", source_map));
    }
}

fn navmesh_sidecar_path(map_path: &Path) -> Option<PathBuf> {
    let filename = map_path.file_name()?.to_str()?;
    if let Some(stem) = filename.strip_suffix(".map.toml") {
        return Some(map_path.with_file_name(format!("{}.navmesh.toml", stem)));
    }
    None
}

fn spawn_map_instance(
    commands: &mut Commands,
    asset_server: &AssetServer,
    model_registry: &ModelRegistry,
    gltf_cache: &GltfHandleCache,
    model: &str,
    transform: Transform,
    id: String,
    source_map: &str,
    reason: &str,
    tile_id: Option<String>,
    debug_log: &mut DebugLog,
) {
    let instance_id = if id.is_empty() {
        model.to_string()
    } else {
        id.clone()
    };
    let mut entity = commands.spawn((
        Name::new(if id.is_empty() {
            format!("map:{}", model)
        } else {
            format!("map:{}:{}", model, id)
        }),
        transform,
        Visibility::Visible,
        MapMeshRoot,
    ));

    let Some(path) = model_registry.path(model) else {
        warn!("[map_viewer] model '{}' not found in native registry", model);
        debug_log.push(format!(
            "instance '{}' from '{}' requested model '{}' but registry path was not found",
            instance_id,
            source_map,
            model,
        ));
        return;
    };
    let bevy_path = path.to_string_lossy().replace('\\', "/");
    let drawable_path = if bevy_path.ends_with(".adm") {
        Some(bevy_path.trim_end_matches(".adm").to_string() + ".drawable")
    } else if bevy_path.ends_with(".glb") || bevy_path.ends_with(".gltf") {
        let stem = bevy_path.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&bevy_path);
        Some(format!("{}.drawable", stem))
    } else {
        None
    };

    debug_log.push(format!(
        "instance '{}' -> model '{}' requested because {}; asset='{}' drawable='{}'",
        instance_id,
        model,
        reason,
        bevy_path,
        drawable_path.as_deref().unwrap_or("<none>"),
    ));

    if bevy_path.ends_with(".adm") {
        let handle: Handle<AdmScene> = asset_server.load(bevy_path);
        entity.insert((
            AdmSceneRoot(handle),
            ModelName(model.to_string()),
            MapAssetDebugInfo {
                reason: reason.to_string(),
                source_map: source_map.to_string(),
                instance_id,
                tile_id,
                model: model.to_string(),
                asset_path: path.display().to_string(),
                drawable_path,
            },
        ));
        return;
    }

    if let Some((gltf_handle, _)) = gltf_cache.0.get(model) {
        let _ = gltf_handle;
        let scene_path = format!("{}#Scene0", bevy_path);
        let scene_handle: Handle<Scene> = asset_server.load(scene_path.clone());
        entity.insert((
            SceneRoot(scene_handle),
            ModelName(model.to_string()),
            MapAssetDebugInfo {
                reason: reason.to_string(),
                source_map: source_map.to_string(),
                instance_id,
                tile_id,
                model: model.to_string(),
                asset_path: scene_path,
                drawable_path,
            },
        ));
        return;
    }

    warn!("[map_viewer] model '{}' has no loadable asset", model);
    debug_log.push(format!(
        "instance '{}' from '{}' resolved model '{}' but no loadable asset kind matched path '{}'",
        instance_id,
        source_map,
        model,
        path.display(),
    ));
}

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<ViewerSettings>,
    mut orbit: ResMut<camera::OrbitState>,
    bounds: Res<SceneBounds>,
    mut visibility_query: Query<&mut Visibility, With<MapMeshRoot>>,
) {
    let mut meshes_toggled = false;

    if keys.just_pressed(KeyCode::KeyG) {
        settings.grid_visible = !settings.grid_visible;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        settings.colliders_visible = !settings.colliders_visible;
    }
    if keys.just_pressed(KeyCode::KeyN) {
        settings.navmesh_visible = !settings.navmesh_visible;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        settings.tile_overlay_visible = !settings.tile_overlay_visible;
    }
    if keys.just_pressed(KeyCode::KeyH) {
        settings.overlay_visible = !settings.overlay_visible;
    }
    if keys.just_pressed(KeyCode::KeyD) {
        settings.debug_visible = !settings.debug_visible;
    }
    if keys.just_pressed(KeyCode::KeyM) {
        settings.meshes_visible = !settings.meshes_visible;
        meshes_toggled = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        orbit.focus = bounds.center();
        orbit.distance = bounds.radius() * 0.85;
    }

    if meshes_toggled {
        for mut visibility in &mut visibility_query {
            *visibility = if settings.meshes_visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
}

fn update_overlay(
    settings: Res<ViewerSettings>,
    input: Res<InputTarget>,
    bounds: Res<SceneBounds>,
    mut query: Query<(&mut Text, &mut Visibility), With<ViewerOverlay>>,
) {
    let Ok((mut text, mut visibility)) = query.single_mut() else {
        return;
    };

    *visibility = if settings.overlay_visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    if !settings.overlay_visible {
        return;
    }

    *text = Text::new(format!(
        "Map Viewer\nSoubor: {}\nG grid | M mesh | C colliders | N navmesh | T tiles | D debug | R refit | H overlay\nGrid: {} | Mesh: {} | Colliders: {} | Navmesh: {} | Tiles: {} | Debug: {}\nBounds center: {:.1}, {:.1}, {:.1} | radius {:.1}",
        input.0.display(),
        on_off(settings.grid_visible),
        on_off(settings.meshes_visible),
        on_off(settings.colliders_visible),
        on_off(settings.navmesh_visible),
        on_off(settings.tile_overlay_visible),
        on_off(settings.debug_visible),
        bounds.center().x,
        bounds.center().y,
        bounds.center().z,
        bounds.radius(),
    ));
}

fn update_debug_overlay(
    settings: Res<ViewerSettings>,
    debug_log: Res<DebugLog>,
    asset_server: Res<AssetServer>,
    navmeshes: Res<NavmeshRegistry>,
    adm_query: Query<(&MapAssetDebugInfo, &AdmSceneRoot)>,
    scene_query: Query<(&MapAssetDebugInfo, &SceneRoot)>,
    mut query: Query<(&mut Text, &mut Visibility), With<ViewerDebugOverlay>>,
) {
    let Ok((mut text, mut visibility)) = query.single_mut() else {
        return;
    };

    *visibility = if settings.debug_visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    if !settings.debug_visible {
        return;
    }

    let mut lines = Vec::new();
    lines.push("Debug Load Info".to_string());
    lines.push(format!("tracked assets: {}", adm_query.iter().count() + scene_query.iter().count()));
    lines.push(format!("loaded navmeshes: {}", navmeshes.navmeshes.len()));
    lines.push(String::new());
    lines.push("Asset States:".to_string());

    for (info, root) in &adm_query {
        lines.push(format!(
            "ADM [{}] model='{}' state={} map='{}' why={} asset='{}' drawable='{}'",
            info.instance_id,
            info.model,
            load_state_label(asset_server.get_load_state(root.0.id())),
            info.source_map,
            info.reason,
            info.asset_path,
            info.drawable_path.as_deref().unwrap_or("<none>"),
        ));
    }

    for (info, root) in &scene_query {
        lines.push(format!(
            "SCENE [{}] model='{}' state={} map='{}' why={} asset='{}' drawable='{}'",
            info.instance_id,
            info.model,
            load_state_label(asset_server.get_load_state(root.0.id())),
            info.source_map,
            info.reason,
            info.asset_path,
            info.drawable_path.as_deref().unwrap_or("<none>"),
        ));
    }

    lines.push(String::new());
    lines.push("Recent Log:".to_string());
    for entry in debug_log.entries.iter().rev().take(24).rev() {
        lines.push(entry.clone());
    }

    *text = Text::new(lines.join("\n"));
}

fn draw_tile_overlays(
    mut gizmos: Gizmos,
    settings: Res<ViewerSettings>,
    tile_index: Res<TileOverlayIndex>,
    root_query: Query<(Entity, &MapAssetDebugInfo)>,
    mesh_query: Query<(Entity, &GlobalTransform, &Aabb), With<Mesh3d>>,
    parents: Query<&ChildOf>,
) {
    if !settings.tile_overlay_visible {
        return;
    }

    for tile in &tile_index.tiles {
        let color = tile.color.with_alpha(0.75);
        let center = tile.center;
        let radius = tile.load_radius.max(1.0);
        draw_circle_xz(&mut gizmos, center, radius, color);
        draw_cross_gizmo(&mut gizmos, center, 0.6, color);
        gizmos.line(center, center + Vec3::Y * 1.5, color);
    }

    let mut bounds_by_root: std::collections::HashMap<Entity, BoundsAccumulator> = std::collections::HashMap::new();
    for (mesh_entity, global, aabb) in &mesh_query {
        let Some(root_entity) = find_debug_root(mesh_entity, &parents, &root_query) else {
            continue;
        };

        let center = global.transform_point(Vec3::from(aabb.center));
        let (scale, rotation, _) = global.to_scale_rotation_translation();
        let scaled_half = Vec3::new(
            aabb.half_extents.x.abs() * scale.x.abs(),
            aabb.half_extents.y.abs() * scale.y.abs(),
            aabb.half_extents.z.abs() * scale.z.abs(),
        );
        let world_half = approx_rotated_half_extents(rotation, scaled_half);
        bounds_by_root.entry(root_entity).or_default().include_aabb(center, world_half);
    }

    for (entity, info) in &root_query {
        let Some(bounds) = bounds_by_root.get(&entity) else {
            continue;
        };
        let Some(center) = bounds.center() else {
            continue;
        };
        let Some(half) = bounds.half_extents() else {
            continue;
        };

        let color = info
            .tile_id
            .as_deref()
            .map(tile_color)
            .unwrap_or_else(|| tile_color(&info.instance_id));
        draw_flat_bounds(&mut gizmos, center, half, color.with_alpha(0.95));
        draw_cross_gizmo(&mut gizmos, center, 0.35, color.with_alpha(0.95));
    }
}

fn reason_to_tile_id(reason: &str) -> Option<String> {
    let prefix = "world index tile '";
    if let Some(rest) = reason.strip_prefix(prefix) {
        return rest.strip_suffix('"').map(|s| s.to_string()).or_else(|| rest.strip_suffix("'").map(|s| s.to_string()));
    }
    None
}

fn find_debug_root(
    start: Entity,
    parents: &Query<&ChildOf>,
    roots: &Query<(Entity, &MapAssetDebugInfo)>,
) -> Option<Entity> {
    let mut current = start;
    loop {
        if roots.get(current).is_ok() {
            return Some(current);
        }
        let Ok(child_of) = parents.get(current) else {
            return None;
        };
        current = child_of.parent();
    }
}

fn draw_cross_gizmo(gizmos: &mut Gizmos, center: Vec3, half: f32, color: Color) {
    gizmos.line(center - Vec3::X * half, center + Vec3::X * half, color);
    gizmos.line(center - Vec3::Y * half, center + Vec3::Y * half, color);
    gizmos.line(center - Vec3::Z * half, center + Vec3::Z * half, color);
}

fn approx_rotated_half_extents(rotation: Quat, half: Vec3) -> Vec3 {
    let matrix = Mat3::from_quat(rotation);
    let x = matrix.x_axis.abs().dot(half);
    let y = matrix.y_axis.abs().dot(half);
    let z = matrix.z_axis.abs().dot(half);
    Vec3::new(x, y, z)
}

fn draw_flat_bounds(gizmos: &mut Gizmos, center: Vec3, half: Vec3, color: Color) {
    let y = center.y;
    let corners = [
        Vec3::new(center.x - half.x, y, center.z - half.z),
        Vec3::new(center.x + half.x, y, center.z - half.z),
        Vec3::new(center.x + half.x, y, center.z + half.z),
        Vec3::new(center.x - half.x, y, center.z + half.z),
    ];
    gizmos.line(corners[0], corners[1], color);
    gizmos.line(corners[1], corners[2], color);
    gizmos.line(corners[2], corners[3], color);
    gizmos.line(corners[3], corners[0], color);
    let top = center + Vec3::Y * 0.75;
    gizmos.line(center, top, color);
}

fn draw_circle_xz(gizmos: &mut Gizmos, center: Vec3, radius: f32, color: Color) {
    draw_arc_gizmo(
        gizmos,
        center,
        Vec3::X,
        Vec3::Z,
        radius,
        0.0,
        std::f32::consts::TAU,
        48,
        color,
    );
}

fn load_state_label(load_state: Option<LoadState>) -> &'static str {
    match load_state {
        Some(LoadState::Loaded) => "Loaded",
        Some(LoadState::Loading) => "Loading",
        Some(LoadState::NotLoaded) => "NotLoaded",
        Some(LoadState::Failed(_)) => "Failed",
        None => "Unknown",
    }
}

fn disable_automatic_batching_on_meshes(
    mut commands: Commands,
    query: Query<Entity, (With<Mesh3d>, Without<NoAutomaticBatching>)>,
) {
    for entity in &query {
        commands.entity(entity).insert(NoAutomaticBatching);
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn draw_grid(mut gizmos: Gizmos, settings: Res<ViewerSettings>) {
    if !settings.grid_visible {
        return;
    }

    let half = 80;
    let gray = Color::srgba(0.38, 0.40, 0.45, 0.35);
    for i in -half..=half {
        let coord = i as f32;
        gizmos.line(
            Vec3::new(coord, 0.0, -(half as f32)),
            Vec3::new(coord, 0.0, half as f32),
            gray,
        );
        gizmos.line(
            Vec3::new(-(half as f32), 0.0, coord),
            Vec3::new(half as f32, 0.0, coord),
            gray,
        );
    }
    gizmos.line(Vec3::new(-80.0, 0.0, 0.0), Vec3::new(80.0, 0.0, 0.0), Color::srgb(0.9, 0.2, 0.2));
    gizmos.line(Vec3::new(0.0, 0.0, -80.0), Vec3::new(0.0, 0.0, 80.0), Color::srgb(0.2, 0.7, 1.0));
}

fn draw_navmeshes(mut gizmos: Gizmos, settings: Res<ViewerSettings>, navmeshes: Res<NavmeshRegistry>) {
    if !settings.navmesh_visible {
        return;
    }

    for (map_id, navmesh) in &navmeshes.navmeshes {
        let label_color = Color::srgba(1.0, 0.7, 0.2, 0.8);
        let _ = map_id;
        for surface in &navmesh.surfaces {
            let color = navmesh_surface_color(&surface.surface_type);
            for face in &surface.faces {
                let a = surface.vertices.get(face[0] as usize).copied();
                let b = surface.vertices.get(face[1] as usize).copied();
                let c = surface.vertices.get(face[2] as usize).copied();
                if let (Some(a), Some(b), Some(c)) = (a, b, c) {
                    gizmos.line(a, b, color);
                    gizmos.line(b, c, color);
                    gizmos.line(c, a, color);
                    let center = (a + b + c) / 3.0;
                    gizmos.line(center, center + Vec3::Y * 0.08, label_color);
                }
            }
        }
    }
}

fn navmesh_surface_color(surface_type: &str) -> Color {
    match surface_type {
        "ground" => Color::srgba(0.15, 0.95, 0.35, 0.85),
        "water" => Color::srgba(0.2, 0.6, 1.0, 0.85),
        "underwater" => Color::srgba(0.1, 0.9, 0.95, 0.85),
        "climb" | "climbable" => Color::srgba(1.0, 0.85, 0.2, 0.85),
        _ => Color::srgba(0.95, 0.25, 0.8, 0.85),
    }
}

fn draw_colliders(
    mut gizmos: Gizmos,
    settings: Res<ViewerSettings>,
    query: Query<(&DrawableCollision, &GlobalTransform)>,
) {
    if !settings.colliders_visible {
        return;
    }

    for (collider, transform) in &query {
        let (scale, rotation, center) = transform.to_scale_rotation_translation();
        let color = collider_color(collider);
        match collider.shape {
            CollisionShape::Box | CollisionShape::Mesh | CollisionShape::Convex | CollisionShape::Navmesh => {
                let half = collider.half_extents.unwrap_or(Vec3::splat(0.5)) * scale;
                draw_box_gizmo(&mut gizmos, center, rotation, half, color);
            }
            CollisionShape::Sphere => {
                let radius = collider.radius.unwrap_or(0.5) * scale.max_element();
                draw_sphere_gizmo(&mut gizmos, center, rotation, radius, color);
            }
            CollisionShape::Capsule => {
                let radial_scale = scale.x.max(scale.z);
                let radius = collider.radius.unwrap_or(0.3) * radial_scale;
                let half_seg = collider
                    .height
                    .map(|height| (height * 0.5 * scale.y - radius).max(0.0))
                    .unwrap_or(0.5);
                draw_capsule_gizmo(&mut gizmos, center, rotation, radius, half_seg, color);
            }
            CollisionShape::Cylinder => {
                let radial_scale = scale.x.max(scale.z);
                let radius = collider.radius.unwrap_or(0.5) * radial_scale;
                let half_h = collider.height.map(|height| height * 0.5 * scale.y).unwrap_or(0.5);
                draw_cylinder_gizmo(&mut gizmos, center, rotation, radius, half_h, color);
            }
        }
    }
}

fn collider_color(collider: &DrawableCollision) -> Color {
    match collider.shape {
        CollisionShape::Navmesh => Color::srgba(0.95, 0.15, 0.8, 0.95),
        CollisionShape::Mesh | CollisionShape::Convex => Color::srgba(0.95, 0.55, 0.15, 0.95),
        CollisionShape::Sphere => Color::srgba(0.30, 0.85, 1.0, 0.95),
        CollisionShape::Capsule => Color::srgba(0.35, 1.0, 0.60, 0.95),
        CollisionShape::Cylinder => Color::srgba(1.0, 0.90, 0.30, 0.95),
        CollisionShape::Box => Color::srgba(1.0, 0.3, 0.3, 0.95),
    }
}

fn draw_box_gizmo(gizmos: &mut Gizmos, center: Vec3, rot: Quat, half: Vec3, color: Color) {
    let corners = [
        center + rot * Vec3::new(-half.x, -half.y, -half.z),
        center + rot * Vec3::new(half.x, -half.y, -half.z),
        center + rot * Vec3::new(half.x, half.y, -half.z),
        center + rot * Vec3::new(-half.x, half.y, -half.z),
        center + rot * Vec3::new(-half.x, -half.y, half.z),
        center + rot * Vec3::new(half.x, -half.y, half.z),
        center + rot * Vec3::new(half.x, half.y, half.z),
        center + rot * Vec3::new(-half.x, half.y, half.z),
    ];
    gizmos.line(corners[0], corners[1], color);
    gizmos.line(corners[1], corners[2], color);
    gizmos.line(corners[2], corners[3], color);
    gizmos.line(corners[3], corners[0], color);
    gizmos.line(corners[4], corners[5], color);
    gizmos.line(corners[5], corners[6], color);
    gizmos.line(corners[6], corners[7], color);
    gizmos.line(corners[7], corners[4], color);
    gizmos.line(corners[0], corners[4], color);
    gizmos.line(corners[1], corners[5], color);
    gizmos.line(corners[2], corners[6], color);
    gizmos.line(corners[3], corners[7], color);
}

fn draw_sphere_gizmo(gizmos: &mut Gizmos, center: Vec3, rot: Quat, radius: f32, color: Color) {
    let (x, y, z) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
    draw_arc_gizmo(gizmos, center, x, y, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, center, x, z, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, center, y, z, radius, 0.0, std::f32::consts::TAU, 24, color);
}

fn draw_cylinder_gizmo(
    gizmos: &mut Gizmos,
    center: Vec3,
    rot: Quat,
    radius: f32,
    half_h: f32,
    color: Color,
) {
    let (up, right, forward) = (rot * Vec3::Y, rot * Vec3::X, rot * Vec3::Z);
    let bottom = center - up * half_h;
    let top = center + up * half_h;
    draw_arc_gizmo(gizmos, bottom, right, forward, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, top, right, forward, radius, 0.0, std::f32::consts::TAU, 24, color);
    gizmos.line(bottom + right * radius, top + right * radius, color);
    gizmos.line(bottom - right * radius, top - right * radius, color);
    gizmos.line(bottom + forward * radius, top + forward * radius, color);
    gizmos.line(bottom - forward * radius, top - forward * radius, color);
}

fn draw_capsule_gizmo(
    gizmos: &mut Gizmos,
    center: Vec3,
    rot: Quat,
    radius: f32,
    half_seg: f32,
    color: Color,
) {
    let (up, right, forward) = (rot * Vec3::Y, rot * Vec3::X, rot * Vec3::Z);
    let bottom = center - up * half_seg;
    let top = center + up * half_seg;
    draw_arc_gizmo(gizmos, bottom, right, forward, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, top, right, forward, radius, 0.0, std::f32::consts::TAU, 24, color);
    gizmos.line(bottom + right * radius, top + right * radius, color);
    gizmos.line(bottom - right * radius, top - right * radius, color);
    gizmos.line(bottom + forward * radius, top + forward * radius, color);
    gizmos.line(bottom - forward * radius, top - forward * radius, color);
    draw_arc_gizmo(gizmos, top, right, up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, top, forward, up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, bottom, right, -up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, bottom, forward, -up, radius, 0.0, std::f32::consts::PI, 12, color);
}

fn draw_arc_gizmo(
    gizmos: &mut Gizmos,
    center: Vec3,
    tangent_a: Vec3,
    tangent_b: Vec3,
    radius: f32,
    start: f32,
    end: f32,
    segments: u32,
    color: Color,
) {
    let step = (end - start) / segments as f32;
    let mut previous = center + (tangent_a * start.cos() + tangent_b * start.sin()) * radius;
    for idx in 1..=segments {
        let angle = start + idx as f32 * step;
        let next = center + (tangent_a * angle.cos() + tangent_b * angle.sin()) * radius;
        gizmos.line(previous, next, color);
        previous = next;
    }
}
