//! Per-resource Lua sandbox.
//!
//! Každý resource dostane vlastní `mlua::Lua` instanci, která žije po dobu
//! existence resource v běžícím procesu. Izolace == oddělená VM = oddělené globals,
//! oddělený GC. Mezi-resource komunikace má jít výhradně přes Bevy event bus
//! (`LuaEvent` v `core_shared`), nikdy ne přes sdílené Lua state.
//!
//! Sandbox pre-loaduje:
//! 1. všechny `shared_scripts` z manifestu (na obou stranách),
//! 2. side-specific (`server_scripts` na serveru, `client_scripts` na klientu).
//!
//! Stdlib je omezená — žádné `io`, `os`, `package`, `require`, `debug`.
//! Z DSL hlediska je to "trusted" sandbox (resources jsou součást gameplay,
//! ne user-supplied), takže `string`/`table`/`math`/`utf8`/`coroutine` necháváme.
//!
//! Phase 2 přidává **funkční** RPC API:
//! * `RegisterEvent(name, fn)` — uloží Lua callback do `mlua::Registry` a
//!   v `dispatch_incoming` se zavolá při doručení `LuaEventMessage`.
//! * `TriggerServerEvent(name, payload)` (jen na klientu) a
//!   `TriggerClientEvent(name, target, payload)` (jen na serveru) zapíše
//!   událost do shared `outgoing` queue, kterou `core_net::lua_rpc` bridge
//!   konzumuje a posílá přes lightyear.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use mlua::{Lua, LuaOptions, MultiValue, RegistryKey, StdLib};

use crate::cmd_queue::{CommandQueue, LuaCommand};
use crate::manifest::Manifest;
use crate::types::{ResourceId, Side};

/// Směr Lua-originated network eventu (komu se má doručit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuaEventDirection {
    /// Klient → server (`TriggerServerEvent`).
    ToServer,
    /// Server → klient (`TriggerClientEvent`). `target = None` ⇒ broadcast.
    ToClient,
}

/// Pending outgoing event z Lua sandboxu — bridge plugin (`core_net::lua_rpc`)
/// si ho v `Update` vyzvedne a pošle přes lightyear `LuaEventMessage`.
#[derive(Debug, Clone)]
pub struct LuaEventOut {
    pub direction: LuaEventDirection,
    pub name: String,
    pub target: Option<u64>,
    pub payload: Vec<u8>,
}

pub struct LuaSandbox {
    pub id: ResourceId,
    pub side: Side,
    lua: Lua,
    /// Sdílený buffer pending outgoing eventů. `Rc<RefCell<...>>` protože
    /// mlua closures musí být `'static + Fn` — vlastní kopii Rc dostanou
    /// při registraci, drainují přes `borrow_mut`.
    outgoing: Rc<RefCell<Vec<LuaEventOut>>>,
    /// Mapping `event_name -> seznam Lua function ref`. RegistryKey je
    /// stabilní reference do `mlua` registry, kterou si funkce vyzvedneme
    /// přes `Lua::registry_value::<Function>(key)`.
    handlers: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
}

impl LuaSandbox {
    /// Vytvoří sandbox a spustí všechny relevantní skripty z manifestu.
    pub fn create(manifest: &Manifest, side: Side, cmd_queue: CommandQueue) -> Result<Self, SandboxError> {
        let lua = Lua::new_with(
            StdLib::TABLE
                | StdLib::STRING
                | StdLib::MATH
                | StdLib::UTF8
                | StdLib::COROUTINE,
            LuaOptions::default(),
        )
        .map_err(SandboxError::Init)?;

        let outgoing = Rc::new(RefCell::new(Vec::new()));
        let handlers: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>> =
            Rc::new(RefCell::new(HashMap::new()));

        install_runtime_api(&lua, &manifest.id, side, &outgoing, &handlers, &cmd_queue)?;

        let scripts = manifest.shared_scripts.iter().chain(match side {
            Side::Server => manifest.server_scripts.iter(),
            Side::Client => manifest.client_scripts.iter(),
        });

        for rel in scripts {
            let abs = manifest.root.join(rel);
            run_script(&lua, &manifest.id, rel, &abs)?;
        }

        Ok(Self {
            id: manifest.id.clone(),
            side,
            lua,
            outgoing,
            handlers,
        })
    }

    /// Drain všech pending outgoing events. Volá bridge plugin v `Update`.
    pub fn drain_outgoing(&self) -> Vec<LuaEventOut> {
        std::mem::take(&mut *self.outgoing.borrow_mut())
    }

