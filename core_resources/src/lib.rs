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
mod npc_brain;
mod plugin;
mod resolver;
mod sandbox;
mod types;
mod vfs;
mod watcher;

pub use cmd_queue::{
    AdsSocketMap, AnimationState, AttachedAnimSets, BlendSpaceState, CollisionEnabled, CommandQueue, EntityHandle, EntitySnapshot,
    ColliderObjectMarker, DummyColliderDef, DummyColliderShape, DummyObjectMarker, DummyPrimitiveKind,
    EnvironmentLightConfig, EnvironmentLightConfigPatch, RuntimeFogVolumeDef, RuntimeLightDef, RuntimeLightKind, StairsCollider,
    EntityStateCache, IkEnabledComponent, Inventory, LocalObjectMarker, LuaCommand, LuaMaterialOverride,
    NpcAgent, NpcAiLodConfig, NpcAiLodConfigPatch, NpcAiLodLevel, NpcAiLodState, NpcLastClientUpdate, NpcMoveGoal,
    NpcOwner, NpcPathWaypoint, NpcScenarioDef, NpcScenarioRegistry, NpcScenarioRuntimeState,
    NpcPopulationDirectorConfig, NpcPopulationDirectorConfigPatch, NpcScenarioClockConfig,
    NpcScenarioClockConfigPatch, NpcScenarioTime, NpcPopulationAssignment, NpcWanderKind, PedProfileOverride,
    ReplicatedNpcSteering, apply_replicated_npc_steering, snapshot_npc_steering,
    LuaWorldState, ModelName, LocalPlayerStats, NetworkedObjectMarker, NpcPedMarker, PendingDamageEvent,
    PlayerEntityMap, PlayerStatsCache, RootMotionState, SocketAttachment, SocketTransformSnapshot, Stats, StatsSnapshot,
    advance_npc_scenario_time, apply_replicated_npc_brain, assign_npc_owners, process_lua_commands,
    run_npc_population_director, sync_entity_state_cache, sync_npc_scenario_runtime,
};
pub use npc_brain::{
    CORE_ANIMAL_BRAIN_ID, CORE_BIRD_BRAIN_ID, CORE_FISH_BRAIN_ID, CORE_HUMAN_BRAIN_ID,
    CORE_VEHICLE_BRAIN_ID, NpcBrainDef, NpcBrainKind, NpcBrainRegistry, NpcBrainState,
    NpcBrainTarget, NpcLocomotionMode, NpcMotionDef, NpcNavigationDef, NpcPerceptionDef,
    NpcTaskKind, ReplicatedNpcBrain,
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
    clear_drawable_shader_override, get_drawable_shader_override, set_drawable_shader_override,
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
