//! Phase 4 — username/password authentication.
//!
//! Sekvence (auth_required = true):
//!
//! ```text
//!   Client                            Server
//!     | <-- ServerHello (auth_req) --  |
//!     | [download + ClientReady] -->   |
//!     |                               [ClientHandshakeComplete]
//!     | <-- AuthChallenge ----------   |  ServerAuthPlugin observer
//!     |                                |
//!     | [player types /login in console]
//!     | -- AuthCredentials ----------> |  server_receive_credentials
//!     |                               [emit auth:credentials to LocalEventBus]
//!     |                               [Lua core/auth queries DB]
//!     |                               [Auth.MarkPlayerAuthenticated(...)]
//!     | <-- AuthResult (success) ----- |  server_send_auth_results
//!     | [auth:result event to Lua]     |
//! ```
//!
//! Hesla jsou posílána v plaintextu přes šifrovaný lightyear kanál.
//! Server hashuje hesla blake3 před uložením / porovnáváním.

use bevy::prelude::*;
use lightyear::prelude::*;

use core_resources::{GameBridges, LocalEventBus};

use crate::net_plugin::AuthChannel;
use crate::protocol::{AuthChallenge, AuthCredentials, AuthResult};

// ---------------------------------------------------------------------------
// ServerAuthPlugin
// ---------------------------------------------------------------------------

pub struct ServerAuthPlugin;

impl Plugin for ServerAuthPlugin {
    fn build(&self, app: &mut App) {
        // Auth challenge goes out immediately on connect — before resource download —
        // so the client can authenticate before it ever requests any Lua files.
        app.add_observer(send_challenge_on_connect);
        app.add_systems(
            Update,
            (server_receive_credentials, server_send_auth_results).chain(),
        );
    }
}

/// Observer on `Add<Connected>`: if auth is required, add player to the PendingAuth
/// set and send AuthChallenge immediately (before resource download).
fn send_challenge_on_connect(
    trigger: On<Add, Connected>,
    mut remote_ids: Query<(&RemoteId, &mut MessageSender<AuthChallenge>)>,
    bridges: Res<GameBridges>,
) {
    if !bridges.auth.is_required() {
        return;
    }

    let entity = trigger.entity;
    let Ok((remote_id, mut sender)) = remote_ids.get_mut(entity) else {
        warn!(
            "[auth/server] entity {:?} has no MessageSender<AuthChallenge>",
            entity
        );
        return;
    };

    let client_id = match remote_id.0 {
        PeerId::Netcode(id) => id,
        _ => 0,
    };

    bridges.auth.add_pending(client_id);
    sender.send::<AuthChannel>(AuthChallenge);
    info!(
        "[auth/server] AuthChallenge sent to client {} — awaiting credentials",
        client_id
    );
}

/// Polls `MessageReceiver<AuthCredentials>` from each peer entity.
/// Hashes the password (blake3) and emits `auth:credentials` to LocalEventBus
/// for the core/auth Lua resource to handle the DB query.
fn server_receive_credentials(
    mut receivers: Query<(&mut MessageReceiver<AuthCredentials>, &RemoteId)>,
    local_bus: Res<LocalEventBus>,
    bridges: Res<GameBridges>,
) {
    for (mut rx, remote_id) in receivers.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => 0,
        };

        for cred in rx.receive() {
            // Only process credentials from players actually pending auth.
            if !bridges.auth.is_pending(client_id) {
                debug!("[auth/server] ignoring credentials from already-authed client {}", client_id);
                continue;
            }

            let action = if cred.action == 1 { "register" } else { "login" };
            // Hash password with blake3 before passing to Lua — plaintext never touches Lua.
            let password_hash = format!("{}", blake3::hash(cred.password.as_bytes()).to_hex());
            // Generate a suggested account_id for new registrations (blake3 of name+time).
            let suggested = generate_account_id(&cred.username);

            let payload = serde_json::json!({
                "player_id": client_id.to_string(),
                "action": action,
                "username": cred.username,
                "password_hash": password_hash,
                "suggested_account_id": suggested,
            });
            let bytes = serde_json::to_vec(&payload).unwrap_or_default();
            local_bus.push("auth:credentials".to_string(), bytes);

            info!(
                "[auth/server] received {} attempt from client {} (user='{}')",
                action, client_id, cred.username
            );
        }
    }
}

