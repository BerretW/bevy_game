use avian3d::prelude::*;
use bevy::prelude::*;
use core_drawable::{OnStairs, TwoBoneIkSolver};
use core_resources::{IkEnabledComponent, LocalEventBus, StairsCollider};
use core_shared::PlayerMarker;
use lightyear::prelude::Predicted;

use super::camera::find_bone_entity;
use super::movement::{has_ground_contact, has_ground_contact_with_state};
use super::LocalClientId;
use crate::drawable::AdmSceneRoot;

const IK_OFF_STAIRS_DECIMATION_FRAMES: u64 = 6;

#[derive(Resource, Default)]
pub(super) struct AdaptiveIkSampleCache {
    pub(super) frame: u64,
    pub(super) on_stairs: bool,
    pub(super) left_foot_y: f32,
    pub(super) right_foot_y: f32,
    pub(super) sampled_this_frame: bool,
}

#[derive(Component, Debug, Default)]
pub(super) struct FootIkBoneState {
    pub(super) smooth_target_y: f32,
    pub(super) blend_weight: f32,
}

pub(super) fn publish_stairs_state_to_lua(
    time: Res<Time>,
    local_bus: Res<LocalEventBus>,
    mut ik_cache: ResMut<AdaptiveIkSampleCache>,
    local_client_id: Option<Res<LocalClientId>>,
    players: Query<
        (Entity, &PlayerMarker, &Transform, Option<&LinearVelocity>, Option<&ShapeHits>),
        With<Predicted>,
    >,
    model_roots: Query<(Entity, &bevy::ecs::hierarchy::ChildOf), With<AdmSceneRoot>>,
    player_q: Query<Has<PlayerMarker>>,
    stairs_q: Query<(), With<StairsCollider>>,
    ik_surface_q: Query<(), With<crate::physics::DummyStairsIkSurface>>,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    children_q: Query<&Children>,
    name_q: Query<&Name>,
    foot_bone_state_q: Query<&FootIkBoneState>,
    spatial_query: SpatialQuery,
) {
    let Some(lid) = local_client_id.as_ref() else {
        return;
    };

    let Some((player_entity, _, tf, vel, shape_hits)) = players
        .iter()
        .find(|(_, marker, _, _, _)| marker.client_id == lid.0)
    else {
        return;
    };

    let is_stairs_or_parent = |entity: Entity| -> bool {
        let mut current = entity;
        loop {
            if stairs_q.get(current).is_ok() {
                return true;
            }
            match child_of_q.get(current) {
                Ok(child_of) => current = child_of.parent(),
                Err(_) => return false,
            }
        }
    };

    let origin = tf.translation + Vec3::new(0.0, 0.20, 0.0);
    let dir = Dir3::NEG_Y;
    let max_dist = 2.2;
    let filter = SpatialQueryFilter::default();
    let not_player = |entity: Entity| -> bool {
        let mut current = entity;
        loop {
            if player_q.get(current).unwrap_or(false) {
                return false;
            }
            match child_of_q.get(current) {
                Ok(child_of) => current = child_of.parent(),
                Err(_) => return true,
            }
        }
    };
    let hit = spatial_query.cast_ray_predicate(origin, dir, max_dist, true, &filter, &not_player);

    let on_stairs = hit
        .as_ref()
        .map(|ray_hit| is_stairs_or_parent(ray_hit.entity))
        .unwrap_or(false);
    ik_cache.on_stairs = on_stairs;
    let hit_distance = hit.as_ref().map(|ray_hit| ray_hit.distance).unwrap_or(-1.0);
    let hit_pos = hit
        .as_ref()
        .map(|ray_hit| origin + Vec3::new(0.0, -ray_hit.distance, 0.0));
    let grounded = has_ground_contact(shape_hits);
    let vy = vel.map(|value| value.y).unwrap_or(0.0);

    ik_cache.frame = ik_cache.frame.wrapping_add(1);
    let sample_now = on_stairs || (ik_cache.frame % IK_OFF_STAIRS_DECIMATION_FRAMES == 0);
    ik_cache.sampled_this_frame = sample_now;

    if sample_now {
        let right = tf.rotation * Vec3::X;
        let forward = tf.rotation * Vec3::Z;
        let foot_height_origin_y = tf.translation.y + 0.35;
        let max_foot_dist = 1.5;

        let is_ik_surface_or_parent = |entity: Entity| -> bool {
            let mut current = entity;
            loop {
                if ik_surface_q.get(current).is_ok() {
                    return true;
                }
                match child_of_q.get(current) {
                    Ok(child_of) => current = child_of.parent(),
                    Err(_) => return false,
                }
            }
        };

        let cast_foot_y = |foot_origin: Vec3| -> Option<f32> {
            let hit_ik = spatial_query.cast_ray_predicate(
                foot_origin,
                Dir3::NEG_Y,
                max_foot_dist,
                true,
                &filter,
                &is_ik_surface_or_parent,
            );

            if let Some(ray_hit) = hit_ik {
                return Some(foot_origin.y - ray_hit.distance);
            }

            let hit_any = spatial_query.cast_ray_predicate(
                foot_origin,
                Dir3::NEG_Y,
                max_foot_dist,
                true,
                &filter,
                &not_player,
            );

            hit_any.map(|ray_hit| foot_origin.y - ray_hit.distance)
        };

        let left_origin = tf.translation - right * 0.14 + forward * 0.05
            + Vec3::Y * (foot_height_origin_y - tf.translation.y);
        let right_origin = tf.translation + right * 0.14 + forward * 0.05
            + Vec3::Y * (foot_height_origin_y - tf.translation.y);

        if let Some(value) = cast_foot_y(left_origin) {
            ik_cache.left_foot_y = value;
        }
        if let Some(value) = cast_foot_y(right_origin) {
            ik_cache.right_foot_y = value;
        }
    }

    let ik_sample_hz = if on_stairs {
        (1.0 / time.delta_secs()).max(1.0)
    } else {
        (1.0 / (time.delta_secs() * IK_OFF_STAIRS_DECIMATION_FRAMES as f32)).max(1.0)
    };
    let ik_quality = if on_stairs { "high" } else { "low" };
    let ik_runtime = model_roots
        .iter()
        .find(|(_, child_of)| child_of.parent() == player_entity)
        .map(|(model_root, _)| {
            let left_thigh = ["DEF_thigh_l", "DEF-thigh_l", "DEF_thigh.L"]
                .iter()
                .find_map(|bone| find_bone_entity(model_root, bone, &children_q, &name_q, 10));
            let right_thigh = ["DEF_thigh_r", "DEF-thigh_r", "DEF_thigh.R"]
                .iter()
                .find_map(|bone| find_bone_entity(model_root, bone, &children_q, &name_q, 10));

            let left_state = left_thigh.and_then(|entity| foot_bone_state_q.get(entity).ok());
            let right_state = right_thigh.and_then(|entity| foot_bone_state_q.get(entity).ok());

            serde_json::json!({
                "left": {
                    "smooth_target_y": left_state.map(|state| state.smooth_target_y),
                    "blend_weight": left_state.map(|state| state.blend_weight),
                },
                "right": {
                    "smooth_target_y": right_state.map(|state| state.smooth_target_y),
                    "blend_weight": right_state.map(|state| state.blend_weight),
                }
            })
        });

    let payload = serde_json::to_vec(&serde_json::json!({
        "on_stairs": on_stairs,
        "reacting": on_stairs && grounded,
        "grounded": grounded,
        "hit_distance": hit_distance,
        "hit_pos": hit_pos.map(|pos| serde_json::json!({
            "x": pos.x,
            "y": pos.y,
            "z": pos.z,
        })),
        "ik": {
            "quality": ik_quality,
            "sample_hz": ik_sample_hz,
            "sampled_this_frame": ik_cache.sampled_this_frame,
            "left_foot_y": ik_cache.left_foot_y,
            "right_foot_y": ik_cache.right_foot_y,
            "runtime": ik_runtime,
        },
        "player": {
            "x": tf.translation.x,
            "y": tf.translation.y,
            "z": tf.translation.z,
            "vy": vy,
        }
    }))
    .unwrap_or_default();

    local_bus.push("stairs:state".to_string(), payload);
}

