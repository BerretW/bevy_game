//! Per-resource Lua sandbox.
//!
//! Phase 3.8 přidává:
//! * Cross-sandbox `TriggerEvent` bus (LocalEventBus — Arc<Mutex>).
//! * Strukturovaný JSON payload pro všechna Trigger* API.
//! * sender player_id v handlerech.
//! Phase 3.4: Engine namespace (Model Registry).
//! Phase 3.5: World.SpawnNetworkedObject.
//! Phase 4: Database.* namespace, Player.* namespace.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;
use mlua::{Lua, LuaOptions, MultiValue, RegistryKey, StdLib};
use serde_json::Value as Json;

use bevy::math::{EulerRot, Quat};

use crate::cmd_queue::{CommandQueue, EntityStateCache, LuaCommand, PlayerStatsCache};
use crate::db_bridge::{DbBridge, DbQueryResult};
use crate::manifest::Manifest;
use crate::model_registry::{ModelCommand, ModelCommandQueue};
use crate::nui_bridge::{NuiOutMsg, NuiOutQueue, resource_id_to_host};
use crate::types::{ResourceId, Side};

// ---------------------------------------------------------------------------
// Raycast bridge — shared Arc pro Raycast.GetGroundPosition()
// ---------------------------------------------------------------------------

/// Sdileny buffer pro aktualni world-space pozici mysi (Y=0 rovina).
/// Klientska gameplay vrstva ho aktualizuje kazdy frame.
/// Lua sandbox cte synchronne pres Raycast.GetGroundPosition().
/// Na serveru zustava [0,0,0] — Raycast namespace existuje, ale vraci nulu.
#[derive(Resource, Clone)]
pub struct RaycastBridge(pub Arc<Mutex<[f32; 3]>>);

impl Default for RaycastBridge {
    fn default() -> Self {
        Self(Arc::new(Mutex::new([0.0_f32; 3])))
    }
}

impl RaycastBridge {
    pub fn set_pos(&self, pos: [f32; 3]) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = pos;
    }
    pub fn get_pos(&self) -> [f32; 3] {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

// ---------------------------------------------------------------------------
// Cross-sandbox event bus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LocalEvent {
    pub name: String,
    pub payload: Vec<u8>,
}

#[derive(Resource, Clone, Default)]
pub struct LocalEventBus(pub Arc<Mutex<Vec<LocalEvent>>>);

impl LocalEventBus {
    pub fn push(&self, name: String, payload: Vec<u8>) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(LocalEvent { name, payload });
    }

    pub fn drain(&self) -> Vec<LocalEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

// ---------------------------------------------------------------------------
// LuaEventOut
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuaEventDirection {
    ToServer,
    ToClient,
}

#[derive(Debug, Clone)]
pub struct LuaEventOut {
    pub direction: LuaEventDirection,
    pub name: String,
    pub target: Option<u64>,
    pub payload: Vec<u8>,
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn lua_value_to_json(val: mlua::Value) -> Json {
    match val {
        mlua::Value::Nil => Json::Null,
        mlua::Value::Boolean(b) => Json::Bool(b),
        mlua::Value::Integer(i) => Json::Number(i.into()),
        mlua::Value::Number(f) => serde_json::Number::from_f64(f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        mlua::Value::String(s) => Json::String(s.to_str().map(|b| b.to_string()).unwrap_or_default()),
        mlua::Value::Table(t) => lua_table_to_json(t),
        _ => Json::Null,
    }
}

fn lua_table_to_json(t: mlua::Table) -> Json {
    let len = t.raw_len();
    if len > 0 {
        let arr: Vec<Json> = (1..=len)
            .filter_map(|i| t.get::<mlua::Value>(i).ok())
            .map(lua_value_to_json)
            .collect();
        if arr.len() == len as usize {
            return Json::Array(arr);
        }
    }
    let mut obj = serde_json::Map::new();
    for pair in t.pairs::<mlua::Value, mlua::Value>() {
        if let Ok((k, v)) = pair {
            let key = match &k {
                mlua::Value::String(s) => s.to_str().map(|b| b.to_string()).unwrap_or_default(),
                mlua::Value::Integer(i) => i.to_string(),
                mlua::Value::Number(f) => f.to_string(),
                _ => continue,
            };
            obj.insert(key, lua_value_to_json(v));
        }
    }
    Json::Object(obj)
}

fn json_to_lua_value(lua: &Lua, val: Json) -> mlua::Result<mlua::Value> {
    match val {
        Json::Null => Ok(mlua::Value::Nil),
        Json::Bool(b) => Ok(mlua::Value::Boolean(b)),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(mlua::Value::Number(f))
            } else {
                Ok(mlua::Value::Nil)
            }
        }
        Json::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        Json::Array(arr) => {
            let t = lua.create_table()?;
            for (i, v) in arr.into_iter().enumerate() {
                t.set(i + 1, json_to_lua_value(lua, v)?)?;
            }
            Ok(mlua::Value::Table(t))
        }
        Json::Object(obj) => {
            let t = lua.create_table()?;
            for (k, v) in obj {
                t.set(k, json_to_lua_value(lua, v)?)?;
            }
            Ok(mlua::Value::Table(t))
        }
    }
}

