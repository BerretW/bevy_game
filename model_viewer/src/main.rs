//! ADS Model Viewer — standalone viewer pro .glb + .drawable assety.
//!
//! Použití:
//!   cargo run -p model_viewer -- path/to/model.glb
//!   cargo run -p model_viewer -- path/to/model.glb path/to/other.glb
//!
//! Ovládání:
//!   Pravé tlačítko + tah  → orbit
//!   Střední + tah         → pan
//!   Kolečko               → zoom
//!   R                     → reset kamery
//!   G                     → přepnout mřížku
//!   H                     → přepnout info overlay
//!   F                     → fit model do pohledu
//!   T                     → přepnout prohlížeč textur
//!   E                     → exportovat textury na disk
//!   W                     → cyklovat weather presety (clean → dirty → wet → snowy)
//!   V                     → cyklovat vertex color debug kanály (off → RGB → R → G → B → A)

use std::path::PathBuf;

use bevy::asset::UnapprovedPathMode;
use bevy::gltf::{Gltf, GltfLoaderSettings};
use bevy::prelude::*;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::window::WindowResolution;

use core_drawable::{
    AdmScene, AdmSceneRoot,
    CollisionShape, DrawableCollision,
    LodGroup,
    DrawableManifest, DrawableManifestRegistry, DrawablePlugin, EntityDef,
    GltfHandleCache, TextureSource,
    StandardPbrMaterial, LayeredEnvMaterial,
};
use core_resources::{ModelName, ModelRegistry};

mod camera;
mod textures;
mod vertex_painter;

// ─── Spuštění ─────────────────────────────────────────────────────────────────

fn main() {
    // Přeskočíme první arg (název exe) a filtrujeme cargo/OS flagy.
    // Když někdo omylem spustí binárku s `--features dynamic_linking`,
    // ignorujeme i následující hodnotu, aby se nebrala jako cesta modelu.
    let mut model_paths: Vec<PathBuf> = Vec::new();
    let mut skip_next = false;
    for arg in std::env::args().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--features" || arg == "-F" {
            skip_next = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        model_paths.push(PathBuf::from(arg));
    }

    let title = match model_paths.as_slice() {
        [] => "ADS Model Viewer".to_string(),
        [p] => format!(
            "ADS Viewer — {}",
            p.file_stem().and_then(|s| s.to_str()).unwrap_or("model")
        ),
        paths => format!("ADS Viewer — {} modelů", paths.len()),
    };

    App::new()
        .init_resource::<WeatherState>()
        .init_resource::<LodViewerState>()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title,
                        resolution: WindowResolution::new(1280, 720),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    // Dev: sdílíme shadery z host_client/assets.
                    // Produkce: assets/ vedle exe musí obsahovat adresář shaders/.
                    file_path: viewer_asset_root(),
                    unapproved_path_mode: UnapprovedPathMode::Allow,
                    ..default()
                }),
        )
        .add_plugins(DrawablePlugin)
        .add_plugins(vertex_painter::VertexPainterPlugin)
        .init_resource::<ModelRegistry>()
        .init_resource::<camera::OrbitState>()
        .init_resource::<textures::TextureBrowser>()
        .insert_resource(ViewerState::default())
        .insert_resource(ModelPaths(model_paths))
        .add_systems(Startup, (setup_scene, load_models.after(setup_scene)))
        .add_systems(
            Update,
            (
                camera::orbit_camera,
                handle_keyboard,
                handle_lod_key,
                handle_material_debug,
                draw_grid,
                draw_colliders,
                draw_lod_circles,
                sync_mesh_visibility_for_collider_mode,
                update_info_overlay,
                update_collider_panel,
                textures::init_texture_browser,
                textures::handle_texture_keys,
                textures::rebuild_panel,
                textures::show_extract_status,
            ),
        )
        // apply_forced_lod must run AFTER update_lod_visibility (which lives in Update via DrawablePlugin).
        // PostUpdate is always after Update, so this ordering is guaranteed without ambiguous SystemTypeSet.
        .add_systems(PostUpdate, (
            apply_forced_lod,
            update_lod_panel.after(apply_forced_lod),
        ))
        .run();
}

// ─── Resources ────────────────────────────────────────────────────────────────

#[derive(Resource)]
struct ModelPaths(Vec<PathBuf>);

#[derive(Resource, Default)]
struct LodViewerState {
    forced: Option<u8>,
    panel_active: bool,
}

#[derive(Resource)]
struct ViewerState {
    grid_visible:      bool,
    overlay_visible:   bool,
    colliders_visible: bool,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self { grid_visible: true, overlay_visible: true, colliders_visible: false }
    }
}

