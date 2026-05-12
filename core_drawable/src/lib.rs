mod adm;
mod hook;
mod ik;
mod loader;
mod lod;
mod map;
mod manifest;
mod material;
mod registry;
mod ped;

pub use adm::{
    AdmAnimNotifyEvent, AdmAnimationClip, AdmAnimationNotify, AdmAnimationPlayback, AdmAnimationTrack, AdmKeyframe,
    AdmBlendSpace, AdmBlendSpaceClip, AdmBlendSpaceKind,
    AdmLoader, AnimSetLoader, AnimationSet, AnimationSetDictionary,
    AdmNode, AdmNodeEntityMap, AdmNodeType, AdmScene, AdmSceneRoot, AdmSceneSpawned,
};
pub use ped::{PedPhysicsDef, PedPhysicsLoader, PedPhysicsRegistry,
    PedCapsule, PedMovement, PedStances, StanceDef,
    PedJump, PedVault, PedStep, PedLean, PedStamina, StaminaExhausted,
    PedWater, PedFootstep, PedRagdoll,
};
pub use hook::{
    DrawableHooked, DrawableSpawnIntent, DrawableFallbackTextures,
    AdsNodeKind, AdsSocket, DrawableCollision, DisableDrawableCollisions, DrawableMaterialsParam,
    classify_ads_node_name,
    attach_drawable_intent, hook_drawable_scenes, observe_scene_ready,
    setup_fallback_textures, auto_hide_col_nodes, apply_material_overrides,
};
pub use ik::{
    OnStairs, IkChain, IkSolverState, IkEnabled, TwoBoneIkSolver,
    detect_stairs_on_collision, raycaster_ground_height, apply_ik_to_skeleton,
};
pub use loader::DrawableManifestLoader;
pub use lod::{DefaultLodDistances, LodGroup, LodLevel, apply_skeletal_pruning, parse_lod_level, update_lod_visibility};
pub use map::{MapInstanceDef, MapManifest};
pub use manifest::{
    CollisionMaterial, CollisionShape, DrawableManifest, EntityDef, LodConfig, MaterialDef,
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
            .register_type::<Name>()
            // IK komponenty
            .register_type::<ik::OnStairs>()
            .register_type::<ik::IkChain>()
            .register_type::<ik::IkSolverState>()
            .register_type::<ik::IkEnabled>();

        app.init_asset::<AdmScene>()
            .add_message::<adm::AdmAnimNotifyEvent>()
            .register_asset_loader(AdmLoader)
            .init_asset::<AnimationSet>()
            .register_asset_loader(AnimSetLoader)
            .init_asset::<DrawableManifest>()
            .register_asset_loader(DrawableManifestLoader)
                        .init_asset::<PedPhysicsDef>()
                        .register_asset_loader(PedPhysicsLoader)
                        .init_resource::<PedPhysicsRegistry>()
            .init_asset::<StandardPbrExtension>()
            .init_asset::<LayeredEnvExtension>()
            .init_asset::<VehicleGlassExtension>()
            .add_plugins(MaterialPlugin::<StandardPbrMaterial>::default())
            .add_plugins(MaterialPlugin::<LayeredEnvMaterial>::default())
            .add_plugins(MaterialPlugin::<VehicleGlassMaterial>::default())
            .init_resource::<DrawableManifestRegistry>()
            .init_resource::<GltfHandleCache>()
            .init_resource::<TextureRegistry>()
            .init_resource::<DefaultLodDistances>()
            .add_observer(observe_scene_ready)
            .add_systems(Startup, setup_fallback_textures)
            .add_systems(
                Update,
                (
                    auto_hide_col_nodes,
                    attach_drawable_intent,
                    hook_drawable_scenes.after(attach_drawable_intent),
                    adm::spawn_adm_scenes.after(hook_drawable_scenes),
                    adm::apply_adm_animations.after(adm::spawn_adm_scenes),
                    adm::forward_adm_anim_notifies_to_local_bus.after(adm::apply_adm_animations),
                    update_lod_visibility.after(hook_drawable_scenes),
                    apply_skeletal_pruning.after(update_lod_visibility),
                    apply_material_overrides,
                    detect_stairs_on_collision,
                    raycaster_ground_height.after(detect_stairs_on_collision),
                    apply_ik_to_skeleton.after(raycaster_ground_height),
                ),
            );
    }
}
