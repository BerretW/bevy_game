//! Phase 3 — gameplay simulace (server-authoritativni pohyb + combat).
//!
//! Phase 3.3 pridava:
//! * `Health` component a `WeaponConfig` resource.
//! * Server-side combat systemy: proximity/angle hit check z PlayerInput.look
//!   a actions bitfield, cooldown management.
//! * Lua eventy: `playerConnecting`, `playerDropped`, `onPlayerHit`, `onPlayerDeath`.

use bevy::prelude::*;
use core_resources::{
    ActiveWeaponSlot, AmmoRegistry, AmmoReserve, FireState, GameBridges, Inventory, LocalEventBus,
    LuaWorldState, NpcLastClientUpdate, NpcOwner, NpcPathWaypoint, NpcPedMarker,
    PlayerEntityMap, PlayerStatsCache, ReloadState, Stats, StatsSnapshot,
    ReplicatedNpcSteering, WeaponRegistry, WeaponSlots, WeaponSwapState,
    apply_replicated_npc_steering,
};
use core_shared::{Health, NetTransform, NetVelocity, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::server::LinkOf;

use crate::net_plugin::StatsChannel;
use crate::protocol::{NpcTransformUpdate, player_action, PlayerInput, PlayerStatsUpdate};

pub const PLAYER_MOVE_SPEED: f32 = 5.0;
pub const PLAYER_SPRINT_MULTIPLIER: f32 = 1.35;
pub const PLAYER_CROUCH_MULTIPLIER: f32 = 0.45;
pub const PLAYER_JUMP_SPEED: f32 = 6.5;
pub const PLAYER_GRAVITY: f32 = 20.0;
pub const GROUND_Y: f32 = 0.0;
const DEFAULT_WEAPON_SWAP_SECS: f32 = 0.25;
const MIN_WEAPON_ACTION_SECS: f32 = 0.05;

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
                // FiveM-style: server důvěřuje klientské fyzikální pozici.
                // apply_inputs_to_velocity + integrate_velocity nahrazeny trust_client_position.
                trust_client_position,
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

// ---------------------------------------------------------------------------
// Observers
// ---------------------------------------------------------------------------

fn attach_replication_sender(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .insert(ReplicationSender::default());
    trace!(
        "[sim/server] ReplicationSender attached to link {:?}",
        trigger.entity
    );
}

fn spawn_player_on_connect(
    trigger: On<Add, Connected>,
    remote_ids: Query<&RemoteId>,
    mut commands: Commands,
    local_bus: Res<LocalEventBus>,
) {
    let entity = trigger.entity;
    let Ok(remote_id) = remote_ids.get(entity) else {
        warn!(
            "[sim/server] connected entity {:?} has no RemoteId — skipping player spawn",
            entity
        );
        return;
    };

    let client_id = match remote_id.0 {
        PeerId::Netcode(id) => id,
        _ => 0,
    };

    let player = commands
        .spawn((
            NetTransform::default(),
            NetVelocity::default(),
            PlayerMarker { client_id },
            Health::default(),
            Stats::default(),
            Inventory::default(),
            WeaponSlots::default(),
            AmmoReserve::default(),
            ActiveWeaponSlot::default(),
            FireState::default(),
            ReloadState::default(),
            WeaponSwapState::default(),
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
        ))
        .id();

    info!(
        "[sim/server] spawned player {:?} for client_id={}",
        player, client_id
    );

    // Phase 3.3 — FiveM-style Lua event: playerConnecting
    let payload = serde_json::to_vec(&serde_json::json!({
        "id": client_id.to_string(),
        "entity": format!("{:?}", player)
    }))
    .unwrap_or_default();
    local_bus.push("playerConnecting".to_string(), payload);
}

fn emit_player_disconnect(
    trigger: On<Remove, Connected>,
    remote_ids: Query<&RemoteId>,
    bridges: Res<GameBridges>,
    local_bus: Res<LocalEventBus>,
) {
    let entity = trigger.entity;
    let client_id = remote_ids
        .get(entity)
        .ok()
        .and_then(|r| match r.0 { PeerId::Netcode(id) => Some(id), _ => None })
        .unwrap_or(0);

    info!("[sim/server] client {} disconnected", client_id);

    // FiveM-style ACE cleanup: remove player principals/identifiers on leave.
    bridges.ace.remove_player(client_id);

    let payload = serde_json::to_vec(&serde_json::json!({
        "id": client_id.to_string(),
        "reason": "disconnect"
    }))
    .unwrap_or_default();
    local_bus.push("playerDropped".to_string(), payload);
}

/// Phase 3.5 — kdyz se spawne NetworkedObjectMarker entita,
/// pridame Replicate aby ji lightyear zacal replikovat klientum.
fn attach_replication_to_networked_object(
    trigger: On<Add, core_resources::NetworkedObjectMarker>,
    mut commands: Commands,
) {
    commands
        .entity(trigger.entity)
        .insert(Replicate::to_clients(NetworkTarget::All));
    debug!(
        "[sim/server] Replicate attached to networked object {:?}",
        trigger.entity
    );
}

// ---------------------------------------------------------------------------
// Pohyb
// ---------------------------------------------------------------------------

/// FiveM-style: server přijme klientem poslanou fyzikální pozici a přímo ji
/// zapíše do NetTransform. Žádná server-side simulace pohybu ani gravitace.
/// Sanitizace: NaN/Inf hodnoty se ignorují (klient zůstane na posledním místě).
fn trust_client_position(
    last_inputs: Res<LastPlayerInputs>,
    mut players: Query<(&PlayerMarker, &mut NetTransform)>,
) {
    for (marker, mut transform) in players.iter_mut() {
        let Some(input) = last_inputs.get(marker.client_id) else { continue };

        let [px, py, pz] = input.position;
        if px.is_finite() && py.is_finite() && pz.is_finite() {
            transform.translation = Vec3::new(px, py, pz);
        }

        // Yaw rotace — stále přenášíme z look[0]
        let yaw_rad = input.look[0].to_radians();
        if yaw_rad.is_finite() {
            transform.rotation = Quat::from_rotation_y(yaw_rad);
        }
    }
}

fn tick_fire_states(mut q: Query<&mut FireState>, time: Res<Time<Fixed>>) {
    let dt = time.delta_secs();
    for mut fire_state in q.iter_mut() {
        fire_state.cooldown_remaining = (fire_state.cooldown_remaining - dt).max(0.0);
    }
}

fn tick_weapon_swap_states(
    mut players: Query<(&mut ActiveWeaponSlot, &mut WeaponSwapState)>,
    time: Res<Time<Fixed>>,
) {
    let dt = time.delta_secs();
    for (mut active_slot, mut swap_state) in players.iter_mut() {
        if swap_state.remaining <= 0.0 {
            continue;
        }

        swap_state.remaining = (swap_state.remaining - dt).max(0.0);
        if swap_state.remaining > 0.0 {
            continue;
        }

        if swap_state.target_slot < 4 {
            active_slot.0 = swap_state.target_slot;
        }
        *swap_state = WeaponSwapState::default();
    }
}

fn tick_reload_states(
    weapon_registry: Res<WeaponRegistry>,
    mut players: Query<(&mut WeaponSlots, &mut AmmoReserve, &mut ReloadState)>,
    time: Res<Time<Fixed>>,
) {
    let dt = time.delta_secs();
    for (mut slots, mut reserve, mut reload_state) in players.iter_mut() {
        if reload_state.remaining <= 0.0 {
            continue;
        }

        reload_state.remaining = (reload_state.remaining - dt).max(0.0);
        if reload_state.remaining > 0.0 {
            continue;
        }

        reload_active_weapon(&weapon_registry, &mut slots, &mut reserve, reload_state.slot);
        *reload_state = ReloadState::default();
    }
}

fn reload_active_weapon(
    weapon_registry: &WeaponRegistry,
    slots: &mut WeaponSlots,
    reserve: &mut AmmoReserve,
    active_slot: u8,
) -> u32 {
    let Some(equipped) = slots.get_mut(active_slot) else {
        return 0;
    };
    let Some(weapon_def) = weapon_registry.get(&equipped.weapon_id) else {
        return 0;
    };
    let mag_capacity = weapon_def.mag_capacity;
    if mag_capacity == 0 || equipped.ammo_in_mag >= mag_capacity {
        return 0;
    }

    let ammo_id = if equipped.ammo_type_id.trim().is_empty() {
        weapon_def.default_ammo.clone()
    } else {
        equipped.ammo_type_id.clone()
    };
    if ammo_id.trim().is_empty() {
        return 0;
    }

    let reserve_entry = reserve.0.entry(ammo_id.clone()).or_insert(0);
    let needed = mag_capacity.saturating_sub(equipped.ammo_in_mag);
    let loaded = (*reserve_entry).min(needed);
    *reserve_entry = reserve_entry.saturating_sub(loaded);
    equipped.ammo_in_mag = equipped.ammo_in_mag.saturating_add(loaded);
    if equipped.ammo_type_id.trim().is_empty() {
        equipped.ammo_type_id = ammo_id;
    }
    loaded
}

fn requested_weapon_slot(actions: u32) -> Option<u8> {
    if actions & player_action::WEAPON_SLOT_1 != 0 {
        Some(0)
    } else if actions & player_action::WEAPON_SLOT_2 != 0 {
        Some(1)
    } else if actions & player_action::WEAPON_SLOT_3 != 0 {
        Some(2)
    } else if actions & player_action::WEAPON_SLOT_4 != 0 {
        Some(3)
    } else {
        None
    }
}

fn weapon_swap_duration(
    weapon_registry: &WeaponRegistry,
    slots: &WeaponSlots,
    active_slot: u8,
    target_slot: u8,
) -> f32 {
    let duration = slots
        .get(target_slot)
        .and_then(|weapon| weapon_registry.get(&weapon.weapon_id))
        .map(|weapon| weapon.ads_time_sec)
        .filter(|duration| *duration > 0.0)
        .or_else(|| {
            slots.get(active_slot)
                .and_then(|weapon| weapon_registry.get(&weapon.weapon_id))
                .map(|weapon| weapon.ads_time_sec)
                .filter(|duration| *duration > 0.0)
        })
        .unwrap_or(DEFAULT_WEAPON_SWAP_SECS);
    duration.max(MIN_WEAPON_ACTION_SECS)
}

fn reload_duration_for_slot(
    weapon_registry: &WeaponRegistry,
    slots: &WeaponSlots,
    reserve: &AmmoReserve,
    active_slot: u8,
) -> Option<f32> {
    let equipped = slots.get(active_slot)?;
    let weapon_def = weapon_registry.get(&equipped.weapon_id)?;
    let mag_capacity = weapon_def.mag_capacity;
    if mag_capacity == 0 || equipped.ammo_in_mag >= mag_capacity {
        return None;
    }

    let ammo_id = if equipped.ammo_type_id.trim().is_empty() {
        weapon_def.default_ammo.clone()
    } else {
        equipped.ammo_type_id.clone()
    };
    if ammo_id.trim().is_empty() {
        return None;
    }

    if reserve.0.get(&ammo_id).copied().unwrap_or(0) == 0 {
        return None;
    }

    let duration = if equipped.ammo_in_mag == 0 {
        weapon_def.reload_empty_sec
    } else {
        weapon_def.reload_tactical_sec
    };

    Some(duration.max(MIN_WEAPON_ACTION_SECS))
}

fn is_reload_active(reload_state: &ReloadState) -> bool {
    reload_state.remaining > 0.0
}

fn is_weapon_swap_active(weapon_swap: &WeaponSwapState) -> bool {
    weapon_swap.remaining > 0.0
}

struct EffectiveCombatWeaponConfig {
    weapon_id: String,
    fire_rate: f32,
    damage: f32,
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
            fire_rate: fallback.fire_rate,
            damage: fallback.damage,
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

    let damage = if let Some(ammo_id) = ammo_id {
        ammo_registry
            .get(&ammo_id)
            .map(|ammo| ammo.base_damage)
            .filter(|value| *value > 0.0)
            .unwrap_or(fallback.damage)
    } else {
        fallback.damage
    };

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
        fire_rate,
        damage,
        range,
        cone_angle: fallback.cone_angle,
        weapon_type,
    }
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
    weapon_cfg: Res<WeaponConfig>,
    mut players: Query<(
        Entity,
        &PlayerMarker,
        &NetTransform,
        &mut Health,
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
        pos: Vec3,
        look_yaw: f32,
        weapon: EffectiveCombatWeaponConfig,
    }

    let mut attackers: Vec<Attacker> = Vec::new();

    // Iterujeme hrace a zjistime, kteri chtejí strilet (na zaklade LastPlayerInputs)
    {
        for (entity, marker, transform, _health, mut slots, reserve, active_slot, mut fire_state, mut reload_state, mut weapon_swap) in players.iter_mut() {
            let Some(input) = last_inputs.get(marker.client_id) else { continue };
            fire_state.trigger_held = input.actions & player_action::PRIMARY_FIRE != 0;

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

            if input.actions & player_action::PRIMARY_FIRE == 0 {
                continue;
            }

            let equipped_snapshot = slots.get(active_slot_value).cloned();
            let effective_weapon = resolve_combat_weapon_config(
                &weapon_registry,
                &ammo_registry,
                &weapon_cfg,
                equipped_snapshot.as_ref(),
            );

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
                pos: transform.translation,
                look_yaw: input.look[0],
                weapon: effective_weapon,
            });
        }
    }

    if attackers.is_empty() {
        return;
    }

    // Snap shot pozic vsech hracu
    let targets: Vec<(Entity, u64, Vec3, f32)> = players
        .iter()
        .map(|(e, m, t, h, _, _, _, _, _, _)| (e, m.client_id, t.translation, h.current))
        .collect();

    for attacker in &attackers {
        let range_sq = attacker.weapon.range * attacker.weapon.range;
        let half_cone = attacker.weapon.cone_angle.to_radians();
        let damage = attacker.weapon.damage;

        for &(target_entity, target_cid, target_pos, _target_hp) in &targets {
            if target_entity == attacker.entity {
                continue; // nelze trefeni sám sebe
            }

            let diff = target_pos - attacker.pos;
            let dist_sq = diff.x * diff.x + diff.z * diff.z;
            if dist_sq > range_sq {
                continue;
            }

            // Uhel mezi look direction a smerem k cili
            let look_dir = Vec3::new(
                attacker.look_yaw.to_radians().sin(),
                0.0,
                attacker.look_yaw.to_radians().cos(),
            );
            let to_target = if dist_sq > 0.0001 {
                Vec3::new(diff.x, 0.0, diff.z).normalize()
            } else {
                look_dir
            };

            if matches!(attacker.weapon.weapon_type, WeaponType::Ranged) {
                let dot = look_dir.dot(to_target).clamp(-1.0, 1.0);
                let angle = dot.acos();
                if angle > half_cone {
                    continue;
                }
            }

            // Zasah!
            if let Ok((_, _, _, mut health, _, _, _, _, _, _)) = players.get_mut(target_entity) {
                health.current -= damage;
                let died = health.current <= 0.0;
                if died {
                    health.current = 0.0;
                }

                let hit_pos = target_pos;
                let payload = serde_json::to_vec(&serde_json::json!({
                    "attacker": attacker.client_id.to_string(),
                    "victim": target_cid.to_string(),
                    "damage": damage,
                    "weapon": attacker.weapon.weapon_id,
                    "position": { "x": hit_pos.x, "y": hit_pos.y, "z": hit_pos.z }
                }))
                .unwrap_or_default();
                local_bus.push("onPlayerHit".to_string(), payload);

                if died {
                    let death_payload = serde_json::to_vec(&serde_json::json!({
                        "victim": target_cid.to_string(),
                        "killer": attacker.client_id.to_string(),
                        "cause": "weapon"
                    }))
                    .unwrap_or_default();
                    local_bus.push("onPlayerDeath".to_string(), death_payload);

                    info!(
                        "[sim/combat] player {} killed by {}",
                        target_cid, attacker.client_id
                    );
                } else {
                    debug!(
                        "[sim/combat] player {} hit {} for {:.1} dmg (hp={:.1})",
                        attacker.client_id, target_cid, damage, health.current
                    );
                }

                // Jeden hrac = jeden zasah za vystrel
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// emit_player_positions — posila pozice vsech hracu do Lua event busu
// ---------------------------------------------------------------------------

/// Kazdy FixedUpdate tick posle `onPlayerPosition` do LocalEventBus.
/// Server Lua resource to muze dale poslat klientum pres TriggerClientEvent.
fn emit_player_positions(
    players: Query<(&NetTransform, &PlayerMarker)>,
    local_bus: Res<LocalEventBus>,
) {
    let list: Vec<serde_json::Value> = players
        .iter()
        .map(|(t, m)| {
            serde_json::json!({
                "id": m.client_id.to_string(),
                "x":  t.translation.x,
                "z":  t.translation.z,
            })
        })
        .collect();

    if list.is_empty() {
        return;
    }

    let payload = serde_json::to_vec(&serde_json::json!({ "players": list }))
        .unwrap_or_default();
    local_bus.push("onPlayerPosition".to_string(), payload);
}

// ---------------------------------------------------------------------------
// LastPlayerInputs — pomocny Resource pro sdileni inputu mezi systemy
// ---------------------------------------------------------------------------

/// Posledni znamy PlayerInput pro kazdeho klienta. Naplnuje se v
/// collect_last_inputs (Update), cte se v process_combat (FixedUpdate
/// behem stejneho framu).
#[derive(Resource, Default)]
pub struct LastPlayerInputs(std::collections::HashMap<u64, PlayerInput>);

impl LastPlayerInputs {
    pub fn update(&mut self, client_id: u64, input: PlayerInput) {
        self.0.insert(client_id, input);
    }
    pub fn get(&self, client_id: u64) -> Option<&PlayerInput> {
        self.0.get(&client_id)
    }
    pub fn remove(&mut self, client_id: u64) {
        self.0.remove(&client_id);
    }
}

/// System, ktery naplnuje LastPlayerInputs z MessageReceiver<PlayerInput>.
/// Musi bezet v Update PRED apply_inputs_to_velocity ve FixedUpdate.
pub fn collect_last_inputs(
    mut receivers: Query<(&mut MessageReceiver<PlayerInput>, &RemoteId)>,
    mut last: ResMut<LastPlayerInputs>,
) {
    for (mut rx, remote_id) in receivers.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => continue,
        };
        for input in rx.receive() {
            last.update(client_id, input);
        }
    }
}

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
        if let Some((_, health, weapon_slots, ammo_reserve, active_slot, fire_state, reload_state, weapon_swap)) =
            players.iter().find(|(m, _, _, _, _, _, _, _)| m.client_id == client_id)
        {
            let msg = PlayerStatsUpdate {
                hp: health.current,
                max_hp: health.max,
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

    for (entity, marker, health, stats, inventory, weapon_slots, ammo_reserve, active_slot, fire_state, reload_state, weapon_swap) in &players {
        seen.insert(marker.client_id);
        player_map.map.insert(marker.client_id, entity);
        stats_cache.update(
            marker.client_id,
            StatsSnapshot {
                stats: stats.map(|value| value.0.clone()).unwrap_or_default(),
                inventory: inventory.map(|value| value.0.clone()).unwrap_or_default(),
                health: health.current,
                max_health: health.max,
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
