//! Phase 3.4 — Global Model Registry.
//!
//! Skenuje `stream/` složky v každém resource a buduje slovník
//! `model_name → absolute_path`. Ref-counting drží model požadovaný
//! z Lua přes `Engine.RequestModel` / `Engine.SetModelAsNoLongerNeeded`.
//!
//! VFS scan modely registruje; `Engine.RequestModel` spouští async preload přes
//! `AssetServer::load_untyped` a `Engine.HasModelLoaded` vrací `true` až po
//! dokončení loadu (na serveru bez AssetServer fallback na okamžité `true`).
//!
//! Lua API (dostupné přes `sandbox.rs`):
//! ```lua
//! Engine.RequestModel("prop_barrel")          -- zvýší ref count
//! Engine.HasModelLoaded("prop_barrel")        -- bool, zda model existuje
//! Engine.SetModelAsNoLongerNeeded("prop_barrel") -- sníží ref count
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::asset::{LoadState, LoadedUntypedAsset};

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
    /// Handle na async preload (AssetServer::load_untyped).
    pub preload: Option<Handle<LoadedUntypedAsset>>,
    /// True pokud je model kompletně nahraný v AssetServer pipeline.
    pub loaded: bool,
}

/// Globální registry modelů. Builduje se z VFS scan a udržuje ref-counting.
#[derive(Resource, Default, Debug, Clone)]
pub struct ModelRegistry {
    models: Arc<Mutex<HashMap<String, ModelEntry>>>,
}

impl ModelRegistry {
    /// Registruje model ze scan výsledku. Konflikt = první vyhraje (+ warning).
    pub fn register(&self, name: String, path: PathBuf) {
        let mut models = self.models.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = models.get(&name) {
            warn!(
                "[model_registry] conflict: '{}' already registered at {:?}, ignoring {:?}",
                name,
                existing.path,
                path
            );
            return;
        }
        models.insert(name, ModelEntry { path, ref_count: 0, native: false, preload: None, loaded: false });
    }

    /// Zvýší ref count pro daný model. Vrací `true`, pokud model existuje.
    pub fn request(&self, name: &str, asset_server: Option<&AssetServer>) -> bool {
        let mut models = self.models.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = models.get_mut(name) {
            entry.ref_count += 1;
            if !entry.loaded && entry.preload.is_none() {
                if let Some(asset_server) = asset_server {
                    let bevy_path = entry.path.to_string_lossy().replace('\\', "/");
                    entry.preload = Some(asset_server.load_untyped(bevy_path));
                } else {
                    // Server/headless fallback: bez AssetServer považujeme model za dostupný.
                    entry.loaded = true;
                }
            }
            debug!("[model_registry] request '{}' → ref_count={}", name, entry.ref_count);
            true
        } else {
            warn!("[model_registry] request '{}' — model not found", name);
            false
        }
    }

    /// Vrací `true`, pokud model je registrován (existuje na disku).
    pub fn has_loaded(&self, name: &str) -> bool {
        self.models
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .map(|e| e.loaded)
            .unwrap_or(false)
    }

    /// Sníží ref count. Saturating (nespadne pod 0).
    pub fn release(&self, name: &str) {
        let mut models = self.models.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = models.get_mut(name) {
            entry.ref_count = entry.ref_count.saturating_sub(1);
            debug!("[model_registry] release '{}' → ref_count={}", name, entry.ref_count);
        }
    }

    /// Registers a native client-side asset (e.g. from `assets/models/`)
    /// using a Bevy relative path as the model path. Does not overwrite
    /// existing VFS-scanned entries so server resources take precedence.
    pub fn register_native(&self, name: String, bevy_path: String) {
        let mut models = self.models.lock().unwrap_or_else(|p| p.into_inner());
        if models.contains_key(&name) {
            return;
        }
        models.insert(name, ModelEntry { path: PathBuf::from(bevy_path), ref_count: 0, native: true, preload: None, loaded: false });
    }

