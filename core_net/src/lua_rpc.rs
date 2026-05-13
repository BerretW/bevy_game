//! Lua RPC bridge — propojuje sandbox outgoing/handlers s lightyear
//! LuaEventMessage kanalem.
//!
//! Phase 3.8 opravy:
//! * server_dispatch_incoming: extrahuje RemoteId => predava sender player_id
//!   do dispatch_incoming misto None.
//! * server_drain_outgoing: respektuje evt.target pro unicast routing.

use bevy::prelude::*;
use core_resources::{GameBridges, LocalPlayerStats, LuaEventDirection, SandboxRegistry};
use core_resources::StatsSnapshot;
use lightyear::prelude::*;

use crate::net_plugin::LuaRpcChannel;
use crate::protocol::{LuaEventMessage, PlayerStatsUpdate};

// ---------------------------------------------------------------------------
// Server side
// ---------------------------------------------------------------------------

pub struct ServerLuaRpcPlugin;

impl Plugin for ServerLuaRpcPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (server_drain_outgoing, server_dispatch_incoming).chain(),
        );
    }
}

fn server_drain_outgoing(
    registry: NonSend<SandboxRegistry>,
    mut senders: Query<(&mut MessageSender<LuaEventMessage>, &RemoteId)>,
) {
    for sandbox in registry.sandboxes.values() {
        let outgoing = sandbox.drain_outgoing();
        for evt in outgoing {
            if evt.direction != LuaEventDirection::ToClient {
                warn!(
                    "[lua_rpc/server] sandbox {} produced {:?} event '{}' — server can only emit ToClient",
                    sandbox.id, evt.direction, evt.name
                );
                continue;
            }

            let msg = LuaEventMessage {
                name: evt.name.clone(),
                target: evt.target,
                payload: evt.payload.clone(),
            };

            // Phase 3.8: per-target unicast vs broadcast
            for (mut sender, remote_id) in senders.iter_mut() {
                let client_id = match remote_id.0 {
                    PeerId::Netcode(id) => id,
                    _ => continue,
                };
                // None = broadcast, Some(id) = unicast
                if evt.target.map_or(true, |t| t == client_id) {
                    sender.send::<LuaRpcChannel>(msg.clone());
                }
            }
        }
    }
}

fn server_dispatch_incoming(
    registry: NonSend<SandboxRegistry>,
    mut receivers: Query<(&mut MessageReceiver<LuaEventMessage>, &RemoteId)>,
    bridges: Res<GameBridges>,
) {
    let mut messages: Vec<(LuaEventMessage, u64)> = Vec::new();

    for (mut rx, remote_id) in receivers.iter_mut() {
        // Phase 3.8: extrahujeme sender client_id z RemoteId
        let sender_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => 0,
        };
        // Block Lua events from players who haven't completed authentication yet.
        if bridges.auth.is_pending(sender_id) {
            // Drain (discard) messages from unauthenticated clients.
            rx.receive().for_each(|_| {});
            continue;
        }
        for msg in rx.receive() {
            messages.push((msg, sender_id));
        }
    }

    for (msg, sender_id) in messages {
        let mut delivered = 0usize;
        for sandbox in registry.sandboxes.values() {
            match sandbox.dispatch_incoming(&msg.name, &msg.payload, Some(sender_id)) {
                Ok(n) => delivered += n,
                Err(e) => warn!(
                    "[lua_rpc/server] handler error in sandbox {} for event '{}': {}",
                    sandbox.id, msg.name, e
                ),
            }
        }
        if delivered == 0 {
            debug!(
                "[lua_rpc/server] no handler for incoming '{}' from client {} (payload {} bytes)",
                msg.name, sender_id, msg.payload.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

pub struct ClientLuaRpcPlugin;

impl Plugin for ClientLuaRpcPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (client_drain_outgoing, client_dispatch_incoming, receive_player_stats).chain(),
        );
    }
}

fn client_drain_outgoing(
    registry: NonSend<SandboxRegistry>,
    mut senders: Query<&mut MessageSender<LuaEventMessage>>,
) {
    for sandbox in registry.sandboxes.values() {
        let outgoing = sandbox.drain_outgoing();
        for evt in outgoing {
            if evt.direction != LuaEventDirection::ToServer {
                warn!(
                    "[lua_rpc/client] sandbox {} produced {:?} event '{}' — client can only emit ToServer",
                    sandbox.id, evt.direction, evt.name
                );
                continue;
            }

            let msg = LuaEventMessage {
                name: evt.name,
                target: None,
                payload: evt.payload,
            };

            for mut sender in senders.iter_mut() {
                sender.send::<LuaRpcChannel>(msg.clone());
            }
        }
    }
}

fn receive_player_stats(
    mut receivers: Query<&mut MessageReceiver<PlayerStatsUpdate>>,
    local_stats: Res<LocalPlayerStats>,
) {
    for mut rx in receivers.iter_mut() {
        // SequencedUnreliable — vezmeme jen poslední zprávu v dávce
        let mut last: Option<PlayerStatsUpdate> = None;
        for msg in rx.receive() {
            last = Some(msg);
        }
        if let Some(update) = last {
            local_stats.update_snapshot(StatsSnapshot {
                health: update.hp,
                max_health: update.max_hp,
                armor: update.armor,
                weapon_slots: update.weapon_slots,
                ammo_reserve: update.ammo_reserve,
                active_weapon_slot: update.active_weapon_slot,
                fire_cooldown_remaining: update.fire_cooldown_remaining,
                fire_trigger_held: update.fire_trigger_held,
                reload_remaining: update.reload_remaining,
                reload_duration: update.reload_duration,
                weapon_swap_remaining: update.weapon_swap_remaining,
                weapon_swap_duration: update.weapon_swap_duration,
                weapon_swap_target_slot: update.weapon_swap_target_slot,
                ..StatsSnapshot::default()
            });
        }
    }
}

fn client_dispatch_incoming(
    registry: NonSend<SandboxRegistry>,
    mut receivers: Query<&mut MessageReceiver<LuaEventMessage>>,
) {
    let mut messages = Vec::new();
    for mut rx in receivers.iter_mut() {
        for msg in rx.receive() {
            messages.push(msg);
        }
    }

    for msg in messages {
        let mut delivered = 0usize;
        for sandbox in registry.sandboxes.values() {
            // target field nese puvodni client_id (server ho poslal jako metadata),
            // pro klienta je sender vzdy None (server nema player_id z pohledu klienta).
            match sandbox.dispatch_incoming(&msg.name, &msg.payload, None) {
                Ok(n) => delivered += n,
                Err(e) => warn!(
                    "[lua_rpc/client] handler error in sandbox {} for event '{}': {}",
                    sandbox.id, msg.name, e
                ),
            }
        }
        if delivered == 0 {
            debug!(
                "[lua_rpc/client] no handler for incoming '{}' (payload {} bytes)",
                msg.name,
                msg.payload.len()
            );
        }
    }
}
