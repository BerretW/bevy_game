//! Phase 3 — gameplay simulace (server-authoritativni pohyb + combat).
//!
//! Phase 3.3 pridava:
//! * `Health` component a `WeaponConfig` resource.
//! * Server-side combat systemy: proximity/angle hit check z PlayerInput.look
//!   a actions bitfield, cooldown management.
//! * Lua eventy: `playerConnecting`, `playerDropped`, `onPlayerHit`, `onPlayerDeath`.

use bevy::prelude::*;
use core_resources::{
    ActiveWeaponSlot, AmmoRegistry, AmmoReserve, ArmorClass, ArmorComponent, ArmorPiece, FireState, Inventory, LocalEventBus,
    HitboxBoneDef, HitboxDef, HitboxRegistry, LuaWorldState, NpcLastClientUpdate, NpcOwner,
    NpcPathWaypoint, NpcPedMarker, PlayerEntityMap, PlayerHitbox, PlayerStatsCache,
    ReloadState, ReplicatedNpcSteering, Stats, StatsSnapshot, WeaponRegistry, WeaponSlots,
    WeaponSwapState,
    apply_replicated_npc_steering,
};
use core_shared::{Health, NetTransform, PlayerMarker};
use lightyear::prelude::*;

use crate::net_plugin::StatsChannel;
use crate::protocol::{NpcTransformUpdate, player_action, PlayerStatsUpdate};

mod players;
mod lifecycle;
mod weapons;

use lifecycle::{
    attach_replication_sender, attach_replication_to_networked_object,
    emit_player_disconnect, spawn_player_on_connect,
};
pub use players::{collect_last_inputs, LastPlayerInputs};
use players::{
    PositionHistory, ServerSimulationTick, emit_player_positions,
    increment_server_simulation_tick, record_position_history, trust_client_position,
};
use weapons::{
    is_reload_active, is_weapon_swap_active, reload_duration_for_slot,
    requested_weapon_slot, tick_fire_states, tick_reload_states, tick_weapon_swap_states,
    weapon_swap_duration,
};

pub const PLAYER_MOVE_SPEED: f32 = 5.0;
pub const PLAYER_SPRINT_MULTIPLIER: f32 = 1.35;
pub const PLAYER_CROUCH_MULTIPLIER: f32 = 0.45;
pub const PLAYER_JUMP_SPEED: f32 = 6.5;
pub const PLAYER_GRAVITY: f32 = 20.0;
pub const GROUND_Y: f32 = 0.0;
const DEFAULT_WEAPON_SWAP_SECS: f32 = 0.25;
const MIN_WEAPON_ACTION_SECS: f32 = 0.05;
const PLAYER_EYE_HEIGHT: f32 = 1.55;
const LAG_COMPENSATION_REWIND_TICKS: u32 = 2;

// ---------------------------------------------------------------------------
// Komponenty
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// WeaponConfig — Bevy Resource (Lua resource mu muze pridat vlastni konfiguraci)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponType {
    /// Vzdalenostni zbran — provede proximity+angle check.
    Ranged,
    /// Melee — jen proximity check, kratky dosah.
    Melee,
}

/// Globalni default konfigurace zbrane. Lua resource (napr. core/weapons)
/// ji muze v Startup zmenit pres `World.ApplyDamage` nebo budoucim
/// `WeaponConfig` Lua API.
#[derive(Resource, Debug, Clone)]
pub struct WeaponConfig {
    pub fire_rate: f32,     // pocet vystrelu za sekundu
    pub damage: f32,        // poskozeni na zasah
    pub range: f32,         // max dosah v herních jednotkach
    pub cone_angle: f32,    // polouhel zasahoveho kuzele ve stupnich
    pub weapon_type: WeaponType,
}

impl Default for WeaponConfig {
    fn default() -> Self {
        Self {
            fire_rate: 5.0,
            damage: 20.0,
            range: 15.0,
            cone_angle: 30.0,
            weapon_type: WeaponType::Ranged,
        }
    }
}