    /// Cesta na disk pro daný model (pro Phase 4 asset loading).
    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.models
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .map(|e| e.path.clone())
    }

    /// Úplný rebuild ze scan výsledků (při hot-reload VFS).
    /// Native modely (assets/models/) jsou zachovány — VFS scan je nepřepíše.
    pub fn rebuild_from_scan(&self, new_models: HashMap<String, PathBuf>) {
        let mut models = self.models.lock().unwrap_or_else(|p| p.into_inner());
        models.retain(|_, entry| entry.native);
        for (name, path) in new_models {
            models.entry(name).or_insert(ModelEntry {
                path,
                ref_count: 0,
                native: false,
                preload: None,
                loaded: false,
            });
        }
        info!("[model_registry] rebuilt — {} model(s) registered", models.len());
    }

    /// Poll AssetServer load state pro preloadované modely.
    pub fn refresh_load_states(&self, asset_server: &AssetServer) {
        let mut models = self.models.lock().unwrap_or_else(|p| p.into_inner());
        for (name, entry) in models.iter_mut() {
            let Some(handle) = entry.preload.as_ref() else { continue };
            let Some(state) = asset_server.get_load_state(handle.id()) else { continue };
            match state {
                LoadState::Loaded => {
                    if !entry.loaded {
                        entry.loaded = true;
                        debug!("[model_registry] '{}' preload finished", name);
                    }
                }
                LoadState::Failed(_) => {
                    entry.loaded = false;
                    entry.preload = None;
                    warn!("[model_registry] '{}' preload failed", name);
                }
                LoadState::Loading | LoadState::NotLoaded => {}
            }
        }
    }

    pub fn count(&self) -> usize {
        self.models.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Snapshot všech registrovaných modelů pro systémy, které potřebují
    /// iterovat přes model -> path mapu (např. discovery animací).
    pub fn entries(&self) -> Vec<(String, PathBuf)> {
        self.models
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(name, entry)| (name.clone(), entry.path.clone()))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// ModelAnimationRegistry — clip metadata cache pro Lua Engine API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ModelAnimationInfo {
    pub clip_names: Vec<String>,
}

impl ModelAnimationInfo {
    pub fn clip_count(&self) -> usize {
        self.clip_names.len()
    }
}

/// Animation Dictionary info — seznam dostupných slovníků a jejich klipů.
#[derive(Debug, Clone, Default)]
pub struct ModelAnimationDictionary {
    pub name: String,
    pub clip_names: Vec<String>,  // Názvy klipů v tomto slovníku
}

/// Metadata o animation dictionaries pro model.
#[derive(Debug, Clone, Default)]
pub struct ModelAnimationDictionaries {
    pub dictionaries: Vec<ModelAnimationDictionary>,
}

/// Sdílená cache animačních clip metadat po modelech.
/// Naplňuje ji klientský runtime (host_client) z ADM/GLTF assetů.
#[derive(Resource, Default, Debug, Clone)]
pub struct ModelAnimationRegistry {
    animations: Arc<Mutex<HashMap<String, ModelAnimationInfo>>>,
    dictionaries: Arc<Mutex<HashMap<String, ModelAnimationDictionaries>>>,
}

impl ModelAnimationRegistry {
    pub fn set_clip_names(&self, model_name: &str, clip_names: Vec<String>) {
        self.animations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(model_name.to_string(), ModelAnimationInfo { clip_names });
    }

    pub fn get_clip_names(&self, model_name: &str) -> Vec<String> {
        self.animations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(model_name)
            .map(|m| m.clip_names.clone())
            .unwrap_or_default()
    }

    pub fn get_clip_count(&self, model_name: &str) -> usize {
        self.animations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(model_name)
            .map(|m| m.clip_count())
            .unwrap_or(0)
    }

    /// Nastaví animation dictionaries pro model.
    pub fn set_animation_dictionaries(&self, model_name: &str, dictionaries: Vec<ModelAnimationDictionary>) {
        self.dictionaries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(model_name.to_string(), ModelAnimationDictionaries { dictionaries });
    }

    /// Vrátí seznam dostupných animation dictionary názvů pro model.
    pub fn get_dictionary_names(&self, model_name: &str) -> Vec<String> {
        self.dictionaries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(model_name)
            .map(|m| m.dictionaries.iter().map(|d| d.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Vrátí seznam clipů v daném dictionary pro model.
    pub fn get_dictionary_clips(&self, model_name: &str, dict_name: &str) -> Vec<String> {
        self.dictionaries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(model_name)
            .and_then(|m| m.dictionaries.iter().find(|d| d.name == dict_name))
            .map(|d| d.clip_names.clone())
            .unwrap_or_default()
    }

    /// Zahodí metadata pro modely, které už nejsou v registru.
    pub fn retain_models(&self, keep: &HashSet<String>) {
        self.animations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|name, _| keep.contains(name));
        self.dictionaries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|name, _| keep.contains(name));
    }
}

// ---------------------------------------------------------------------------
// AnimSetRegistry — samostatné animace pro late binding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnimSetEntry {
    pub path: PathBuf,
    pub ref_count: usize,
    pub native: bool,
    pub preload: Option<Handle<LoadedUntypedAsset>>,
    pub loaded: bool,
}

#[derive(Resource, Default, Debug, Clone)]
pub struct AnimSetRegistry {
    sets: Arc<Mutex<HashMap<String, AnimSetEntry>>>,
}

fn normalize_anim_set_path(path: &str) -> String {
    if path.ends_with(".ads_anim") {
        path.to_string()
    } else {
        format!("{path}.ads_anim")
    }
}

