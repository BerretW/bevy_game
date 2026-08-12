use avian3d::prelude::*;
use bevy::prelude::*;
use core_resources::{NpcAgent, NpcOwner, NpcPedMarker};
use core_shared::PlayerMarker;

use crate::gameplay::LocalClientId;

pub(crate) fn update_owned_npc_avoidance(
    local_client_id: Option<Res<LocalClientId>>,
    spatial_query: SpatialQuery,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    player_q: Query<Has<PlayerMarker>>,
    npc_q: Query<Has<NpcPedMarker>>,
    mut owned_npcs: Query<
        (Entity, &Transform, &NpcOwner, &mut NpcAgent),
        (With<NpcPedMarker>, With<NpcAgent>),
    >,
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

pub(crate) fn terrain_snap_owned_npcs(
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
        let Some(hit) = spatial_query.cast_ray(origin, Dir3::NEG_Y, 25.0, true, &filter) else {
            continue;
        };

        let target_y = origin.y - hit.distance;
        if !target_y.is_finite() {
            continue;
        }

        let diff = target_y - transform.translation.y;
        if diff.abs() < 0.002 || diff.abs() > 20.0 {
            continue;
        }

        transform.translation.y = target_y;
    }
}