/// Presety pro weather efekty (snow, dirt, wetness, porosity).
const WEATHER_PRESETS: &[(f32, f32, f32, f32, &str)] = &[
    (0.0, 0.0, 0.0, 0.0, "clean"),
    (0.0, 1.0, 0.0, 0.5, "dirty"),
    (0.0, 0.0, 1.0, 0.8, "wet"),
    (1.0, 0.0, 0.0, 0.0, "snowy"),
    (0.2, 0.6, 0.3, 0.5, "combined"),
];

/// Debug kanály pro vertex color visualizaci (tiling.y kódování pro shader).
/// 0.0 = normální render; záporné hodnoty → debug mode.
const VCOL_DEBUG_MODES: &[(f32, &str)] = &[
    ( 1.0, "normal"),
    (-1.0, "vcol RGB"),
    (-2.0, "vcol R (normal suppress)"),
    (-3.0, "vcol G (dirt)"),
    (-4.0, "vcol B (wet)"),
    (-5.0, "vcol A (palette)"),
];

#[derive(Resource, Default)]
struct WeatherState {
    preset_idx: usize,
    vcol_idx:   usize,
}

// ─── Asset root ───────────────────────────────────────────────────────────────

/// Dev:  CARGO_MANIFEST_DIR = model_viewer/ → sdílíme host_client/assets.
/// Prod: assets/ vedle exe.
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

// ─── Scene setup ──────────────────────────────────────────────────────────────

fn setup_scene(mut commands: Commands) {
    // Kamera
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(Vec3::new(0.0, 2.0, 5.0)).looking_at(Vec3::ZERO, Vec3::Y),
        camera::OrbitCamera,
    ));

    // Hlavní světlo (key light — zleva shora)
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.5, 0.0)),
    ));

    // Fill light (zprava zdola, bez stínů — eliminuje černé stěny)
    commands.spawn((
        DirectionalLight {
            illuminance: 3_500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.5, 0.5 + std::f32::consts::PI, 0.0)),
    ));

    // Rim light (zezadu shora — oddělí model od pozadí)
    commands.spawn((
        DirectionalLight {
            illuminance: 1_500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.4, 0.5 + std::f32::consts::PI * 0.5, 0.0)),
    ));

    // Ambientní složka — v Bevy 0.18 je Component, ne Resource
    commands.spawn(AmbientLight {
        color: Color::WHITE,
        brightness: 600.0,
        ..default()
    });

    // Help text (dole vlevo)
    commands.spawn((
        Text::new(
            "Pravé drag: orbit  |  Střední drag: pan  |  Kolečko: zoom  \
             |  R: reset  |  G: mřížka  |  H: info  |  T: textury  |  E: export\n\
             W: weather preset  |  V: vertex color debug  |  P: vertex paint  |  C: collidery  |  L: LOD úroveň",
        ),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.65)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));

    // Info overlay (nahoře vlevo)
    commands.spawn((
        Text::new("Načítám model…"),
        TextFont { font_size: 14.0, ..default() },
        TextColor(Color::srgba(0.9, 0.9, 0.9, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
        InfoOverlay,
    ));

    // Debug status (nahoře vpravo) — weather preset + vcol debug
    let (_, _, _, _, w_name) = WEATHER_PRESETS[0];
    let (_, v_name)          = VCOL_DEBUG_MODES[0];
    commands.spawn((
        Text::new(format!("Weather: {}  |  VCol: {}", w_name, v_name)),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(1.0, 1.0, 0.6, 0.80)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(8.0),
            ..default()
        },
        DebugStatus,
    ));

    // Collider info panel (nahoře vpravo, pod debug status) — skrytý dokud se nestiskne C
    commands.spawn((
        Text::new(""),
        TextFont { font_size: 12.5, ..default() },
        TextColor(Color::srgba(0.3, 1.0, 0.6, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(30.0),
            right: Val::Px(8.0),
            max_width: Val::Px(440.0),
            ..default()
        },
        Visibility::Hidden,
        ColliderPanel,
    ));

    // LOD info panel (dole vpravo) — zobrazí se pokud model má LOD skupiny
    commands.spawn((
        Text::new(""),
        TextFont { font_size: 12.5, ..default() },
        TextColor(Color::srgba(0.4, 0.9, 1.0, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(50.0),
            right: Val::Px(8.0),
            max_width: Val::Px(500.0),
            ..default()
        },
        Visibility::Hidden,
        LodPanel,
    ));
}

// ─── Markery ──────────────────────────────────────────────────────────────────

#[derive(Component)]
struct InfoOverlay;

#[derive(Component)]
struct DebugStatus;

#[derive(Component)]
struct ColliderPanel;

#[derive(Component)]
struct LodPanel;

// ─── Model loading ────────────────────────────────────────────────────────────

fn load_models(
    paths: Res<ModelPaths>,
    asset_server: Res<AssetServer>,
    mut drawable_reg: ResMut<DrawableManifestRegistry>,
    mut gltf_cache: ResMut<GltfHandleCache>,
    mut model_reg: ResMut<ModelRegistry>,
    mut commands: Commands,
    mut overlay: Query<&mut Text, With<InfoOverlay>>,
) {
    if paths.0.is_empty() {
        if let Ok(mut txt) = overlay.single_mut() {
            txt.0 = "Žádný model.\nSpusť: model_viewer cesta/k/modelu.glb".to_string();
        }
        return;
    }

    for path in &paths.0 {
        load_one_model(
            path,
            &asset_server,
            &mut drawable_reg,
            &mut gltf_cache,
            &mut model_reg,
            &mut commands,
        );
    }

    if let Ok(mut txt) = overlay.single_mut() {
        let names: Vec<_> = paths
            .0
            .iter()
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()))
            .collect();
        txt.0 = format!("Modely: {}", names.join(", "));
    }
}

