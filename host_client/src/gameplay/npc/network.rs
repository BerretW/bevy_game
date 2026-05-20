use avian3d::prelude::*;
use bevy::prelude::*;
use core_net::{NpcTransformChannel, NpcTransformUpdate};
use core_resources::{EntityHandle, NpcAgent, NpcOwner, NpcPedMarker};
use core_shared::NetTransform;
use lightyear::prelude::*;

use super::NpcVisualAttached;
use crate::gameplay::LocalClientId;

pub(crate) fn sync_npc_net_transform(
    local_client_id: Option<Res<LocalClientId>>,
    spatial_query: SpatialQuery,
    mut query: Query<
        (&mut Transform, &NetTransform, Option<&NpcOwner>),
        (With<NpcPedMarker>, With<NpcVisualAttached>),
    >,
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
        let Some(hit) = spatial_query.cast_ray(origin, Dir3::NEG_Y, 25.0, true, &filter) else {
            continue;
        };

        let target_y = origin.y - hit.distance;
        if target_y.is_finite() {
            let diff = target_y - transform.translation.y;
            if diff.abs() >= 0.001 && diff.abs() <= 20.0 {
                transform.translation.y = target_y;
            }
        }
    }
}

pub(crate) fn send_owned_npc_transforms(
    local_client_id: Option<Res<LocalClientId>>,
    owned_npcs: Query<
        (&EntityHandle, &Transform, &NpcOwner, &NpcAgent),
        (With<NpcPedMarker>, With<NpcAgent>),
    >,
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
