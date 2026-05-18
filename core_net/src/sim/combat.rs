use bevy::prelude::*;
use core_resources::{
    ActiveWeaponSlot, AmmoRegistry, AmmoReserve, ArmorClass, ArmorComponent, ArmorPiece,
    FireState, HitboxBoneDef, HitboxDef, HitboxRegistry, LocalEventBus, PlayerHitbox,
    ReloadState, WeaponRegistry, WeaponSlots, WeaponSwapState,
};
use core_shared::{Health, NetTransform, PlayerMarker};

use crate::protocol::player_action;

use super::players::{PositionHistory, ServerSimulationTick};
use super::weapons::{
    is_reload_active, is_weapon_swap_active, reload_duration_for_slot, requested_weapon_slot,
    weapon_swap_duration,
};
use super::{LastPlayerInputs, WeaponConfig, WeaponType};

const PLAYER_EYE_HEIGHT: f32 = 1.55;
const LAG_COMPENSATION_REWIND_TICKS: u32 = 2;

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
    let (ray_point, capsule_point, _ray_t, _) =
        closest_points_between_segments(ray_origin, ray_end, start, end);
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

/// Server-side combat systemy:
/// 1. Nacte nejnovejsi input kazdeho klienta.
/// 2. Zkontroluje PRIMARY_FIRE bitflag a FireState.
/// 3. Proximity + angle check vuci ostatnim hracum.
/// 4. Aplikuje damage, emituje Lua eventy onPlayerHit / onPlayerDeath.
pub fn process_combat(
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
    struct Attacker {
        entity: Entity,
        client_id: u64,
        origin: Vec3,
        look_yaw: f32,
        look_pitch: f32,
        weapon: EffectiveCombatWeaponConfig,
    }

    let mut attackers: Vec<Attacker> = Vec::new();

    {
        for (entity, marker, _hitbox_profile, transform, _history, _health, _armor, mut slots, reserve, active_slot, mut fire_state, mut reload_state, mut weapon_swap) in players.iter_mut() {
            let Some(input) = last_inputs.get(marker.client_id) else {
                continue;
            };
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
                        let duration =
                            weapon_swap_duration(&weapon_registry, &slots, active_slot.0, slot);
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
                if let Some(duration) =
                    reload_duration_for_slot(&weapon_registry, &slots, &reserve, active_slot_value)
                {
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

    let rewind_tick = rewind_tick_for_shot(sim_tick.0);
    let targets: Vec<(Entity, u64, Vec3, f32, String)> = players
        .iter()
        .map(|(e, m, hitbox, t, history, h, _, _, _, _, _, _, _)| {
            (
                e,
                m.client_id,
                history
                    .position_at_or_before(rewind_tick)
                    .unwrap_or(t.translation),
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
                        let angle = look_dir.dot(to_target.normalize()).clamp(-1.0, 1.0).acos();
                        if angle > attacker.weapon.cone_angle.to_radians() {
                            None
                        } else {
                            let hitbox = hitbox_registry
                                .get(hitbox_profile)
                                .or_else(|| hitbox_registry.resolve_default());
                            hitbox.and_then(|def| {
                                resolve_hitbox_hit(
                                    &def,
                                    *target_pos,
                                    attacker.origin,
                                    look_dir,
                                    attacker.weapon.range,
                                )
                            })
                        }
                    }
                }
            };

            let Some(hit) = maybe_hit else {
                continue;
            };
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

        if let Ok((_, _, _, _, _, mut health, mut armor, _, _, _, _, _, _)) =
            players.get_mut(target_entity)
        {
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
                    attacker.client_id,
                    target_cid,
                    hit.hitzone,
                    hit.armor_zone,
                    damage,
                    armor_absorbed,
                    health.current
                );
            }
        }
    }
}