impl AnimSetRegistry {
    pub fn register(&self, name: String, path: PathBuf) {
        let mut sets = self.sets.lock().unwrap_or_else(|p| p.into_inner());
        if sets.contains_key(&name) {
            return;
        }
        sets.insert(name, AnimSetEntry { path, ref_count: 0, native: false, preload: None, loaded: false });
    }

    pub fn request(&self, name: &str, asset_server: Option<&AssetServer>) -> bool {
        let mut sets = self.sets.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = sets.get_mut(name) {
            entry.ref_count += 1;
            if !entry.loaded && entry.preload.is_none() {
                if let Some(asset_server) = asset_server {
                    let bevy_path = normalize_anim_set_path(&entry.path.to_string_lossy().replace('\\', "/"));
                    entry.preload = Some(asset_server.load_untyped(bevy_path));
                } else {
                    entry.loaded = true;
                }
            }
            true
        } else {
            warn!("[anim_set_registry] request '{}' — anim set not found", name);
            false
        }
    }

    pub fn has_loaded(&self, name: &str) -> bool {
        self.sets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .map(|entry| entry.loaded)
            .unwrap_or(false)
    }

    pub fn release(&self, name: &str) {
        let mut sets = self.sets.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = sets.get_mut(name) {
            entry.ref_count = entry.ref_count.saturating_sub(1);
        }
    }

    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.sets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .map(|entry| entry.path.clone())
    }

    pub fn rebuild_from_scan(&self, new_sets: HashMap<String, PathBuf>) {
        let mut sets = self.sets.lock().unwrap_or_else(|p| p.into_inner());
        sets.retain(|_, entry| entry.native);
        for (name, path) in new_sets {
            sets.entry(name).or_insert(AnimSetEntry {
                path,
                ref_count: 0,
                native: false,
                preload: None,
                loaded: false,
            });
        }
    }

    pub fn refresh_load_states(&self, asset_server: &AssetServer) {
        let mut sets = self.sets.lock().unwrap_or_else(|p| p.into_inner());
        for (name, entry) in sets.iter_mut() {
            let Some(handle) = entry.preload.as_ref() else { continue };
            let Some(state) = asset_server.get_load_state(handle.id()) else { continue };
            match state {
                LoadState::Loaded => {
                    entry.loaded = true;
                }
                LoadState::Failed(_) => {
                    entry.loaded = false;
                    entry.preload = None;
                    warn!("[anim_set_registry] '{}' preload failed", name);
                }
                LoadState::Loading | LoadState::NotLoaded => {}
            }
        }
    }
}

#[derive(Debug)]
pub enum AnimSetCommand {
    Request(String),
    Release(String),
}

#[derive(Clone, Resource, Default)]
pub struct AnimSetCommandQueue(pub Arc<Mutex<Vec<AnimSetCommand>>>);

impl AnimSetCommandQueue {
    pub fn push(&self, cmd: AnimSetCommand) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(cmd);
    }

    pub fn drain(&self) -> Vec<AnimSetCommand> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
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
    registry: Res<ModelRegistry>,
    asset_server: Option<Res<AssetServer>>,
) {
    let asset_server = asset_server.as_deref();
    for cmd in queue.drain() {
        match cmd {
            ModelCommand::Request(name) => {
                registry.request(&name, asset_server);
            }
            ModelCommand::Release(name) => {
                registry.release(&name);
            }
        }
    }
}

/// Zpracuje všechny pending `AnimSetCommand`y z Lua sandboxů.
pub fn process_anim_set_commands(
    queue: Res<AnimSetCommandQueue>,
    registry: Res<AnimSetRegistry>,
    asset_server: Option<Res<AssetServer>>,
) {
    let asset_server = asset_server.as_deref();
    for cmd in queue.drain() {
        match cmd {
            AnimSetCommand::Request(name) => {
                registry.request(&name, asset_server);
            }
            AnimSetCommand::Release(name) => {
                registry.release(&name);
            }
        }
    }
}

/// Aktualizuje stav preloadovaných anim-setů podle AssetServer `LoadState`.
pub fn refresh_anim_set_load_states(
    registry: Res<AnimSetRegistry>,
    asset_server: Option<Res<AssetServer>>,
) {
    let Some(asset_server) = asset_server else { return };
    registry.refresh_load_states(asset_server.as_ref());
}

/// Aktualizuje stav preloadovaných modelů podle AssetServer `LoadState`.
pub fn refresh_model_load_states(
    registry: Res<ModelRegistry>,
    asset_server: Option<Res<AssetServer>>,
) {
    let Some(asset_server) = asset_server else { return };
    registry.refresh_load_states(asset_server.as_ref());
}
