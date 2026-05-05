//! DB bridge — sdílené typy mezi `core_resources` (sandbox) a `core_db` (sqlx).
//!
//! `core_resources` nezávisí na `sqlx`. Místo toho definujeme:
//! * `DbQueryResult` — výsledek libovolného dotazu (rows, affected, error).
//! * `DbCallbackEntry` — čekající callback z async DB tasku.
//! * `DbCallbackQueue` — Bevy Resource (Arc<Mutex>): async task → push,
//!   Bevy systém → drain a dispatch do Lua sandboxů.
//! * `DbExecutorTrait` — trait pro spouštění queries; implementuje `core_db`.
//! * `DbBridge` — klon Arc na executor + sdílenou frontu; předává se sandboxům.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;
use serde_json::Value as Json;

use crate::types::ResourceId;

// ---------------------------------------------------------------------------
// Výsledky query
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum DbQueryResult {
    /// INSERT / UPDATE / DELETE — počet ovlivněných řádků.
    RowsAffected(u64),
    /// SELECT — řádky jako Vec<HashMap<sloupec, JSON hodnota>>.
    Rows(Vec<HashMap<String, Json>>),
    /// Chyba na úrovni SQL nebo backendu.
    Error(String),
}

// ---------------------------------------------------------------------------
// Callback queue
// ---------------------------------------------------------------------------

pub struct DbCallbackEntry {
    pub resource_id: ResourceId,
    pub callback_id: u64,
    pub result: DbQueryResult,
}

/// Fronta výsledků z async DB tasků. `core_db` do ní pushuje, Bevy systém v
/// `core_resources::plugin` ji drainuje a předá Lua sandboxům.
#[derive(Resource, Clone, Default)]
pub struct DbCallbackQueue(pub Arc<Mutex<Vec<DbCallbackEntry>>>);

impl DbCallbackQueue {
    pub fn push(&self, entry: DbCallbackEntry) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(entry);
    }

    pub fn drain(&self) -> Vec<DbCallbackEntry> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

// ---------------------------------------------------------------------------
// DbExecutorTrait — implementuje sqlx backend v core_db
// ---------------------------------------------------------------------------

/// Abstrakce nad sqlx poolem. `core_resources` ho nikdy neimportuje přímo —
/// jen udrží `Arc<dyn DbExecutorTrait>` a volá tyto metody.
pub trait DbExecutorTrait: Send + Sync + 'static {
    /// Spustí non-SELECT dotaz. Výsledek (RowsAffected nebo Error) pushne do `queue`.
    fn execute(
        &self,
        sql: String,
        params: Vec<Json>,
        resource_id: ResourceId,
        callback_id: u64,
        queue: DbCallbackQueue,
    );

    /// Spustí SELECT dotaz. Výsledek (Rows nebo Error) pushne do `queue`.
    fn query(
        &self,
        sql: String,
        params: Vec<Json>,
        resource_id: ResourceId,
        callback_id: u64,
        queue: DbCallbackQueue,
    );

    /// Je pool připojený a ready?
    fn is_connected(&self) -> bool;
}

// ---------------------------------------------------------------------------
// DbBridge — předává se do sandboxů
// ---------------------------------------------------------------------------

/// Klonovatelný handle na DB executor + sdílenou callback frontu.
/// `None` na klientovi (DB je server-only).
#[derive(Clone)]
pub struct DbBridge {
    pub executor: Arc<dyn DbExecutorTrait>,
    pub queue: DbCallbackQueue,
}

/// Bevy Resource umožňující ostatním pluginům (core_db) vložit
/// a ResourcesPlugin číst DbBridge při rebuild sandboxů.
#[derive(Resource, Default)]
pub struct DatabaseBridgeResource(pub Option<DbBridge>);