// ---------------------------------------------------------------------------
// ServerSimPlugin
// ---------------------------------------------------------------------------

pub struct ServerSimPlugin;

impl Plugin for ServerSimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeaponConfig>();
        app.init_resource::<LastPlayerInputs>();
        app.init_resource::<ServerSimulationTick>();
        app.add_observer(attach_replication_sender);
        app.add_observer(spawn_player_on_connect);
        app.add_observer(emit_player_disconnect);
        app.add_observer(attach_replication_to_networked_object);
        // collect_last_inputs bezi v Update, drainuje MessageReceiver.
        // apply_inputs + process_combat cte LastPlayerInputs v FixedUpdate.
        app.add_systems(Update, collect_last_inputs);
        app.add_systems(Update, receive_npc_transform_updates);
        app.add_systems(
            FixedUpdate,
            (
                increment_server_simulation_tick,
                // FiveM-style: server důvěřuje klientské fyzikální pozici.
                // apply_inputs_to_velocity + integrate_velocity nahrazeny trust_client_position.
                trust_client_position,
                record_position_history,
                emit_player_positions,
                tick_fire_states,
                tick_weapon_swap_states,
                tick_reload_states,
                process_combat,
                sync_player_state_cache,
                broadcast_player_stats,
            )
                .chain(),
        );
    }
}

struct EffectiveCombatWeaponConfig {
    weapon_id: String,
    ammo_id: Option<String>,
    fire_mode: String,
    fire_rate: f32,
    damage: f32,
    penetration_class: u32,
    armor_penetration: f32,
    wound_mult: f32,
    range: f32,
    cone_angle: f32,
    weapon_type: WeaponType,
}

fn resolve_combat_weapon_config(
    weapon_registry: &WeaponRegistry,
    ammo_registry: &AmmoRegistry,
    fallback: &WeaponConfig,
    equipped_weapon: Option<&core_resources::EquippedWeapon>,
) -> EffectiveCombatWeaponConfig {
    let weapon_def = equipped_weapon
        .and_then(|weapon| weapon_registry.get(&weapon.weapon_id))
        .or_else(|| weapon_registry.resolve_default());

    let Some(weapon_def) = weapon_def else {
        return EffectiveCombatWeaponConfig {
            weapon_id: "default".to_string(),
            ammo_id: None,
            fire_mode: "full".to_string(),
            fire_rate: fallback.fire_rate,
            damage: fallback.damage,
            penetration_class: 0,
            armor_penetration: 0.0,
            wound_mult: 1.0,
            range: fallback.range,
            cone_angle: fallback.cone_angle,
            weapon_type: fallback.weapon_type.clone(),
        };
    };

    let fire_rate = if weapon_def.rpm > 0.0 {
        (weapon_def.rpm / 60.0).max(0.01)
    } else {
        fallback.fire_rate
    };

    let ammo_id = equipped_weapon
        .and_then(|weapon| {
            let ammo_id = weapon.ammo_type_id.trim();
            if ammo_id.is_empty() {
                None
            } else {
                Some(ammo_id.to_string())
            }
        })
        .filter(|ammo_id| !ammo_id.is_empty())
        .or_else(|| {
            if weapon_def.default_ammo.trim().is_empty() {
                None
            } else {
                Some(weapon_def.default_ammo.clone())
            }
        });

    let fire_mode = equipped_weapon
        .and_then(|weapon| {
            let mode = weapon.fire_mode.trim();
            if mode.is_empty() {
                None
            } else {
                Some(mode.to_ascii_lowercase())
            }
        })
        .or_else(|| {
            let mode = weapon_def.default_fire_mode.trim();
            if mode.is_empty() {
                None
            } else {
                Some(mode.to_ascii_lowercase())
            }
        })
        .or_else(|| {
            weapon_def
                .fire_modes
                .first()
                .map(|mode| mode.to_ascii_lowercase())
        })
        .unwrap_or_else(|| "full".to_string());

    let damage = if let Some(ammo_id) = ammo_id.as_ref() {
        ammo_registry
            .get(ammo_id)
            .map(|ammo| ammo.base_damage)
            .filter(|value| *value > 0.0)
            .unwrap_or(fallback.damage)
    } else {
        fallback.damage
    };

    let (penetration_class, armor_penetration, wound_mult) = ammo_id
        .as_ref()
        .and_then(|ammo_id| ammo_registry.get(ammo_id))
        .map(|ammo| {
            (
                ammo.penetration_class,
                ammo.armor_penetration.clamp(0.0, 1.0),
                ammo.wound_mult.max(0.0),
            )
        })
        .unwrap_or((0, 0.0, 1.0));

    let weapon_type = match weapon_def.category.trim().to_ascii_lowercase().as_str() {
        "melee" => WeaponType::Melee,
        _ => WeaponType::Ranged,
    };

    let range = match weapon_type {
        WeaponType::Melee => 3.0,
        WeaponType::Ranged => fallback.range,
    };

    EffectiveCombatWeaponConfig {
        weapon_id: weapon_def.id,
        ammo_id,
        fire_mode,
        fire_rate,
        damage,
        penetration_class,
        armor_penetration,
        wound_mult,
        range,
        cone_angle: fallback.cone_angle,
        weapon_type,
    }
}

