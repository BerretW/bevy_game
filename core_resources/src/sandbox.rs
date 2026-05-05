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
//! API mostí (`TriggerEvent`, `Database.execute`, …) sem v Phase 1 doplníme
//! jen jako stuby, které logují volání — skutečné mostíky vyrobí Phase 3.

use std::path::Path;

use mlua::{Lua, LuaOptions, MultiValue, StdLib};

use crate::manifest::Manifest;
use crate::types::{ResourceId, Side};

pub struct LuaSandbox {
    pub id: ResourceId,
    pub side: Side,
    lua: Lua,
}

impl LuaSandbox {
    /// Vytvoří sandbox a spustí všechny relevantní skripty z manifestu.
    pub fn create(manifest: &Manifest, side: Side) -> Result<Self, SandboxError> {
        let lua = Lua::new_with(
            StdLib::TABLE
                | StdLib::STRING
                | StdLib::MATH
                | StdLib::UTF8
                | StdLib::COROUTINE,
            LuaOptions::default(),
        )
        .map_err(SandboxError::Init)?;

        install_runtime_api(&lua, &manifest.id, side)?;

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
        })
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

/// Nainstaluje minimální runtime API, které Lua skripty očekávají.
/// V Phase 1 to jsou stuby; Phase 3 je nahradí skutečnými ECS bridges.
fn install_runtime_api(lua: &Lua, id: &ResourceId, side: Side) -> Result<(), SandboxError> {
    install_runtime_api_inner(lua, id, side).map_err(|e| SandboxError::Api {
        id: id.clone(),
        source: e,
    })
}

fn install_runtime_api_inner(lua: &Lua, id: &ResourceId, side: Side) -> mlua::Result<()> {
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

    // Phase 3 stuby — neházejí, jen logují, aby skripty napsané proti finálnímu API
    // i v Phase 1 nepadaly.
    let id_evt = id.clone();
    globals.set(
        "TriggerEvent",
        lua.create_function(move |_, (name, _args): (String, MultiValue)| {
            bevy::log::trace!("[lua:{}] TriggerEvent stub: {}", id_evt, name);
            Ok(())
        })?,
    )?;
    let id_evt2 = id.clone();
    globals.set(
        "RegisterEvent",
        lua.create_function(move |_, (name, _handler): (String, mlua::Function)| {
            bevy::log::trace!("[lua:{}] RegisterEvent stub: {}", id_evt2, name);
            Ok(())
        })?,
    )?;

    Ok(())
}
