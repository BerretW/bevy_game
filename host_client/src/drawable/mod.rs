mod hook;
mod loader;
mod manifest;
mod material;
mod registry;

pub use hook::{attach_drawable_intent, hook_drawable_scenes, observe_scene_ready};
pub use loader::DrawableManifestLoader;
pub use manifest::DrawableManifest;
pub use material::{DrawableExtension, DrawableMaterial};
pub use registry::{DrawableManifestRegistry, GltfHandleCache, TextureRegistry};

use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;

pub struct DrawablePlugin;

impl Plugin for DrawablePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<DrawableManifest>()
            .register_asset_loader(DrawableManifestLoader)
            .init_asset::<DrawableExtension>()
            .add_plugins(MaterialPlugin::<DrawableMaterial>::default())
            .init_resource::<DrawableManifestRegistry>()
            .init_resource::<GltfHandleCache>()
            .init_resource::<TextureRegistry>()
            .add_observer(observe_scene_ready)
            .add_systems(
                Update,
                (
                    attach_drawable_intent,
                    hook_drawable_scenes.after(attach_drawable_intent),
                ),
            );
    }
}
