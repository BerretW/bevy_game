//! `core_shared` — společná logika mezi serverem a klientem.
//!
//! Drží *pouze* věci, které musí znát obě strany sítě:
//! definice komponent pro replikaci, společné eventy, příprava
//! `lightyear` networking konfigurace a Lua event bus interface.
//!
//! Herní logika do tohoto crate **nepatří** — ta žije v Lua resources.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Kořenový plugin, který se přidává **jak na serveru, tak na klientu**.
///
/// V této fázi pouze inicializuje sdílené eventy a registry.
/// Konkrétní `lightyear` `ClientPlugin` / `ServerPlugin` se přidávají
/// až v `host_client` / `host_server`, protože mají rozdílnou konfiguraci.
pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<LuaEvent>()
            .init_resource::<LuaEventRegistry>();
    }
}

/// Univerzální payload pro Rust ↔ Lua most.
///
/// Rust core zachytává síťové / ECS události a překládá je
/// na `LuaEvent`, který Lua side handler může odebírat hookem
/// jako `RegisterEvent("onPlayerJoin", fn)`. Naopak Lua může
/// `TriggerServerEvent` převést také na `LuaEvent`.
///
/// Payload držíme jako pre-serialized bytes, aby Rust core
/// nepotřeboval znát konkrétní tvar dat — to ví jen Lua resource.
#[derive(Event, Debug, Clone, Serialize, Deserialize)]
pub struct LuaEvent {
    /// Jméno eventu, např. `"onPlayerJoin"` nebo `"core_inventory:itemUsed"`.
    pub name: String,
    /// Volitelný cíl: `None` = broadcast, `Some(id)` = konkrétní entity / player.
    pub target: Option<u64>,
    /// Sériové argumenty (typicky MessagePack / JSON, dohoda mezi resources).
    pub payload: Vec<u8>,
}

/// Registry, která drží mapování `event_name -> seznam Lua handlerů`.
///
/// Pro teď prázdný kontejner — Phase 3 implementace ho naplní,
/// až bude k dispozici Lua sandbox runtime.
#[derive(Resource, Default)]
pub struct LuaEventRegistry {
    handlers: Vec<LuaHandlerEntry>,
}

impl LuaEventRegistry {
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }
}

/// Placeholder záznam — `script_id` bude jednou opaque ID
/// z `bevy_mod_scripting` runtime, `event_name` se shoduje s `LuaEvent::name`.
#[allow(dead_code)] // doplníme v Phase 3, až napojíme Lua sandbox
#[derive(Debug, Clone)]
struct LuaHandlerEntry {
    pub event_name: String,
    pub script_id: u64,
}

