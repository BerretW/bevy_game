use std::collections::HashMap;
use std::f32::consts::TAU;

use bevy::prelude::*;
use core_shared::{NetTransform, PlayerMarker};

use crate::cmd_queue::{
    EntityHandle, LuaWorldState, NpcAgent, NpcAiLodConfig, NpcAiLodLevel, NpcAiLodState,
    NpcLastClientUpdate, NpcMoveGoal, NpcOwner, NpcScenarioRuntimeState,
    NpcWanderKind, ReplicatedNpcSteering, snapshot_npc_steering,
};
use crate::npc_brain::{NpcBrainRegistry, NpcBrainState, NpcTaskKind, ReplicatedNpcBrain};
use crate::plugin::{ResourcesSide};
use crate::types::Side;

const NPC_OWNERSHIP_RADIUS: f32 = 200.0;
const NPC_OWNERSHIP_RELEASE_RADIUS: f32 = 230.0;
const NPC_OWNERSHIP_HANDOFF_ADVANTAGE: f32 = 20.0;
const NPC_OWNERSHIP_HANDOFF_COOLDOWN: f32 = 6.0;
const NPC_OWNERSHIP_ASSIGN_INTERVAL: f32 = 2.0;
const NPC_CLIENT_UPDATE_TIMEOUT_SECS: f32 = 5.0;

fn npc_zone_key(position: Vec3, zone_size: f32) -> (i32, i32) {
    let size = zone_size.max(1.0);
    (
        (position.x / size).floor() as i32,
        (position.z / size).floor() as i32,
    )
}

fn lod_allows_tick(
    handle: u64,
    level: NpcAiLodLevel,
    config: &NpcAiLodConfig,
    elapsed_secs: f32,
    fixed_dt: f32,
) -> bool {
    match level {
        NpcAiLodLevel::Full => true,
        NpcAiLodLevel::Background => false,
        NpcAiLodLevel::Reduced => {
            let interval_ticks = (config.reduced_tick_interval / fixed_dt)
                .round()
                .max(1.0) as u64;
            let tick = (elapsed_secs / fixed_dt).round().max(0.0) as u64;
            tick % interval_ticks == handle % interval_ticks
        }
    }
}

fn npc_lod_priority_score(
    brain_registry: &NpcBrainRegistry,
    scenario_runtime: Option<&NpcScenarioRuntimeState>,
    brain_state: Option<&NpcBrainState>,
    replicated_brain: &ReplicatedNpcBrain,
) -> i32 {
    let brain_id = brain_state
        .map(|state| state.brain_id.as_str())
        .unwrap_or(replicated_brain.brain_id.as_str());
    let def = brain_registry.resolve_or_fallback(brain_id);
    let scenario_bonus = scenario_runtime
        .map(|runtime| runtime.lod_priority as i32 * 20)
        .unwrap_or(0);
    let occupancy_bonus = scenario_runtime
        .map(|runtime| if runtime.occupancy_granted { 8 } else { -12 })
        .unwrap_or(0);

    scenario_bonus
        + occupancy_bonus
        + task_priority(replicated_brain.task)
        + brain_kind_priority(def.kind)
}

fn task_priority(task: NpcTaskKind) -> i32 {
    match task {
        NpcTaskKind::Combat => 90,
        NpcTaskKind::ChaseTarget => 80,
        NpcTaskKind::Flee => 75,
        NpcTaskKind::FollowTarget => 65,
        NpcTaskKind::Investigate => 55,
        NpcTaskKind::UseScenarioPoint => 35,
        NpcTaskKind::DriveRoute | NpcTaskKind::FlyRoute | NpcTaskKind::SwimRoute => 30,
        NpcTaskKind::PatrolRoute => 24,
        NpcTaskKind::WanderZone | NpcTaskKind::Ambient => 16,
        NpcTaskKind::Idle => 0,
    }
}

