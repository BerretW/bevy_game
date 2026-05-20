//! Phase 3 — gameplay simulace (server-authoritativni pohyb + combat).
//!
//! Phase 3.3 pridava:
//! * `Health` component a `WeaponConfig` resource.
//! * Server-side combat systemy: proximity/angle hit check z PlayerInput.look
//!   a actions bitfield, cooldown management.
//! * Lua eventy: `playerConnecting`, `playerDropped`, `onPlayerHit`, `onPlayerDeath`.

use bevy::prelude::*;

mod combat;
mod lifecycle;
mod npc;
mod players;
mod stats;
mod weapons;

use combat::process_combat;
use lifecycle::{
    attach_replication_sender, attach_replication_to_networked_object, emit_player_disconnect,
    spawn_player_on_connect,
};
use npc::receive_npc_transform_updates;
pub use players::{LastPlayerInputs, collect_last_inputs};
use players::{
    ServerSimulationTick, emit_player_positions, increment_server_simulation_tick,
    record_position_history, trust_client_position,
};
use stats::{broadcast_player_stats, sync_player_state_cache};
use weapons::{tick_fire_states, tick_reload_states, tick_weapon_swap_states};

pub const PLAYER_MOVE_SPEED: f32 = 5.0;
pub const PLAYER_SPRINT_MULTIPLIER: f32 = 1.35;
pub const PLAYER_CROUCH_MULTIPLIER: f32 = 0.45;
pub const PLAYER_JUMP_SPEED: f32 = 6.5;
pub const PLAYER_GRAVITY: f32 = 20.0;
pub const GROUND_Y: f32 = 0.0;
const DEFAULT_WEAPON_SWAP_SECS: f32 = 0.25;
const MIN_WEAPON_ACTION_SECS: f32 = 0.05;

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
    pub fire_rate: f32,
    pub damage: f32,
    pub range: f32,
    pub cone_angle: f32,
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
        app.add_systems(Update, collect_last_inputs);
        app.add_systems(Update, receive_npc_transform_updates);
        app.add_systems(
            FixedUpdate,
            (
                increment_server_simulation_tick,
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
