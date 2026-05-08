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

use std::path::PathBuf;

use bevy::asset::UnapprovedPathMode;
use bevy::gltf::{Gltf, GltfLoaderSettings};
use bevy::prelude::*;
use bevy::window::WindowResolution;

use core_drawable::{
    DrawableManifest, DrawableManifestRegistry, DrawablePlugin, EntityDef,
    GltfHandleCache, TextureSource,
};
use core_resources::{ModelName, ModelRegistry};

mod camera;
mod textures;

// ─── Spuštění ─────────────────────────────────────────────────────────────────

fn main() {
    // Přeskočíme první arg (název exe) a filtrujeme cargo/OS flagy (začínají '-').
    let model_paths: Vec<PathBuf> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .collect();

    let title = match model_paths.as_slice() {
        [] => "ADS Model Viewer".to_string(),
        [p] => format!(
            "ADS Viewer — {}",
            p.file_stem().and_then(|s| s.to_str()).unwrap_or("model")
        ),
        paths => format!("ADS Viewer — {} modelů", paths.len()),
    };

    App::new()
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
                draw_grid,
                update_info_overlay,
                textures::init_texture_browser,
                textures::handle_texture_keys,
                textures::rebuild_panel,
                textures::show_extract_status,
            ),
        )
        .run();
}

// ─── Resources ────────────────────────────────────────────────────────────────

#[derive(Resource)]
struct ModelPaths(Vec<PathBuf>);

#[derive(Resource)]
struct ViewerState {
    grid_visible:    bool,
    overlay_visible: bool,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self { grid_visible: true, overlay_visible: true }
    }
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

    // Sluneční světlo
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.5, 0.0)),
    ));

    // Ambientní složka — v Bevy 0.18 je Component, ne Resource
    commands.spawn(AmbientLight {
        color: Color::WHITE,
        brightness: 300.0,
        ..default()
    });

    // Help text (dole vlevo)
    commands.spawn((
        Text::new(
            "Pravé drag: orbit  |  Střední drag: pan  |  Kolečko: zoom  \
             |  R: reset  |  G: mřížka  |  H: info  |  T: textury  |  E: export",
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
}

// ─── Markery ──────────────────────────────────────────────────────────────────

#[derive(Component)]
struct InfoOverlay;

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

    // GLTF handle s include_source pro embedded texture extraction
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
    let all_ready = gltf_cache.0.values().all(|(h, _)| gltf_assets.get(h).is_some());
    if gltf_cache.0.is_empty() || !all_ready { return; }

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
}
