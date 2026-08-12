// Phase 3 — klientska gameplay vrstva.
//
// Phase 3.7: `RaycastBridge` se aktualizuje kazdy frame z pozice mysi.
// Lua sandbox cte pres `Raycast.GetGroundPosition()`.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::transform::{TransformSystems, components::TransformTreeChanged};
use bevy_gltf::{
    GltfExtras, GltfMaterialExtras, GltfMaterialName, GltfMeshExtras, GltfMeshName,
    GltfSceneExtras,
};
use core_resources::{EntityHandle, LuaWorldState, process_lua_commands, sync_entity_state_cache};

use crate::AppState;
use crate::drawable::{PedPhysicsDef, PedPhysicsRegistry};

mod animation;
mod bridge;
mod camera;
mod environment;
mod movement;
mod npc;
mod stairs;
mod visuals;
mod weapons;

use animation::{
    LuaAnimationGraphCache, ModelAnimationDiscoveryCache, apply_lua_animation_state,
    update_model_animation_registry, update_player_state_driven_animations,
};
use bridge::{
    handle_engine_cmds, reset_connection_bridge, reset_engine_state, update_connection_bridge,
    update_input_bridge,
};
use camera::{
    CameraLookState, add_fire_trauma, apply_cursor_mode, setup_scene_and_camera,
    toggle_camera_mode, update_camera_follow, update_camera_look_from_mouse, update_camera_shake,
    update_fov_and_lean, update_head_bob, update_raycast_bridge,
};
use environment::{
    VolumetricFogDebugState, apply_environment_light, publish_volumetric_fog_state_to_lua,
};
use movement::{
    MovementGroundState, apply_player_movement, collect_and_send_input, post_physics_vel_y_damp,
    publish_input_state_to_lua,
};
use npc::{
    attach_capsule_to_new_npcs, attach_model_to_new_npcs, bootstrap_owned_npc_agents,
    cleanup_unowned_npc_agents, drive_npc_animations, send_owned_npc_transforms,
    sync_npc_net_transform, terrain_snap_owned_npcs, update_owned_npc_avoidance,
};
use stairs::{
    AdaptiveIkSampleCache, apply_local_stairs_foot_ik, publish_stairs_state_to_lua,
    terrain_snap_kinematic,
};
use visuals::{
    attach_mesh_to_dummy_objects, attach_mesh_to_local_objects, attach_player_model_to_new_players,
    prefer_predicted_player_visuals, sync_net_transform_to_render, update_crosshair_entity,
    update_local_player_visibility,
};
use weapons::{
    apply_weapon_recoil, apply_weapon_sway, init_weapon_feel_components, spawn_muzzle_flash,
    tick_muzzle_flash, tick_spread_accumulation,
};

#[derive(Resource, Clone, Copy)]
pub struct LocalClientId(pub u64);

