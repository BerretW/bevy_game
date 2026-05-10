//! Phase 4 — username/password authentication (pure Rust, no Lua).
//!
//! Sekvence (auth_required = true):
//!
//! ```text
//!   Client                            Server
//!     | --- lightyear connect -------> |
//!     | <-- AuthChallenge ----------   |  send_challenge_on_connect observer
//!     |                                |
//!     | [native UI: user types creds]  |
//!     | -- AuthCredentials ----------> |  server_receive_credentials
//!     |                               [DB query: login or register]
//!     | <-- AuthResult (ok/fail) ----- |  server_send_auth_results
//!     | [handshake continues: download resources]
//! ```
//!
//! Hesla jsou posílána v plaintextu přes šifrovaný lightyear kanál.
//! Server hashuje blake3 před porovnáváním / ukládáním do DB.
//! Celý auth flow probíhá čistě v Rustu — žádné Lua hooky.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use lightyear::prelude::*;

use core_resources::{
    DatabaseBridgeResource, DbQueryResult, GameBridges, LocalEventBus, PendingAuthResult,
};

use crate::net_plugin::AuthChannel;
use crate::protocol::{AuthChallenge, AuthCredentials, AuthResult};

// ---------------------------------------------------------------------------
// Server auth resource — holds pending results from async DB tasks
// ---------------------------------------------------------------------------

/// Server config injected by host_server.
#[derive(Resource, Clone)]
pub struct ServerAuthConfig {
    pub require_auth: bool,
}

// ---------------------------------------------------------------------------
// ServerAuthPlugin
// ---------------------------------------------------------------------------

pub struct ServerAuthPlugin;

impl Plugin for ServerAuthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerAuthConfig>();
        app.add_observer(send_challenge_on_connect);
        app.add_systems(
            Update,
            (
                ensure_users_table,
                server_receive_credentials,
                server_send_auth_results,
            ),
        );
    }
}

impl Default for ServerAuthConfig {
    fn default() -> Self {
        Self { require_auth: false }
    }
}

// ---------------------------------------------------------------------------
// Ensure users table exists (runs once after DB connects)
// ---------------------------------------------------------------------------

fn ensure_users_table(db: Res<DatabaseBridgeResource>, mut done: Local<bool>) {
    if *done {
        return;
    }
    let Some(bridge) = &db.0 else { return };
    if !bridge.executor.is_connected() {
        return;
    }
    *done = true;
    bridge.executor.execute_rust(
        "CREATE TABLE IF NOT EXISTS users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT,\
            username TEXT NOT NULL UNIQUE COLLATE NOCASE,\
            password_hash TEXT NOT NULL,\
            account_id TEXT NOT NULL\
        )"
        .to_string(),
        vec![],
        Box::new(|result| match result {
            DbQueryResult::RowsAffected(_) => {
                info!("[auth/server] users table ready");
            }
            DbQueryResult::Error(e) => {
                error!("[auth/server] failed to ensure users table: {}", e);
            }
            _ => {}
        }),
    );
}

// ---------------------------------------------------------------------------
// Observer: send AuthChallenge immediately on connect
// ---------------------------------------------------------------------------

fn send_challenge_on_connect(
    trigger: On<Add, Connected>,
    mut remote_ids: Query<(&RemoteId, Option<&mut MessageSender<AuthChallenge>>)>,
    bridges: Res<GameBridges>,
    auth_cfg: Res<ServerAuthConfig>,
) {
    if !auth_cfg.require_auth {
        return;
    }

    let entity = trigger.entity;
    let Ok((remote_id, sender_opt)) = remote_ids.get_mut(entity) else {
        return;
    };

    let client_id = match remote_id.0 {
        PeerId::Netcode(id) => id,
        _ => 0,
    };

    // Always register as pending — decouple from whether sender exists yet.
    bridges.auth.add_pending(client_id);

    if let Some(mut sender) = sender_opt {
        sender.send::<AuthChannel>(AuthChallenge);
        info!(
            "[auth/server] AuthChallenge sent to client {} — awaiting credentials",
            client_id
        );
    } else {
        warn!(
            "[auth/server] no MessageSender<AuthChallenge> for client {} — pending but no challenge sent",
            client_id
        );
    }
}

