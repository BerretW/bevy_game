use bevy::prelude::*;
use core_resources::{
    LuaWorldState, NpcLastClientUpdate, NpcOwner, NpcPathWaypoint, NpcPedMarker,
    ReplicatedNpcSteering, apply_replicated_npc_steering,
};
use core_shared::NetTransform;
use lightyear::prelude::*;

use crate::protocol::NpcTransformUpdate;

pub fn receive_npc_transform_updates(
    mut receivers: Query<(&mut MessageReceiver<NpcTransformUpdate>, &RemoteId)>,
    world_state: Res<LuaWorldState>,
    time: Res<Time>,
    mut npcs: Query<
        (
            &mut Transform,
            &mut NetTransform,
            &NpcOwner,
            &mut NpcLastClientUpdate,
            &mut core_resources::NpcAgent,
            Option<&mut ReplicatedNpcSteering>,
        ),
        With<NpcPedMarker>,
    >,
) {
    for (mut rx, remote_id) in receivers.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => continue,
        };

        for update in rx.receive() {
            let Some(entity) = world_state.entity_for(update.handle) else {
                continue;
            };
            let Ok((mut transform, mut net_transform, owner, mut last_update, mut agent, steering_opt)) = npcs.get_mut(entity) else {
                continue;
            };
            if owner.0 != Some(client_id) {
                continue;
            }

            let [px, py, pz] = update.translation;
            let [rx, ry, rz, rw] = update.rotation;
            if !(px.is_finite()
                && py.is_finite()
                && pz.is_finite()
                && rx.is_finite()
                && ry.is_finite()
                && rz.is_finite()
                && rw.is_finite())
            {
                continue;
            }

            let raw_rotation = Quat::from_xyzw(rx, ry, rz, rw);
            let rotation = if raw_rotation.length_squared() > 1.0e-6 {
                raw_rotation.normalize()
            } else {
                Quat::IDENTITY
            };
            let translation = Vec3::new(px, py, pz);
            let steering = ReplicatedNpcSteering {
                home: Vec3::new(update.home[0], update.home[1], update.home[2]),
                wander_target: Vec3::new(
                    update.wander_target[0],
                    update.wander_target[1],
                    update.wander_target[2],
                ),
                wander_timer: update.wander_timer,
                orbit_angle: update.orbit_angle,
                patrol_to_target: update.patrol_to_target,
                current_path: update
                    .current_path
                    .iter()
                    .map(|p| NpcPathWaypoint {
                        target: Vec3::new(p[0], p[1], p[2]),
                    })
                    .collect(),
                waypoint_index: update.waypoint_index,
                map_id: update.map_id.clone(),
                last_nav_target: update.last_nav_target.map(|p| Vec3::new(p[0], p[1], p[2])),
                entity_target_position: update
                    .entity_target_position
                    .map(|p| Vec3::new(p[0], p[1], p[2])),
                entity_target_velocity: Vec3::new(
                    update.entity_target_velocity[0],
                    update.entity_target_velocity[1],
                    update.entity_target_velocity[2],
                ),
                formation_offset: Vec3::new(
                    update.formation_offset[0],
                    update.formation_offset[1],
                    update.formation_offset[2],
                ),
                avoidance_offset: Vec3::new(
                    update.avoidance_offset[0],
                    update.avoidance_offset[1],
                    update.avoidance_offset[2],
                ),
                avoidance_timer: update.avoidance_timer,
            };
            transform.translation = translation;
            transform.rotation = rotation;
            net_transform.translation = translation;
            net_transform.rotation = rotation;
            apply_replicated_npc_steering(&mut agent, &steering);
            if let Some(mut replicated_steering) = steering_opt {
                *replicated_steering = steering;
            }
            last_update.client_id = client_id;
            last_update.received_at = time.elapsed_secs();
        }
    }
}
