//! `core_resources` — VFS, manifest parser, dependency resolver a Lua sandbox.
//!
//! Phase 1 zodpovědnosti:
//! 1. Najít všechny `manifest.lua` ve sledované složce (`/resources/`).
//! 2. Parsovat manifesty v izolovaném mini-Lua interpreteru.
//! 3. Vyřešit dependencies (topological sort, detekce cyklů a missing).
//! 4. Pro každý resource vytvořit izolovaný runtime Lua sandbox a nahrát
//!    jeho `shared_scripts` + side-specific (`server_scripts`/`client_scripts`).
//! 5. Sledovat filesystem (`notify`) a při změně provést hot-reload.

mod ace;
mod cmd_queue;
mod db_bridge;
pub mod gui;
mod manifest;
mod model_registry;
mod plugin;
mod resolver;
mod sandbox;
mod types;
mod vfs;
mod watcher;

pub use cmd_queue::{
    AdsSocketMap, AnimationState, AttachedAnimSets, BlendSpaceState, CollisionEnabled, CommandQueue, EntityHandle, EntitySnapshot,
    EntityStateCache, Inventory, LocalObjectMarker, LuaCommand, LuaMaterialOverride,
    LuaWorldState, ModelName, LocalPlayerStats, NetworkedObjectMarker, PendingDamageEvent,
    PlayerEntityMap, PlayerStatsCache, SocketAttachment, SocketTransformSnapshot, Stats, StatsSnapshot,
    process_lua_commands, sync_entity_state_cache,
};
pub use db_bridge::{
    DatabaseBridgeResource, DbBridge, DbCallbackEntry, DbCallbackQueue,
    DbExecutorTrait, DbQueryResult,
};
pub use gui::{DrawCommand, FontLoadQueue, FontLoadRequest, GuiDrawBuffer, ImageLoadQueue, ImageLoadRequest, SpriteFit};
pub use manifest::{FontDef, ImageDef, Manifest, ManifestError, ResourceKind};
pub use model_registry::{
    AnimSetCommand, AnimSetCommandQueue, AnimSetRegistry,
    ModelAnimationDictionary, ModelAnimationDictionaries, ModelAnimationInfo, ModelAnimationRegistry,
    ModelCommand, ModelCommandQueue, ModelRegistry,
    process_anim_set_commands, process_model_commands, refresh_anim_set_load_states,
    refresh_model_load_states,
};
pub use ace::AceRegistry;
pub use plugin::{ResourcesPlugin, ResourcesSide, SandboxRegistry, ServerResourceAllowlist};
pub use resolver::{resolve_load_order, ResolveError};
pub use sandbox::{
    AuthBridge, PendingAuthCredentials, PendingAuthResult,
    CameraAttachment, CameraBridge, CameraRig,
    ConnectionBridge, ConnectionInfo,
    EngineStateBridge,
    GameBridges,
    InputBridge, InputSnapshot,
    LocalEvent, LocalEventBus,
    LuaCmdDispatch, PendingCmd,
    LuaEventDirection, LuaEventOut,
    CrosshairBridge, CrosshairHit,
    LuaSandbox, RaycastBridge, SandboxError,
};
pub use types::{IdError, ResourceId, Side};
pub use vfs::{ScanError, ScanReport, Vfs};
pub use watcher::{ResourcesDirty, VfsWatcher, WatcherError};