fn brain_kind_priority(kind: crate::npc_brain::NpcBrainKind) -> i32 {
    match kind {
        crate::npc_brain::NpcBrainKind::Human => 8,
        crate::npc_brain::NpcBrainKind::Vehicle => 7,
        crate::npc_brain::NpcBrainKind::Animal => 5,
        crate::npc_brain::NpcBrainKind::Bird => 3,
        crate::npc_brain::NpcBrainKind::Fish => 2,
    }
}

pub fn assign_npc_owners(
    time: Res<Time>,
    lod_config: Res<NpcAiLodConfig>,
    mut timer: Local<f32>,
    brain_registry: Res<NpcBrainRegistry>,
    mut npcs: Query<(
        Entity,
        &Transform,
        &ReplicatedNpcBrain,
        Option<&NpcScenarioRuntimeState>,
        Option<&NpcBrainState>,
        &mut NpcOwner,
        &mut crate::cmd_queue::NpcOwnershipLease,
        &mut NpcAiLodState,
    ), With<NpcAgent>>,
    players: Query<(&NetTransform, &PlayerMarker)>,
) {
    *timer += time.delta_secs();
    if *timer < NPC_OWNERSHIP_ASSIGN_INTERVAL {
        return;
    }
    *timer = 0.0;

    let now = time.elapsed_secs();
    let player_entries: Vec<(u64, Vec3)> = players
        .iter()
        .map(|(tf, marker)| (marker.client_id, tf.translation))
        .collect();

    let mut desired_lod_by_entity: HashMap<Entity, NpcAiLodLevel> = HashMap::new();
    let mut controlling_player_by_entity: HashMap<Entity, u64> = HashMap::new();
    let mut full_candidates: HashMap<u64, Vec<(Entity, f32, i32)>> = HashMap::new();
    let mut active_candidates: HashMap<u64, Vec<(Entity, f32, i32)>> = HashMap::new();
    let mut zone_candidates: HashMap<(i32, i32), Vec<(Entity, f32, i32)>> = HashMap::new();

    for (entity, npc_tf, brain, scenario_runtime, brain_state, _owner, _lease, _lod_state) in &mut npcs {
        let nearest_player = player_entries
            .iter()
            .map(|(client_id, pos)| (*client_id, pos.distance(npc_tf.translation)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let priority = npc_lod_priority_score(&brain_registry, scenario_runtime, brain_state, brain);

        let base_lod = match nearest_player {
            Some((_, distance)) if distance <= lod_config.full_radius => NpcAiLodLevel::Full,
            Some((_, distance)) if distance <= lod_config.reduced_radius => NpcAiLodLevel::Reduced,
            _ => NpcAiLodLevel::Background,
        };
        desired_lod_by_entity.insert(entity, base_lod);

        if let Some((client_id, distance)) = nearest_player {
            if !matches!(base_lod, NpcAiLodLevel::Background) {
                controlling_player_by_entity.insert(entity, client_id);
                active_candidates
                    .entry(client_id)
                    .or_default()
                    .push((entity, distance, priority));
                zone_candidates
                    .entry(npc_zone_key(npc_tf.translation, lod_config.zone_size))
                    .or_default()
                    .push((entity, distance, priority));
                if matches!(base_lod, NpcAiLodLevel::Full) {
                    full_candidates
                        .entry(client_id)
                        .or_default()
                        .push((entity, distance, priority));
                }
            }
        }
    }

    for candidates in full_candidates.values_mut() {
        candidates.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        for (idx, (entity, _, _)) in candidates.iter().enumerate() {
            if idx >= lod_config.full_budget_per_player {
                desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Reduced);
            }
        }
    }

    let total_active_budget = lod_config
        .full_budget_per_player
        .saturating_add(lod_config.reduced_budget_per_player);
    for candidates in active_candidates.values_mut() {
        candidates.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        for (idx, (entity, _, _)) in candidates.iter().enumerate() {
            if idx >= total_active_budget {
                desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Background);
            }
        }
    }

    let total_zone_budget = lod_config
        .full_budget_per_zone
        .saturating_add(lod_config.reduced_budget_per_zone);
    for candidates in zone_candidates.values_mut() {
        candidates.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        for (idx, (entity, _, _)) in candidates.iter().enumerate() {
            if idx >= total_zone_budget {
                desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Background);
            } else if idx >= lod_config.full_budget_per_zone {
                if matches!(desired_lod_by_entity.get(entity), Some(NpcAiLodLevel::Full)) {
                    desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Reduced);
                }
            }
        }
    }

    for (entity, npc_tf, _brain, _scenario_runtime, _brain_state, mut owner, mut lease, mut lod_state) in
        &mut npcs
    {
        lod_state.level = desired_lod_by_entity
            .get(&entity)
            .copied()
            .unwrap_or(NpcAiLodLevel::Background);

        if matches!(lod_state.level, NpcAiLodLevel::Background) {
            owner.0 = None;
            continue;
        }

        let assigned_player = controlling_player_by_entity.get(&entity).copied();
        let current_owner_distance = owner.0.and_then(|owner_id| {
            player_entries.iter().find_map(|(client_id, pos)| {
                if *client_id == owner_id {
                    Some(pos.distance(npc_tf.translation))
                } else {
                    None
                }
            })
        });

        let nearest = assigned_player.and_then(|client_id| {
            player_entries.iter().find_map(|(candidate_id, pos)| {
                if *candidate_id != client_id {
                    return None;
                }
                let dist = pos.distance(npc_tf.translation);
                if dist <= NPC_OWNERSHIP_RADIUS {
                    Some((dist, client_id))
                } else {
                    None
                }
            })
        });

        let owner_still_valid = current_owner_distance
            .map(|distance| distance <= NPC_OWNERSHIP_RELEASE_RADIUS)
            .unwrap_or(false);
        let cooldown_active = (now - lease.last_handoff_at) < NPC_OWNERSHIP_HANDOFF_COOLDOWN;

        let new_owner = if owner_still_valid {
            match (owner.0, current_owner_distance, nearest) {
                (Some(current_owner), Some(current_dist), Some((candidate_dist, candidate_id)))
                    if candidate_id != current_owner
                        && !cooldown_active
                        && candidate_dist + NPC_OWNERSHIP_HANDOFF_ADVANTAGE < current_dist =>
                {
                    Some(candidate_id)
                }
                (current_owner, _, _) => current_owner,
            }
        } else {
            nearest.map(|(_, id)| id)
        };

        if owner.0 != new_owner {
            if let Some(id) = new_owner {
                debug!(
                    "[npc_owner] NPC at {:?} -> client {} (prev={:?}, cooldown_active={})",
                    npc_tf.translation,
                    id,
                    owner.0,
                    cooldown_active
                );
            } else {
                debug!("[npc_owner] NPC at {:?} -> frozen (no player nearby)", npc_tf.translation);
            }
            lease.last_owner = owner.0;
            lease.last_handoff_at = now;
            owner.0 = new_owner;
        }
    }
}

