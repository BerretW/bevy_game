//! Lua RPC bridge — propojuje sandbox-level outgoing/handlers s lightyear
//! `LuaEventMessage` kanálem.
//!
//! Dva pluginy se shodným tvarem (server/klient se liší jen směrem, který
//! je legitimní):
//!
//! * [`ServerLuaRpcPlugin`]: drainuje outgoing `ToClient` eventy ze server-side
//!   sandboxů a posílá je klientům přes lightyear; přijaté `LuaEventMessage`
//!   doručuje do server-side sandboxů.
//! * [`ClientLuaRpcPlugin`]: zrcadlově — outgoing `ToServer`, incoming
//!   broadcast / unicast od serveru.
//!
//! Phase 2 implementuje **broadcast** doručení (server posílá všem
//! klientům, target field zatím není respektován). Per-target unicast
//! přidáme v Phase 3, jakmile budou existovat player_id ↔ entity mapy.

use bevy::prelude::*;
use core_resources::{LuaEventDirection, SandboxRegistry};
use lightyear::prelude::*;

use crate::net_plugin::LuaRpcChannel;
use crate::protocol::LuaEventMessage;

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
    mut senders: Query<&mut MessageSender<LuaEventMessage>>,
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
                name: evt.name,
                target: evt.target,
                payload: evt.payload,
            };

            // Broadcast: pošleme všem MessageSender komponentám (= všem
            // aktivním klientským spojením). target=Some(player_id) zatím
            // ignorujeme, Phase 3 doplní player_id ↔ entity routing.
            for mut sender in senders.iter_mut() {
                sender.send::<LuaRpcChannel>(msg.clone());
            }
        }
    }
}

fn server_dispatch_incoming(
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
            match sandbox.dispatch_incoming(&msg.name, &msg.payload, msg.target) {
                Ok(n) => delivered += n,
                Err(e) => warn!(
                    "[lua_rpc/server] handler error in sandbox {} for event '{}': {}",
                    sandbox.id, msg.name, e
                ),
            }
        }
        if delivered == 0 {
            debug!(
                "[lua_rpc/server] no handler for incoming '{}' (payload {} bytes)",
                msg.name,
                msg.payload.len()
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
            (client_drain_outgoing, client_dispatch_incoming).chain(),
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
                target: None, // Server určí podle transport (kdo zprávu poslal).
                payload: evt.payload,
            };

            for mut sender in senders.iter_mut() {
                sender.send::<LuaRpcChannel>(msg.clone());
            }
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
            match sandbox.dispatch_incoming(&msg.name, &msg.payload, msg.target) {
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
