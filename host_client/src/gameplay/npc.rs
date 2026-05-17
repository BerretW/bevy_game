use avian3d::prelude::*;
use bevy::prelude::*;
use core_net::{NpcTransformChannel, NpcTransformUpdate};
use core_resources::{
    AnimationState, AttachedAnimSets, EntityHandle, ModelName, NpcAgent, NpcBrainRegistry,
    NpcBrainState, NpcOwner, NpcPedMarker, NpcScenarioRegistry, ReplicatedNpcBrain,
    ReplicatedNpcSteering, apply_replicated_npc_brain, apply_replicated_npc_steering,
};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;

use super::{LocalClientId, resolve_ped_profile_for_model};
use crate::drawable::{AdmSceneRoot, PedPhysicsDef, PedPhysicsRegistry};
use crate::native_assets::PedAdsAnimIndex;

#[derive(Component)]
pub(super) struct NpcCapsuleAttached;

#[derive(Component)]
pub(super) struct NpcVisualAttached;

#[derive(Component, Default)]
pub(super) struct NpcMotionTracker {
    prev_pos: Vec3,
    current_state: u8,
}

pub(super) fn attach_capsule_to_new_npcs(
    mut commands: Commands,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    new_npcs: Query<
        (Entity, &ModelName, Option<&core_resources::PedProfileOverride>),
        (With<NpcPedMarker>, Without<NpcCapsuleAttached>),
    >,
) {
    for (entity, model_name, ped_override) in &new_npcs {
        let ped = if let Some(override_comp) = ped_override {
            ped_reg
                .0
                .get(&override_comp.0)
                .and_then(|handle| ped_assets.get(handle))
                .or_else(|| resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets))
        } else {
            resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
        };
        let cap_radius = ped.map(|p| p.capsule.radius).unwrap_or(0.35_f32);
        let cap_height = ped.map(|p| p.capsule.height).unwrap_or(1.80_f32);
        let cap_body = (cap_height - cap_radius * 2.0).max(0.001);

        commands.entity(entity).insert((
            NpcCapsuleAttached,
            RigidBody::Kinematic,
            LockedAxes::new()
                .lock_rotation_x()
                .lock_rotation_y()
                .lock_rotation_z(),
        ));

        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Collider::capsule(cap_radius, cap_body),
                CollisionLayers::new(LayerMask(0b10), LayerMask::ALL),
                Friction::new(0.1).with_combine_rule(CoefficientCombine::Min),
                Restitution::ZERO,
                Transform::from_xyz(0.0, cap_height / 2.0, 0.0),
                GlobalTransform::default(),
            ));
        });
    }
}

pub(super) fn attach_model_to_new_npcs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    ped_anim_index: Res<PedAdsAnimIndex>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    new_npcs: Query<
        (Entity, &ModelName, Option<&core_resources::PedProfileOverride>, Option<&NetTransform>),
        (With<NpcPedMarker>, Without<NpcVisualAttached>),
    >,
) {
    for (entity, model_name, ped_override, net_tf) in &new_npcs {
        let model_path = format!("models/{}.adm", model_name.0);
        let model_handle = asset_server.load::<crate::drawable::AdmScene>(model_path);

        let attached_anim_sets = ped_anim_index
            .0
            .get(&model_name.0)
            .cloned()
            .or_else(|| ped_override.and_then(|ov| ped_anim_index.0.get(&ov.0).cloned()))
            .or_else(|| {
                resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
                    .and_then(|ped| ped_anim_index.0.get(&ped.identity.model).cloned())
            })
            .unwrap_or_default();

        let spawn_transform = net_tf
            .map(|net_transform| Transform {
                translation: net_transform.translation,
                rotation: net_transform.rotation,
                scale: Vec3::ONE,
            })
            .unwrap_or_default();

        commands.entity(entity).insert((
            NpcVisualAttached,
            NpcMotionTracker {
                prev_pos: spawn_transform.translation,
                current_state: 0,
            },
            spawn_transform,
            GlobalTransform::default(),
            Visibility::Visible,
            InheritedVisibility::default(),
            ViewVisibility::default(),
        ));

        commands.entity(entity).with_children(|parent| {
            let mut child = parent.spawn((
                AdmSceneRoot(model_handle.clone()),
                crate::drawable::DisableDrawableCollisions,
                ModelName(model_name.0.clone()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
                InheritedVisibility::default(),
                ViewVisibility::default(),
            ));

            let ped = if let Some(override_comp) = ped_override {
                ped_reg
                    .0
                    .get(&override_comp.0)
                    .and_then(|handle| ped_assets.get(handle))
                    .or_else(|| resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets))
            } else {
                resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
            };

            let initial_clip = ped
                .and_then(|ped| {
                    let idle = ped.animations.idle.clone();
                    if idle.is_empty() { None } else { Some(idle) }
                })
                .unwrap_or_else(|| "clip:0".to_string());

            child.insert(AnimationState {
                current: Some(initial_clip),
                speed: 1.0,
                looping: true,
                paused: false,
                blend_time: 0.0,
                flags: 1,
            });

            if !attached_anim_sets.is_empty() {
                child.insert(AttachedAnimSets {
                    sets: attached_anim_sets,
                });
            }
        });
    }
}