pub(super) fn apply_local_stairs_foot_ik(
    local_client_id: Option<Res<LocalClientId>>,
    ik_cache: Res<AdaptiveIkSampleCache>,
    players: Query<(Entity, &PlayerMarker), With<Predicted>>,
    model_roots: Query<(Entity, &bevy::ecs::hierarchy::ChildOf), With<AdmSceneRoot>>,
    children_q: Query<&Children>,
    name_q: Query<&Name>,
    globals_q: Query<&GlobalTransform>,
    parent_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    mut transforms_q: Query<&mut Transform>,
    mut bone_state_q: Query<&mut FootIkBoneState>,
    mut commands: Commands,
) {
    let Some(lid) = local_client_id.as_ref() else {
        return;
    };

    let Some((player_entity, _)) = players.iter().find(|(_, marker)| marker.client_id == lid.0) else {
        return;
    };

    let Some((model_root, _)) = model_roots
        .iter()
        .find(|(_, child_of)| child_of.parent() == player_entity)
    else {
        return;
    };

    let resolve_bone = |names: &[&str]| -> Option<Entity> {
        for name in names {
            if let Some(entity) = find_bone_entity(model_root, name, &children_q, &name_q, 10) {
                return Some(entity);
            }
        }
        None
    };

    let left_thigh = resolve_bone(&["DEF_thigh_l", "DEF-thigh_l", "DEF_thigh.L"]);
    let left_shin = resolve_bone(&["DEF_shin_l", "DEF-shin_l", "DEF_shin.L"]);
    let left_foot = resolve_bone(&["DEF_foot_l", "DEF-foot_l", "DEF_foot.L"]);
    let right_thigh = resolve_bone(&["DEF_thigh_r", "DEF-thigh_r", "DEF_thigh.R"]);
    let right_shin = resolve_bone(&["DEF_shin_r", "DEF-shin_r", "DEF_shin.R"]);
    let right_foot = resolve_bone(&["DEF_foot_r", "DEF-foot_r", "DEF_foot.R"]);

    let ensure_state = |
        thigh_entity: Entity,
        foot_pos_y: f32,
        bone_state_q: &mut Query<&mut FootIkBoneState>,
        commands: &mut Commands,
    | -> bool {
        if bone_state_q.get(thigh_entity).is_err() {
            commands.entity(thigh_entity).insert(FootIkBoneState {
                smooth_target_y: foot_pos_y,
                blend_weight: 0.0,
            });
            return true;
        }
        false
    };

    let apply_leg_ik = |
        thigh: Option<Entity>,
        shin: Option<Entity>,
        foot: Option<Entity>,
        target_world_y: f32,
        ik_cache: &AdaptiveIkSampleCache,
        globals_q: &Query<&GlobalTransform>,
        parent_q: &Query<&bevy::ecs::hierarchy::ChildOf>,
        transforms_q: &mut Query<&mut Transform>,
        bone_state_q: &mut Query<&mut FootIkBoneState>,
        commands: &mut Commands,
    | {
        let (Some(thigh_entity), Some(shin_entity), Some(foot_entity)) = (thigh, shin, foot) else {
            return;
        };

        let Ok(thigh_gt) = globals_q.get(thigh_entity) else {
            return;
        };
        let Ok(shin_gt) = globals_q.get(shin_entity) else {
            return;
        };
        let Ok(foot_gt) = globals_q.get(foot_entity) else {
            return;
        };

        let thigh_pos = thigh_gt.translation();
        let shin_pos = shin_gt.translation();
        let foot_pos = foot_gt.translation();

        let len_a = thigh_pos.distance(shin_pos);
        let len_b = shin_pos.distance(foot_pos);
        if len_a < 0.001 || len_b < 0.001 {
            return;
        }

        if ensure_state(thigh_entity, foot_pos.y, bone_state_q, commands) {
            return;
        }
        let Ok(mut state) = bone_state_q.get_mut(thigh_entity) else {
            return;
        };

        if ik_cache.on_stairs {
            if state.blend_weight < 0.01 {
                state.smooth_target_y = foot_pos.y;
            }
            state.smooth_target_y += (target_world_y - state.smooth_target_y) * 0.20;
            state.blend_weight = (state.blend_weight + 0.15).min(1.0);
        } else {
            state.blend_weight = (state.blend_weight - 0.06).max(0.0);
        }

        if state.blend_weight < 0.01 {
            return;
        }

        let ik_target = Vec3::new(foot_pos.x, state.smooth_target_y, foot_pos.z);
        let midpoint = (thigh_pos + foot_pos) * 0.5;
        let pole_hint = if (shin_pos - midpoint).length_squared() > 0.0001 {
            (shin_pos - midpoint).normalize()
        } else {
            Vec3::Z
        };

        let (new_shin_pos, new_foot_pos) =
            TwoBoneIkSolver::solve(thigh_pos, ik_target, len_a, len_b, pole_hint);

        let thigh_local_rot = transforms_q
            .get(thigh_entity)
            .map(|transform| transform.rotation)
            .unwrap_or(Quat::IDENTITY);
        let shin_local_rot = transforms_q
            .get(shin_entity)
            .map(|transform| transform.rotation)
            .unwrap_or(Quat::IDENTITY);
        let foot_local_t = transforms_q
            .get(foot_entity)
            .map(|transform| transform.translation)
            .unwrap_or(Vec3::NEG_Y * len_b);

        let thigh_parent_rot = parent_q
            .get(thigh_entity)
            .ok()
            .and_then(|child_of| globals_q.get(child_of.parent()).ok())
            .map(|transform| transform.to_scale_rotation_translation().1)
            .unwrap_or(Quat::IDENTITY);

        let thigh_world_rot = thigh_parent_rot * thigh_local_rot;
        let old_dir_sq = (shin_pos - thigh_pos).length_squared();
        let new_dir_sq = (new_shin_pos - thigh_pos).length_squared();
        if old_dir_sq < 0.000001 || new_dir_sq < 0.000001 {
            return;
        }
        let old_thigh_dir = (shin_pos - thigh_pos) / old_dir_sq.sqrt();
        let new_thigh_dir = (new_shin_pos - thigh_pos) / new_dir_sq.sqrt();

        let delta_thigh = if old_thigh_dir.dot(new_thigh_dir) < 0.99999 {
            Quat::from_rotation_arc(old_thigh_dir, new_thigh_dir)
        } else {
            Quat::IDENTITY
        };

        let new_thigh_world_rot = delta_thigh * thigh_world_rot;
        let new_thigh_local = thigh_parent_rot.inverse() * new_thigh_world_rot;

        let foot_local_len_sq = foot_local_t.length_squared();
        let new_foot_vec_sq = (new_foot_pos - new_shin_pos).length_squared();
        let new_shin_local = if foot_local_len_sq > 0.000001 && new_foot_vec_sq > 0.000001 {
            let foot_local_dir = foot_local_t / foot_local_len_sq.sqrt();
            let current_dir = (shin_local_rot * foot_local_dir).normalize();
            let target_dir = (new_thigh_world_rot.inverse() * (new_foot_pos - new_shin_pos)).normalize();
            if current_dir.dot(target_dir) < 0.99999 {
                Quat::from_rotation_arc(current_dir, target_dir) * shin_local_rot
            } else {
                shin_local_rot
            }
        } else {
            shin_local_rot
        };

        let final_thigh = thigh_local_rot.slerp(new_thigh_local, state.blend_weight);
        let final_shin = shin_local_rot.slerp(new_shin_local, state.blend_weight);

        if let Ok(mut transform) = transforms_q.get_mut(thigh_entity) {
            transform.rotation = final_thigh;
        }
        if let Ok(mut transform) = transforms_q.get_mut(shin_entity) {
            transform.rotation = final_shin;
        }
    };

    apply_leg_ik(
        left_thigh,
        left_shin,
        left_foot,
        ik_cache.left_foot_y + 0.015,
        &ik_cache,
        &globals_q,
        &parent_q,
        &mut transforms_q,
        &mut bone_state_q,
        &mut commands,
    );
    apply_leg_ik(
        right_thigh,
        right_shin,
        right_foot,
        ik_cache.right_foot_y + 0.015,
        &ik_cache,
        &globals_q,
        &parent_q,
        &mut transforms_q,
        &mut bone_state_q,
        &mut commands,
    );
}

