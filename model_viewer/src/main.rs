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

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use bevy::asset::UnapprovedPathMode;
use bevy::ecs::hierarchy::ChildOf;
use bevy::gltf::{Gltf, GltfLoaderSettings};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy::window::WindowResolution;
use rfd::FileDialog;

use core_drawable::{
    AdmScene, AdmSceneRoot, AnimationSet,
    AdsNodeKind,
    CollisionShape, DrawableCollision,
    LodGroup,
    DrawableManifest, DrawableManifestRegistry, DrawablePlugin, EntityDef,
    TextureSource,
    StandardPbrMaterial, LayeredEnvMaterial,
};
use core_resources::{AnimationState, AttachedAnimSets, ModelName, ModelRegistry};

mod camera;
mod state;
mod scene;
mod loader;
mod animation;
mod runtime;
mod textures;
mod vertex_painter;

pub use state::GltfHandleCache;

use state::*;

fn main() {
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
                    file_path: scene::viewer_asset_root(),
                    unapproved_path_mode: UnapprovedPathMode::Allow,
                    ..default()
                }),
        )
        .add_plugins(DrawablePlugin)
        .add_plugins(vertex_painter::VertexPainterPlugin)
        .init_resource::<ModelRegistry>()
        .init_resource::<camera::OrbitState>()
        .init_resource::<textures::TextureBrowser>()
        .init_resource::<AdmAnimationBrowser>()
        .init_resource::<RigViewerState>()
        .init_resource::<ViewerSessionAnimHandles>()
        .init_resource::<AnimDebugState>()
        .insert_resource(ViewerState::default())
        .insert_resource(ModelPaths(model_paths))
        .add_systems(Startup, (scene::setup_scene, loader::load_models.after(scene::setup_scene)))
        .add_systems(
            Update,
            (
                camera::orbit_camera,
                loader::handle_loader_buttons,
                runtime::handle_keyboard,
                animation::handle_animation_keyboard,
                runtime::sync_rig_viewer_state,
                runtime::handle_rig_keyboard,
                runtime::handle_lod_key,
                runtime::handle_material_debug,
                runtime::draw_grid,
                runtime::draw_colliders,
                runtime::draw_skeleton_overlay,
                runtime::draw_lod_circles,
                runtime::sync_mesh_visibility_for_collider_mode,
                runtime::update_info_overlay,
                animation::sync_adm_animation_browser,
                animation::apply_animation_browser_state,
                animation::update_animation_overlay,
                animation::update_anim_debug_overlay,
                runtime::update_rig_overlay,
                runtime::update_collider_panel,
            ),
        )
        .add_systems(
            PostUpdate,
            (
                runtime::apply_forced_lod,
                runtime::update_lod_panel.after(runtime::apply_forced_lod),
            ),
        )
        .run();
}

// ─── Info overlay ─────────────────────────────────────────────────────────────