fn fire_mode_allows_shot(fire_mode: &str, trigger_pressed: bool, trigger_was_held: bool) -> bool {
    if !trigger_pressed {
        return false;
    }

    match fire_mode {
        "semi" | "burst" => !trigger_was_held,
        _ => true,
    }
}

#[derive(Debug, Clone)]
struct ResolvedHitboxHit {
    hitzone: String,
    armor_zone: Option<String>,
    armor_bypass: f32,
    mult: f32,
    headshot: bool,
    distance_m: f32,
    hit_pos: Vec3,
}

fn armor_zone_for_bone(hitbox: &HitboxDef, bone_name: &str) -> Option<String> {
    hitbox
        .armor_zones
        .iter()
        .find(|(_, zone)| zone.bones.iter().any(|bone| bone == bone_name))
        .map(|(zone_name, _)| zone_name.clone())
}

fn armor_class_rank(class: ArmorClass) -> u32 {
    match class {
        ArmorClass::I => 1,
        ArmorClass::Ii => 2,
        ArmorClass::Iiia => 3,
        ArmorClass::Iii => 4,
        ArmorClass::Iv => 5,
    }
}

fn armor_piece_for_zone_mut<'a>(armor: &'a mut ArmorComponent, zone: &str) -> Option<&'a mut ArmorPiece> {
    match zone {
        "helmet" => armor.helmet.as_mut(),
        "vest" => armor.vest.as_mut(),
        _ => None,
    }
}

fn resolve_armor_damage(
    armor: &mut ArmorComponent,
    armor_zone: Option<&str>,
    armor_bypass: f32,
    penetration_class: u32,
    armor_penetration: f32,
    incoming_damage: f32,
) -> (f32, f32, bool) {
    let Some(zone) = armor_zone else {
        return (incoming_damage.max(0.0), 0.0, false);
    };
    let Some(piece) = armor_piece_for_zone_mut(armor, zone) else {
        return (incoming_damage.max(0.0), 0.0, false);
    };

    let durability_ratio = if piece.max_durability > 0.0 {
        (piece.durability / piece.max_durability).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let class_rank = armor_class_rank(piece.class);
    let class_factor = if penetration_class <= class_rank {
        1.0
    } else if penetration_class == class_rank + 1 {
        0.65
    } else {
        0.35
    };
    let retained_fraction = ((1.0 - armor_bypass.clamp(0.0, 1.0))
        * (1.0 - armor_penetration.clamp(0.0, 0.95))
        * class_factor
        * durability_ratio)
        .clamp(0.0, 0.95);
    let absorbed = incoming_damage.max(0.0) * retained_fraction;
    let final_damage = (incoming_damage.max(0.0) - absorbed).max(0.0);
    let penetrated = final_damage > 0.01;

    let durability_loss = (incoming_damage.max(0.0)
        * (0.35 + armor_penetration.clamp(0.0, 1.0) + armor_bypass.clamp(0.0, 1.0) * 0.5))
        .max(1.0);
    piece.durability = (piece.durability - durability_loss).max(0.0);

    if piece.durability <= 0.0 {
        match zone {
            "helmet" => armor.helmet = None,
            "vest" => armor.vest = None,
            _ => {}
        }
    }

    (final_damage, absorbed, penetrated)
}

fn look_direction(yaw_deg: f32, pitch_deg: f32) -> Vec3 {
    let yaw = yaw_deg.to_radians();
    let pitch = pitch_deg.to_radians();
    Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    )
    .normalize_or_zero()
}