pub fn tick_npc_agents(
    time: Res<Time<Fixed>>,
    lod_config: Res<NpcAiLodConfig>,
    side: Res<ResourcesSide>,
    world_state: Res<LuaWorldState>,
    mut npcs: Query<(
        &EntityHandle,
        &mut Transform,
        Option<&mut NetTransform>,
        &mut NpcAgent,
        Option<&mut ReplicatedNpcSteering>,
        Option<&NpcOwner>,
        Option<&NpcLastClientUpdate>,
        Option<&NpcAiLodState>,
    )>,
    globals: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let now = time.elapsed_secs();

    for (handle, mut transform, net_tf_opt, mut agent, steering_opt, owner, last_client_update, lod_state) in
        &mut npcs
    {
        let lod_level = lod_state.map(|lod| lod.level).unwrap_or(NpcAiLodLevel::Full);
        if !lod_allows_tick(handle.0, lod_level, &lod_config, now, dt) {
            if let Some(mut steering) = steering_opt {
                *steering = snapshot_npc_steering(&agent);
            }
            continue;
        }

        if let Some(o) = owner {
            if o.0.is_none() {
                if matches!(lod_level, NpcAiLodLevel::Background) {
                    continue;
                }
            }
            if matches!(side.0, Side::Server) {
                let client_owned_is_fresh = match (o.0, last_client_update) {
                    (Some(owner_id), Some(last_update)) => {
                        last_update.client_id == owner_id
                            && (now - last_update.received_at) <= NPC_CLIENT_UPDATE_TIMEOUT_SECS
                    }
                    _ => false,
                };

                if client_owned_is_fresh {
                    continue;
                }
            }
        }

        if matches!(agent.goal, NpcMoveGoal::Idle) {
            agent.reset_navigation_state();
            continue;
        }

        let mut stop_distance = agent.arrive_distance.max(0.01);
        let mut complete_goal = false;
        let mut advance_waypoint = false;

        if agent.avoidance_timer > 0.0 {
            agent.avoidance_timer = (agent.avoidance_timer - dt).max(0.0);
            if agent.avoidance_timer <= 0.0 {
                agent.avoidance_offset = Vec3::ZERO;
            }
        }

        let goal_snapshot = agent.goal.clone();
        let mut target_pos = if let Some(waypoint) = agent.current_path.get(agent.waypoint_index) {
            stop_distance = stop_distance.max(0.1);
            waypoint.target
        } else {
            match goal_snapshot {
                NpcMoveGoal::Idle => continue,
                NpcMoveGoal::GoToCoord {
                    target,
                    stop_distance: stop,
                } => {
                    stop_distance = stop.max(agent.arrive_distance).max(0.01);
                    target
                }
                NpcMoveGoal::GoToEntity {
                    target_handle,
                    stop_distance: stop,
                } => {
                    if let Some(target_entity) = world_state.entity_for(target_handle) {
                        if let Ok(t) = globals.get(target_entity) {
                            let target_translation = t.translation();
                            if let Some(previous) = agent.entity_target_position.replace(target_translation)
                            {
                                let observed_velocity = Vec3::new(
                                    (target_translation.x - previous.x) / dt,
                                    0.0,
                                    (target_translation.z - previous.z) / dt,
                                );
                                if observed_velocity.is_finite() {
                                    agent.entity_target_velocity =
                                        agent.entity_target_velocity.lerp(observed_velocity, 0.35);
                                }
                            }

                            stop_distance = stop.max(agent.arrive_distance).max(0.01);
                            if stop_distance >= 1.35 {
                                let max_offset = stop_distance.min(4.0).max(0.75);
                                if agent.formation_offset.length_squared() <= 0.0001 {
                                    let relative = Vec3::new(
                                        transform.translation.x - target_translation.x,
                                        0.0,
                                        transform.translation.z - target_translation.z,
                                    );
                                    if relative.length_squared() > 0.01 {
                                        agent.formation_offset = relative.clamp_length_max(max_offset);
                                    } else {
                                        agent.formation_offset = Vec3::new(max_offset * 0.6, 0.0, 0.0);
                                    }
                                } else {
                                    agent.formation_offset =
                                        agent.formation_offset.clamp_length_max(max_offset);
                                }
                                target_translation + agent.formation_offset
                            } else {
                                agent.formation_offset = Vec3::ZERO;
                                let pursuit_lead = Vec3::new(
                                    agent.entity_target_velocity.x,
                                    0.0,
                                    agent.entity_target_velocity.z,
                                )
                                .clamp_length_max(stop_distance.max(1.0) * 1.5)
                                    * 0.25;
                                target_translation + pursuit_lead
                            }
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                NpcMoveGoal::Wander {
                    kind,
                    radius,
                    retarget_sec,
                    orbit_angular_speed,
                    patrol_point,
                    clockwise,
                } => {
                    let radius = radius.max(0.1);
                    let retarget_sec = retarget_sec.max(0.05);

                    match kind {
                        NpcWanderKind::Random => {
                            agent.wander_timer -= dt;
                            let dist_to_curr = Vec2::new(
                                transform.translation.x - agent.wander_target.x,
                                transform.translation.z - agent.wander_target.z,
                            )
                            .length();
                            if agent.wander_timer <= 0.0 || dist_to_curr <= stop_distance {
                                let a = agent.next_rand01() * TAU;
                                let d = radius * (0.35 + 0.65 * agent.next_rand01());
                                agent.wander_target = Vec3::new(
                                    agent.home.x + a.cos() * d,
                                    transform.translation.y,
                                    agent.home.z + a.sin() * d,
                                );
                                agent.wander_timer = retarget_sec;
                            }
                            agent.wander_target
                        }
                        NpcWanderKind::Patrol => {
                            let patrol =
                                patrol_point.unwrap_or_else(|| agent.home + Vec3::new(radius, 0.0, 0.0));
                            let curr_target = if agent.patrol_to_target {
                                patrol
                            } else {
                                agent.home
                            };

                            let d = Vec2::new(
                                transform.translation.x - curr_target.x,
                                transform.translation.z - curr_target.z,
                            )
                            .length();
                            if d <= stop_distance {
                                agent.patrol_to_target = !agent.patrol_to_target;
                            }

                            agent.wander_target = if agent.patrol_to_target {
                                patrol
                            } else {
                                agent.home
                            };
                            agent.wander_target
                        }
                        NpcWanderKind::Orbit => {
                            let sign = if clockwise { -1.0 } else { 1.0 };
                            agent.orbit_angle =
                                (agent.orbit_angle + sign * orbit_angular_speed.max(0.05) * dt)
                                    .rem_euclid(TAU);
                            stop_distance = (radius * 0.15).max(agent.arrive_distance).max(0.1);
                            agent.wander_target = Vec3::new(
                                agent.home.x + radius * agent.orbit_angle.cos(),
                                transform.translation.y,
                                agent.home.z + radius * agent.orbit_angle.sin(),
                            );
                            agent.wander_target
                        }
                    }
                }
            }
        };

        if agent.avoidance_timer > 0.0 {
            target_pos += agent.avoidance_offset;
        }

        let to_target = Vec2::new(
            target_pos.x - transform.translation.x,
            target_pos.z - transform.translation.z,
        );
        let dist = to_target.length();

        if dist <= stop_distance {
            if agent.waypoint_index < agent.current_path.len() {
                advance_waypoint = true;
            } else if matches!(
                goal_snapshot,
                NpcMoveGoal::GoToCoord { .. } | NpcMoveGoal::GoToEntity { .. }
            ) {
                complete_goal = true;
            }
        } else {
            let dir = to_target / dist;
            let step = (agent.move_speed.max(0.0) * dt).min(dist - stop_distance);
            transform.translation.x += dir.x * step;
            transform.translation.z += dir.y * step;

            let desired_yaw = dir.x.atan2(dir.y);
            let desired_rot = Quat::from_rotation_y(desired_yaw);
            let t = (agent.turn_speed.max(0.0) * dt).clamp(0.0, 1.0);
            transform.rotation = transform.rotation.slerp(desired_rot, t);
        }

        if advance_waypoint {
            agent.waypoint_index += 1;
            if agent.waypoint_index >= agent.current_path.len() {
                agent.current_path.clear();
                agent.waypoint_index = 0;
                if matches!(
                    goal_snapshot,
                    NpcMoveGoal::GoToCoord { .. } | NpcMoveGoal::GoToEntity { .. }
                ) {
                    complete_goal = true;
                }
            }
        }

        if complete_goal {
            agent.goal = NpcMoveGoal::Idle;
        }

        if let Some(mut net_tf) = net_tf_opt {
            net_tf.translation = transform.translation;
            net_tf.rotation = transform.rotation;
        }
        if let Some(mut steering) = steering_opt {
            *steering = snapshot_npc_steering(&agent);
        }
    }
}
