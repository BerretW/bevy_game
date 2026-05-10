//! Načítá nativní assety z `assets/` adresáře klienta do Bevy registrů.
//!
//! Kategorie:
//! - `assets/fonts/`        → `GuiFontRegistry`
//! - `assets/models/`       → `ModelRegistry`, `DrawableManifestRegistry`, `PedPhysicsRegistry`
//! - `assets/shaders/`      → přístupné přes Bevy AssetServer bez extra registrace
//!
//! Font ID = název souboru bez přípony (case-sensitive).
//! Model ID = název souboru bez přípony (case-sensitive).

use std::path::PathBuf;

use bevy::prelude::*;

use core_resources::ModelRegistry;

use crate::drawable::{AdmScene, DrawableManifestRegistry, GltfHandleCache};
use crate::drawable::{PedPhysicsDef, PedPhysicsRegistry};
use crate::gui_render::GuiFontRegistry;

/// Cache ADM assetů — analogie GltfHandleCache, ale pro .adm formát.
#[derive(Resource, Default)]
pub struct AdmHandleCache(pub std::collections::HashMap<String, Handle<AdmScene>>);

/// Udržuje silné handle na načítané `.ped.toml` assety dokud je nepřevezme PedPhysicsRegistry.
#[derive(Resource, Default)]
pub struct PedPhysicsHandleCache(pub Vec<Handle<PedPhysicsDef>>);

pub struct NativeAssetsPlugin;

impl Plugin for NativeAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AdmHandleCache>()
            .init_resource::<PedPhysicsHandleCache>()
            .add_systems(Startup, load_native_assets)
            .add_systems(Update, sync_ped_physics_registry);
    }
}

/// Mirrors Bevy's own asset root resolution:
/// - dev (`cargo run`): `$CARGO_MANIFEST_DIR/assets`
/// - production: `<exe_dir>/assets`
fn asset_root() -> PathBuf {
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        return PathBuf::from(manifest).join("assets");
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("assets")))
        .unwrap_or_else(|| PathBuf::from("assets"))
}

fn load_native_assets(
    asset_server: Res<AssetServer>,
    mut font_reg: ResMut<GuiFontRegistry>,
    mut model_reg: ResMut<ModelRegistry>,
    mut drawable_reg: ResMut<DrawableManifestRegistry>,
    mut gltf_cache: ResMut<GltfHandleCache>,
    mut adm_cache: ResMut<AdmHandleCache>,
    mut ped_cache: ResMut<PedPhysicsHandleCache>,
) {
    let root = asset_root();
    load_native_fonts(&asset_server, &mut font_reg, &root);
    register_native_models(&asset_server, &mut model_reg, &mut gltf_cache, &mut adm_cache, &root);
    load_native_drawables(&asset_server, &mut drawable_reg, &root);
    load_native_ped_physics(&asset_server, &mut ped_cache, &root);
}

/// Průběžně plní `PedPhysicsRegistry` z `PedPhysicsHandleCache`.
/// Čeká dokud není asset plně načtený, pak ho přesune do registry.
fn sync_ped_physics_registry(
    ped_assets: Res<Assets<PedPhysicsDef>>,
    handle_cache: Res<PedPhysicsHandleCache>,
    mut ped_reg: ResMut<PedPhysicsRegistry>,
) {
    for handle in &handle_cache.0 {
        if ped_reg.0.values().any(|h| h.id() == handle.id()) {
            continue; // Tento handle je už v registry
        }
        if let Some(ped) = ped_assets.get(handle) {
            let key = ped.identity.model.clone();
            if !ped_reg.0.contains_key(&key) {
                ped_reg.0.insert(key.clone(), handle.clone());
                info!("[native_assets] ped physics ready: '{}'", key);
            }
        }
    }
}

/// Skenuje `assets/models/*.ped.toml` a předplní `PedPhysicsHandleCache` silnými handle.
/// Handle musí být uložen — jinak Bevy asset okamžitě zruší načítání.
fn load_native_ped_physics(
    asset_server: &AssetServer,
    handle_cache: &mut PedPhysicsHandleCache,
    root: &PathBuf,
) {
    let model_dir = root.join("models");
    let Ok(entries) = std::fs::read_dir(&model_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        // Hledáme soubory se složenou příponou `.ped.toml`
        // → stem souboru = "<name>.ped", přípona = "toml"
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if s.ends_with(".ped") => s.trim_end_matches(".ped").to_string(),
            _ => continue,
        };
        let rel = format!("models/{}.ped.toml", stem);
        let handle: Handle<PedPhysicsDef> = asset_server.load(rel);
        handle_cache.0.push(handle);
        info!("[native_assets] ped physics queued: '{}'", stem);
    }
}

fn load_native_fonts(
    asset_server: &AssetServer,
    font_reg: &mut GuiFontRegistry,
    root: &PathBuf,
) {
    let font_dir = root.join("fonts");
    let entries = match std::fs::read_dir(&font_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("[native_assets] cannot scan fonts dir {:?}: {}", font_dir, e);
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "ttf" | "otf") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        let rel = format!("fonts/{}.{}", stem, ext);
        let handle: Handle<Font> = asset_server.load(rel);
        font_reg.0.insert(stem.to_string(), handle);
        info!("[native_assets] font registered: '{}'", stem);
    }
}

fn load_native_drawables(
    asset_server: &AssetServer,
    drawable_reg: &mut DrawableManifestRegistry,
    root: &PathBuf,
) {
    use crate::drawable::DrawableManifest;

    let model_dir = root.join("models");
    let entries = match std::fs::read_dir(&model_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "drawable" {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let rel = format!("models/{}.drawable", stem);
        let handle: Handle<DrawableManifest> = asset_server.load(rel);
        drawable_reg.0.insert(stem.to_string(), handle);
        info!("[native_assets] drawable manifest registered: '{}'", stem);
    }
}

fn register_native_models(
    asset_server: &AssetServer,
    model_reg: &mut ModelRegistry,
    gltf_cache: &mut GltfHandleCache,
    adm_cache: &mut AdmHandleCache,
    root: &PathBuf,
) {
    let model_dir = root.join("models");
    let entries = match std::fs::read_dir(&model_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("[native_assets] cannot scan models dir {:?}: {}", model_dir, e);
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        match ext {
            "adm" => {
                let bevy_path = format!("models/{}.adm", stem);
                model_reg.register_native(stem.to_string(), bevy_path.clone());
                let handle: Handle<AdmScene> = asset_server.load(bevy_path);
                adm_cache.0.insert(stem.to_string(), handle);
                info!("[native_assets] adm model registered: '{}'", stem);
            }
            "glb" | "gltf" => {
                // ADM má přednost — pokud existuje .adm se stejným stemem, přeskočíme GLB.
                let adm_path = path.with_extension("adm");
                if adm_path.exists() {
                    info!("[native_assets] skipping '{}' (adm exists)", stem);
                    continue;
                }
                let bevy_path = format!("models/{}.{}", stem, ext);
                model_reg.register_native(stem.to_string(), bevy_path.clone());
                let handle: Handle<bevy::gltf::Gltf> = asset_server.load_with_settings(
                    bevy_path.clone(),
                    |s: &mut bevy::gltf::GltfLoaderSettings| {
                        s.include_source = true;
                    },
                );
                gltf_cache.0.insert(stem.to_string(), (handle, bevy_path));
                info!("[native_assets] glb model registered: '{}'", stem);
            }
            _ => {}
        }
    }
}
