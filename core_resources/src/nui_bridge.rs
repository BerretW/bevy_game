//! Phase 4 — NUI bridge: sdílené fronty pro Lua ↔ WebView komunikaci.
//!
//! Architektura napodobuje FiveM NUI:
//! * Každý resource s `ui_page` v manifestu dostane transparentní iframe v NUI overlay.
//! * `SendNUIMessage(data)` → iframe obdrží `window.postMessage(data, '*')`.
//! * JS v iframe volá `fetch('nui://resource_host/callback/name', {method:'POST', body:json})`
//!   → `NuiInQueue` → Lua `RegisterNUICallback` handler.
//! * `SetNUIFocus(hasFocus, hasCursor)` → overlay zachytí nebo propustí vstup.
//!
//! `NuiOutQueue` a `NuiInQueue` jsou `Arc<Mutex<>>` resources sdílené mezi
//! Lua sandboxy (NonSend, main thread) a `NuiPlugin` (system, main thread).
//! Žádný Send constraint není nutný — vše na main threadu.

use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;

use crate::types::ResourceId;

// ---------------------------------------------------------------------------
// Zprávy game → prohlížeč
// ---------------------------------------------------------------------------

/// Zpráva odeslaná Lua sandboxem do WebView.
#[derive(Debug, Clone)]
pub enum NuiOutMsg {
    /// Dispatch JSON zprávy do iframe daného resource.
    /// Voláno z `SendNUIMessage(data)` v Lua.
    Dispatch { resource_host: String, json: String },
    /// Přepne, zda NUI overlay zachytí myš / klávesnici.
    SetFocus { has_focus: bool, has_cursor: bool },
    /// Přidá iframe pro daný resource do host stránky.
    /// Automaticky posílá sandbox při `create()`, pokud manifest má `ui_page`.
    AddFrame { resource_host: String },
    /// Odstraní iframe resource (hot-reload, despawn).
    RemoveFrame { resource_host: String },
}

// ---------------------------------------------------------------------------
// Zprávy prohlížeč → game
// ---------------------------------------------------------------------------

/// Callback z JS do Lua — přichází přes POST na `nui://resource_host/callback/name`.
#[derive(Debug, Clone)]
pub struct NuiInMsg {
    pub resource_id: ResourceId,
    pub callback_name: String,
    /// JSON data z těla POST requestu (prázdné = `{}`).
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Sdílené fronty (Bevy Resources + Clone přes Arc)
// ---------------------------------------------------------------------------

/// Lua sandbox sem push-uje; `NuiPlugin` drain-uje a evaluuje JS.
#[derive(Resource, Clone, Default)]
pub struct NuiOutQueue(pub Arc<Mutex<Vec<NuiOutMsg>>>);

impl NuiOutQueue {
    pub fn push(&self, msg: NuiOutMsg) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(msg);
    }

    pub fn drain(&self) -> Vec<NuiOutMsg> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// `NuiPlugin` sem push-uje (z custom protocol handleru); `dispatch_nui_callbacks`
/// v `plugin.rs` drain-uje a routuje do příslušných Lua sandboxů.
#[derive(Resource, Clone, Default)]
pub struct NuiInQueue(pub Arc<Mutex<Vec<NuiInMsg>>>);

impl NuiInQueue {
    pub fn push(&self, msg: NuiInMsg) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(msg);
    }

    pub fn drain(&self) -> Vec<NuiInMsg> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

// ---------------------------------------------------------------------------
// URL helpers — ResourceId ↔ NUI hostname
// ---------------------------------------------------------------------------

/// Převede ResourceId na URL-safe hostname pro NUI custom protocol.
/// `example/hello` → `example__hello`
/// `core/init`     → `core__init`
pub fn resource_id_to_host(id: &ResourceId) -> String {
    id.as_str().replace('/', "__")
}

/// Zpět z NUI hostname na ResourceId string.
/// `example__hello` → `example/hello`
pub fn host_to_resource_id_str(host: &str) -> String {
    host.replace("__", "/")
}
