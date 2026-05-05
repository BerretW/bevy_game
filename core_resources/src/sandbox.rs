//! Per-resource Lua sandbox.
//!
//! Phase 3.8 přidává:
//! * Cross-sandbox `TriggerEvent` bus (LocalEventBus — Arc<Mutex>).
//! * Strukturovaný JSON payload pro všechna Trigger* API.
//! * sender player_id v handlerech.
//! Phase 3.4: Engine namespace (Model Registry).
//! Phase 3.5: World.SpawnNetworkedObject.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;
use mlua::{Lua, LuaOptions, MultiValue, RegistryKey, StdLib};
use serde_json::Value as Json;

use crate::cmd_queue::{CommandQueue, LuaCommand};
use crate::manifest::Manifest;
use crate::model_registry::{ModelCommand, ModelCommandQueue};
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
}

impl LuaSandbox {
    pub fn create(
        manifest: &Manifest,
        side: Side,
        cmd_queue: CommandQueue,
        local_bus: LocalEventBus,
        model_cmds: ModelCommandQueue,
        raycast: RaycastBridge,
    ) -> Result<Self, SandboxError> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
            LuaOptions::default(),
        )
        .map_err(SandboxError::Init)?;

        let outgoing = Rc::new(RefCell::new(Vec::new()));
        let handlers: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>> =
            Rc::new(RefCell::new(HashMap::new()));

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
        )?;

        let scripts = manifest.shared_scripts.iter().chain(match side {
            Side::Server => manifest.server_scripts.iter(),
            Side::Client => manifest.client_scripts.iter(),
        });

        for rel in scripts {
            let abs = manifest.root.join(rel);
            run_script(&lua, &manifest.id, rel, &abs)?;
        }

        Ok(Self { id: manifest.id.clone(), side, lua, outgoing, handlers })
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

// ---------------------------------------------------------------------------
// Runtime API installer
// ---------------------------------------------------------------------------

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
) -> Result<(), SandboxError> {
    install_runtime_api_inner(lua, id, side, outgoing, handlers, cmd_queue, local_bus, model_cmds, raycast)
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

    Ok(())
}