pub(super) fn sync_npc_net_transform(
    local_client_id: Option<Res<LocalClientId>>,
    spatial_query: SpatialQuery,
    mut query: Query<(&mut Transform, &NetTransform, Option<&NpcOwner>), (With<NpcPedMarker>, With<NpcVisualAttached>)>,
) {
    let local_id = local_client_id.map(|value| value.0);
    let filter = SpatialQueryFilter::from_mask(LayerMask::DEFAULT);
    for (mut transform, net_transform, owner) in &mut query {
        if let (Some(local_id), Some(owner)) = (local_id, owner) {
            if owner.0 == Some(local_id) {
                continue;
            }
        }
        transform.translation = net_transform.translation;
        transform.rotation = net_transform.rotation;

        let origin = transform.translation + Vec3::new(0.0, 0.6, 0.0);
        let Some(hit) = spatial_query.cast_ray(origin, Dir3::NEG_Y, 2.5, true, &filter) else {
            continue;
        };

        let target_y = origin.y - hit.distance;
        if target_y.is_finite() {
            let diff = target_y - transform.translation.y;
            if diff.abs() <= 0.75 {
                transform.translation.y = target_y;
            }
        }
    }
}

pub(super) fn bootstrap_owned_npc_agents(
    local_client_id: Option<Res<LocalClientId>>,
    brain_registry: Res<NpcBrainRegistry>,
    scenario_registry: Res<NpcScenarioRegistry>,
    mut commands: Commands,
    npcs: Query<
        (
            Entity,
            &EntityHandle,
            &Transform,
            &NpcOwner,
            &ReplicatedNpcBrain,
            &ReplicatedNpcSteering,
        ),
        (With<NpcPedMarker>, Without<NpcAgent>),
    >,
) {
    let Some(local_id) = local_client_id.map(|value| value.0) else {
        return;
    };

    for (entity, handle, transform, owner, brain, steering) in &npcs {
        if owner.0 != Some(local_id) {
            continue;
        }
        let mut agent = NpcAgent::new(handle.0, transform.translation);
        let mut local_state = NpcBrainState::new(brain.brain_id.clone());
        apply_replicated_npc_brain(
            &brain_registry,
            &scenario_registry,
            brain,
            &mut local_state,
            &mut agent,
        );
        apply_replicated_npc_steering(&mut agent, steering);
        commands.entity(entity).insert((agent, local_state));
    }
}

pub(super) fn cleanup_unowned_npc_agents(
    local_client_id: Option<Res<LocalClientId>>,
    mut commands: Commands,
    npcs: Query<(Entity, &NpcOwner), (With<NpcPedMarker>, With<NpcAgent>)>,
) {
    let Some(local_id) = local_client_id.map(|value| value.0) else {
        return;
    };

    for (entity, owner) in &npcs {
        if owner.0 == Some(local_id) {
            continue;
        }
        commands.entity(entity).remove::<NpcAgent>();
    }
}