pub(super) fn terrain_snap_kinematic(
    local_client_id: Option<Res<LocalClientId>>,
    mut players: Query<
        (
            &PlayerMarker,
            &GlobalTransform,
            &OnStairs,
            &IkEnabledComponent,
            &mut LinearVelocity,
            Option<&ShapeHits>,
        ),
        With<Predicted>,
    >,
) {
    let Some(lid) = local_client_id else {
        return;
    };

    for (marker, global_tf, stairs, ik_enabled, mut vel, shape_hits) in players.iter_mut() {
        if marker.client_id != lid.0 {
            continue;
        }

        let blend = ik_enabled.blend_weight;
        if blend < 0.01 {
            continue;
        }

        if !has_ground_contact_with_state(shape_hits, true) {
            continue;
        }

        let left_h = stairs.left_foot_height;
        let right_h = stairs.right_foot_height;
        if left_h < 0.01 && right_h < 0.01 {
            continue;
        }

        let target_y = match (left_h > 0.01, right_h > 0.01) {
            (true, true) => (left_h + right_h) * 0.5,
            (true, false) => left_h,
            (false, true) => right_h,
            (false, false) => continue,
        };

        let current_y = global_tf.translation().y;
        let diff = target_y - current_y;
        if diff.abs() < 0.003 {
            continue;
        }

        const SNAP_SPEED_MAX: f32 = 4.0;
        if diff >= 0.0 {
            continue;
        }

        const SNAP_SPRING_K: f32 = 12.0;
        let snap_vel = (diff.abs() * SNAP_SPRING_K).min(SNAP_SPEED_MAX) * blend;
        if vel.y > -1.5 {
            vel.y = -snap_vel;
        }
    }
}