fn closest_points_between_segments(
    p1: Vec3,
    q1: Vec3,
    p2: Vec3,
    q2: Vec3,
) -> (Vec3, Vec3, f32, f32) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);

    const EPS: f32 = 1e-6;

    let (mut s, t);
    if a <= EPS && e <= EPS {
        return (p1, p2, 0.0, 0.0);
    }

    if a <= EPS {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            if denom.abs() > EPS {
                s = ((b * f - c * e) / denom).clamp(0.0, 1.0);
            } else {
                s = 0.0;
            }
            let tnom = b * s + f;
            if tnom <= 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if tnom >= e {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = tnom / e;
            }
        }
    }

    let c1 = p1 + d1 * s;
    let c2 = p2 + d2 * t;
    (c1, c2, s, t)
}

fn resolve_hitbox_hit(
    hitbox: &HitboxDef,
    target_base: Vec3,
    ray_origin: Vec3,
    ray_dir: Vec3,
    max_range: f32,
) -> Option<ResolvedHitboxHit> {
    let ray_end = ray_origin + ray_dir * max_range.max(0.01);
    let mut best: Option<ResolvedHitboxHit> = None;

    for (bone_name, bone) in &hitbox.bones {
        let candidate = resolve_hitbox_bone_hit(
            bone_name,
            bone,
            armor_zone_for_bone(hitbox, bone_name),
            target_base,
            ray_origin,
            ray_end,
        )?;
        let replace = best
            .as_ref()
            .map(|current| candidate.distance_m < current.distance_m)
            .unwrap_or(true);
        if replace {
            best = Some(candidate);
        }
    }

    best
}

fn resolve_hitbox_bone_hit(
    bone_name: &str,
    bone: &HitboxBoneDef,
    armor_zone: Option<String>,
    target_base: Vec3,
    ray_origin: Vec3,
    ray_end: Vec3,
) -> Option<ResolvedHitboxHit> {
    let start = target_base + Vec3::Y * (bone.capsule.oy - bone.capsule.hh);
    let end = target_base + Vec3::Y * (bone.capsule.oy + bone.capsule.hh);
    let (ray_point, capsule_point, _ray_t, _) = closest_points_between_segments(ray_origin, ray_end, start, end);
    let radius = bone.capsule.r.max(0.001);
    if ray_point.distance_squared(capsule_point) > radius * radius {
        return None;
    }

    Some(ResolvedHitboxHit {
        hitzone: bone_name.to_string(),
        armor_zone,
        armor_bypass: bone.armor_bypass.clamp(0.0, 1.0),
        mult: bone.mult.max(0.0),
        headshot: matches!(bone_name, "head" | "neck"),
        distance_m: ray_origin.distance(ray_point),
        hit_pos: capsule_point,
    })
}

fn rewind_tick_for_shot(current_tick: u32) -> u32 {
    current_tick.saturating_sub(LAG_COMPENSATION_REWIND_TICKS)
}

// ---------------------------------------------------------------------------
// Combat (Phase 3.3)
// ---------------------------------------------------------------------------

