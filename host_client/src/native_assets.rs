//! Načítá nativní assety z `assets/` adresáře klienta do Bevy registrů.
//!
//! Tři kategorie:
//! - `assets/fonts/`   → `GuiFontRegistry`  — Lua: `Gui.DrawText(..., "SephoraHayden")`
//! - `assets/models/`  → `ModelRegistry`    — Lua: `Engine.HasModelLoaded("player")`
//! - `assets/shaders/` → přístupné přes Bevy AssetServer bez extra registrace
//!
//! Font ID = název souboru bez přípony (case-sensitive).
//! Model ID = název souboru bez přípony (case-sensitive).

use std::path::PathBuf;

use bevy::prelude::*;

use core_resources::ModelRegistry;

use crate::drawable::{DrawableManifestRegistry, GltfHandleCache};
use crate::gui_render::GuiFontRegistry;

pub struct NativeAssetsPlugin;

impl Plugin for NativeAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_native_assets);
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
) {
    let root = asset_root();
    load_native_fonts(&asset_server, &mut font_reg, &root);
    register_native_models(&asset_server, &mut model_reg, &mut gltf_cache, &root);
    load_native_drawables(&asset_server, &mut drawable_reg, &root);
}

fn load_native_fonts(asset_server: &AssetServer, font_reg: &mut GuiFontRegistry, root: &PathBuf) {
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
        if ext != "drawable" { continue; }
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
        if !matches!(ext, "glb" | "gltf") { continue; }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        let bevy_path = format!("models/{}.{}", stem, ext);
        model_reg.register_native(stem.to_string(), bevy_path.clone());
        // include_source = true: zachová raw gltf::Gltf v Gltf::source →
        // umožní lookup embedded textur podle jména přes gltf.images().
        let handle: Handle<bevy::gltf::Gltf> = asset_server.load_with_settings(
            bevy_path.clone(),
            |s: &mut bevy::gltf::GltfLoaderSettings| {
                s.include_source = true;
            },
        );
        gltf_cache.0.insert(stem.to_string(), (handle, bevy_path));
        info!("[native_assets] model registered: '{}'", stem);
    }
}