fn load_one_model(
    path: &PathBuf,
    asset_server: &AssetServer,
    drawable_reg: &mut DrawableManifestRegistry,
    gltf_cache: &mut GltfHandleCache,
    model_reg: &mut ModelRegistry,
    commands: &mut Commands,
) {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            warn!("[viewer] nelze zpřesnit cestu {:?}: {}", path, e);
            return;
        }
    };

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();

    // Windows backslash → forward slash
    let bevy_path = path.to_string_lossy().replace('\\', "/");

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    if ext == "adm" {
        // Load as AdmScene
        let adm_handle: Handle<AdmScene> = asset_server.load(bevy_path.clone());
        model_reg.register_native(stem.clone(), bevy_path.clone());

        // Auto-detect .drawable manifest beside .adm
        let drawable_path = path.with_extension("drawable");
        if drawable_path.exists() {
            let dp = drawable_path.to_string_lossy().replace('\\', "/");
            let handle = asset_server.load(dp.clone());
            drawable_reg.0.insert(stem.clone(), handle);
            info!("[viewer] drawable manifest: '{}'", dp);
        } else {
            info!("[viewer] žádný .drawable pro '{}', použijí se výchozí materiály", stem);
        }

        commands.spawn((AdmSceneRoot(adm_handle), Transform::default(), ModelName(stem)));
    } else {
        // GLB / GLTF path
        let gltf_handle: Handle<bevy::gltf::Gltf> = asset_server.load_with_settings(
            bevy_path.clone(),
            |s: &mut GltfLoaderSettings| {
                s.include_source = true;
            },
        );
        gltf_cache.0.insert(stem.clone(), (gltf_handle, bevy_path.clone()));
        model_reg.register_native(stem.clone(), bevy_path.clone());

        // Auto-detekce .drawable vedle .glb
        let drawable_path = path.with_extension("drawable");
        if drawable_path.exists() {
            let dp = drawable_path.to_string_lossy().replace('\\', "/");
            let handle = asset_server.load(dp.clone());
            drawable_reg.0.insert(stem.clone(), handle);
            info!("[viewer] drawable manifest: '{}'", dp);
        } else {
            info!("[viewer] žádný .drawable pro '{}', použijí se výchozí materiály", stem);
        }

        // Spawn scene
        let scene_path = format!("{}#Scene0", bevy_path);
        let scene_handle: Handle<Scene> = asset_server.load(scene_path);

        commands.spawn((SceneRoot(scene_handle), Transform::default(), ModelName(stem)));
    }
}

// ─── Info overlay ─────────────────────────────────────────────────────────────

