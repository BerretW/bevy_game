//! Phase 3 — gameplay simulace (server-authoritativni pohyb + combat).
//!
//! Phase 3.3 pridava:
//! * `Health` component a `WeaponConfig` resource.
//! * Server-side combat systemy: proximity/angle hit check z PlayerInput.look
//!   a actions bitfield, cooldown management.
//! * Lua eventy: `playerConnecting`, `playerDropped`, `onPlayerHit`, `onPlayerDeath`.

use bevy::prelude::*;
use core_resources::{GameBridges, LocalEventBus};
use core_shared::{Health, NetTransform, NetVelocity, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::server::LinkOf;

use crate::net_plugin::StatsChannel;
use crate::protocol::{player_action, PlayerInput, PlayerStatsUpdate};

pub const PLAYER_MOVE_SPEED: f32 = 5.0;
pub const PLAYER_SPRINT_MULTIPLIER: f32 = 1.35;
pub const PLAYER_CROUCH_MULTIPLIER: f32 = 0.45;
pub const PLAYER_JUMP_SPEED: f32 = 6.5;
pub const PLAYER_GRAVITY: f32 = 20.0;
pub const GROUND_Y: f32 = 0.0;

// ---------------------------------------------------------------------------
// Komponenty
// ---------------------------------------------------------------------------

/// Cooldown zbrane per hrac.
#[derive(Component, Debug, Default)]
pub struct WeaponCooldown {
    /// Sekundy do dalsho mozneho vystrely. 0 = ready.
    pub remaining: f32,
}

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
        app.add_systems(
            FixedUpdate,
            (
                // FiveM-style: server důvěřuje klientské fyzikální pozici.
                // apply_inputs_to_velocity + integrate_velocity nahrazeny trust_client_position.
                trust_client_position,
                emit_player_positions,
                tick_weapon_cooldowns,
                process_combat,
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
            WeaponCooldown::default(),
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

fn tick_weapon_cooldowns(mut q: Query<&mut WeaponCooldown>, time: Res<Time<Fixed>>) {
    let dt = time.delta_secs();
    for mut cd in q.iter_mut() {
        cd.remaining = (cd.remaining - dt).max(0.0);
    }
}

// ---------------------------------------------------------------------------
// Combat (Phase 3.3)
// ---------------------------------------------------------------------------

/// Server-side combat systemy:
/// 1. Nacte nejnovejsi input kazdeho klienta.
/// 2. Zkontroluje PRIMARY_FIRE bitflag a WeaponCooldown.
/// 3. Proximity + angle check vuci ostatnim hracum.
/// 4. Aplikuje damage, emituje Lua eventy onPlayerHit / onPlayerDeath.
fn process_combat(
    // Cte vsechny prisly inputy — POZOR: inputs uz byly drainovany v
    // apply_inputs_to_velocity. Musime mit vlastni kopii nebo drat last input.
    // Reseni: pouzijeme LastInput resource naplnenou v apply_inputs_to_velocity.
    last_inputs: Res<LastPlayerInputs>,
    weapon_cfg: Res<WeaponConfig>,
    mut players: Query<(
        Entity,
        &PlayerMarker,
        &NetTransform,
        &mut Health,
        &mut WeaponCooldown,
    )>,
    local_bus: Res<LocalEventBus>,
) {
    let fire_interval = if weapon_cfg.fire_rate > 0.0 {
        1.0 / weapon_cfg.fire_rate
    } else {
        f32::MAX
    };

    // Sbir potencialnich strelcu
    struct Attacker {
        entity: Entity,
        client_id: u64,
        pos: Vec3,
        look_yaw: f32,
    }

    let mut attackers: Vec<Attacker> = Vec::new();

    // Iterujeme hrace a zjistime, kteri chtejí strilet (na zaklade LastPlayerInputs)
    {
        for (entity, marker, transform, _health, mut cooldown) in players.iter_mut() {
            let Some(input) = last_inputs.get(marker.client_id) else { continue };
            if input.actions & player_action::PRIMARY_FIRE == 0 {
                continue;
            }
            if cooldown.remaining > 0.0 {
                continue;
            }
            cooldown.remaining = fire_interval;
            attackers.push(Attacker {
                entity,
                client_id: marker.client_id,
                pos: transform.translation,
                look_yaw: input.look[0],
            });
        }
    }

    if attackers.is_empty() {
        return;
    }

    // Pro kazdého strelce najdeme zasazeného hrace
    let range_sq = weapon_cfg.range * weapon_cfg.range;
    let half_cone = weapon_cfg.cone_angle.to_radians();

    // Snap shot pozic vsech hracu
    let targets: Vec<(Entity, u64, Vec3, f32)> = players
        .iter()
        .map(|(e, m, t, h, _)| (e, m.client_id, t.translation, h.current))
        .collect();

    let damage = weapon_cfg.damage;

    for attacker in &attackers {
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

            let dot = look_dir.dot(to_target).clamp(-1.0, 1.0);
            let angle = dot.acos();
            if angle > half_cone {
                continue;
            }

            // Zasah!
            if let Ok((_, _, _, mut health, _)) = players.get_mut(target_entity) {
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
                    "weapon": "default",
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

// ---------------------------------------------------------------------------
// broadcast_player_stats — každých 6 FixedUpdate ticků (~10 Hz)
// ---------------------------------------------------------------------------

fn broadcast_player_stats(
    players: Query<(&PlayerMarker, &Health)>,
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
        if let Some((_, health)) = players.iter().find(|(m, _)| m.client_id == client_id) {
            let msg = PlayerStatsUpdate {
                hp: health.current,
                max_hp: health.max,
            };
            sender.send::<StatsChannel>(msg);
        }
    }
}
