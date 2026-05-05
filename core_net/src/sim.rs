//! Phase 3 — gameplay simulace (server-authoritativní pohyb).
//!
//! `ServerSimPlugin` zařídí:
//! 1. **Spawn hráčské entity** při `Add<Connected>` (pošle se klientovi přes
//!    `Replicate::to_clients(NetworkTarget::All)` — hráče vidí všichni
//!    včetně sebe).
//! 2. **Aplikaci klientského inputu** na `NetVelocity` autoritativní entity
//!    (pairing přes `client_id` z `RemoteId` a `PlayerMarker`).
//! 3. **Integraci pohybu** v `FixedUpdate` (deterministická simulace, kterou
//!    Phase 3 step 8 doplní o client-side prediction + rollback).
//!
//! Klientská strana (rendering replikovaných entit + sběr WASD) je v
//! `host_client` — tady jen herní logika, žádné windowing.

use bevy::prelude::*;
use core_shared::{NetTransform, NetVelocity, PlayerMarker};
use lightyear::prelude::*;

use crate::protocol::PlayerInput;

/// Maximální rychlost hráče (jednotky / sekunda) pro WASD vstup.
/// Phase 3+ bude per-class konfigurovatelné z Lua resource.
pub const PLAYER_MOVE_SPEED: f32 = 5.0;

pub struct ServerSimPlugin;

impl Plugin for ServerSimPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(spawn_player_on_connect);
        app.add_systems(
            FixedUpdate,
            (apply_inputs_to_velocity, integrate_velocity).chain(),
        );
    }
}

fn spawn_player_on_connect(
    trigger: On<Add, Connected>,
    remote_ids: Query<&RemoteId>,
    mut commands: Commands,
) {
    let entity = trigger.entity;
    let Ok(remote_id) = remote_ids.get(entity) else {
        warn!(
            "[sim/server] connected entity {:?} has no RemoteId — skipping player spawn",
            entity
        );
        return;
    };

    // Netcode klient_id je primární klíč; ostatní variants (Steam, Local, …)
    // dostanou 0 a budou se chovat jako jeden anonymous slot. Phase 4 dohledá.
    let client_id = match remote_id.0 {
        PeerId::Netcode(id) => id,
        _ => 0,
    };

    let player = commands
        .spawn((
            NetTransform::default(),
            NetVelocity::default(),
            PlayerMarker { client_id },
            // Replikujeme všem připojeným klientům — i sebe-sama vidí, aby
            // si mohl renderovat svého hráče (Phase 3 step 8 přepneme na
            // owner-only prediction + remote interpolation).
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();

    info!(
        "[sim/server] spawned player entity {:?} for client_id={}",
        player, client_id
    );
}

fn apply_inputs_to_velocity(
    mut receivers: Query<(&mut MessageReceiver<PlayerInput>, &RemoteId)>,
    mut players: Query<(&PlayerMarker, &mut NetVelocity)>,
) {
    for (mut rx, remote_id) in receivers.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => continue,
        };

        // Drainujeme všechny pending inputs; "latest wins" — bereme ten
        // poslední tick. Sequenced unreliable kanál zaručuje, že stale
        // pakety už dostatne dropuje transport.
        let mut latest: Option<PlayerInput> = None;
        for input in rx.receive() {
            latest = Some(input);
        }

        let Some(input) = latest else {
            continue;
        };

        for (marker, mut vel) in players.iter_mut() {
            if marker.client_id == client_id {
                // 2D ground-plane mapping: WASD x ↦ X, WASD y ↦ Z.
                vel.0.x = input.move_dir[0] * PLAYER_MOVE_SPEED;
                vel.0.z = input.move_dir[1] * PLAYER_MOVE_SPEED;
                break;
            }
        }
    }
}

fn integrate_velocity(
    mut q: Query<(&mut NetTransform, &NetVelocity)>,
    time: Res<Time<Fixed>>,
) {
    let dt = time.delta_secs();
    for (mut t, v) in q.iter_mut() {
        t.translation += v.0 * dt;
    }
}