// ---------------------------------------------------------------------------
// Receive credentials → run DB query (async, Tokio)
// ---------------------------------------------------------------------------

fn server_receive_credentials(
    mut receivers: Query<(&mut MessageReceiver<AuthCredentials>, &RemoteId)>,
    db: Res<DatabaseBridgeResource>,
    bridges: Res<GameBridges>,
    auth_cfg: Res<ServerAuthConfig>,
) {
    if !auth_cfg.require_auth {
        return;
    }

    let ace_registry = bridges.ace.clone();

    for (mut rx, remote_id) in receivers.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => 0,
        };

        for cred in rx.receive() {
            if !bridges.auth.is_pending(client_id) {
                debug!(
                    "[auth/server] ignoring credentials from already-authed client {}",
                    client_id
                );
                continue;
            }

            // action 0 = login, 1 = register  (matches auth_ui.rs + sandbox.rs convention)
            let is_register = cred.action == 1;
            let password_hash = blake3::hash(cred.password.as_bytes())
                .to_hex()
                .to_string();
            let username = cred.username.trim().to_string();

            info!(
                "[auth/server] {} attempt from client {} (user='{}')",
                if is_register { "register" } else { "login" },
                client_id,
                username
            );

            // No DB → reject immediately.
            let Some(bridge) = &db.0 else {
                push_result(
                    &bridges.auth.results,
                    client_id,
                    false,
                    "",
                    "Server database is not configured.",
                );
                continue;
            };

            if !is_register {
                // ---- LOGIN ----
                let results = bridges.auth.results.clone();
                let pending = bridges.auth.pending.clone();
                let ace = ace_registry.clone();
                bridge.executor.query_rust(
                    "SELECT account_id, password_hash FROM users WHERE username = ?".into(),
                    vec![serde_json::Value::String(username.clone())],
                    Box::new(move |result| match result {
                        DbQueryResult::Rows(rows) if !rows.is_empty() => {
                            let stored = rows[0]
                                .get("password_hash")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if stored == password_hash {
                                let account_id = rows[0]
                                    .get("account_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                // FiveM-style authority principal for ACL rules:
                                // identifier.user:<account_id>
                                ace.add_identifier(client_id, format!("user:{}", account_id));
                                pending
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .remove(&client_id);
                                push_result(&results, client_id, true, &account_id, "");
                                info!(
                                    "[auth/server] client {} logged in as '{}'",
                                    client_id, account_id
                                );
                            } else {
                                push_result(
                                    &results,
                                    client_id,
                                    false,
                                    "",
                                    "Invalid username or password.",
                                );
                            }
                        }
                        DbQueryResult::Rows(_) => {
                            push_result(
                                &results,
                                client_id,
                                false,
                                "",
                                "Invalid username or password.",
                            );
                        }
                        DbQueryResult::Error(e) => {
                            error!("[auth/server] DB login query error: {}", e);
                            push_result(&results, client_id, false, "", "Database error.");
                        }
                        _ => {}
                    }),
                );
            } else {
                // ---- REGISTER ----
                let results = bridges.auth.results.clone();
                let pending = bridges.auth.pending.clone();
                let executor = bridge.executor.clone();
                let ace = ace_registry.clone();

                // Step 1: check username is free
                bridge.executor.query_rust(
                    "SELECT COUNT(*) as cnt FROM users WHERE username = ?".into(),
                    vec![serde_json::Value::String(username.clone())],
                    Box::new(move |check_result| {
                        let taken = match &check_result {
                            DbQueryResult::Rows(rows) if !rows.is_empty() => rows[0]
                                .get("cnt")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0)
                                > 0,
                            _ => false,
                        };

                        if taken {
                            push_result(
                                &results,
                                client_id,
                                false,
                                "",
                                "Username is already taken.",
                            );
                            return;
                        }

                        // Step 2: insert new user
                        let account_id = generate_account_id(&username);
                        let results2 = results.clone();
                        let pending2 = pending.clone();
                        let account_id2 = account_id.clone();
                        executor.execute_rust(
                            "INSERT INTO users (username, password_hash, account_id) VALUES (?, ?, ?)"
                                .into(),
                            vec![
                                serde_json::Value::String(username.clone()),
                                serde_json::Value::String(password_hash.clone()),
                                serde_json::Value::String(account_id.clone()),
                            ],
                            Box::new(move |insert_result| match insert_result {
                                DbQueryResult::RowsAffected(_) => {
                                    // FiveM-style authority principal for ACL rules:
                                    // identifier.user:<account_id>
                                    ace.add_identifier(client_id, format!("user:{}", account_id2));
                                    pending2
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .remove(&client_id);
                                    push_result(
                                        &results2,
                                        client_id,
                                        true,
                                        &account_id2,
                                        "",
                                    );
                                    info!(
                                        "[auth/server] client {} registered as '{}'",
                                        client_id, account_id2
                                    );
                                }
                                DbQueryResult::Error(e) => {
                                    error!(
                                        "[auth/server] DB insert error for '{}': {}",
                                        username, e
                                    );
                                    push_result(
                                        &results2,
                                        client_id,
                                        false,
                                        "",
                                        "Registration failed.",
                                    );
                                }
                                _ => {}
                            }),
                        );
                    }),
                );
            }
        }
    }
}

