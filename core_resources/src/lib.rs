//! `core_resources` — VFS, manifest parser, dependency resolver a Lua sandbox.
//!
//! Phase 1 zodpovědnosti:
//! 1. Najít všechny `manifest.lua` ve sledované složce (`/resources/`).
//! 2. Parsovat manifesty v izolovaném mini-Lua interpreteru.
//! 3. Vyřešit dependencies (topological sort, detekce cyklů a missing).
//! 4. Pro každý resource vytvořit izolovaný runtime Lua sandbox a nahrát
//!    jeho `shared_scripts` + side-specific (`server_scripts`/`client_scripts`).
//! 5. Sledovat filesystem (`notify`) a při změně provést hot-reload.

mod cmd_queue;
mod manifest;
mod plugin;
mod resolver;
mod sandbox;
mod types;
mod vfs;
mod watcher;

pub use cmd_queue::{
    CommandQueue, LocalObjectMarker, LuaCommand, LuaWorldState, PendingDamageEvent,
};
pub use manifest::{Manifest, ManifestError, ResourceKind};
pub use plugin::{ResourcesPlugin, ResourcesSide, SandboxRegistry};
pub use resolver::{resolve_load_order, ResolveError};
pub use sandbox::{LuaEventDirection, LuaEventOut, LuaSandbox, SandboxError};
pub use types::{IdError, ResourceId, Side};
pub use vfs::{ScanError, ScanReport, Vfs};
pub use watcher::{ResourcesDirty, VfsWatcher, WatcherError};