fn update_info_overlay(
    mut done: Local<bool>,
    drawable_reg: Res<DrawableManifestRegistry>,
    gltf_cache:   Res<GltfHandleCache>,
    manifests:    Res<Assets<DrawableManifest>>,
    gltf_assets:  Res<Assets<Gltf>>,
    mut overlay:  Query<&mut Text, With<InfoOverlay>>,
) {
    if *done { return; }

    // Čekáme dokud nejsou načteny GLB i drawable (aspoň jeden model).
    // Pro .adm soubory gltf_cache bude prázdný — v tom případě přeskočíme GLB check.
    let all_ready = gltf_cache.0.values().all(|(h, _)| gltf_assets.get(h).is_some());
    if !gltf_cache.0.is_empty() && !all_ready { return; }
    // Pokud jsou obě caches prázdné, nechceme nic zobrazovat.
    if gltf_cache.0.is_empty() && drawable_reg.0.is_empty() { return; }

    let mut lines: Vec<String> = Vec::new();

    for (stem, (gltf_handle, _path)) in &gltf_cache.0 {
        let Some(gltf) = gltf_assets.get(gltf_handle) else { continue };

        // ── GLB statistiky ────────────────────────────────────────────────────
        let tex_count = gltf.source.as_ref().map(|s| s.textures().count()).unwrap_or(0);
        let anim_count = gltf.source.as_ref().map(|s| s.animations().count()).unwrap_or(0);
        lines.push(format!("── {} ──", stem));
        lines.push(format!(
            "GLB  meshes:{} materials:{} textures:{} nodes:{} animations:{}",
            gltf.meshes.len(),
            gltf.materials.len(),
            tex_count,
            gltf.nodes.len(),
            anim_count,
        ));

        // ── Drawable manifest ─────────────────────────────────────────────────
        if let Some(manifest_handle) = drawable_reg.0.get(stem) {
            if let Some(manifest) = manifests.get(manifest_handle) {
                lines.push(format!(
                    "Drawable  asset:{} v{}",
                    manifest.asset_name, manifest.version
                ));

                // Entity summary
                let mesh_count = manifest.entities.values()
                    .filter(|e| matches!(e, EntityDef::MESH { .. })).count();
                let col_count = manifest.entities.values()
                    .filter(|e| matches!(e, EntityDef::COLLISION { .. })).count();
                if !manifest.entities.is_empty() {
                    lines.push(format!("Entities  MESH:{} COLLISION:{}", mesh_count, col_count));
                }

                // Per-material blocks
                let mut mat_names: Vec<&String> = manifest.materials.keys().collect();
                mat_names.sort();
                for mat_name in mat_names {
                    let mat = &manifest.materials[mat_name];
                    lines.push(format!("  [{}]  template:{}", mat_name, mat.template));

                    // Textures
                    let mut tex_names: Vec<&String> = mat.textures.keys().collect();
                    tex_names.sort();
                    for slot in tex_names {
                        let tex = &mat.textures[slot];
                        let src = match tex.source {
                            TextureSource::Embedded => "embedded",
                            TextureSource::Shared => "shared",
                        };
                        lines.push(format!("    {} → {} ({})", slot, tex.name, src));
                    }

                    // Non-default params
                    let p = &mat.params;
                    let mut params: Vec<String> = Vec::new();
                    if let Some(t) = p.tiling       { params.push(format!("tiling:{:.2}", t)); }
                    if let Some(t) = p.l0_tiling     { params.push(format!("l0_tiling:{:.2}", t)); }
                    if let Some(t) = p.l1_tiling     { params.push(format!("l1_tiling:{:.2}", t)); }
                    if let Some(v) = p.snow_level    { params.push(format!("snow:{:.2}", v)); }
                    if let Some(v) = p.dirt_level    { params.push(format!("dirt:{:.2}", v)); }
                    if let Some(v) = p.wetness       { params.push(format!("wet:{:.2}", v)); }
                    if let Some(v) = p.porosity      { params.push(format!("porosity:{:.2}", v)); }
                    if let Some(c) = p.tint {
                        params.push(format!("tint:[{:.2},{:.2},{:.2},{:.2}]", c[0], c[1], c[2], c[3]));
                    }
                    if let Some(m) = &p.opacity_mode {
                        let at = p.alpha_threshold.map(|v| format!("@{:.2}", v)).unwrap_or_default();
                        params.push(format!("alpha:{}{}", m, at));
                    }
                    if !params.is_empty() {
                        lines.push(format!("    params: {}", params.join("  ")));
                    }
                }
            }
        } else {
            lines.push("Drawable  (žádný manifest)".to_string());
        }
    }

    if let Ok(mut txt) = overlay.single_mut() {
        txt.0 = lines.join("\n");
    }
    *done = true;
}

// ─── Mřížka (Gizmos) ──────────────────────────────────────────────────────────

fn draw_grid(mut gizmos: Gizmos, state: Res<ViewerState>) {
    if !state.grid_visible {
        return;
    }
    let half: i32 = 10;
    let gray = Color::srgba(0.45, 0.45, 0.45, 0.45);
    for i in -half..=half {
        let f = i as f32;
        gizmos.line(Vec3::new(f, 0.0, -(half as f32)), Vec3::new(f, 0.0, half as f32), gray);
        gizmos.line(Vec3::new(-(half as f32), 0.0, f), Vec3::new(half as f32, 0.0, f), gray);
    }
    gizmos.line(
        Vec3::new(-(half as f32), 0.001, 0.0),
        Vec3::new(half as f32, 0.001, 0.0),
        Color::srgb(0.8, 0.2, 0.2),
    );
    gizmos.line(
        Vec3::new(0.0, 0.001, -(half as f32)),
        Vec3::new(0.0, 0.001, half as f32),
        Color::srgb(0.2, 0.2, 0.8),
    );
}