pub(super) fn send_owned_npc_transforms(
    local_client_id: Option<Res<LocalClientId>>,
    owned_npcs: Query<(&EntityHandle, &Transform, &NpcOwner, &NpcAgent), (With<NpcPedMarker>, With<NpcAgent>)>,
    mut senders: Query<&mut MessageSender<NpcTransformUpdate>>,
) {
    let Some(local_id) = local_client_id.map(|value| value.0) else {
        return;
    };

    for (handle, transform, owner, agent) in &owned_npcs {
        if owner.0 != Some(local_id) {
            continue;
        }
        let translation = transform.translation;
        let rotation = transform.rotation.normalize();
        if !(translation.x.is_finite() && translation.y.is_finite() && translation.z.is_finite()) {
            continue;
        }
        let msg = NpcTransformUpdate {
            handle: handle.0,
            translation: [translation.x, translation.y, translation.z],
            rotation: [rotation.x, rotation.y, rotation.z, rotation.w],
            home: [agent.home.x, agent.home.y, agent.home.z],
            wander_target: [agent.wander_target.x, agent.wander_target.y, agent.wander_target.z],
            wander_timer: agent.wander_timer,
            orbit_angle: agent.orbit_angle,
            patrol_to_target: agent.patrol_to_target,
            current_path: agent
                .current_path
                .iter()
                .map(|waypoint| [waypoint.target.x, waypoint.target.y, waypoint.target.z])
                .collect(),
            waypoint_index: agent.waypoint_index,
            map_id: agent.map_id.clone(),
            last_nav_target: agent.last_nav_target.map(|target| [target.x, target.y, target.z]),
            entity_target_position: agent
                .entity_target_position
                .map(|target| [target.x, target.y, target.z]),
            entity_target_velocity: [
                agent.entity_target_velocity.x,
                agent.entity_target_velocity.y,
                agent.entity_target_velocity.z,
            ],
            formation_offset: [
                agent.formation_offset.x,
                agent.formation_offset.y,
                agent.formation_offset.z,
            ],
            avoidance_offset: [
                agent.avoidance_offset.x,
                agent.avoidance_offset.y,
                agent.avoidance_offset.z,
            ],
            avoidance_timer: agent.avoidance_timer,
        };
        for mut sender in &mut senders {
            let _ = sender.send::<NpcTransformChannel>(msg.clone());
        }
    }
}

pub(super) fn update_owned_npc_avoidance(
    local_client_id: Option<Res<LocalClientId>>,
    spatial_query: SpatialQuery,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    player_q: Query<Has<PlayerMarker>>,
    npc_q: Query<Has<NpcPedMarker>>,
    mut owned_npcs: Query<(Entity, &Transform, &NpcOwner, &mut NpcAgent), (With<NpcPedMarker>, With<NpcAgent>)>,
) {
    let Some(local_id) = local_client_id.map(|value| value.0) else {
        return;
    };
    let filter = SpatialQueryFilter::from_mask(LayerMask::DEFAULT);

    let root_is_blocking_other = |candidate: Entity, self_root: Entity| -> bool {
        let mut current = candidate;
        loop {
            if current == self_root {
                return false;
            }
            if player_q.get(current).unwrap_or(false) {
                return false;
            }
            if npc_q.get(current).unwrap_or(false) {
                return true;
            }
            match child_of_q.get(current) {
                Ok(child_of) => current = child_of.parent(),
                Err(_) => return true,
            }
        }
    };

    for (entity, transform, owner, mut agent) in &mut owned_npcs {
        if owner.0 != Some(local_id) || matches!(agent.goal, core_resources::NpcMoveGoal::Idle) {
            continue;
        }

        let forward = Vec3::new(transform.forward().x, 0.0, transform.forward().z);
        if forward.length_squared() <= 0.0001 {
            continue;
        }
        let forward = forward.normalize();
        let right = Vec3::new(forward.z, 0.0, -forward.x);
        let origin = transform.translation + Vec3::new(0.0, 0.55, 0.0);
        let lookahead = (agent.move_speed * 0.35).clamp(0.9, 1.8);

        let hit = spatial_query.cast_ray_predicate(
            origin,
            Dir3::new(forward).unwrap_or(Dir3::Z),
            lookahead,
            true,
            &filter,
            &|candidate| root_is_blocking_other(candidate, entity),
        );

        let Some(_) = hit else {
            continue;
        };

        let side_probe = 0.45;
        let left_origin = origin - right * side_probe;
        let right_origin = origin + right * side_probe;
        let left_hit = spatial_query.cast_ray_predicate(
            left_origin,
            Dir3::new(forward).unwrap_or(Dir3::Z),
            lookahead,
            true,
            &filter,
            &|candidate| root_is_blocking_other(candidate, entity),
        );
        let right_hit = spatial_query.cast_ray_predicate(
            right_origin,
            Dir3::new(forward).unwrap_or(Dir3::Z),
            lookahead,
            true,
            &filter,
            &|candidate| root_is_blocking_other(candidate, entity),
        );

        let preferred_sign = if left_hit.is_some() && right_hit.is_none() {
            1.0
        } else if right_hit.is_some() && left_hit.is_none() {
            -1.0
        } else if agent.avoidance_offset.dot(right) >= 0.0 {
            1.0
        } else {
            -1.0
        };

        agent.avoidance_offset = right * preferred_sign * 1.15;
        agent.avoidance_timer = 0.32;
    }
}

