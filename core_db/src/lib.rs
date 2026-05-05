//! `core_db` — sqlx backend pro Lua `Database.*` API.
//!
//! Implementuje `DbExecutorTrait` z `core_resources::db_bridge` pomocí
//! `sqlx::AnyPool` (podporuje SQLite i PostgreSQL dle connection URL).
//!
//! `DatabasePlugin`:
//! 1. Při Startup vytvoří sqlx pool ze `[database].url` v server configu.
//! 2. Nastaví `DatabaseBridgeResource` — ResourcesPlugin pak předá DbBridge
//!    každému Lua sandboxu.
//! 3. Spustí volitelné migrace ze složky `[database].migrations_path`.
//!
//! Rust ECS nikdy nesmí čekat na SQL dotaz — vše běží přes `tokio::spawn`
//! a výsledky se předávají přes `DbCallbackQueue` (drainované v PostUpdate).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use serde_json::Value as Json;
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{Column, Row};

use core_resources::{
    DatabaseBridgeResource, DbBridge, DbCallbackEntry, DbCallbackQueue,
    DbExecutorTrait, DbQueryResult, ResourceId,
};

// ---------------------------------------------------------------------------
// SqlxExecutor — implementace DbExecutorTrait
// ---------------------------------------------------------------------------

struct SqlxExecutor {
    pool: sqlx::AnyPool,
    connected: Arc<Mutex<bool>>,
    tokio: tokio::runtime::Handle,
}

impl DbExecutorTrait for SqlxExecutor {
    fn execute(
        &self,
        sql: String,
        params: Vec<Json>,
        resource_id: ResourceId,
        callback_id: u64,
        queue: DbCallbackQueue,
    ) {
        let pool = self.pool.clone();
        self.tokio.spawn(async move {
            let mut q = sqlx::query(&sql);
            for p in &params {
                q = bind_json(q, p);
            }
            let result = match q.execute(&pool).await {
                Ok(r) => DbQueryResult::RowsAffected(r.rows_affected()),
                Err(e) => DbQueryResult::Error(e.to_string()),
            };
            queue.push(DbCallbackEntry { resource_id, callback_id, result });
        });
    }

    fn query(
        &self,
        sql: String,
        params: Vec<Json>,
        resource_id: ResourceId,
        callback_id: u64,
        queue: DbCallbackQueue,
    ) {
        let pool = self.pool.clone();
        self.tokio.spawn(async move {
            let mut q = sqlx::query(&sql);
            for p in &params {
                q = bind_json(q, p);
            }
            let result = match q.fetch_all(&pool).await {
                Ok(rows) => DbQueryResult::Rows(rows.into_iter().map(row_to_map).collect()),
                Err(e) => DbQueryResult::Error(e.to_string()),
            };
            queue.push(DbCallbackEntry { resource_id, callback_id, result });
        });
    }

    fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap_or_else(|p| p.into_inner())
    }
}

// ---------------------------------------------------------------------------
// Pomocné funkce
// ---------------------------------------------------------------------------

fn bind_json<'q>(
    query: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    val: &Json,
) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    match val {
        Json::Null => query.bind(Option::<String>::None),
        Json::Bool(b) => query.bind(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                query.bind(i)
            } else if let Some(f) = n.as_f64() {
                query.bind(f)
            } else {
                query.bind(Option::<String>::None)
            }
        }
        Json::String(s) => query.bind(s.clone()),
        Json::Array(_) | Json::Object(_) => {
            query.bind(serde_json::to_string(val).unwrap_or_default())
        }
    }
}

fn row_to_map(row: AnyRow) -> HashMap<String, Json> {
    let mut map = HashMap::new();
    for col in row.columns() {
        let name = col.name().to_string();
        let val: Json = if let Ok(v) = row.try_get::<i64, _>(col.ordinal()) {
            Json::Number(v.into())
        } else if let Ok(v) = row.try_get::<f64, _>(col.ordinal()) {
            serde_json::Number::from_f64(v)
                .map(Json::Number)
                .unwrap_or(Json::Null)
        } else if let Ok(v) = row.try_get::<bool, _>(col.ordinal()) {
            Json::Bool(v)
        } else if let Ok(v) = row.try_get::<String, _>(col.ordinal()) {
            Json::String(v)
        } else {
            Json::Null
        };
        map.insert(name, val);
    }
    map
}

// ---------------------------------------------------------------------------
// Konfigurace předaná z host_server
// ---------------------------------------------------------------------------

/// Konfigurace DB pluginu předaná při startu serveru.
#[derive(Resource, Debug, Clone)]
pub struct DatabaseConfig {
    /// Connection URL — `"sqlite://bevy_game.db"` nebo `"postgres://..."`.
    /// `None` = DB vypnutá.
    pub url: Option<String>,
    /// Max connections v poolu.
    pub pool_size: u32,
    /// Volitelná složka s migracemi (`.sql` soubory).
    pub migrations_path: Option<std::path::PathBuf>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: None,
            pool_size: 8,
            migrations_path: None,
        }
    }
}

// ---------------------------------------------------------------------------
// DatabasePlugin
// ---------------------------------------------------------------------------

pub struct DatabasePlugin;

impl Plugin for DatabasePlugin {
    fn build(&self, app: &mut App) {
        // Registrujeme sqlx::Any driver — bez tohoto volání Any pool nefunguje.
        sqlx::any::install_default_drivers();

        app.init_resource::<DatabaseConfig>();
        app.add_systems(Startup, init_database);
    }
}

fn init_database(
    db_config: Res<DatabaseConfig>,
    db_callback_queue: Res<DbCallbackQueue>,
    mut bridge_res: ResMut<DatabaseBridgeResource>,
    tokio_handle: Res<TokioHandle>,
) {
    let Some(url) = &db_config.url else {
        info!("[core_db] No database URL configured — Database.* API disabled.");
        return;
    };

    let url = url.clone();
    let pool_size = db_config.pool_size;
    let migrations_path = db_config.migrations_path.clone();
    let queue = db_callback_queue.clone();
    let handle = tokio_handle.0.clone();
    let connected = Arc::new(Mutex::new(false));
    let connected_clone = connected.clone();

    let pool = handle.block_on(async move {
        let pool = AnyPoolOptions::new()
            .max_connections(pool_size)
            .connect(&url)
            .await
            .expect("[core_db] failed to connect to database");

        if let Some(ref path) = migrations_path {
            info!("[core_db] running migrations from {}", path.display());
            sqlx::migrate::Migrator::new(path.as_path())
                .await
                .expect("[core_db] failed to load migrations")
                .run(&pool)
                .await
                .expect("[core_db] migration failed");
            info!("[core_db] migrations complete");
        }

        *connected_clone.lock().unwrap_or_else(|p| p.into_inner()) = true;
        info!("[core_db] connected to {}", url);
        pool
    });

    let executor = Arc::new(SqlxExecutor {
        pool,
        connected,
        tokio: handle,
    });

    bridge_res.0 = Some(DbBridge {
        executor,
        queue,
    });
}

// ---------------------------------------------------------------------------
// TokioHandle — wrapper kolem tokio::runtime::Handle (z host_server)
// ---------------------------------------------------------------------------

/// Bevy Resource wrapping tokio runtime handle.
/// Definujeme ho tady (i v host_server), protože `DatabasePlugin` ho
/// potřebuje při Startup. host_server ho vloží před přidáním pluginu.
#[derive(Resource, Clone)]
pub struct TokioHandle(pub tokio::runtime::Handle);