pub struct ClientGameplayPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        // GLTF SceneRoot spawn vyzaduje reflektovane registrace komponent,
        // jinak scene_spawner panicne na "unregistered type".
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

        app.init_resource::<CameraLookState>();
        app.init_resource::<LuaAnimationGraphCache>();
        app.init_resource::<ModelAnimationDiscoveryCache>();
        app.init_resource::<AdaptiveIkSampleCache>();
        app.init_resource::<MovementGroundState>();
        app.init_resource::<movement::MovementFovState>();
        app.init_resource::<VolumetricFogDebugState>();

        // Scena a kamera se nastavuji az pri vstupu do InGame, ne na Startup
        app.add_systems(OnEnter(AppState::InGame), (setup_scene_and_camera, reset_engine_state));
        app.add_systems(OnExit(AppState::InGame), reset_connection_bridge);

        app.add_systems(
            Update,
            apply_environment_light.run_if(in_state(AppState::InGame)),
        );

        // Core camera + player update chain.
        app.add_systems(
            Update,
            (
                toggle_camera_mode,
                update_camera_look_from_mouse,
                apply_cursor_mode,
                attach_player_model_to_new_players,
                prefer_predicted_player_visuals,
                sync_net_transform_to_render,
                bootstrap_owned_npc_agents,
                cleanup_unowned_npc_agents,
                update_camera_follow,
                update_raycast_bridge,
                update_crosshair_entity,
                update_input_bridge,
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        );

        // Cyberpunk-style camera effects layered on top of update_camera_follow.
        // Order: add_fire_trauma -> update_camera_shake -> update_head_bob -> update_fov_and_lean
        app.add_systems(
            Update,
            (
                add_fire_trauma,
                update_camera_shake,
                update_head_bob,
                update_fov_and_lean,
            )
                .chain()
                .after(update_camera_follow)
                .run_if(in_state(AppState::InGame)),
        );

        app.add_systems(
            Update,
            (
                update_connection_bridge,
                update_local_player_visibility,
                publish_input_state_to_lua,
                publish_stairs_state_to_lua,
                publish_volumetric_fog_state_to_lua,
                attach_mesh_to_local_objects,
                attach_mesh_to_dummy_objects,
                update_player_state_driven_animations,
                update_model_animation_registry,
                apply_lua_animation_state,
                handle_engine_cmds,
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        );

        // Weapon feel systems -- run after camera look update so recoil/sway
        // offsets are added on top of the freshly computed yaw/pitch.
        app.add_systems(
            Update,
            (init_weapon_feel_components, tick_muzzle_flash).run_if(in_state(AppState::InGame)),
        );
        app.add_systems(
            Update,
            (
                apply_weapon_recoil,
                apply_weapon_sway,
                tick_spread_accumulation,
                spawn_muzzle_flash,
            )
                .chain()
                .after(update_camera_look_from_mouse)
                .run_if(in_state(AppState::InGame)),
        );

        app.add_systems(
            Update,
            attach_model_to_new_npcs.run_if(in_state(AppState::InGame)),
        );
        app.add_systems(
            Update,
            attach_capsule_to_new_npcs.run_if(in_state(AppState::InGame)),
        );
        app.add_systems(
            Update,
            sync_npc_net_transform.run_if(in_state(AppState::InGame)),
        );
        app.add_systems(
            Update,
            drive_npc_animations
                .after(sync_npc_net_transform)
                .run_if(in_state(AppState::InGame)),
        );

        // IK foot placement: PostUpdate pred propagaci, aby animace (Update) kosti nastavila
        // a IK je prepsal driv nez se pocitaji GlobalTransforms pro render -> zadny frame-lag.
        app.add_systems(
            PostUpdate,
            apply_local_stairs_foot_ik
                .before(TransformSystems::Propagate)
                .run_if(in_state(AppState::InGame)),
        );

        app.add_systems(
            FixedUpdate,
            (apply_player_movement, terrain_snap_kinematic, collect_and_send_input)
                .chain()
                .run_if(in_state(AppState::InGame)),
        );

        // Runs after Avian's Writeback phase (vel written back to LinearVelocity).
        // Damps contact-response vel.y spikes before Update systems read them.
        app.add_systems(
            FixedPostUpdate,
            (
                post_physics_vel_y_damp.after(PhysicsSystems::Writeback),
                update_owned_npc_avoidance,
                terrain_snap_owned_npcs,
                send_owned_npc_transforms,
            )
                .run_if(in_state(AppState::InGame)),
        );

        // Registrace replicated entity handles v LuaWorldState.
        // Musi bezet po process_lua_commands (lokalni spawny) ale pred sync_entity_state_cache
        // (aby replicated entity byly v cache ve stejnem framu).
        app.add_systems(
            PostUpdate,
            sync_replicated_entity_handles
                .after(process_lua_commands)
                .before(sync_entity_state_cache),
        );
    }
}

/// Registruje entity replicated pres lightyear v LuaWorldState.
fn sync_replicated_entity_handles(
    new_handles: Query<(Entity, &EntityHandle), Added<EntityHandle>>,
    mut world_state: ResMut<LuaWorldState>,
) {
    for (entity, handle) in &new_handles {
        if world_state.entity_for(handle.0).is_none() {
            world_state.register(handle.0, entity);
            debug!(
                "[gameplay] registered replicated entity handle={} entity={:?}",
                handle.0, entity
            );
        }
    }
}

/// (player.ped.toml) pokud je nacten -- jinak se pouziji fallback konstanty.
fn resolve_default_ped_profile<'a>(
    ped_reg: &'a PedPhysicsRegistry,
    ped_assets: &'a Assets<PedPhysicsDef>,
) -> Option<&'a PedPhysicsDef> {
    ped_reg
        .0
        .get("player")
        .and_then(|h| ped_assets.get(h))
        .or_else(|| ped_reg.0.values().find_map(|h| ped_assets.get(h)))
}

fn resolve_ped_profile_for_model<'a>(
    model_name: &str,
    ped_reg: &'a PedPhysicsRegistry,
    ped_assets: &'a Assets<PedPhysicsDef>,
) -> Option<&'a PedPhysicsDef> {
    ped_reg
        .0
        .values()
        .filter_map(|handle| ped_assets.get(handle))
        .find(|ped| ped.identity.model == model_name)
        .or_else(|| resolve_default_ped_profile(ped_reg, ped_assets))
}
