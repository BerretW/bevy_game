//! `core_shared` — společná logika mezi serverem a klientem.
//!
//! Drží *pouze* věci, které musí znát obě strany sítě:
//! definice komponent pro replikaci, společné eventy, příprava
//! `lightyear` networking konfigurace a Lua event bus interface.
//!
//! Herní logika do tohoto crate **nepatří** — ta žije v Lua resources.

use bevy::math::{Quat, Vec3};
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
        app.add_message::<LuaEvent>()
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
///
/// V Bevy 0.18 jsou buffered eventy přejmenované na "messages"; držíme
/// `LuaEvent` jako jméno typu (smysluplné z herního pohledu), ale
/// derivujeme `Message` (Bevy 0.18 trait pro `MessageReader`/`MessageWriter`).
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// Phase 3 — replicated game-state components
// ---------------------------------------------------------------------------
//
// Komponenty, které lightyear replikuje server → klient. Záměrně držíme
// úzké, sériově-deterministické tvary (ne plný `Transform` z Bevy s globální
// matrixí a scale), aby:
//
// * client-side prediction měl deterministickou simulaci (rollback potřebuje
//   bit-shodný state na obou stranách),
// * delta compression byla efektivní (málo bitů za zprávu),
// * `Reflect` derive nebyl povinný (lightyear bere `Component + Serde`).

/// Minimální transform pro replikaci. `translation` je world-space pozice,
/// `rotation` je orientace; scale držíme zvlášť (typicky se nemění a netřeba
/// posílat každý tick).
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NetTransform {
    pub translation: Vec3,
    pub rotation: Quat,
}

impl Default for NetTransform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

/// Lineární rychlost pro deterministickou simulaci pohybu.
/// Phase 3 simulace: `translation += velocity.0 * dt` v `FixedUpdate`.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct NetVelocity(pub Vec3);

/// Marker pro entitu reprezentující hráče. `client_id` odpovídá netcode
/// `client_id` z `Authentication::Manual` — server tak ví, který input
/// patří ke které autoritativní entitě.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerMarker {
    pub client_id: u64,
}

/// Zdraví entity. Server-autoritativní.
/// Sdíleno přes core_shared pro použití v core_resources entity cache.
#[derive(Component, Debug, Clone)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Default for Health {
    fn default() -> Self {
        Self { current: 100.0, max: 100.0 }
    }
}

impl Health {
    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }
}

