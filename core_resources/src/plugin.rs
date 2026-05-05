//! `ResourcesPlugin` — Bevy plugin, který drží VFS, watcher a sandbox registry.

use std::collections::HashMap;
use std::path::PathBuf;

use bevy::prelude::*;

use crate::resolver::resolve_load_order;
use crate::sandbox::LuaSandbox;
use crate::types::{ResourceId, Side};
use crate::vfs::Vfs;
use crate::watcher::{drain_watcher, ResourcesDirty, VfsWatcher};

/// Konfigurace plug-inu — předá se z `host_server` / `host_client`.
#[derive(Clone)]
pub struct ResourcesPlugin {
    pub root: PathBuf,
    pub side: Side,
    /// Když true, plugin spustí `notify` watcher a hot-reloaduje při změnách.
    /// V produkci na serveru chceme typicky false, v dev/playtest true.
    pub watch: bool,
}

impl ResourcesPlugin {
    pub fn new(root: impl Into<PathBuf>, side: Side) -> Self {
        Self {
            root: root.into(),
            side,
            watch: cfg!(debug_assertions),
        }
    }

    pub fn with_watch(mut self, watch: bool) -> Self {
        self.watch = watch;
        self
    }
}

/// NonSend kontejner Lua sandboxů. Lua je `!Send`, takže musíme držet
/// na main threadu — Bevy `NonSend` resource to zařídí.
#[derive(Default)]
pub struct SandboxRegistry {
    pub sandboxes: HashMap<ResourceId, LuaSandbox>,
    /// Pořadí, ve kterém byly sandboxy nahrané (dep-resolved load order).
    pub order: Vec<ResourceId>,
}

/// `Side` jako Bevy `Resource`, aby ho viděly všechny systémy.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ResourcesSide(pub Side);

impl Plugin for ResourcesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Vfs::new(&self.root));
        app.insert_resource(ResourcesSide(self.side));
        app.insert_non_send_resource(SandboxRegistry::default());
        app.add_event::<ResourcesDirty>();

        // Initial load běží jednou v Startup.
        app.add_systems(Startup, initial_load);

        if self.watch {
            match VfsWatcher::start(self.root.clone()) {
                Ok(watcher) => {
                    app.insert_resource(watcher);
                    app.add_systems(Update, (drain_watcher, hot_reload_on_dirty).chain());
                    info!(
                        "[core_resources] watcher started for {}",
                        self.root.display()
                    );
                }
                Err(e) => {
                    error!(
                        "[core_resources] watcher disabled — failed to start: {e}; \
                         hot-reload will not work for this run"
                    );
                }
            }
        }
    }
}

fn initial_load(
    mut vfs: ResMut<Vfs>,
    side: Res<ResourcesSide>,
    mut registry: NonSendMut<SandboxRegistry>,
) {
    rebuild(&mut vfs, side.0, &mut registry);
}

fn hot_reload_on_dirty(
    mut events: EventReader<ResourcesDirty>,
    mut vfs: ResMut<Vfs>,
    side: Res<ResourcesSide>,
    mut registry: NonSendMut<SandboxRegistry>,
) {
    if events.is_empty() {
        return;
    }
    // Drain všechny pending eventy — chceme jen jeden rebuild za frame
    // bez ohledu na to, kolik souborů se najednou změnilo.
    let count = events.read().count();
    info!(
        "[core_resources] filesystem change detected ({count} event{}), reloading resources",
        if count == 1 { "" } else { "s" }
    );
    rebuild(&mut vfs, side.0, &mut registry);
}

fn rebuild(vfs: &mut Vfs, side: Side, registry: &mut SandboxRegistry) {
    // 1. Rescan VFS
    let report = vfs.rescan();
    for err in &report.errors {
        error!("[core_resources] scan error: {err}");
    }
    info!(
        "[core_resources] scanned {} resource(s) from {}",
        report.loaded.len(),
        vfs.root().display()
    );

    // 2. Resolve dependencies → load order
    let order = match resolve_load_order(vfs.manifests()) {
        Ok(o) => o,
        Err(e) => {
            error!("[core_resources] dependency resolution failed: {e}");
            // Necháme staré sandboxy běžet, neshodíme proces.
            return;
        }
    };

    // 3. Drop staré sandboxy a postav nové.
    //    (Phase 1 = full rebuild. Phase 2 si dovolíme inkrementální.)
    registry.sandboxes.clear();
    registry.order.clear();

    for id in &order {
        let manifest = match vfs.get(id) {
            Some(m) => m,
            None => continue,
        };
        match LuaSandbox::create(manifest, side) {
            Ok(sandbox) => {
                debug!("[core_resources] sandbox ready: {}", id);
                registry.sandboxes.insert(id.clone(), sandbox);
                registry.order.push(id.clone());
            }
            Err(e) => {
                error!("[core_resources] sandbox failed for {id}: {e}");
            }
        }
    }

    info!(
        "[core_resources] {} sandbox(es) active on {} (order: {:?})",
        registry.sandboxes.len(),
        side.label(),
        registry.order
    );
}