/// Drains PendingAuthResult queue (filled by Lua via `Auth.MarkPlayerAuthenticated` /
/// `Auth.RejectPlayer`) and sends `AuthResult` back to the appropriate peer entity.
fn server_send_auth_results(
    mut senders: Query<(&mut MessageSender<AuthResult>, &RemoteId)>,
    bridges: Res<GameBridges>,
) {
    let results = bridges.auth.drain_results();
    if results.is_empty() {
        return;
    }

    for result in results {
        let mut sent = false;
        for (mut sender, remote_id) in senders.iter_mut() {
            let client_id = match remote_id.0 {
                PeerId::Netcode(id) => id,
                _ => continue,
            };
            if client_id == result.player_id {
                if result.success {
                    info!(
                        "[auth/server] player {} authenticated as '{}'",
                        client_id, result.account_id
                    );
                } else {
                    info!(
                        "[auth/server] auth failed for player {}: {}",
                        client_id, result.error
                    );
                }
                sender.send::<AuthChannel>(AuthResult {
                    success: result.success,
                    account_id: result.account_id.clone(),
                    error: result.error.clone(),
                });
                sent = true;
                break;
            }
        }
        if !sent {
            warn!(
                "[auth/server] could not find sender for auth result to player {}",
                result.player_id
            );
        }
    }
}

/// Generates a suggested account_id for new user registrations.
/// Uses blake3(username_lower + timestamp_nanos) → 64-char hex, prefixed with "user:".
fn generate_account_id(username: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let data = format!("{}:{}", username.to_lowercase(), nanos);
    format!("{}", blake3::hash(data.as_bytes()).to_hex())
}

// ---------------------------------------------------------------------------
// ClientAuthPlugin
// ---------------------------------------------------------------------------

pub struct ClientAuthPlugin;

impl Plugin for ClientAuthPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, client_process_auth);
    }
}

/// Polls `MessageReceiver<AuthChallenge>` and `MessageReceiver<AuthResult>` each frame.
/// - AuthChallenge → emits `auth:challenge {}` to LocalEventBus
/// - AuthResult    → emits `auth:result {success, account_id, error}` to LocalEventBus
/// Also drains `auth_bridge.outgoing` and sends `AuthCredentials` via lightyear.
fn client_process_auth(
    mut challenge_rx: Query<&mut MessageReceiver<AuthChallenge>>,
    mut result_rx: Query<&mut MessageReceiver<AuthResult>>,
    mut cred_senders: Query<&mut MessageSender<AuthCredentials>>,
    bridges: Res<GameBridges>,
    local_bus: Res<LocalEventBus>,
) {
    // Receive AuthChallenge → emit local event
    for mut rx in challenge_rx.iter_mut() {
        for _challenge in rx.receive() {
            info!("[auth/client] AuthChallenge received — auth required");
            let bytes = serde_json::to_vec(&serde_json::json!({})).unwrap_or_default();
            local_bus.push("auth:challenge".to_string(), bytes);
        }
    }

    // Receive AuthResult → signal bridge + emit local event for Lua
    for mut rx in result_rx.iter_mut() {
        for result in rx.receive() {
            if result.success {
                info!("[auth/client] authenticated as '{}'", result.account_id);
                bridges.auth.set_client_authenticated();
            } else {
                warn!("[auth/client] auth failed: {}", result.error);
                bridges.auth.set_client_error(result.error.clone());
            }

            // Also emit to LocalEventBus so Lua resources know the outcome
            let payload = serde_json::json!({
                "success": result.success,
                "account_id": result.account_id,
                "error": result.error,
            });
            let bytes = serde_json::to_vec(&payload).unwrap_or_default();
            local_bus.push("auth:result".to_string(), bytes);
        }
    }

    // Drain outgoing credentials (queued by Lua via Auth.SendCredentials) and send
    let outgoing = bridges.auth.drain_outgoing();
    if !outgoing.is_empty() {
        for mut sender in cred_senders.iter_mut() {
            for cred in &outgoing {
                sender.send::<AuthChannel>(AuthCredentials {
                    action: cred.action,
                    username: cred.username.clone(),
                    password: cred.password.clone(),
                });
            }
        }
    }
}