fn encode_payload(val: mlua::Value) -> Vec<u8> {
    match &val {
        mlua::Value::Nil => Vec::new(),
        _ => serde_json::to_vec(&lua_value_to_json(val)).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// LuaSandbox
// ---------------------------------------------------------------------------

pub struct LuaSandbox {
    pub id: ResourceId,
    pub side: Side,
    lua: Lua,
    outgoing: Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    /// Phase 4 — pending DB callbacks: callback_id → RegistryKey funkce.
    db_callbacks: Rc<RefCell<HashMap<u64, RegistryKey>>>,
    db_counter: Rc<RefCell<u64>>,
    /// Phase 4 — NUI callbacks: callback_name → list of handler RegistryKeys.
    nui_callbacks: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
}

impl LuaSandbox {
    pub fn create(
        manifest: &Manifest,
        side: Side,
        cmd_queue: CommandQueue,
        local_bus: LocalEventBus,
        model_cmds: ModelCommandQueue,
        raycast: RaycastBridge,
        stats_cache: PlayerStatsCache,
        entity_cache: EntityStateCache,
        db_bridge: Option<DbBridge>,
        nui_out: Option<NuiOutQueue>,
    ) -> Result<Self, SandboxError> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
            LuaOptions::default(),
        )
        .map_err(SandboxError::Init)?;

        let outgoing = Rc::new(RefCell::new(Vec::new()));
        let handlers: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let db_callbacks: Rc<RefCell<HashMap<u64, RegistryKey>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let db_counter: Rc<RefCell<u64>> = Rc::new(RefCell::new(0));
        let nui_callbacks: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>> =
            Rc::new(RefCell::new(HashMap::new()));

        // Pokud má resource ui_page, ihned enqueujeme AddFrame → NuiPlugin přidá iframe.
        if let (Some(ref nq), Some(_)) = (&nui_out, &manifest.ui_page) {
            nq.push(NuiOutMsg::AddFrame { resource_host: resource_id_to_host(&manifest.id) });
        }

        install_runtime_api(
            &lua,
            &manifest.id,
            side,
            &outgoing,
            &handlers,
            &cmd_queue,
            &local_bus,
            &model_cmds,
            &raycast,
            &stats_cache,
            &entity_cache,
            &db_bridge,
            &db_callbacks,
            &db_counter,
            &nui_out,
            &nui_callbacks,
        )?;

        let scripts = manifest.shared_scripts.iter().chain(match side {
            Side::Server => manifest.server_scripts.iter(),
            Side::Client => manifest.client_scripts.iter(),
        });

        for rel in scripts {
            let abs = manifest.root.join(rel);
            run_script(&lua, &manifest.id, rel, &abs)?;
        }

        Ok(Self { id: manifest.id.clone(), side, lua, outgoing, handlers, db_callbacks, db_counter, nui_callbacks })
    }

    pub fn drain_outgoing(&self) -> Vec<LuaEventOut> {
        std::mem::take(&mut *self.outgoing.borrow_mut())
    }

    /// Dispatch incoming event do tohoto sandboxu.
    /// payload = JSON bytes (nebo prazdny Vec pro nil).
    /// sender  = client_id kdo poslal (None pro lokalni TriggerEvent).
    pub fn dispatch_incoming(
        &self,
        name: &str,
        payload: &[u8],
        sender: Option<u64>,
    ) -> Result<usize, mlua::Error> {
        let handlers = self.handlers.borrow();
        let Some(keys) = handlers.get(name) else {
            return Ok(0);
        };

        let lua_payload: mlua::Value = if payload.is_empty() {
            mlua::Value::Nil
        } else if let Ok(json) = serde_json::from_slice::<Json>(payload) {
            json_to_lua_value(&self.lua, json)?
        } else {
            mlua::Value::String(self.lua.create_string(payload)?)
        };

        let mut count = 0;
        for key in keys.iter() {
            let f: mlua::Function = self.lua.registry_value(key)?;
            f.call::<()>((lua_payload.clone(), sender))?;
            count += 1;
        }
        Ok(count)
    }

    pub fn handler_count(&self, name: &str) -> usize {
        self.handlers.borrow().get(name).map(|v| v.len()).unwrap_or(0)
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Phase 4 — zavolá všechny Lua handlery registrované přes `RegisterNUICallback`.
    /// `data` jsou JSON bytes z těla POST requestu (`[]` = prázdné / `{}`).
    pub fn invoke_nui_callback(&self, callback_name: &str, data: &[u8]) {
        let callbacks = self.nui_callbacks.borrow();
        let Some(keys) = callbacks.get(callback_name) else { return };
        let lua_payload: mlua::Value = if data.is_empty() {
            mlua::Value::Nil
        } else if let Ok(json) = serde_json::from_slice::<Json>(data) {
            match json_to_lua_value(&self.lua, json) {
                Ok(v) => v,
                Err(e) => {
                    bevy::log::warn!("[nui:{}] json_to_lua error for cb '{}': {}", self.id, callback_name, e);
                    mlua::Value::Nil
                }
            }
        } else {
            mlua::Value::Nil
        };
        for key in keys.iter() {
            let f: mlua::Function = match self.lua.registry_value(key) {
                Ok(f) => f,
                Err(e) => {
                    bevy::log::warn!("[nui:{}] registry_value error for cb '{}': {}", self.id, callback_name, e);
                    continue;
                }
            };
            if let Err(e) = f.call::<()>(lua_payload.clone()) {
                bevy::log::warn!("[nui:{}] callback '{}' error: {}", self.id, callback_name, e);
            }
        }
    }

    /// Phase 4 — zavolá Lua callback registrovaný pro DB dotaz `callback_id`.
    pub fn invoke_db_callback(&self, callback_id: u64, result: DbQueryResult) {
        let key = self.db_callbacks.borrow_mut().remove(&callback_id);
        let Some(key) = key else { return };
        let f: mlua::Function = match self.lua.registry_value(&key) {
            Ok(f) => f,
            Err(e) => {
                bevy::log::warn!("[db:{}] registry_value error for cb {}: {}", self.id, callback_id, e);
                return;
            }
        };
        let lua_val = db_result_to_lua(&self.lua, result);
        if let Err(e) = f.call::<()>(lua_val) {
            bevy::log::warn!("[db:{}] callback {} error: {}", self.id, callback_id, e);
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to initialize Lua VM: {0}")]
    Init(#[source] mlua::Error),
    #[error("failed to install runtime API for {id}: {source}")]
    Api { id: ResourceId, #[source] source: mlua::Error },
    #[error("io error reading script {script_rel} of {id}: {source}")]
    Io { id: ResourceId, script_rel: String, #[source] source: std::io::Error },
    #[error("lua error in {id}/{script_rel}: {source}")]
    Lua { id: ResourceId, script_rel: String, #[source] source: mlua::Error },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_script(lua: &Lua, id: &ResourceId, rel: &str, abs: &Path) -> Result<(), SandboxError> {
    let source = std::fs::read_to_string(abs).map_err(|e| SandboxError::Io {
        id: id.clone(),
        script_rel: rel.to_string(),
        source: e,
    })?;
    lua.load(&source)
        .set_name(format!("{}:{}", id, rel))
        .exec()
        .map_err(|e| SandboxError::Lua {
            id: id.clone(),
            script_rel: rel.to_string(),
            source: e,
        })
}

fn table_to_vec3(t: &mlua::Table) -> [f32; 3] {
    let get = |name: &str, idx: i32| -> f32 {
        t.get::<f32>(name).or_else(|_| t.get::<f32>(idx)).unwrap_or(0.0)
    };
    [get("x", 1), get("y", 2), get("z", 3)]
}

/// Převede Lua player_id (integer, number nebo string) na u64. 
fn lua_value_to_u64(v: &mlua::Value) -> Option<u64> {
    match v {
        mlua::Value::Integer(i) if *i > 0 => Some(*i as u64),
        mlua::Value::Number(f) if *f > 0.0 => Some(*f as u64),
        mlua::Value::String(s) => s.to_str().ok().and_then(|s| s.parse::<u64>().ok()),
        _ => None,
    }
}

/// Převede Lua table parametrů (nebo nil) na Vec<Json> pro DB executor.
fn json_params(v: mlua::Value) -> Vec<Json> {
    match v {
        mlua::Value::Table(t) => {
            let len = t.raw_len();
            (1..=len)
                .filter_map(|i| t.get::<mlua::Value>(i).ok())
                .map(lua_value_to_json)
                .collect()
        }
        mlua::Value::Nil => Vec::new(),
        other => vec![lua_value_to_json(other)],
    }
}

// ---------------------------------------------------------------------------
// Runtime API installer
// ---------------------------------------------------------------------------

/// Converts DbQueryResult to Lua value.
/// RowsAffected → integer, Rows → table of row-tables, Error → (nil, error_string)
fn db_result_to_lua(lua: &Lua, result: DbQueryResult) -> mlua::Value {
    match result {
        DbQueryResult::RowsAffected(n) => mlua::Value::Integer(n as i64),
        DbQueryResult::Rows(rows) => {
            let table = match lua.create_table() {
                Ok(t) => t,
                Err(_) => return mlua::Value::Nil,
            };
            for (i, row) in rows.into_iter().enumerate() {
                let row_table = match lua.create_table() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                for (k, v) in row {
                    let lua_val = json_to_lua_value(lua, v).unwrap_or(mlua::Value::Nil);
                    let _ = row_table.set(k, lua_val);
                }
                let _ = table.set(i + 1, row_table);
            }
            mlua::Value::Table(table)
        }
        DbQueryResult::Error(e) => {
            bevy::log::warn!("[db] query error: {}", e);
            mlua::Value::Nil
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn install_runtime_api(
    lua: &Lua,
    id: &ResourceId,
    side: Side,
    outgoing: &Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    cmd_queue: &CommandQueue,
    local_bus: &LocalEventBus,
    model_cmds: &ModelCommandQueue,
    raycast: &RaycastBridge,
    stats_cache: &PlayerStatsCache,
    entity_cache: &EntityStateCache,
    db_bridge: &Option<DbBridge>,
    db_callbacks: &Rc<RefCell<HashMap<u64, RegistryKey>>>,
    db_counter: &Rc<RefCell<u64>>,
    nui_out: &Option<NuiOutQueue>,
    nui_callbacks: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
) -> Result<(), SandboxError> {
    install_runtime_api_inner(lua, id, side, outgoing, handlers, cmd_queue, local_bus, model_cmds, raycast, stats_cache, entity_cache, db_bridge, db_callbacks, db_counter, nui_out, nui_callbacks)
        .map_err(|e| SandboxError::Api { id: id.clone(), source: e })
}

#[allow(clippy::too_many_arguments)]
fn install_runtime_api_inner(
    lua: &Lua,
    id: &ResourceId,
    side: Side,
    outgoing: &Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    cmd_queue: &CommandQueue,
    local_bus: &LocalEventBus,
    model_cmds: &ModelCommandQueue,
    raycast: &RaycastBridge,
    stats_cache: &PlayerStatsCache,
    entity_cache: &EntityStateCache,
    db_bridge: &Option<DbBridge>,
    db_callbacks: &Rc<RefCell<HashMap<u64, RegistryKey>>>,
    db_counter: &Rc<RefCell<u64>>,
    nui_out: &Option<NuiOutQueue>,
    nui_callbacks: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
) -> mlua::Result<()> {
    let globals = lua.globals();

    // print
    let id_p = id.clone();
    globals.set("print", lua.create_function(move |_, args: MultiValue| {
        let parts: Vec<String> = args.iter().map(|v| format!("{:?}", v)).collect();
        bevy::log::info!("[lua:{}] {}", id_p, parts.join("\t"));
        Ok(())
    })?)?;

    let id_d = id.clone();
    globals.set("log_debug", lua.create_function(move |_, msg: String| {
        bevy::log::debug!("[lua:{}] {}", id_d, msg); Ok(())
    })?)?;
    let id_i = id.clone();
    globals.set("log_info", lua.create_function(move |_, msg: String| {
        bevy::log::info!("[lua:{}] {}", id_i, msg); Ok(())
    })?)?;
    let id_w = id.clone();
    globals.set("log_warn", lua.create_function(move |_, msg: String| {
        bevy::log::warn!("[lua:{}] {}", id_w, msg); Ok(())
    })?)?;

    globals.set("RESOURCE_ID", id.as_str())?;
    globals.set("SIDE", side.label())?;
    globals.set("IS_SERVER", side == Side::Server)?;
    globals.set("IS_CLIENT", side == Side::Client)?;

    // RegisterEvent
    let handlers_for_reg = handlers.clone();
    globals.set("RegisterEvent", lua.create_function(move |lua, (name, f): (String, mlua::Function)| {
        let key = lua.create_registry_value(f)?;
        handlers_for_reg.borrow_mut().entry(name).or_default().push(key);
        Ok(())
    })?)?;

    // TriggerEvent — cross-sandbox same-process bus (Phase 3.8)
    let bus = local_bus.clone();
    let id_e = id.clone();
    globals.set("TriggerEvent", lua.create_function(move |_, (name, payload): (String, mlua::Value)| {
        let bytes = encode_payload(payload);
        bevy::log::trace!("[lua:{}] TriggerEvent '{}'", id_e, name);
        bus.push(name, bytes);
        Ok(())
    })?)?;

    // Side-specific network RPC
    match side {
        Side::Client => {
            let out = outgoing.clone();
            globals.set("TriggerServerEvent", lua.create_function(
                move |_, (name, payload): (String, mlua::Value)| {
                    out.borrow_mut().push(LuaEventOut {
                        direction: LuaEventDirection::ToServer,
                        name,
                        target: None,
                        payload: encode_payload(payload),
                    });
                    Ok(())
                },
            )?)?;
            globals.set("TriggerClientEvent", lua.create_function(|_, _: MultiValue| -> mlua::Result<()> {
                Err(mlua::Error::RuntimeError("TriggerClientEvent is server-only".into()))
            })?)?;
        }
        Side::Server => {
            // TriggerClientEvent(name, target, payload)
            // target: nil/false = broadcast, positive integer = unicast player_id
            let out = outgoing.clone();
            globals.set("TriggerClientEvent", lua.create_function(
                move |_, (name, target, payload): (String, mlua::Value, mlua::Value)| {
                    let target_id: Option<u64> = match &target {
                        mlua::Value::Nil | mlua::Value::Boolean(false) => None,
                        mlua::Value::Integer(i) if *i > 0 => Some(*i as u64),
                        mlua::Value::Number(f) if *f > 0.0 => Some(*f as u64),
                        mlua::Value::String(s) => s
                            .to_str()
                            .ok()
                            .and_then(|v| v.parse::<u64>().ok()),
                        _ => None,
                    };
                    out.borrow_mut().push(LuaEventOut {
                        direction: LuaEventDirection::ToClient,
                        name,
                        target: target_id,
                        payload: encode_payload(payload),
                    });
                    Ok(())
                },
            )?)?;
            globals.set("TriggerServerEvent", lua.create_function(|_, _: MultiValue| -> mlua::Result<()> {
                Err(mlua::Error::RuntimeError("TriggerServerEvent is client-only".into()))
            })?)?;
        }
    }

    // World namespace
    let world = lua.create_table()?;

    let cq = cmd_queue.clone();
    world.set("SpawnLocalObject", lua.create_function(
        move |_, (model, pos_t, rot_t): (String, mlua::Table, mlua::Table)| {
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnLocalObject { handle, model, pos, rot });
            Ok(handle)
        },
    )?)?;

    // World.SpawnNetworkedObject — server-only, replikovana entita (Phase 3.5)
    let cq = cmd_queue.clone();
    world.set("SpawnNetworkedObject", lua.create_function(
        move |_, (model, pos_t, rot_t): (String, mlua::Table, mlua::Table)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError(
                    "World.SpawnNetworkedObject is server-only".into(),
                ));
            }
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnNetworkedObject { handle, model, pos, rot });
            Ok(handle)
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("DeleteObject", lua.create_function(move |_, handle: u64| {
        cq.push(LuaCommand::DespawnEntity { handle });
        Ok(())
    })?)?;

    let cq = cmd_queue.clone();
    world.set("SetTransform", lua.create_function(
        move |_, (handle, pos_t, rot_t): (u64, mlua::Table, mlua::Table)| {
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            cq.push(LuaCommand::SetTransform { handle, pos, rot });
            Ok(())
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("ApplyDamage", lua.create_function(
        move |_, (target_handle, amount, source_handle): (u64, f32, Option<u64>)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("World.ApplyDamage is server-only".into()));
            }
            cq.push(LuaCommand::ApplyDamage { target_handle, amount, source_handle });
            Ok(())
        },
    )?)?;

    // -- Entity state getters (synchronous read from EntityStateCache) --------

    let ec = entity_cache.clone();
    world.set("IsValid", lua.create_function(move |_, handle: u64| {
        Ok(ec.is_valid(handle))
    })?)?;

    let ec = entity_cache.clone();
    world.set("IsAlive", lua.create_function(move |_, handle: u64| {
        Ok(ec.get(handle).map(|s| s.alive).unwrap_or(false))
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetHealth", lua.create_function(move |_, handle: u64| -> mlua::Result<mlua::Value> {
        Ok(match ec.get(handle).and_then(|s| s.health) {
            Some(v) => mlua::Value::Number(v as f64),
            None => mlua::Value::Nil,
        })
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetModel", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        Ok(match ec.get(handle).and_then(|s| s.model) {
            Some(m) => mlua::Value::String(lua.create_string(m)?),
            None => mlua::Value::Nil,
        })
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetPosition", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let t = lua.create_table()?;
        t.set("x", snap.pos[0])?;
        t.set("y", snap.pos[1])?;
        t.set("z", snap.pos[2])?;
        Ok(mlua::Value::Table(t))
    })?)?;

    // Vrátí rotaci jako Euler XYZ ve stupních — stejný formát jako SetTransform.
    let ec = entity_cache.clone();
    world.set("GetRotation", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let q = Quat::from_array(snap.rot);
        let (ex, ey, ez) = q.to_euler(EulerRot::XYZ);
        let t = lua.create_table()?;
        t.set("x", ex.to_degrees())?;
        t.set("y", ey.to_degrees())?;
        t.set("z", ez.to_degrees())?;
        Ok(mlua::Value::Table(t))
    })?)?;

    // Vrátí rotaci jako kvaternion {x, y, z, w} pro přesné výpočty (bez gimbal locku).
    let ec = entity_cache.clone();
    world.set("GetQuaternion", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let t = lua.create_table()?;
        t.set("x", snap.rot[0])?;
        t.set("y", snap.rot[1])?;
        t.set("z", snap.rot[2])?;
        t.set("w", snap.rot[3])?;
        Ok(mlua::Value::Table(t))
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetScale", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let t = lua.create_table()?;
        t.set("x", snap.scale[0])?;
        t.set("y", snap.scale[1])?;
        t.set("z", snap.scale[2])?;
        Ok(mlua::Value::Table(t))
    })?)?;

    // Vrátí celý transform najednou: {pos={x,y,z}, rot={x,y,z}, scale={x,y,z}}
    let ec = entity_cache.clone();
    world.set("GetTransform", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let q = Quat::from_array(snap.rot);
        let (ex, ey, ez) = q.to_euler(EulerRot::XYZ);

        let pos_t = lua.create_table()?;
        pos_t.set("x", snap.pos[0])?;
        pos_t.set("y", snap.pos[1])?;
        pos_t.set("z", snap.pos[2])?;

        let rot_t = lua.create_table()?;
        rot_t.set("x", ex.to_degrees())?;
        rot_t.set("y", ey.to_degrees())?;
        rot_t.set("z", ez.to_degrees())?;

        let scale_t = lua.create_table()?;
        scale_t.set("x", snap.scale[0])?;
        scale_t.set("y", snap.scale[1])?;
        scale_t.set("z", snap.scale[2])?;

        let t = lua.create_table()?;
        t.set("pos", pos_t)?;
        t.set("rot", rot_t)?;
        t.set("scale", scale_t)?;
        Ok(mlua::Value::Table(t))
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetAnimation", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        Ok(match ec.get(handle).and_then(|s| s.animation) {
            Some(a) => mlua::Value::String(lua.create_string(a)?),
            None => mlua::Value::Nil,
        })
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetAnimationSpeed", lua.create_function(move |_, handle: u64| -> mlua::Result<f64> {
        Ok(ec.get(handle).map(|s| s.anim_speed as f64).unwrap_or(1.0))
    })?)?;

    // -- Entity state setters (queued commands) --------------------------------

    let cq = cmd_queue.clone();
    world.set("SetModel", lua.create_function(
        move |_, (handle, model): (u64, String)| {
            cq.push(LuaCommand::SetModel { handle, model });
            Ok(())
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("SetPosition", lua.create_function(
        move |_, (handle, pos_t): (u64, mlua::Table)| {
            let pos = table_to_vec3(&pos_t);
            cq.push(LuaCommand::SetPosition { handle, pos });
            Ok(())
        },
    )?)?;

    // SetRotation přijímá Euler XYZ ve stupních — stejný formát jako SetTransform.
    let cq = cmd_queue.clone();
    world.set("SetRotation", lua.create_function(
        move |_, (handle, rot_t): (u64, mlua::Table)| {
            let rot = table_to_vec3(&rot_t);
            cq.push(LuaCommand::SetRotation { handle, rot });
            Ok(())
        },
    )?)?;

    // SetScale přijímá číslo (uniform) nebo tabulku {x, y, z}.
    let cq = cmd_queue.clone();
    world.set("SetScale", lua.create_function(
        move |_, (handle, scale_v): (u64, mlua::Value)| {
            let scale = match &scale_v {
                mlua::Value::Number(f) => [*f as f32; 3],
                mlua::Value::Integer(i) => [*i as f32; 3],
                mlua::Value::Table(t) => table_to_vec3(t),
                _ => [1.0; 3],
            };
            cq.push(LuaCommand::SetScale { handle, scale });
            Ok(())
        },
    )?)?;

    // PlayAnimation(handle, name, looping?, speed?) — looping=true, speed=1.0 by default
    let cq = cmd_queue.clone();
    world.set("PlayAnimation", lua.create_function(
        move |_, (handle, name, looping_v, speed_v): (u64, String, Option<bool>, Option<f32>)| {
            cq.push(LuaCommand::PlayAnimation {
                handle,
                name,
                looping: looping_v.unwrap_or(true),
                speed: speed_v.unwrap_or(1.0),
            });
            Ok(())
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("StopAnimation", lua.create_function(move |_, handle: u64| {
        cq.push(LuaCommand::StopAnimation { handle });
        Ok(())
    })?)?;

    globals.set("World", world)?;

    // Engine namespace — Model Registry (Phase 3.4)
    let engine = lua.create_table()?;

    let mc = model_cmds.clone();
    engine.set("RequestModel", lua.create_function(move |_, name: String| {
        mc.push(ModelCommand::Request(name));
        Ok(())
    })?)?;

    // HasModelLoaded — Phase 3 stub (always false); Phase 4 doplni async callback
    engine.set("HasModelLoaded", lua.create_function(|_, _name: String| -> mlua::Result<bool> {
        Ok(false)
    })?)?;

    let mc = model_cmds.clone();
    engine.set("SetModelAsNoLongerNeeded", lua.create_function(move |_, name: String| {
        mc.push(ModelCommand::Release(name));
        Ok(())
    })?)?;

    globals.set("Engine", engine)?;

    // -- Raycast namespace (Phase 3.7) ---------------------------------------
    // Klientsky gameplay system aktualizuje RaycastBridge kazdy frame.
    // Raycast.GetGroundPosition() vraci posledni znamou pozici mysi
    // na Y=0 rovine. Na serveru vzdy vraci {0,0,0}.
    let rc_ns = lua.create_table()?;
    let pos_arc = raycast.0.clone();
    rc_ns.set(
        "GetGroundPosition",
        lua.create_function(move |lua, ()| {
            let pos = *pos_arc.lock().unwrap_or_else(|p| p.into_inner());
            let t = lua.create_table()?;
            t.set("x", pos[0])?;
            t.set("y", pos[1])?;
            t.set("z", pos[2])?;
            Ok(t)
        })?,
    )?;
    globals.set("Raycast", rc_ns)?;

    // -- Player namespace (Phase 4) -----------------------------------------
    // Server-only: čtení z PlayerStatsCache (Arc), zápisy přes CommandQueue.
    let player_ns = lua.create_table()?;

    // Player.GetStat(player_id, stat_name) -> value|nil
    let sc = stats_cache.clone();
    player_ns.set("GetStat", lua.create_function(
        move |_, (player_id_v, name): (mlua::Value, String)| {
            let pid = lua_value_to_u64(&player_id_v);
            let val = pid.and_then(|id| sc.get(id))
                         .and_then(|snap| snap.stats.get(&name).copied());
            Ok(val)
        },
    )?)?;

    // Player.GetStats(player_id) -> table|nil
    let sc = stats_cache.clone();
    player_ns.set("GetStats", lua.create_function(
        move |lua, player_id_v: mlua::Value| {
            let pid = lua_value_to_u64(&player_id_v);
            let Some(snap) = pid.and_then(|id| sc.get(id)) else {
                return Ok(mlua::Value::Nil);
            };
            let t = lua.create_table()?;
            for (k, v) in &snap.stats {
                t.set(k.as_str(), *v)?;
            }
            Ok(mlua::Value::Table(t))
        },
    )?)?;

    // Player.SetStat(player_id, name, value)  — server only
    let cq = cmd_queue.clone();
    player_ns.set("SetStat", lua.create_function(
        move |_, (player_id_v, name, value): (mlua::Value, String, f64)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Player.SetStat is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            cq.push(LuaCommand::SetStat { player_id, name, value });
            Ok(())
        },
    )?)?;

    // Player.GetHealth(player_id) -> number|nil
    let sc = stats_cache.clone();
    player_ns.set("GetHealth", lua.create_function(
        move |_, player_id_v: mlua::Value| {
            let pid = lua_value_to_u64(&player_id_v);
            let val = pid.and_then(|id| sc.get(id)).map(|snap| snap.health);
            Ok(val)
        },
    )?)?;

    // Player.GetInventory(player_id) -> table|nil
    let sc = stats_cache.clone();
    player_ns.set("GetInventory", lua.create_function(
        move |lua, player_id_v: mlua::Value| {
            let pid = lua_value_to_u64(&player_id_v);
            let Some(snap) = pid.and_then(|id| sc.get(id)) else {
                return Ok(mlua::Value::Nil);
            };
            let t = lua.create_table()?;
            for (k, v) in &snap.inventory {
                t.set(k.as_str(), *v)?;
            }
            Ok(mlua::Value::Table(t))
        },
    )?)?;

    // Player.GetItemCount(player_id, item) -> integer
    let sc = stats_cache.clone();
    player_ns.set("GetItemCount", lua.create_function(
        move |_, (player_id_v, item): (mlua::Value, String)| {
            let pid = lua_value_to_u64(&player_id_v);
            let count = pid.and_then(|id| sc.get(id))
                           .and_then(|snap| snap.inventory.get(&item).copied())
                           .unwrap_or(0);
            Ok(count)
        },
    )?)?;

    // Player.GiveItem(player_id, item, count) — server only
    let cq = cmd_queue.clone();
    player_ns.set("GiveItem", lua.create_function(
        move |_, (player_id_v, item, count): (mlua::Value, String, i32)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Player.GiveItem is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            cq.push(LuaCommand::GiveItem { player_id, item, count });
            Ok(())
        },
    )?)?;

    // Player.TakeItem(player_id, item, count) — server only (alias GiveItem negative)
    let cq = cmd_queue.clone();
    player_ns.set("TakeItem", lua.create_function(
        move |_, (player_id_v, item, count): (mlua::Value, String, u32)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Player.TakeItem is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            cq.push(LuaCommand::GiveItem { player_id, item, count: -(count as i32) });
            Ok(())
        },
    )?)?;

    globals.set("Player", player_ns)?;

    // -- Database namespace (Phase 4) — server only, jen pokud je bridge k dispozici --
    if let Some(bridge) = db_bridge {
        let db_ns = lua.create_table()?;
        let res_id = id.clone();

        // Database.execute(sql, params, callback)
        // callback(rows_affected: integer)  — INSERT/UPDATE/DELETE
        {
            let bridge = bridge.clone();
            let res_id = res_id.clone();
            let cbs = db_callbacks.clone();
            let cnt = db_counter.clone();
            db_ns.set("execute", lua.create_function(
                move |lua, (sql, params_v, cb): (String, mlua::Value, mlua::Function)| {
                    if side != Side::Server {
                        return Err(mlua::Error::RuntimeError("Database.execute is server-only".into()));
                    }
                    let cb_id = { let mut c = cnt.borrow_mut(); *c += 1; *c };
                    let key = lua.create_registry_value(cb)?;
                    cbs.borrow_mut().insert(cb_id, key);
                    let params = json_params(params_v);
                    bridge.executor.execute(
                        sql, params,
                        res_id.clone(), cb_id,
                        bridge.queue.clone(),
                    );
                    Ok(())
                },
            )?)?;
        }

        // Database.query(sql, params, callback)
        // callback(rows: table)  — SELECT
        {
            let bridge = bridge.clone();
            let res_id = res_id.clone();
            let cbs = db_callbacks.clone();
            let cnt = db_counter.clone();
            db_ns.set("query", lua.create_function(
                move |lua, (sql, params_v, cb): (String, mlua::Value, mlua::Function)| {
                    if side != Side::Server {
                        return Err(mlua::Error::RuntimeError("Database.query is server-only".into()));
                    }
                    let cb_id = { let mut c = cnt.borrow_mut(); *c += 1; *c };
                    let key = lua.create_registry_value(cb)?;
                    cbs.borrow_mut().insert(cb_id, key);
                    let params = json_params(params_v);
                    bridge.executor.query(
                        sql, params,
                        res_id.clone(), cb_id,
                        bridge.queue.clone(),
                    );
                    Ok(())
                },
            )?)?;
        }

        // Database.isConnected() -> bool
        {
            let bridge = bridge.clone();
            db_ns.set("isConnected", lua.create_function(
                move |_, ()| Ok(bridge.executor.is_connected()),
            )?)?;
        }

        globals.set("Database", db_ns)?;
    } else {
        // Stub na klientu / bez DB — vrátí runtime error při volání
        let db_ns = lua.create_table()?;
        for fname in &["execute", "query", "isConnected"] {
            let n = fname.to_string();
            db_ns.set(*fname, lua.create_function(move |_, _: MultiValue| -> mlua::Result<mlua::Value> {
                Err(mlua::Error::RuntimeError(format!("Database.{} is not available (no DB configured)", n)))
            })?)?;
        }
        globals.set("Database", db_ns)?;
    }

    // -- NUI namespace (Phase 4) — client only --------------------------------
    // Napodobuje FiveM NUI API:
    //   SendNUIMessage(data)               — odešle JSON zprávu do ui_page iframe
    //   RegisterNUICallback(name, handler) — zaregistruje handler pro JS fetch callback
    //   SetNUIFocus(hasFocus, hasCursor?)  — přepne zachycování vstupu
    if side == Side::Client {
        let resource_host = resource_id_to_host(id);

        // SendNUIMessage(data) — dispatch do iframe přes window.postMessage
        if let Some(nq) = nui_out.clone() {
            let rh = resource_host.clone();
            globals.set("SendNUIMessage", lua.create_function(move |_, data: mlua::Value| {
                let json = serde_json::to_string(&lua_value_to_json(data))
                    .unwrap_or_else(|_| "null".to_string());
                nq.push(NuiOutMsg::Dispatch { resource_host: rh.clone(), json });
                Ok(())
            })?)?;
        } else {
            globals.set("SendNUIMessage", lua.create_function(|_, _: mlua::Value| -> mlua::Result<()> {
                bevy::log::warn!("[nui] SendNUIMessage called but NUI is not active");
                Ok(())
            })?)?;
        }

        // RegisterNUICallback(name, handler) — handler(data) volán z JS fetch
        let nui_cbs = nui_callbacks.clone();
        globals.set("RegisterNUICallback", lua.create_function(
            move |lua, (name, f): (String, mlua::Function)| {
                let key = lua.create_registry_value(f)?;
                nui_cbs.borrow_mut().entry(name).or_default().push(key);
                Ok(())
            },
        )?)?;

        // SetNUIFocus(hasFocus, hasCursor?) — přepne overlay input routing
        if let Some(nq) = nui_out.clone() {
            globals.set("SetNUIFocus", lua.create_function(
                move |_, (has_focus, has_cursor): (bool, Option<bool>)| {
                    nq.push(NuiOutMsg::SetFocus {
                        has_focus,
                        has_cursor: has_cursor.unwrap_or(false),
                    });
                    Ok(())
                },
            )?)?;
        } else {
            globals.set("SetNUIFocus", lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;
        }
    } else {
        // Server stubs — tyto API jsou client-only
        for name in &["SendNUIMessage", "RegisterNUICallback", "SetNUIFocus"] {
            let n = name.to_string();
            globals.set(*name, lua.create_function(move |_, _: MultiValue| -> mlua::Result<()> {
                Err(mlua::Error::RuntimeError(format!("{} is client-only", n)))
            })?)?;
        }
    }

    Ok(())
}
