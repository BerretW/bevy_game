mod adm;
mod hook;
mod loader;
mod manifest;
mod material;
mod registry;

pub use adm::{AdmLoader, AdmScene, AdmSceneRoot, AdmSceneSpawned, AdmNode, AdmNodeType};
pub use hook::{
    DrawableHooked, DrawableSpawnIntent, DrawableFallbackTextures,
    DrawableCollision,
    attach_drawable_intent, hook_drawable_scenes, observe_scene_ready,
    setup_fallback_textures,
};
pub use loader::DrawableManifestLoader;
pub use manifest::{
    CollisionMaterial, CollisionShape, DrawableManifest, EntityDef, MaterialDef,
    MaterialParams, TextureInfo, TextureSource,
};
#[allow(unused_imports)]
pub use material::{DrawableExtension, DrawableMaterial, DrawableParams};
pub use material::{
    LayeredEnvExtension, LayeredEnvMaterial,
    StandardPbrExtension, StandardPbrMaterial,
    VehicleGlassExtension, VehicleGlassMaterial,
};
pub use registry::{DrawableManifestRegistry, GltfHandleCache, TextureRegistry};

use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy::transform::components::TransformTreeChanged;
use bevy_gltf::{
    GltfExtras, GltfMaterialExtras, GltfMaterialName,
    GltfMeshExtras, GltfMeshName, GltfSceneExtras,
};

pub struct DrawablePlugin;

impl Plugin for DrawablePlugin {
    fn build(&self, app: &mut App) {
        // Registrace typů nutných pro GLTF scene spawning —
        // bez těchto se scene_spawner panikuje na unregistered type.
        app.register_type::<Transform>()
            .register_type::<GlobalTransform>()
            .register_type::<Visibility>()
            .register_type::<InheritedVisibility>()
            .register_type::<ViewVisibility>()
            .register_type::<TransformTreeChanged>()
            .register_type::<Mesh3d>()
            .register_type::<MeshMaterial3d<StandardMaterial>>()
            .register_type::<bevy::camera::primitives::Aabb>()
            .register_type::<bevy::mesh::skinning::SkinnedMesh>()
            .register_type::<GltfExtras>()
            .register_type::<GltfSceneExtras>()
            .register_type::<GltfMeshExtras>()
            .register_type::<GltfMeshName>()
            .register_type::<GltfMaterialExtras>()
            .register_type::<GltfMaterialName>()
            .register_type::<bevy::ecs::hierarchy::ChildOf>()
            .register_type::<bevy::ecs::hierarchy::Children>()
            .register_type::<Name>();

        app.init_asset::<AdmScene>()
            .register_asset_loader(AdmLoader)
            .init_asset::<DrawableManifest>()
            .register_asset_loader(DrawableManifestLoader)
            .init_asset::<StandardPbrExtension>()
            .init_asset::<LayeredEnvExtension>()
            .init_asset::<VehicleGlassExtension>()
            .add_plugins(MaterialPlugin::<StandardPbrMaterial>::default())
            .add_plugins(MaterialPlugin::<LayeredEnvMaterial>::default())
            .add_plugins(MaterialPlugin::<VehicleGlassMaterial>::default())
            .init_resource::<DrawableManifestRegistry>()
            .init_resource::<GltfHandleCache>()
            .init_resource::<TextureRegistry>()
            .add_observer(observe_scene_ready)
            .add_systems(Startup, setup_fallback_textures)
            .add_systems(
                Update,
                (
                    attach_drawable_intent,
                    hook_drawable_scenes.after(attach_drawable_intent),
                    adm::spawn_adm_scenes.after(hook_drawable_scenes),
                ),
            );
    }
}