pub(super) fn terrain_snap_owned_npcs(
    local_client_id: Option<Res<LocalClientId>>,
    spatial_query: SpatialQuery,
    mut owned_npcs: Query<(&mut Transform, &NpcOwner), (With<NpcPedMarker>, With<NpcAgent>)>,
) {
    let Some(local_id) = local_client_id.map(|value| value.0) else {
        return;
    };
    let filter = SpatialQueryFilter::from_mask(LayerMask::DEFAULT);

    for (mut transform, owner) in &mut owned_npcs {
        if owner.0 != Some(local_id) {
            continue;
        }

        let origin = transform.translation + Vec3::new(0.0, 0.6, 0.0);
        let Some(hit) = spatial_query.cast_ray(origin, Dir3::NEG_Y, 2.5, true, &filter) else {
            continue;
        };

        let target_y = origin.y - hit.distance;
        if !target_y.is_finite() {
            continue;
        }

        let diff = target_y - transform.translation.y;
        if diff.abs() < 0.002 {
            continue;
        }

        if diff.abs() <= 0.75 {
            transform.translation.y = target_y;
        }
    }
}

pub(super) fn drive_npc_animations(
    time: Res<Time>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    mut npcs: Query<
        (
            &Transform,
            &ModelName,
            Option<&core_resources::PedProfileOverride>,
            &mut NpcMotionTracker,
            &Children,
        ),
        With<NpcPedMarker>,
    >,
    mut adm_roots: Query<&mut AnimationState, With<AdmSceneRoot>>,
) {
    let dt = time.delta_secs().max(0.001);

    for (transform, model_name, ped_override, mut tracker, children) in &mut npcs {
        let speed = transform.translation.distance(tracker.prev_pos) / dt;
        tracker.prev_pos = transform.translation;

        let ped = if let Some(override_comp) = ped_override {
            ped_reg
                .0
                .get(&override_comp.0)
                .and_then(|handle| ped_assets.get(handle))
                .or_else(|| resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets))
        } else {
            resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
        };

        let idle_threshold = ped.map(|p| p.movement.idle_threshold).unwrap_or(0.10);
        let run_threshold = ped.map(|p| p.movement.run_speed * 0.6).unwrap_or(3.0);

        let new_state = if speed <= idle_threshold {
            0u8
        } else if speed < run_threshold {
            1u8
        } else {
            2u8
        };

        if new_state == tracker.current_state {
            continue;
        }
        tracker.current_state = new_state;

        let clip = ped
            .map(|ped| match new_state {
                1 => ped.animations.walk.clone(),
                2 => ped.animations.run.clone(),
                _ => ped.animations.idle.clone(),
            })
            .unwrap_or_else(|| "clip:0".to_string());

        for child in children.iter() {
            if let Ok(mut anim) = adm_roots.get_mut(child) {
                anim.current = Some(clip.clone());
                anim.blend_time = 0.15;
            }
        }
    }
}