// ─── Klávesové zkratky ────────────────────────────────────────────────────────

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut cam: ResMut<camera::OrbitState>,
    mut state: ResMut<ViewerState>,
    mut overlay: Query<&mut Visibility, With<InfoOverlay>>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        *cam = camera::OrbitState::default();
    }
    if keys.just_pressed(KeyCode::KeyG) {
        state.grid_visible = !state.grid_visible;
    }
    if keys.just_pressed(KeyCode::KeyH) {
        state.overlay_visible = !state.overlay_visible;
        for mut vis in &mut overlay {
            *vis = if state.overlay_visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
    if keys.just_pressed(KeyCode::KeyF) {
        cam.focus    = Vec3::ZERO;
        cam.distance = 2.5;
        cam.yaw      = -0.5;
        cam.pitch    = 0.35;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        state.colliders_visible = !state.colliders_visible;
    }
}

// ─── Collider vizualizace ─────────────────────────────────────────────────────

/// Při vstupu do collider módu skryje všechny normální mesh entity.
/// Při výstupu je znovu zobrazí (s výjimkou COL_ uzlů — ty zůstanou skryté).
fn sync_mesh_visibility_for_collider_mode(
    state: Res<ViewerState>,
    mut last: Local<bool>,
    mut meshes: Query<(&mut Visibility, Option<&Name>), (With<Mesh3d>, Without<DrawableCollision>)>,
) {
    if *last == state.colliders_visible { return; }
    *last = state.colliders_visible;

    if state.colliders_visible {
        for (mut vis, _) in &mut meshes {
            *vis = Visibility::Hidden;
        }
    } else {
        for (mut vis, name) in &mut meshes {
            let is_col = name.map(|n| n.as_str().starts_with("COL_")).unwrap_or(false);
            if !is_col {
                *vis = Visibility::Inherited;
            }
        }
    }
}

fn draw_colliders(
    mut gizmos: Gizmos,
    state: Res<ViewerState>,
    query: Query<(&DrawableCollision, &GlobalTransform, Option<&Mesh3d>)>,
    mesh_assets: Res<Assets<Mesh>>,
) {
    if !state.colliders_visible { return; }
    for (dc, gt, mesh3d) in &query {
        let (scale, rot, center) = gt.to_scale_rotation_translation();
        let color = collider_color(dc);
        match &dc.shape {
            CollisionShape::Box => {
                let he = dc.half_extents.unwrap_or(Vec3::splat(0.5)) * scale;
                draw_box_gizmo(&mut gizmos, center, rot, he, color);
            }
            CollisionShape::Mesh | CollisionShape::Convex => {
                if !draw_mesh_wireframe(&mut gizmos, mesh3d, &mesh_assets, gt, color) {
                    // Fallback to AABB box if mesh data not available
                    let he = dc.half_extents.unwrap_or(Vec3::splat(0.5)) * scale;
                    draw_box_gizmo(&mut gizmos, center, rot, he, color);
                }
            }
            CollisionShape::Navmesh => {
                if !draw_mesh_wireframe(&mut gizmos, mesh3d, &mesh_assets, gt, Color::srgb(0.95, 0.15, 0.8)) {
                    let he = dc.half_extents.unwrap_or(Vec3::splat(0.5)) * scale;
                    draw_box_gizmo(&mut gizmos, center, rot, he, Color::srgb(0.95, 0.15, 0.8));
                }
            }
            CollisionShape::Sphere => {
                let r = dc.radius.unwrap_or(0.5) * scale.max_element();
                draw_sphere_gizmo(&mut gizmos, center, rot, r, color);
            }
            CollisionShape::Capsule => {
                let radial_scale = scale.x.max(scale.z);
                let r = dc.radius.unwrap_or(0.3) * radial_scale;
                let half_seg = dc.height
                    .map(|h| (h * 0.5 * scale.y - r).max(0.0))
                    .unwrap_or(0.5);
                draw_capsule_gizmo(&mut gizmos, center, rot, r, half_seg, color);
            }
            CollisionShape::Cylinder => {
                let radial_scale = scale.x.max(scale.z);
                let r = dc.radius.unwrap_or(0.5) * radial_scale;
                let half_h = dc.height.map(|h| h * 0.5 * scale.y).unwrap_or(0.5);
                draw_cylinder_gizmo(&mut gizmos, center, rot, r, half_h, color);
            }
        }
    }
}

/// Draws the actual mesh geometry as wireframe. Returns false if mesh data is unavailable.
fn draw_mesh_wireframe(
    gizmos: &mut Gizmos,
    mesh3d: Option<&Mesh3d>,
    mesh_assets: &Assets<Mesh>,
    gt: &GlobalTransform,
    color: Color,
) -> bool {
    let Some(mesh_h) = mesh3d else { return false };
    let Some(mesh) = mesh_assets.get(mesh_h.id()) else { return false };

    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return false };

    let indices: Vec<u32> = match mesh.indices() {
        Some(Indices::U32(idx)) => idx.clone(),
        Some(Indices::U16(idx)) => idx.iter().map(|&i| i as u32).collect(),
        None => return false,
    };

    use std::collections::HashSet;
    let mut drawn: HashSet<(u32, u32)> = HashSet::new();

    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]];
        for &(i, j) in &[(a, b), (b, c), (c, a)] {
            let edge = if i < j { (i, j) } else { (j, i) };
            if drawn.insert(edge) {
                let pa = gt.transform_point(Vec3::from(positions[i as usize]));
                let pb = gt.transform_point(Vec3::from(positions[j as usize]));
                gizmos.line(pa, pb, color);
            }
        }
    }

    !drawn.is_empty()
}

