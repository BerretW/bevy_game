use std::collections::HashMap;

use bevy::gltf::Gltf;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;

use crate::manifest::DrawableManifest;

/// Mapuje jméno modelu (stem, např. `"barrel"`) → Handle<DrawableManifest>.
/// Plní se z `NativeAssetsPlugin` (scan `assets/models/*.drawable`)
/// a z VFS scan streamových assetů.
#[derive(Resource, Default)]
pub struct DrawableManifestRegistry(pub HashMap<String, Handle<DrawableManifest>>);

/// Drží strong Handle<Gltf> + asset cestu pro každý nativní model.
/// Tuple: `(handle, bevy_asset_path)` — cesta je nutná pro `GltfAssetLabel::Texture(n)` lookup.
/// Klíč = model stem (`"untitled"`) — stejný jako v DrawableManifestRegistry.
#[derive(Resource, Default)]
pub struct GltfHandleCache(pub HashMap<String, (Handle<Gltf>, String)>);

/// Globální cache sdílených textur (`source = "shared"`).
/// Zabraňuje opakovanému uploadu stejné textury na GPU.
/// Textury jsou konvencí v `stream/textures/<name>.dds` (nebo jiná přípona).
#[derive(Resource, Default)]
pub struct TextureRegistry(HashMap<String, Handle<Image>>);

impl TextureRegistry {
    /// Vrátí handle na sdílenou texturu. Pokud ještě není načtena, spustí load.
    pub fn request(&mut self, name: &str, asset_server: &AssetServer) -> Handle<Image> {
        if let Some(h) = self.0.get(name) {
            return h.clone();
        }
        // Konvence: sdílené textury žijí ve stream/textures/.
        // DDS je preferovaný formát; fallback přípony lze přidat podle potřeby.
        let path = format!("stream/textures/{}.dds", name);
        let handle: Handle<Image> = asset_server.load_with_settings(
            path,
            |settings: &mut ImageLoaderSettings| {
                settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    address_mode_w: ImageAddressMode::Repeat,
                    ..default()
                });
            },
        );
        self.0.insert(name.to_string(), handle.clone());
        handle
    }
}