/// Server-side combat systemy:
/// 1. Nacte nejnovejsi input kazdeho klienta.
/// 2. Zkontroluje PRIMARY_FIRE bitflag a FireState.
/// 3. Proximity + angle check vuci ostatnim hracum.
/// 4. Aplikuje damage, emituje Lua eventy onPlayerHit / onPlayerDeath.
fn process_combat(
    // Cte vsechny prisly inputy — POZOR: inputs uz byly drainovany v
    // apply_inputs_to_velocity. Musime mit vlastni kopii nebo drat last input.
    // Reseni: pouzijeme LastInput resource naplnenou v apply_inputs_to_velocity.
    last_inputs: Res<LastPlayerInputs>,
    weapon_registry: Res<WeaponRegistry>,
    ammo_registry: Res<AmmoRegistry>,
    hitbox_registry: Res<HitboxRegistry>,
    weapon_cfg: Res<WeaponConfig>,
    sim_tick: Res<ServerSimulationTick>,
    mut players: Query<(
        Entity,
        &PlayerMarker,
        &PlayerHitbox,
        &NetTransform,
        &PositionHistory,
        &mut Health,
        &mut ArmorComponent,
        &mut WeaponSlots,
        &mut AmmoReserve,
        &mut ActiveWeaponSlot,
        &mut FireState,
        &mut ReloadState,
        &mut WeaponSwapState,
    )>,
    local_bus: Res<LocalEventBus>,
) {
    // Sbir potencialnich strelcu
    struct Attacker {
        entity: Entity,
        client_id: u64,
        origin: Vec3,
        look_yaw: f32,
        look_pitch: f32,
        weapon: EffectiveCombatWeaponConfig,
    }

    let mut attackers: Vec<Attacker> = Vec::new();

    // Iterujeme hrace a zjistime, kteri chtejí strilet (na zaklade LastPlayerInputs)
    {
        for (entity, marker, _hitbox_profile, transform, _history, _health, _armor, mut slots, reserve, active_slot, mut fire_state, mut reload_state, mut weapon_swap) in players.iter_mut() {
            let Some(input) = last_inputs.get(marker.client_id) else { continue };
            let trigger_pressed = input.actions & player_action::PRIMARY_FIRE != 0;
            let trigger_was_held = fire_state.trigger_held;
            fire_state.trigger_held = trigger_pressed;

            if let Some(slot) = requested_weapon_slot(input.actions) {
                if slot < 4 && slot != active_slot.0 {
                    *fire_state = FireState::default();
                    if is_reload_active(&reload_state) {
                        *reload_state = ReloadState::default();
                    }
                    if !is_weapon_swap_active(&weapon_swap) || weapon_swap.target_slot != slot {
                        let duration = weapon_swap_duration(&weapon_registry, &slots, active_slot.0, slot);
                        *weapon_swap = WeaponSwapState {
                            remaining: duration,
                            duration,
                            target_slot: slot,
                        };
                    }
                }
            }

            let active_slot_value = active_slot.0;

            if input.actions & player_action::RELOAD != 0
                && !is_weapon_swap_active(&weapon_swap)
                && !is_reload_active(&reload_state)
            {
                *fire_state = FireState::default();
                if let Some(duration) = reload_duration_for_slot(&weapon_registry, &slots, &reserve, active_slot_value) {
                    *reload_state = ReloadState {
                        remaining: duration,
                        duration,
                        slot: active_slot_value,
                    };
                }
            }

            if is_weapon_swap_active(&weapon_swap) || is_reload_active(&reload_state) {
                continue;
            }

            let equipped_snapshot = slots.get(active_slot_value).cloned();
            let effective_weapon = resolve_combat_weapon_config(
                &weapon_registry,
                &ammo_registry,
                &weapon_cfg,
                equipped_snapshot.as_ref(),
            );

            if !fire_mode_allows_shot(&effective_weapon.fire_mode, trigger_pressed, trigger_was_held) {
                continue;
            }

            let fire_interval = if effective_weapon.fire_rate > 0.0 {
                1.0 / effective_weapon.fire_rate
            } else {
                f32::MAX
            };
            fire_state.shot_interval = fire_interval;

            if fire_state.cooldown_remaining > 0.0 {
                continue;
            }

            if let Some(equipped) = slots.get_mut(active_slot_value) {
                if equipped.ammo_in_mag == 0 {
                    continue;
                }
                equipped.ammo_in_mag = equipped.ammo_in_mag.saturating_sub(1);
            }

            fire_state.cooldown_remaining = fire_interval;
            attackers.push(Attacker {
                entity,
                client_id: marker.client_id,
                origin: transform.translation + Vec3::Y * PLAYER_EYE_HEIGHT,
                look_yaw: input.look[0],
                look_pitch: input.look[1],
                weapon: effective_weapon,
            });
        }
    }

    if attackers.is_empty() {
        return;
    }

    // Snap shot pozic vsech hracu
    let rewind_tick = rewind_tick_for_shot(sim_tick.0);
    let targets: Vec<(Entity, u64, Vec3, f32, String)> = players
        .iter()
        .map(|(e, m, hitbox, t, history, h, _, _, _, _, _, _, _)| {
            (
                e,
                m.client_id,
                history.position_at_or_before(rewind_tick).unwrap_or(t.translation),
                h.current,
                hitbox.0.clone(),
            )
        })
        .collect();

    for attacker in &attackers {
        let look_dir = look_direction(attacker.look_yaw, attacker.look_pitch);
        if look_dir.length_squared() <= 0.0 {
            continue;
        }

        let mut best_hit: Option<(Entity, u64, ResolvedHitboxHit)> = None;

        for (target_entity, target_cid, target_pos, _target_hp, hitbox_profile) in &targets {
            if *target_entity == attacker.entity {
                continue;
            }

            let maybe_hit = match attacker.weapon.weapon_type {
                WeaponType::Melee => {
                    let diff = *target_pos - attacker.origin;
                    let dist_sq = diff.length_squared();
                    if dist_sq > attacker.weapon.range * attacker.weapon.range {
                        None
                    } else {
                        Some(ResolvedHitboxHit {
                            hitzone: "chest".to_string(),
                            armor_zone: Some("vest".to_string()),
                            armor_bypass: 0.0,
                            mult: 1.0,
                            headshot: false,
                            distance_m: diff.length(),
                            hit_pos: *target_pos + Vec3::Y * 1.3,
                        })
                    }
                }
                WeaponType::Ranged => {
                    let to_target = (*target_pos + Vec3::Y * 1.2) - attacker.origin;
                    if to_target.length_squared() <= 0.0001 {
                        None
                    } else {
                        let angle = look_dir
                            .dot(to_target.normalize())
                            .clamp(-1.0, 1.0)
                            .acos();
                        if angle > attacker.weapon.cone_angle.to_radians() {
                            None
                        } else {
                            let hitbox = hitbox_registry
                                .get(hitbox_profile)
                                .or_else(|| hitbox_registry.resolve_default());
                            hitbox.and_then(|def| {
                                resolve_hitbox_hit(&def, *target_pos, attacker.origin, look_dir, attacker.weapon.range)
                            })
                        }
                    }
                }
            };

            let Some(hit) = maybe_hit else { continue };
            let replace = best_hit
                .as_ref()
                .map(|(_, _, current)| hit.distance_m < current.distance_m)
                .unwrap_or(true);
            if replace {
                best_hit = Some((*target_entity, *target_cid, hit));
            }
        }

        let Some((target_entity, target_cid, hit)) = best_hit else {
            continue;
        };

        if let Ok((_, _, _, _, _, mut health, mut armor, _, _, _, _, _, _)) = players.get_mut(target_entity) {
            let raw_damage = attacker.weapon.damage;
            let zone_damage = raw_damage * hit.mult.max(0.0) * attacker.weapon.wound_mult.max(0.0);
            let (damage, armor_absorbed, penetrated_armor) = resolve_armor_damage(
                &mut armor,
                hit.armor_zone.as_deref(),
                hit.armor_bypass,
                attacker.weapon.penetration_class,
                attacker.weapon.armor_penetration,
                zone_damage,
            );
            health.current -= damage;
            let died = health.current <= 0.0;
            if died {
                health.current = 0.0;
            }

            let payload = serde_json::to_vec(&serde_json::json!({
                "attacker": attacker.client_id.to_string(),
                "victim": target_cid.to_string(),
                "damage": damage,
                "raw_damage": raw_damage,
                "armor_absorbed": armor_absorbed,
                "penetrated_armor": penetrated_armor,
                "hitzone": hit.hitzone,
                "armor_zone": hit.armor_zone,
                "headshot": hit.headshot,
                "weapon": attacker.weapon.weapon_id,
                "ammo": attacker.weapon.ammo_id,
                "distance_m": hit.distance_m,
                "position": { "x": hit.hit_pos.x, "y": hit.hit_pos.y, "z": hit.hit_pos.z }
            }))
            .unwrap_or_default();
            local_bus.push("onPlayerHit".to_string(), payload.clone());
            local_bus.push("onPlayerDamage".to_string(), payload);

            if died {
                let death_payload = serde_json::to_vec(&serde_json::json!({
                    "victim": target_cid.to_string(),
                    "killer": attacker.client_id.to_string(),
                    "cause": "weapon",
                    "weapon": attacker.weapon.weapon_id,
                    "hitzone": hit.hitzone,
                    "headshot": hit.headshot,
                }))
                .unwrap_or_default();
                local_bus.push("onPlayerDeath".to_string(), death_payload);

                info!(
                    "[sim/combat] player {} killed by {} hitzone={} headshot={}",
                    target_cid, attacker.client_id, hit.hitzone, hit.headshot
                );
            } else {
                debug!(
                    "[sim/combat] player {} hit {} zone={} armor_zone={:?} for {:.1} dmg (absorbed {:.1}, hp={:.1})",
                    attacker.client_id, target_cid, hit.hitzone, hit.armor_zone, damage, armor_absorbed, health.current
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// emit_player_positions — posila pozice vsech hracu do Lua event busu
// ---------------------------------------------------------------------------

/// Kazdy FixedUpdate tick posle `onPlayerPosition` do LocalEventBus.
/// Server Lua resource to muze dale poslat klientum pres TriggerClientEvent.
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
            if !(px.is_finite() && py.is_finite() && pz.is_finite() && rx.is_finite() && ry.is_finite() && rz.is_finite() && rw.is_finite()) {
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
                wander_target: Vec3::new(update.wander_target[0], update.wander_target[1], update.wander_target[2]),
                wander_timer: update.wander_timer,
                orbit_angle: update.orbit_angle,
                patrol_to_target: update.patrol_to_target,
                current_path: update.current_path.iter().map(|p| NpcPathWaypoint {
                    target: Vec3::new(p[0], p[1], p[2]),
                }).collect(),
                waypoint_index: update.waypoint_index,
                map_id: update.map_id.clone(),
                last_nav_target: update.last_nav_target.map(|p| Vec3::new(p[0], p[1], p[2])),
                entity_target_position: update.entity_target_position.map(|p| Vec3::new(p[0], p[1], p[2])),
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

// ---------------------------------------------------------------------------
// broadcast_player_stats — každých 6 FixedUpdate ticků (~10 Hz)
// ---------------------------------------------------------------------------

fn broadcast_player_stats(
    players: Query<(
        &PlayerMarker,
        &Health,
        Option<&ArmorComponent>,
        Option<&WeaponSlots>,
        Option<&AmmoReserve>,
        Option<&ActiveWeaponSlot>,
        Option<&FireState>,
        Option<&ReloadState>,
        Option<&WeaponSwapState>,
    )>,
    mut senders: Query<(&mut MessageSender<PlayerStatsUpdate>, &RemoteId)>,
    mut tick: Local<u32>,
) {
    *tick = tick.wrapping_add(1);
    if *tick % 6 != 0 {
        return;
    }

    for (mut sender, remote_id) in senders.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => continue,
        };
        // Najdi entitu hráče patřící tomuto klientovi
        if let Some((_, health, armor, weapon_slots, ammo_reserve, active_slot, fire_state, reload_state, weapon_swap)) =
            players.iter().find(|(m, _, _, _, _, _, _, _, _)| m.client_id == client_id)
        {
            let msg = PlayerStatsUpdate {
                hp: health.current,
                max_hp: health.max,
                armor: armor.cloned().unwrap_or_default(),
                weapon_slots: weapon_slots
                    .map(|value| value.0.iter().cloned().collect())
                    .unwrap_or_else(|| vec![None, None, None, None]),
                ammo_reserve: ammo_reserve.map(|value| value.0.clone()).unwrap_or_default(),
                active_weapon_slot: active_slot.map(|value| value.0).unwrap_or(0),
                fire_cooldown_remaining: fire_state.map(|value| value.cooldown_remaining).unwrap_or(0.0),
                fire_trigger_held: fire_state.map(|value| value.trigger_held).unwrap_or(false),
                reload_remaining: reload_state.map(|value| value.remaining).unwrap_or(0.0),
                reload_duration: reload_state.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_remaining: weapon_swap.map(|value| value.remaining).unwrap_or(0.0),
                weapon_swap_duration: weapon_swap.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_target_slot: weapon_swap
                    .map(|value| if value.remaining > 0.0 { Some(value.target_slot) } else { None })
                    .unwrap_or(None),
            };
            sender.send::<StatsChannel>(msg);
        }
    }
}

fn sync_player_state_cache(
    players: Query<
        (
            Entity,
            &PlayerMarker,
            &Health,
            Option<&Stats>,
            Option<&Inventory>,
            Option<&ArmorComponent>,
            Option<&WeaponSlots>,
            Option<&AmmoReserve>,
            Option<&ActiveWeaponSlot>,
            Option<&FireState>,
            Option<&ReloadState>,
            Option<&WeaponSwapState>,
        ),
    >,
    mut player_map: ResMut<PlayerEntityMap>,
    stats_cache: Res<PlayerStatsCache>,
) {
    let mut seen = std::collections::HashSet::new();

    for (entity, marker, health, stats, inventory, armor, weapon_slots, ammo_reserve, active_slot, fire_state, reload_state, weapon_swap) in &players {
        seen.insert(marker.client_id);
        player_map.map.insert(marker.client_id, entity);
        stats_cache.update(
            marker.client_id,
            StatsSnapshot {
                stats: stats.map(|value| value.0.clone()).unwrap_or_default(),
                inventory: inventory.map(|value| value.0.clone()).unwrap_or_default(),
                health: health.current,
                max_health: health.max,
                armor: armor.cloned().unwrap_or_default(),
                weapon_slots: weapon_slots
                    .map(|value| value.0.iter().cloned().collect())
                    .unwrap_or_else(|| vec![None, None, None, None]),
                ammo_reserve: ammo_reserve.map(|value| value.0.clone()).unwrap_or_default(),
                active_weapon_slot: active_slot.map(|value| value.0).unwrap_or(0),
                fire_cooldown_remaining: fire_state.map(|value| value.cooldown_remaining).unwrap_or(0.0),
                fire_trigger_held: fire_state.map(|value| value.trigger_held).unwrap_or(false),
                reload_remaining: reload_state.map(|value| value.remaining).unwrap_or(0.0),
                reload_duration: reload_state.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_remaining: weapon_swap.map(|value| value.remaining).unwrap_or(0.0),
                weapon_swap_duration: weapon_swap.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_target_slot: weapon_swap
                    .map(|value| if value.remaining > 0.0 { Some(value.target_slot) } else { None })
                    .unwrap_or(None),
            },
        );
    }

    player_map.map.retain(|client_id, _| seen.contains(client_id));
    stats_cache.retain_ids(&seen);
}