fn push_result(
    results: &Arc<Mutex<Vec<PendingAuthResult>>>,
    player_id: u64,
    success: bool,
    account_id: &str,
    error: &str,
) {
    results
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(PendingAuthResult {
            player_id,
            success,
            account_id: account_id.to_string(),
            error: error.to_string(),
        });
}

// ---------------------------------------------------------------------------
// Drain result queue → send AuthResult back to clients
// ---------------------------------------------------------------------------

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
                "[auth/server] no sender found for auth result to player {}",
                result.player_id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn generate_account_id(username: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let data = format!("{}:{}", username.to_lowercase(), nanos);
    blake3::hash(data.as_bytes()).to_hex().to_string()
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

fn client_process_auth(
    mut challenge_rx: Query<&mut MessageReceiver<AuthChallenge>>,
    mut result_rx: Query<&mut MessageReceiver<AuthResult>>,
    mut cred_senders: Query<&mut MessageSender<AuthCredentials>>,
    bridges: Res<GameBridges>,
    local_bus: Res<LocalEventBus>,
) {
    // Receive AuthChallenge (informational — UI is driven by HandshakeStatus::AwaitingAuth)
    for mut rx in challenge_rx.iter_mut() {
        for _challenge in rx.receive() {
            info!("[auth/client] AuthChallenge received — showing login UI");
        }
    }

    // Receive AuthResult → signal bridge
    for mut rx in result_rx.iter_mut() {
        for result in rx.receive() {
            if result.success {
                info!("[auth/client] authenticated as '{}'", result.account_id);
                bridges.auth.set_client_authenticated();
            } else {
                warn!("[auth/client] auth failed: {}", result.error);
                bridges.auth.set_client_error(result.error.clone());
            }
            // Emit to Lua LocalEventBus so resources can react (e.g. show welcome)
            let payload = serde_json::json!({
                "success": result.success,
                "account_id": result.account_id,
                "error": result.error,
            });
            let bytes = serde_json::to_vec(&payload).unwrap_or_default();
            local_bus.push("auth:result".to_string(), bytes);
        }
    }

    // Drain outgoing credentials queued by native UI and send via lightyear
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