    /// Vyvolá všechny zaregistrované handlery pro daný incoming event name.
    /// Vrací počet úspěšně zavolaných handlerů.
    ///
    /// Phase 2: payload se předává jako Lua string. Phase 3 ho rozšíří
    /// na strukturovaný `LuaValue` přes JSON / MessagePack deserializaci.
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
        let payload_str = String::from_utf8_lossy(payload).into_owned();
        let mut count = 0;
        for key in keys {
            let f: mlua::Function = self.lua.registry_value(key)?;
            f.call::<()>((payload_str.clone(), sender))?;
            count += 1;
        }
        Ok(count)
    }

    /// Počet zaregistrovaných handlerů pro daný event (debug helper).
    pub fn handler_count(&self, name: &str) -> usize {
        self.handlers
            .borrow()
            .get(name)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// Vystavený přístup k `Lua` — Phase 3 přes něj zavoláme handlery
    /// při doručení `LuaEvent`.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to initialize Lua VM: {0}")]
    Init(#[source] mlua::Error),
    #[error("failed to install runtime API for {id}: {source}")]
    Api {
        id: ResourceId,
        #[source]
        source: mlua::Error,
    },
    #[error("io error reading script {script_rel} of {id}: {source}")]
    Io {
        id: ResourceId,
        script_rel: String,
        #[source]
        source: std::io::Error,
    },
    #[error("lua error in {id}/{script_rel}: {source}")]
    Lua {
        id: ResourceId,
        script_rel: String,
        #[source]
        source: mlua::Error,
    },
}

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

/// Extrahuje Vec3 z Lua tabulky — akceptuje pojmenované klíče `{x,y,z}`
/// i indexované pole `{1,2,3}`. Chybějící klíče defaultují na 0.
fn table_to_vec3(t: &mlua::Table) -> [f32; 3] {
    let get = |name: &str, idx: i32| -> f32 {
        t.get::<f32>(name)
            .or_else(|_| t.get::<f32>(idx))
            .unwrap_or(0.0)
    };
    [get("x", 1), get("y", 2), get("z", 3)]
}

fn install_runtime_api(
    lua: &Lua,
    id: &ResourceId,
    side: Side,
    outgoing: &Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    cmd_queue: &CommandQueue,
) -> Result<(), SandboxError> {
    install_runtime_api_inner(lua, id, side, outgoing, handlers, cmd_queue).map_err(|e| {
        SandboxError::Api {
            id: id.clone(),
            source: e,
        }
    })
}

fn install_runtime_api_inner(
    lua: &Lua,
    id: &ResourceId,
    side: Side,
    outgoing: &Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    cmd_queue: &CommandQueue,
) -> mlua::Result<()> {
    let globals = lua.globals();

    // `print` → bevy log na info úrovni
    let id_for_print = id.clone();
    let print = lua.create_function(move |_, args: MultiValue| {
        let mut parts = Vec::with_capacity(args.len());
        for v in args.iter() {
            parts.push(format!("{:?}", v));
        }
        bevy::log::info!("[lua:{}] {}", id_for_print, parts.join("\t"));
        Ok(())
    })?;
    globals.set("print", print)?;

    // Tři rozlišené úrovně, ať skripty mohou logovat strukturovaně.
    let id_dbg = id.clone();
    globals.set(
        "log_debug",
        lua.create_function(move |_, msg: String| {
            bevy::log::debug!("[lua:{}] {}", id_dbg, msg);
            Ok(())
        })?,
    )?;
    let id_info = id.clone();
    globals.set(
        "log_info",
        lua.create_function(move |_, msg: String| {
            bevy::log::info!("[lua:{}] {}", id_info, msg);
            Ok(())
        })?,
    )?;
    let id_warn = id.clone();
    globals.set(
        "log_warn",
        lua.create_function(move |_, msg: String| {
            bevy::log::warn!("[lua:{}] {}", id_warn, msg);
            Ok(())
        })?,
    )?;

    // Konstanty popisující kontext sandboxu — Lua si je může načíst
    // jen pro čtení, vlastní mutace se nepropagují.
    globals.set("RESOURCE_ID", id.as_str())?;
    globals.set("SIDE", side.label())?;
    globals.set("IS_SERVER", side == Side::Server)?;
    globals.set("IS_CLIENT", side == Side::Client)?;

    // RegisterEvent — uloží Lua function do registru, později ji bude moci
    // dispatch_incoming vyvolat.
    let handlers_for_reg = handlers.clone();
    globals.set(
        "RegisterEvent",
        lua.create_function(move |lua, (name, f): (String, mlua::Function)| {
            let key = lua.create_registry_value(f)?;
            handlers_for_reg
                .borrow_mut()
                .entry(name)
                .or_default()
                .push(key);
            Ok(())
        })?,
    )?;

    // TriggerEvent — Phase 3 cross-sandbox event bus.
    // Phase 2 zatím no-op s trace logem (zavolání nepadá, jen nic nedělá).
    let id_evt = id.clone();
    globals.set(
        "TriggerEvent",
        lua.create_function(move |_, (name, _args): (String, MultiValue)| {
            bevy::log::trace!(
                "[lua:{}] TriggerEvent (cross-sandbox bus, Phase 3 stub): {}",
                id_evt,
                name
            );
            Ok(())
        })?,
    )?;

    // Side-specific Lua RPC. `TriggerServerEvent` jen na klientu, naopak
    // `TriggerClientEvent` jen na serveru — když skript zavolá špatnou
    // stranu, zvedáme runtime error, aby si toho dev všiml v logu.
    match side {
        Side::Client => {
            let outgoing_to_server = outgoing.clone();
            globals.set(
                "TriggerServerEvent",
                lua.create_function(
                    move |_, (name, payload): (String, Option<String>)| {
                        outgoing_to_server.borrow_mut().push(LuaEventOut {
                            direction: LuaEventDirection::ToServer,
                            name,
                            target: None,
                            payload: payload.unwrap_or_default().into_bytes(),
                        });
                        Ok(())
                    },
                )?,
            )?;
            globals.set(
                "TriggerClientEvent",
                lua.create_function(|_, _args: MultiValue| -> mlua::Result<()> {
                    Err(mlua::Error::RuntimeError(
                        "TriggerClientEvent is server-only".to_string(),
                    ))
                })?,
            )?;
        }
        Side::Server => {
            let outgoing_to_client = outgoing.clone();
            globals.set(
                "TriggerClientEvent",
                lua.create_function(
                    move |_,
                          (name, target, payload): (
                        String,
                        Option<u64>,
                        Option<String>,
                    )| {
                        outgoing_to_client.borrow_mut().push(LuaEventOut {
                            direction: LuaEventDirection::ToClient,
                            name,
                            target,
                            payload: payload.unwrap_or_default().into_bytes(),
                        });
                        Ok(())
                    },
                )?,
            )?;
            globals.set(
                "TriggerServerEvent",
                lua.create_function(|_, _args: MultiValue| -> mlua::Result<()> {
                    Err(mlua::Error::RuntimeError(
                        "TriggerServerEvent is client-only".to_string(),
                    ))
                })?,
            )?;
        }
    }

    // -----------------------------------------------------------------------
    // World namespace — Phase 3.2 Command Queue API
    // -----------------------------------------------------------------------
    //
    // Každá closure dostane vlastní Arc-klon CommandQueue (levné).
    // Rotace jsou Euler XYZ ve stupních; pos je world-space {x,y,z}.
    let world = lua.create_table()?;

    // World.SpawnLocalObject(model, pos, rot) -> handle
    let cq = cmd_queue.clone();
    world.set(
        "SpawnLocalObject",
        lua.create_function(
            move |_, (model, pos_t, rot_t): (String, mlua::Table, mlua::Table)| {
                let pos = table_to_vec3(&pos_t);
                let rot = table_to_vec3(&rot_t);
                let handle = cq.alloc_handle();
                cq.push(LuaCommand::SpawnLocalObject { handle, model, pos, rot });
                Ok(handle)
            },
        )?,
    )?;

    // World.DeleteObject(handle)
    let cq = cmd_queue.clone();
    world.set(
        "DeleteObject",
        lua.create_function(move |_, handle: u64| {
            cq.push(LuaCommand::DespawnEntity { handle });
            Ok(())
        })?,
    )?;

    // World.SetTransform(handle, pos, rot)
    let cq = cmd_queue.clone();
    world.set(
        "SetTransform",
        lua.create_function(
            move |_, (handle, pos_t, rot_t): (u64, mlua::Table, mlua::Table)| {
                let pos = table_to_vec3(&pos_t);
                let rot = table_to_vec3(&rot_t);
                cq.push(LuaCommand::SetTransform { handle, pos, rot });
                Ok(())
            },
        )?,
    )?;

    // World.ApplyDamage(target_handle, amount, source_handle?) — server only
    let cq = cmd_queue.clone();
    world.set(
        "ApplyDamage",
        lua.create_function(
            move |_,
                  (target_handle, amount, source_handle): (u64, f32, Option<u64>)| {
                if side != Side::Server {
                    return Err(mlua::Error::RuntimeError(
                        "World.ApplyDamage is server-only".to_string(),
                    ));
                }
                cq.push(LuaCommand::ApplyDamage {
                    target_handle,
                    amount,
                    source_handle,
                });
                Ok(())
            },
        )?,
    )?;

    globals.set("World", world)?;

    Ok(())
}
