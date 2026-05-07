//! Phase 3.4 — Global Model Registry.
//!
//! Skenuje `stream/` složky v každém resource a buduje slovník
//! `model_name → absolute_path`. Ref-counting drží model požadovaný
//! z Lua přes `Engine.RequestModel` / `Engine.SetModelAsNoLongerNeeded`.
//!
//! VFS scan modely jen registruje; skutečné načtení do GPU patří do Phase 4+
//! (Bevy AssetServer). Registry zatím garantuje, že `Engine.HasModelLoaded`
//! vrátí `true` pro jakýkoliv model, jehož soubor existuje na disku.
//!
//! Lua API (dostupné přes `sandbox.rs`):
//! ```lua
//! Engine.RequestModel("prop_barrel")          -- zvýší ref count
//! Engine.HasModelLoaded("prop_barrel")        -- bool, zda model existuje
//! Engine.SetModelAsNoLongerNeeded("prop_barrel") -- sníží ref count
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

// ---------------------------------------------------------------------------
// ModelRegistry — Bevy Resource
// ---------------------------------------------------------------------------

/// Slot pro jeden model v registru.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub path: PathBuf,
    /// Počet Lua sandboxů, které model aktuálně "drží".
    pub ref_count: usize,
    /// True = registrován přes `register_native` (assets/models/); přežije `rebuild_from_scan`.
    pub native: bool,
}

/// Globální registry modelů. Builduje se z VFS scan a udržuje ref-counting.
#[derive(Resource, Default, Debug)]
pub struct ModelRegistry {
    models: HashMap<String, ModelEntry>,
}

impl ModelRegistry {
    /// Registruje model ze scan výsledku. Konflikt = první vyhraje (+ warning).
    pub fn register(&mut self, name: String, path: PathBuf) {
        if self.models.contains_key(&name) {
            warn!(
                "[model_registry] conflict: '{}' already registered at {:?}, ignoring {:?}",
                name,
                self.models[&name].path,
                path
            );
            return;
        }
        self.models.insert(name, ModelEntry { path, ref_count: 0, native: false });
    }

    /// Zvýší ref count pro daný model. Vrací `true`, pokud model existuje.
    pub fn request(&mut self, name: &str) -> bool {
        if let Some(entry) = self.models.get_mut(name) {
            entry.ref_count += 1;
            debug!("[model_registry] request '{}' → ref_count={}", name, entry.ref_count);
            true
        } else {
            warn!("[model_registry] request '{}' — model not found", name);
            false
        }
    }

    /// Vrací `true`, pokud model je registrován (existuje na disku).
    pub fn has_loaded(&self, name: &str) -> bool {
        self.models.contains_key(name)
    }

    /// Sníží ref count. Saturating (nespadne pod 0).
    pub fn release(&mut self, name: &str) {
        if let Some(entry) = self.models.get_mut(name) {
            entry.ref_count = entry.ref_count.saturating_sub(1);
            debug!("[model_registry] release '{}' → ref_count={}", name, entry.ref_count);
        }
    }

    /// Registers a native client-side asset (e.g. from `assets/models/`)
    /// using a Bevy relative path as the model path. Does not overwrite
    /// existing VFS-scanned entries so server resources take precedence.
    pub fn register_native(&mut self, name: String, bevy_path: String) {
        if self.models.contains_key(&name) {
            return;
        }
        self.models.insert(name, ModelEntry { path: PathBuf::from(bevy_path), ref_count: 0, native: true });
    }

    /// Cesta na disk pro daný model (pro Phase 4 asset loading).
    pub fn path(&self, name: &str) -> Option<&PathBuf> {
        self.models.get(name).map(|e| &e.path)
    }

    /// Úplný rebuild ze scan výsledků (při hot-reload VFS).
    /// Native modely (assets/models/) jsou zachovány — VFS scan je nepřepíše.
    pub fn rebuild_from_scan(&mut self, new_models: HashMap<String, PathBuf>) {
        self.models.retain(|_, entry| entry.native);
        for (name, path) in new_models {
            self.models.entry(name).or_insert(ModelEntry { path, ref_count: 0, native: false });
        }
        info!("[model_registry] rebuilt — {} model(s) registered", self.models.len());
    }

    pub fn count(&self) -> usize {
        self.models.len()
    }
}

// ---------------------------------------------------------------------------
// ModelCommandQueue — thread-safe bridge pro Lua closures
// ---------------------------------------------------------------------------

/// Příkaz z Lua Engine API do Bevy systému.
#[derive(Debug)]
pub enum ModelCommand {
    Request(String),
    Release(String),
}

/// Thread-safe fronta příkazů. Každý sandbox dostane Arc-klon (levné).
#[derive(Clone, Resource, Default)]
pub struct ModelCommandQueue(pub Arc<Mutex<Vec<ModelCommand>>>);

impl ModelCommandQueue {
    pub fn push(&self, cmd: ModelCommand) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(cmd);
    }

    pub fn drain(&self) -> Vec<ModelCommand> {
        std::mem::take(
            &mut *self.0.lock().unwrap_or_else(|p| p.into_inner()),
        )
    }
}

// ---------------------------------------------------------------------------
// Bevy systém: zpracování ModelCommand z Lua
// ---------------------------------------------------------------------------

/// Zpracuje všechny pending `ModelCommand`y z Lua sandboxů.
pub fn process_model_commands(
    queue: Res<ModelCommandQueue>,
    mut registry: ResMut<ModelRegistry>,
) {
    for cmd in queue.drain() {
        match cmd {
            ModelCommand::Request(name) => {
                registry.request(&name);
            }
            ModelCommand::Release(name) => {
                registry.release(&name);
            }
        }
    }
}