fn collider_color(dc: &DrawableCollision) -> Color {
    if dc.ladder         { Color::srgb(1.00, 0.85, 0.00) }
    else if dc.climbable { Color::srgb(0.00, 1.00, 0.40) }
    else if !dc.is_static { Color::srgb(1.00, 0.40, 0.10) }
    else                 { Color::srgb(0.10, 0.80, 1.00) }
}

fn update_collider_panel(
    state: Res<ViewerState>,
    colliders: Query<(&DrawableCollision, &GlobalTransform)>,
    mut panel: Query<(&mut Text, &mut Visibility), With<ColliderPanel>>,
) {
    let Ok((mut txt, mut vis)) = panel.single_mut() else { return };
    if !state.colliders_visible {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Visible;

    let mut lines: Vec<String> = vec![
        "── Collidery (cyan=static  orange=dynamic  green=climbable  yellow=ladder) ──".to_string(),
    ];

    let items: Vec<_> = colliders.iter()
        .map(|(dc, gt)| (dc, gt.translation()))
        .collect();

    if items.is_empty() {
        lines.push("(žádné DrawableCollision entity)".to_string());
        lines.push("Model vyžaduje .drawable nebo .adm manifest.".to_string());
    } else {
        for (i, (dc, pos)) in items.iter().enumerate() {
            let size_str = match &dc.shape {
                CollisionShape::Box | CollisionShape::Convex | CollisionShape::Mesh | CollisionShape::Navmesh => {
                    let he = dc.half_extents.unwrap_or(Vec3::splat(0.5));
                    format!("he:[{:.2},{:.2},{:.2}]", he.x, he.y, he.z)
                }
                CollisionShape::Sphere => format!("r:{:.2}", dc.radius.unwrap_or(0.5)),
                CollisionShape::Capsule | CollisionShape::Cylinder => format!(
                    "r:{:.2}  h:{:.2}", dc.radius.unwrap_or(0.3), dc.height.unwrap_or(1.0)
                ),
            };
            let mut flags = vec![if dc.is_static { "static" } else { "dynamic" }];
            if dc.climbable { flags.push("climbable"); }
            if dc.ladder    { flags.push("ladder"); }

            lines.push(format!(
                "[{}] {:?}  {:?}  {}",
                i + 1, dc.shape, dc.material, flags.join("+")
            ));
            lines.push(format!(
                "    {}  @ [{:.2},{:.2},{:.2}]",
                size_str, pos.x, pos.y, pos.z
            ));
            if dc.friction != 0.0 || dc.restitution != 0.0 || dc.mass != 0.0 {
                lines.push(format!(
                    "    frict:{:.2}  rest:{:.2}  mass:{:.1}",
                    dc.friction, dc.restitution, dc.mass
                ));
            }
            if !dc.tags.is_empty() {
                lines.push(format!("    tags: {}", dc.tags.join(", ")));
            }
        }
    }
    txt.0 = lines.join("\n");
}

// ─── Gizmo helpers ────────────────────────────────────────────────────────────

fn draw_box_gizmo(gizmos: &mut Gizmos, center: Vec3, rot: Quat, half: Vec3, color: Color) {
    let c = [
        center + rot * Vec3::new(-half.x, -half.y, -half.z),
        center + rot * Vec3::new( half.x, -half.y, -half.z),
        center + rot * Vec3::new( half.x,  half.y, -half.z),
        center + rot * Vec3::new(-half.x,  half.y, -half.z),
        center + rot * Vec3::new(-half.x, -half.y,  half.z),
        center + rot * Vec3::new( half.x, -half.y,  half.z),
        center + rot * Vec3::new( half.x,  half.y,  half.z),
        center + rot * Vec3::new(-half.x,  half.y,  half.z),
    ];
    // Přední a zadní plocha
    gizmos.line(c[0], c[1], color); gizmos.line(c[1], c[2], color);
    gizmos.line(c[2], c[3], color); gizmos.line(c[3], c[0], color);
    gizmos.line(c[4], c[5], color); gizmos.line(c[5], c[6], color);
    gizmos.line(c[6], c[7], color); gizmos.line(c[7], c[4], color);
    // Boční hrany
    gizmos.line(c[0], c[4], color); gizmos.line(c[1], c[5], color);
    gizmos.line(c[2], c[6], color); gizmos.line(c[3], c[7], color);
}

fn draw_sphere_gizmo(gizmos: &mut Gizmos, center: Vec3, rot: Quat, radius: f32, color: Color) {
    let (rx, ry, rz) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
    draw_arc_gizmo(gizmos, center, rx, ry, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, center, rx, rz, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, center, ry, rz, radius, 0.0, std::f32::consts::TAU, 24, color);
}

fn draw_cylinder_gizmo(
    gizmos: &mut Gizmos, center: Vec3, rot: Quat,
    radius: f32, half_h: f32, color: Color,
) {
    let (up, right, fwd) = (rot * Vec3::Y, rot * Vec3::X, rot * Vec3::Z);
    let (bot, top) = (center - up * half_h, center + up * half_h);
    draw_arc_gizmo(gizmos, bot, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, top, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    gizmos.line(bot + right * radius, top + right * radius, color);
    gizmos.line(bot - right * radius, top - right * radius, color);
    gizmos.line(bot + fwd   * radius, top + fwd   * radius, color);
    gizmos.line(bot - fwd   * radius, top - fwd   * radius, color);
}

fn draw_capsule_gizmo(
    gizmos: &mut Gizmos, center: Vec3, rot: Quat,
    radius: f32, half_seg: f32, color: Color,
) {
    let (up, right, fwd) = (rot * Vec3::Y, rot * Vec3::X, rot * Vec3::Z);
    let (bot, top) = (center - up * half_seg, center + up * half_seg);
    // Válec
    draw_arc_gizmo(gizmos, bot, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, top, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    gizmos.line(bot + right * radius, top + right * radius, color);
    gizmos.line(bot - right * radius, top - right * radius, color);
    gizmos.line(bot + fwd   * radius, top + fwd   * radius, color);
    gizmos.line(bot - fwd   * radius, top - fwd   * radius, color);
    // Horní polokoule (2 oblouky)
    draw_arc_gizmo(gizmos, top, right,  up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, top, fwd,    up, radius, 0.0, std::f32::consts::PI, 12, color);
    // Dolní polokoule (2 oblouky)
    draw_arc_gizmo(gizmos, bot, right, -up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, bot, fwd,   -up, radius, 0.0, std::f32::consts::PI, 12, color);
}

/// Oblouk v rovině `(t1, t2)` od `start` do `end` radiánů.
fn draw_arc_gizmo(
    gizmos: &mut Gizmos,
    center: Vec3, t1: Vec3, t2: Vec3,
    radius: f32, start: f32, end: f32, segments: u32,
    color: Color,
) {
    let step = (end - start) / segments as f32;
    let mut prev = center + (t1 * start.cos() + t2 * start.sin()) * radius;
    for i in 1..=segments {
        let a = start + i as f32 * step;
        let next = center + (t1 * a.cos() + t2 * a.sin()) * radius;
        gizmos.line(prev, next, color);
        prev = next;
    }
}

// ─── LOD viewer ───────────────────────────────────────────────────────────────

/// L = zobrazí panel / cykluje auto → LOD0 → LOD1 → ... → auto
fn handle_lod_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut lod_state: ResMut<LodViewerState>,
    lod_groups: Query<&LodGroup>,
) {
    if !keys.just_pressed(KeyCode::KeyL) { return; }

    lod_state.panel_active = true;

    let max_levels = lod_groups.iter()
        .map(|g| g.lod_entities.len())
        .max()
        .unwrap_or(0);

    if max_levels == 0 { return; } // panel shows "0 skupin" message

    lod_state.forced = match lod_state.forced {
        None => Some(0),
        Some(n) => {
            let next = n as usize + 1;
            if next >= max_levels { None } else { Some(next as u8) }
        }
    };
}

/// Přepíše viditelnost LOD entit na vybrané úrovni; resetuje active_lod na u8::MAX
/// aby update_lod_visibility vždy přepočítalo při přepnutí zpět do auto módu.
fn apply_forced_lod(
    lod_state: Res<LodViewerState>,
    mut lod_groups: Query<&mut LodGroup>,
    mut visibility_q: Query<&mut Visibility>,
) {
    let Some(forced) = lod_state.forced else { return };

    for mut lod in &mut lod_groups {
        lod.active_lod = u8::MAX; // Invalidate so auto mode recalculates next frame
        for (level_idx, entities) in lod.lod_entities.iter().enumerate() {
            let vis = if level_idx as u8 == forced {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            for &entity in entities {
                if let Ok(mut v) = visibility_q.get_mut(entity) {
                    *v = vis;
                }
            }
        }
    }
}

fn update_lod_panel(
    mut lod_state: ResMut<LodViewerState>,
    lod_groups: Query<&LodGroup>,
    mut panel: Query<(&mut Text, &mut Visibility), With<LodPanel>>,
) {
    let Ok((mut txt, mut vis)) = panel.single_mut() else { return };

    let groups: Vec<&LodGroup> = lod_groups.iter().collect();

    // Auto-show panel as soon as any LOD group is detected
    if !groups.is_empty() {
        lod_state.panel_active = true;
    }

    if !lod_state.panel_active {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Visible;

    if groups.is_empty() {
        txt.0 = "── LOD ──\nŽádné LOD skupiny (model nemá _LOD1+ uzly)".to_string();
        return;
    }

    let total_levels: usize = groups.iter().map(|g| g.lod_entities.len()).sum();
    let mode_str = match lod_state.forced {
        None => "auto".to_string(),
        Some(n) => format!("LOD{} vynuceno", n),
    };
    let mut lines = vec![
        format!("── LOD ({}) ──", mode_str),
        format!("Skupiny: {}  celkem úrovní: {}", groups.len(), total_levels),
    ];

    for (i, group) in groups.iter().enumerate() {
        let num_levels = group.lod_entities.len();
        let dists: Vec<String> = group.lod_dist_sq.iter()
            .map(|&d| format!("{:.0}m", d.sqrt()))
            .collect();
        let active_str = if group.active_lod == u8::MAX {
            "—".to_string()
        } else {
            format!("LOD{}", group.active_lod)
        };
        lines.push(format!(
            "  [{i}] {num_levels} úrovní  vzdálenosti:[{}]  aktivní:{active_str}{}",
            if dists.is_empty() { "výchozí".to_string() } else { dists.join(", ") },
            if group.cull_beyond_last { "  cull" } else { "" },
        ));
    }

    txt.0 = lines.join("\n");
}

/// Kreslí kruhy ve vzdálenostech LOD přechodů jako orientační gizma.
fn draw_lod_circles(
    mut gizmos: Gizmos,
    lod_groups: Query<(&GlobalTransform, &LodGroup)>,
) {
    const LOD_COLORS: [Color; 3] = [
        Color::srgb(0.2, 1.0, 0.2),  // LOD0→1 zelená
        Color::srgb(1.0, 1.0, 0.2),  // LOD1→2 žlutá
        Color::srgb(1.0, 0.4, 0.2),  // LOD2→3 oranžová
    ];

    for (gt, lod) in &lod_groups {
        let center = gt.translation();
        for (i, &dist_sq) in lod.lod_dist_sq.iter().enumerate() {
            let radius = dist_sq.sqrt();
            let color = LOD_COLORS.get(i).copied()
                .unwrap_or(Color::srgb(0.8, 0.2, 0.8));
            draw_arc_gizmo(
                &mut gizmos,
                Vec3::new(center.x, 0.001, center.z),
                Vec3::X, Vec3::Z,
                radius, 0.0, std::f32::consts::TAU, 64,
                color,
            );
        }
    }
}

/// W = weather presety, V = vertex color debug kanály.
fn handle_material_debug(
    keys:          Res<ButtonInput<KeyCode>>,
    mut weather:   ResMut<WeatherState>,
    mut std_mats:  ResMut<Assets<StandardPbrMaterial>>,
    mut env_mats:  ResMut<Assets<LayeredEnvMaterial>>,
    mut status:    Query<&mut Text, With<DebugStatus>>,
) {
    let w_pressed = keys.just_pressed(KeyCode::KeyW);
    let v_pressed = keys.just_pressed(KeyCode::KeyV);
    if !w_pressed && !v_pressed {
        return;
    }

    if w_pressed {
        weather.preset_idx = (weather.preset_idx + 1) % WEATHER_PRESETS.len();
    }
    if v_pressed {
        weather.vcol_idx = (weather.vcol_idx + 1) % VCOL_DEBUG_MODES.len();
    }

    let (snow, dirt, wet, por, weather_name) = WEATHER_PRESETS[weather.preset_idx];
    let (tiling_y, vcol_name)               = VCOL_DEBUG_MODES[weather.vcol_idx];
    let weather_vec = Vec4::new(snow, dirt, wet, por);

    for (_, mat) in std_mats.iter_mut() {
        mat.extension.params.weather  = weather_vec;
        mat.extension.params.tiling.y = tiling_y;
    }
    for (_, mat) in env_mats.iter_mut() {
        mat.extension.params.weather  = weather_vec;
    }

    if let Ok(mut txt) = status.single_mut() {
        txt.0 = format!("Weather: {}  |  VCol: {}", weather_name, vcol_name);
    }
}
