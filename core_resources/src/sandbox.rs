//! Per-resource Lua sandbox.
//!
//! Phase 3.8 přidává:
//! * Cross-sandbox `TriggerEvent` bus (LocalEventBus — Arc<Mutex>).
//! * Strukturovaný JSON payload pro všechna Trigger* API.
//! * sender player_id v handlerech.
//! Phase 3.4: Engine namespace (Model Registry).
//! Phase 3.5: World.SpawnNetworkedObject.
//! Phase 4: Database.* namespace, Player.* namespace.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use mlua::{Lua, LuaOptions, MultiValue, RegistryKey, StdLib, ThreadStatus};
use serde_json::Value as Json;

use crate::gui::{DrawCommand, GuiDrawBuffer, SpriteFit};

use bevy::math::{EulerRot, Quat};

use crate::ace::AceRegistry;
use crate::cmd_queue::{
    CommandQueue, DummyColliderDef, DummyColliderShape, DummyObjectMarker, DummyPrimitiveKind,
    EntityStateCache, LocalPlayerStats, LuaCommand, PlayerStatsCache,
};
use crate::db_bridge::{DbBridge, DbQueryResult};
use crate::manifest::Manifest;
use crate::model_registry::{
    AnimSetCommand, AnimSetCommandQueue, AnimSetRegistry,
    ModelAnimationRegistry,
    ModelCommand, ModelCommandQueue, ModelRegistry,
};
use crate::types::{ResourceId, Side};

// ---------------------------------------------------------------------------
// Raycast bridge — shared Arc pro Raycast.GetGroundPosition()
// ---------------------------------------------------------------------------

/// Sdileny buffer pro aktualni world-space pozici mysi (Y=0 rovina).
/// Klientska gameplay vrstva ho aktualizuje kazdy frame.
/// Lua sandbox cte synchronne pres Raycast.GetGroundPosition().
/// Na serveru zustava [0,0,0] — Raycast namespace existuje, ale vraci nulu.
#[derive(Resource, Clone)]
pub struct RaycastBridge(pub Arc<Mutex<[f32; 3]>>);

impl Default for RaycastBridge {
    fn default() -> Self {
        Self(Arc::new(Mutex::new([0.0_f32; 3])))
    }
}

// ---------------------------------------------------------------------------
// CrosshairBridge — entity pod středem obrazovky (world-space raycast)
// ---------------------------------------------------------------------------

/// Entita, na kterou hráč právě míří (crosshair raycast z kamery).
/// Gameplay systém ji aktualizuje každý frame přes Avian SpatialQuery.
/// Lua sandbox čte přes `Raycast.GetEntityUnderCrosshair(max_dist?)`.
/// Na serveru je vždy `None`.
#[derive(Default, Clone)]
pub struct CrosshairHit {
    pub handle: u64,
    pub distance: f32,
}

#[derive(Resource, Clone)]
pub struct CrosshairBridge(pub Arc<Mutex<Option<CrosshairHit>>>);

impl Default for CrosshairBridge {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
}

impl CrosshairBridge {
    pub fn set(&self, hit: Option<CrosshairHit>) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = hit;
    }
    pub fn get(&self) -> Option<CrosshairHit> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl RaycastBridge {
    pub fn set_pos(&self, pos: [f32; 3]) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = pos;
    }
    pub fn get_pos(&self) -> [f32; 3] {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

// ---------------------------------------------------------------------------
// CameraBridge — Lua-controlled camera rig system
// ---------------------------------------------------------------------------

/// Jak je kamera připojena ke světu.
#[derive(Debug, Clone)]
pub enum CameraAttachment {
    /// Statická pozice + volitelný lookAt bod (None = použije mouse look)
    Position { pos: [f32; 3], look_at: Option<[f32; 3]> },
    /// Sleduje entitu; look_at=true → dívá se na entitu, false → mouse look
    Entity { handle: u64, offset: [f32; 3], look_at: bool },
    /// Přichycena na kost entity (dědí transformaci kosti + offset v bone-space)
    Bone { handle: u64, bone: String, offset: [f32; 3] },
}

/// Pojmenovaná kamera spravovaná přes Lua `Camera.*` API.
#[derive(Debug, Clone)]
pub struct CameraRig {
    pub id: String,
    pub attachment: CameraAttachment,
    /// FOV ve stupních; None = použije default (60°)
    pub fov: Option<f32>,
}

#[derive(Default)]
struct CameraBridgeState {
    rigs: HashMap<String, CameraRig>,
    /// id aktivní custom kamery; None = player kamera (first/third person)
    active: Option<String>,
    /// true = first-person player kamera, false = third-person (default)
    first_person: bool,
}

/// Arc-based bridge sdílený mezi Lua sandboxy a Bevy systémem v gameplay.rs.
#[derive(Resource, Clone)]
pub struct CameraBridge(Arc<Mutex<CameraBridgeState>>);

impl Default for CameraBridge {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(CameraBridgeState::default())))
    }
}

impl CameraBridge {
    fn lock(&self) -> std::sync::MutexGuard<'_, CameraBridgeState> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn create(&self, id: String, fov: Option<f32>) {
        let mut g = self.lock();
        g.rigs.entry(id.clone()).or_insert_with(|| CameraRig {
            id: id.clone(),
            attachment: CameraAttachment::Position { pos: [0.0; 3], look_at: None },
            fov,
        });
    }

    pub fn delete(&self, id: &str) {
        let mut g = self.lock();
        g.rigs.remove(id);
        if g.active.as_deref() == Some(id) {
            g.active = None;
        }
    }

    pub fn set_active(&self, id: Option<String>) {
        self.lock().active = id;
    }

    pub fn get_active_id(&self) -> Option<String> {
        self.lock().active.clone()
    }

    pub fn get_active_rig(&self) -> Option<CameraRig> {
        let g = self.lock();
        g.active.as_ref().and_then(|id| g.rigs.get(id).cloned())
    }

    pub fn set_attachment(&self, id: &str, attachment: CameraAttachment) {
        let mut g = self.lock();
        if let Some(rig) = g.rigs.get_mut(id) {
            rig.attachment = attachment;
        }
    }

    pub fn set_fov(&self, id: &str, fov: f32) {
        let mut g = self.lock();
        if let Some(rig) = g.rigs.get_mut(id) {
            rig.fov = Some(fov);
        }
    }

    pub fn set_first_person(&self, first: bool) {
        self.lock().first_person = first;
    }

    pub fn is_first_person(&self) -> bool {
        self.lock().first_person
    }

    pub fn has_rig(&self, id: &str) -> bool {
        self.lock().rigs.contains_key(id)
    }
}

// ---------------------------------------------------------------------------
// Engine state bridge — Lua → Rust control flow (cursor lock, quit, disconnect)
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct EngineState {
    pub cursor_locked: bool,
    pub quit_requested: bool,
    pub disconnect_requested: bool,
}

#[derive(Resource, Clone)]
pub struct EngineStateBridge(pub Arc<Mutex<EngineState>>);

impl Default for EngineStateBridge {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(EngineState {
            cursor_locked: true,
            quit_requested: false,
            disconnect_requested: false,
        })))
    }
}

impl EngineStateBridge {
    pub fn cursor_locked(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).cursor_locked
    }
    pub fn set_cursor_locked(&self, locked: bool) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).cursor_locked = locked;
    }
    pub fn take_quit(&self) -> bool {
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::replace(&mut s.quit_requested, false)
    }
    pub fn take_disconnect(&self) -> bool {
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::replace(&mut s.disconnect_requested, false)
    }
    pub fn reset(&self) {
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        s.cursor_locked = true;
        s.quit_requested = false;
        s.disconnect_requested = false;
    }
}

// ---------------------------------------------------------------------------
// Input bridge — synchronous key/mouse state snapshot for Lua
// ---------------------------------------------------------------------------

/// One-frame snapshot of all key and mouse button states.
/// Updated every frame by the client gameplay system.
#[derive(Default, Clone)]
pub struct InputSnapshot {
    pub pressed: HashSet<String>,
    pub just_pressed: HashSet<String>,
    pub just_released: HashSet<String>,
    pub mouse_pressed: HashSet<String>,
    pub mouse_just_pressed: HashSet<String>,
    pub mouse_just_released: HashSet<String>,
    /// Normalized cursor position (0.0–1.0, top-left origin). (0,0) when cursor is hidden.
    pub cursor_x: f32,
    pub cursor_y: f32,
}

#[derive(Resource, Clone)]
pub struct InputBridge(pub Arc<Mutex<InputSnapshot>>);

impl Default for InputBridge {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(InputSnapshot::default())))
    }
}

impl InputBridge {
    pub fn update(&self, snap: InputSnapshot) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = snap;
    }
}

// ---------------------------------------------------------------------------
// Connection bridge — network / server info for Lua
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct ConnectionInfo {
    pub connected: bool,
    pub server_addr: String,
    pub ping_ms: u32,
    pub client_id: u64,
}

#[derive(Resource, Clone)]
pub struct ConnectionBridge(pub Arc<Mutex<ConnectionInfo>>);

impl Default for ConnectionBridge {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(ConnectionInfo::default())))
    }
}

impl ConnectionBridge {
    pub fn set(&self, info: ConnectionInfo) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = info;
    }
    pub fn set_disconnected(&self) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).connected = false;
    }
    pub fn set_ping(&self, ms: u32) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).ping_ms = ms;
    }
}

// ---------------------------------------------------------------------------
// GameBridges — single Resource bundling all Arc bridges
// ---------------------------------------------------------------------------

/// All Arc-based bridges in one Bevy Resource.
/// Using a single `Res<GameBridges>` instead of separate `Res<T>` params
/// keeps system function param counts well under Bevy's 16-param limit.
#[derive(Resource, Clone, Default)]
pub struct GameBridges {
    pub raycast:      RaycastBridge,
    pub engine:       EngineStateBridge,
    pub input:        InputBridge,
    pub connection:   ConnectionBridge,
    pub cmd_dispatch: LuaCmdDispatch,
    pub ace:          AceRegistry,
    pub auth:         AuthBridge,
    pub crosshair:    CrosshairBridge,
    pub camera:       CameraBridge,
}

// ---------------------------------------------------------------------------
// Auth bridge — login/register flow
// ---------------------------------------------------------------------------

/// Client-side: credentials waiting to be sent via lightyear.
#[derive(Debug, Clone)]
pub struct PendingAuthCredentials {
    /// 0 = login, 1 = register
    pub action: u8,
    pub username: String,
    /// Plaintext — sent over encrypted lightyear channel; server hashes it.
    pub password: String,
}

/// Server-side: result queued by Lua (`Auth.MarkPlayerAuthenticated` / `Auth.RejectPlayer`)
/// to be dispatched by the auth Bevy system back to the peer entity's `MessageSender`.
#[derive(Debug, Clone)]
pub struct PendingAuthResult {
    pub player_id: u64,
    pub success: bool,
    /// Permanent account ID (e.g. "user:abc123") — empty on failure.
    pub account_id: String,
    pub error: String,
}

#[derive(Resource, Clone, Default)]
pub struct AuthBridge {
    /// Server: player_ids awaiting auth — their `LuaEventMessage`s are blocked.
    pub pending: Arc<Mutex<HashSet<u64>>>,
    /// Server: results pushed by Lua, drained by Bevy auth system each frame.
    pub results: Arc<Mutex<Vec<PendingAuthResult>>>,
    /// Client: credentials waiting to be sent via lightyear.
    pub outgoing: Arc<Mutex<Vec<PendingAuthCredentials>>>,
    /// Whether the server requires authentication (set from ServerHello).
    pub required: Arc<Mutex<bool>>,
    /// Client: set to true after successful AuthResult; handshake polls this
    /// to know when to proceed with resource download.
    pub client_authenticated: Arc<Mutex<bool>>,
    /// Client: last auth error string from the server (take-once semantics).
    pub client_error: Arc<Mutex<Option<String>>>,
}

impl AuthBridge {
    pub fn is_pending(&self, player_id: u64) -> bool {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).contains(&player_id)
    }
    pub fn add_pending(&self, player_id: u64) {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(player_id);
    }
    pub fn remove_pending(&self, player_id: u64) {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&player_id);
    }
    pub fn push_result(&self, result: PendingAuthResult) {
        self.results.lock().unwrap_or_else(|p| p.into_inner()).push(result);
    }
    pub fn drain_results(&self) -> Vec<PendingAuthResult> {
        std::mem::take(&mut *self.results.lock().unwrap_or_else(|p| p.into_inner()))
    }
    pub fn push_outgoing(&self, cred: PendingAuthCredentials) {
        self.outgoing.lock().unwrap_or_else(|p| p.into_inner()).push(cred);
    }
    pub fn drain_outgoing(&self) -> Vec<PendingAuthCredentials> {
        std::mem::take(&mut *self.outgoing.lock().unwrap_or_else(|p| p.into_inner()))
    }
    pub fn is_required(&self) -> bool {
        *self.required.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn set_required(&self, req: bool) {
        *self.required.lock().unwrap_or_else(|p| p.into_inner()) = req;
    }
    /// Called by client auth system after a successful `AuthResult`.
    pub fn set_client_authenticated(&self) {
        *self.client_authenticated.lock().unwrap_or_else(|p| p.into_inner()) = true;
    }
    pub fn is_client_authenticated(&self) -> bool {
        *self.client_authenticated.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Store an error message from the server for the native login UI to display.
    pub fn set_client_error(&self, err: String) {
        *self.client_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(err);
    }
    /// Take the pending error (one-shot — returns None if already read).
    pub fn take_client_error(&self) -> Option<String> {
        self.client_error.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}

// ---------------------------------------------------------------------------
// Cross-sandbox event bus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LocalEvent {
    pub name: String,
    pub payload: Vec<u8>,
}

#[derive(Resource, Clone, Default)]
pub struct LocalEventBus(pub Arc<Mutex<Vec<LocalEvent>>>);

impl LocalEventBus {
    pub fn push(&self, name: String, payload: Vec<u8>) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(LocalEvent { name, payload });
    }

    pub fn drain(&self) -> Vec<LocalEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

// ---------------------------------------------------------------------------
// Command dispatch — console a chat příkazy z Lua
// ---------------------------------------------------------------------------

/// Jeden příkaz čekající na dispatch do Lua sandboxů.
#[derive(Debug, Clone)]
pub struct PendingCmd {
    /// Název příkazu (lowercase, bez úvodního lomítka).
    pub name: String,
    /// Pozicionalní argumenty (split podle mezer).
    pub args: Vec<String>,
    /// Zdroj: 0 = lokální konzole / server konzole; player_id = z chatu.
    pub source: u64,
    /// Původní celý vstupní řetězec (před parseováním).
    pub raw: String,
}

/// Sdílený buffer konzolových / chat příkazů.
/// `console.rs` / chat handler pushuje; Bevy systém drainuje a dispatchuje do sandboxů.
#[derive(Resource, Clone, Default)]
pub struct LuaCmdDispatch(pub Arc<Mutex<Vec<PendingCmd>>>);

impl LuaCmdDispatch {
    pub fn push(&self, cmd: PendingCmd) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(cmd);
    }
    pub fn drain(&self) -> Vec<PendingCmd> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

// ---------------------------------------------------------------------------
// LuaEventOut
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuaEventDirection {
    ToServer,
    ToClient,
}

#[derive(Debug, Clone)]
pub struct LuaEventOut {
    pub direction: LuaEventDirection,
    pub name: String,
    pub target: Option<u64>,
    pub payload: Vec<u8>,
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn lua_value_to_json(val: mlua::Value) -> Json {
    match val {
        mlua::Value::Nil => Json::Null,
        mlua::Value::Boolean(b) => Json::Bool(b),
        mlua::Value::Integer(i) => Json::Number(i.into()),
        mlua::Value::Number(f) => serde_json::Number::from_f64(f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        mlua::Value::String(s) => Json::String(s.to_str().map(|b| b.to_string()).unwrap_or_default()),
        mlua::Value::Table(t) => lua_table_to_json(t),
        _ => Json::Null,
    }
}

fn lua_table_to_json(t: mlua::Table) -> Json {
    let len = t.raw_len();
    if len > 0 {
        let arr: Vec<Json> = (1..=len)
            .filter_map(|i| t.get::<mlua::Value>(i).ok())
            .map(lua_value_to_json)
            .collect();
        if arr.len() == len as usize {
            return Json::Array(arr);
        }
    }
    let mut obj = serde_json::Map::new();
    for pair in t.pairs::<mlua::Value, mlua::Value>() {
        if let Ok((k, v)) = pair {
            let key = match &k {
                mlua::Value::String(s) => s.to_str().map(|b| b.to_string()).unwrap_or_default(),
                mlua::Value::Integer(i) => i.to_string(),
                mlua::Value::Number(f) => f.to_string(),
                _ => continue,
            };
            obj.insert(key, lua_value_to_json(v));
        }
    }
    Json::Object(obj)
}

fn json_to_lua_value(lua: &Lua, val: Json) -> mlua::Result<mlua::Value> {
    match val {
        Json::Null => Ok(mlua::Value::Nil),
        Json::Bool(b) => Ok(mlua::Value::Boolean(b)),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(mlua::Value::Number(f))
            } else {
                Ok(mlua::Value::Nil)
            }
        }
        Json::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        Json::Array(arr) => {
            let t = lua.create_table()?;
            for (i, v) in arr.into_iter().enumerate() {
                t.set(i + 1, json_to_lua_value(lua, v)?)?;
            }
            Ok(mlua::Value::Table(t))
        }
        Json::Object(obj) => {
            let t = lua.create_table()?;
            for (k, v) in obj {
                t.set(k, json_to_lua_value(lua, v)?)?;
            }
            Ok(mlua::Value::Table(t))
        }
    }
}

fn encode_payload(val: mlua::Value) -> Vec<u8> {
    match &val {
        mlua::Value::Nil => Vec::new(),
        _ => serde_json::to_vec(&lua_value_to_json(val)).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Threading
// ---------------------------------------------------------------------------

struct ThreadEntry {
    key: RegistryKey,
    wake_at_ms: u64,
}

type ThreadPool = Rc<RefCell<Vec<ThreadEntry>>>;

// ---------------------------------------------------------------------------
// LuaSandbox
// ---------------------------------------------------------------------------

pub struct LuaSandbox {
    pub id: ResourceId,
    pub side: Side,
    lua: Lua,
    outgoing: Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    /// (handler_key, restricted) — restricted=true checks ACE "command.<name>" for non-console callers
    command_handlers: Rc<RefCell<HashMap<String, Vec<(RegistryKey, bool)>>>>,
    ace: AceRegistry,
    db_callbacks: Rc<RefCell<HashMap<u64, RegistryKey>>>,
    db_counter: Rc<RefCell<u64>>,
    local_stats: Option<LocalPlayerStats>,
    thread_pool: ThreadPool,
    elapsed_ms: u64,
}

impl LuaSandbox {
    pub fn create(
        manifest: &Manifest,
        side: Side,
        cmd_queue: CommandQueue,
        local_bus: LocalEventBus,
        model_cmds: ModelCommandQueue,
        model_registry: ModelRegistry,
        model_anims: ModelAnimationRegistry,
        raycast: RaycastBridge,
        engine_state: EngineStateBridge,
        input_bridge: InputBridge,
        connection: ConnectionBridge,
        stats_cache: PlayerStatsCache,
        entity_cache: EntityStateCache,
        db_bridge: Option<DbBridge>,
        local_stats: Option<LocalPlayerStats>,
        draw_buffer: GuiDrawBuffer,
        ace_registry: AceRegistry,
        auth_bridge: AuthBridge,
        crosshair: CrosshairBridge,
        camera_bridge: CameraBridge,
        anim_set_cmds: AnimSetCommandQueue,
        anim_set_registry: AnimSetRegistry,
    ) -> Result<Self, SandboxError> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
            LuaOptions::default(),
        )
        .map_err(SandboxError::Init)?;

        let outgoing = Rc::new(RefCell::new(Vec::new()));
        let handlers: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let command_handlers: Rc<RefCell<HashMap<String, Vec<(RegistryKey, bool)>>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let db_callbacks: Rc<RefCell<HashMap<u64, RegistryKey>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let db_counter: Rc<RefCell<u64>> = Rc::new(RefCell::new(0));
        let thread_pool: ThreadPool = Rc::new(RefCell::new(Vec::new()));

        install_runtime_api(
            &lua,
            &manifest.id,
            side,
            &outgoing,
            &handlers,
            &command_handlers,
            &cmd_queue,
            &local_bus,
            &model_cmds,
            &model_registry,
            &model_anims,
            &raycast,
            &engine_state,
            &input_bridge,
            &connection,
            &stats_cache,
            &entity_cache,
            &db_bridge,
            &db_callbacks,
            &db_counter,
            &local_stats,
            &thread_pool,
            &draw_buffer,
            &ace_registry,
            &auth_bridge,
            &crosshair,
            &camera_bridge,
            &anim_set_cmds,
            &anim_set_registry,
        )?;

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
            outgoing,
            handlers,
            command_handlers,
            ace: ace_registry,
            db_callbacks,
            db_counter,
            local_stats,
            thread_pool,
            elapsed_ms: 0,
        })
    }

    pub fn drain_outgoing(&self) -> Vec<LuaEventOut> {
        std::mem::take(&mut *self.outgoing.borrow_mut())
    }

    /// Dispatch incoming event do tohoto sandboxu.
    /// payload = JSON bytes (nebo prazdny Vec pro nil).
    /// sender  = client_id kdo poslal (None pro lokalni TriggerEvent).
    pub fn dispatch_incoming(
        &self,
        name: &str,
        payload: &[u8],
        sender: Option<u64>,
    ) -> Result<usize, mlua::Error> {
        let handlers = self.handlers.borrow();
        let Some(keys) = handlers.get(name) else {
            return Ok(0);
        };

        let lua_payload: mlua::Value = if payload.is_empty() {
            mlua::Value::Nil
        } else if let Ok(json) = serde_json::from_slice::<Json>(payload) {
            json_to_lua_value(&self.lua, json)?
        } else {
            mlua::Value::String(self.lua.create_string(payload)?)
        };

        let mut count = 0;
        for key in keys.iter() {
            let f: mlua::Function = self.lua.registry_value(key)?;
            f.call::<()>((lua_payload.clone(), sender))?;
            count += 1;
        }
        Ok(count)
    }

    pub fn handler_count(&self, name: &str) -> usize {
        self.handlers.borrow().get(name).map(|v| v.len()).unwrap_or(0)
    }

    /// Dispatch konzolového / chat příkazu do tohoto sandboxu.
    /// Handler: `function(source, args, rawCommand)`.
    /// Vrátí počet volaných handlerů (0 = příkaz neznámý pro tento sandbox).
    pub fn dispatch_command(&self, cmd: &PendingCmd) -> Result<usize, mlua::Error> {
        let handlers = self.command_handlers.borrow();
        let Some(entries) = handlers.get(&cmd.name) else {
            return Ok(0);
        };
        let args_t = self.lua.create_table()?;
        for (i, arg) in cmd.args.iter().enumerate() {
            args_t.set(i + 1, self.lua.create_string(arg.as_bytes())?)?;
        }
        let mut count = 0;
        for (key, restricted) in entries.iter() {
            // source=0 is console/server — always allowed.
            // For player sources with restricted=true, check ACE "command.<name>".
            if *restricted && cmd.source != 0 {
                if !self.ace.is_player_allowed(cmd.source, &format!("command.{}", cmd.name)) {
                    bevy::log::warn!(
                        "[console] ACE denied '{}' for player {}",
                        cmd.name, cmd.source
                    );
                    continue;
                }
            }
            let f: mlua::Function = self.lua.registry_value(key)?;
            f.call::<()>((cmd.source, args_t.clone(), cmd.raw.clone()))?;
            count += 1;
        }
        Ok(count)
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Zavolá Lua callback registrovaný pro DB dotaz `callback_id`.
    pub fn invoke_db_callback(&self, callback_id: u64, result: DbQueryResult) {
        let key = self.db_callbacks.borrow_mut().remove(&callback_id);
        let Some(key) = key else { return };
        let f: mlua::Function = match self.lua.registry_value(&key) {
            Ok(f) => f,
            Err(e) => {
                bevy::log::warn!("[db:{}] registry_value error for cb {}: {}", self.id, callback_id, e);
                return;
            }
        };
        let lua_val = db_result_to_lua(&self.lua, result);
        if let Err(e) = f.call::<()>(lua_val) {
            bevy::log::warn!("[db:{}] callback {} error: {}", self.id, callback_id, e);
        }
    }
}

// ---------------------------------------------------------------------------
// Thread tick
// ---------------------------------------------------------------------------

impl LuaSandbox {
    /// Resumuje všechny thready jejichž wake_at_ms <= aktuálního elapsed_ms.
    /// Volat jednou za frame z PreUpdate systému.
    pub fn tick_threads(&mut self, delta_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        let now = self.elapsed_ms;

        let mut pool = self.thread_pool.borrow_mut();
        let mut i = 0;
        while i < pool.len() {
            if pool[i].wake_at_ms > now {
                i += 1;
                continue;
            }
            let thread = match self.lua.registry_value::<mlua::Thread>(&pool[i].key) {
                Ok(t) => t,
                Err(e) => {
                    error!("[lua:{} thread key] {}", self.id, e);
                    pool.remove(i);
                    continue;
                }
            };
            match thread.resume::<MultiValue>(()) {
                Ok(vals) => {
                    if thread.status() == ThreadStatus::Resumable {
                        let wait_ms: u64 = vals
                            .into_iter()
                            .next()
                            .and_then(|v| match v {
                                mlua::Value::Integer(n) => Some(n.max(0) as u64),
                                mlua::Value::Number(n) => Some(n.max(0.0) as u64),
                                _ => None,
                            })
                            .unwrap_or(0);
                        pool[i].wake_at_ms = now + wait_ms;
                        i += 1;
                    } else {
                        pool.remove(i);
                    }
                }
                Err(e) => {
                    error!("[lua:{} thread] {}", self.id, e);
                    pool.remove(i);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to initialize Lua VM: {0}")]
    Init(#[source] mlua::Error),
    #[error("failed to install runtime API for {id}: {source}")]
    Api { id: ResourceId, #[source] source: mlua::Error },
    #[error("io error reading script {script_rel} of {id}: {source}")]
    Io { id: ResourceId, script_rel: String, #[source] source: std::io::Error },
    #[error("lua error in {id}/{script_rel}: {source}")]
    Lua { id: ResourceId, script_rel: String, #[source] source: mlua::Error },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn table_to_vec3(t: &mlua::Table) -> [f32; 3] {
    let get = |name: &str, idx: i32| -> f32 {
        t.get::<f32>(name).or_else(|_| t.get::<f32>(idx)).unwrap_or(0.0)
    };
    [get("x", 1), get("y", 2), get("z", 3)]
}

fn parse_dummy_kind(name: &str) -> DummyPrimitiveKind {
    match name.to_ascii_lowercase().as_str() {
        "cuboid" | "kvadr" | "kvádr" => DummyPrimitiveKind::Cuboid,
        "sphere" | "koule" => DummyPrimitiveKind::Sphere,
        "cube" | "krychle" => DummyPrimitiveKind::Cube,
        "stairs" | "schody" => DummyPrimitiveKind::Stairs,
        "arch" | "oblouk" => DummyPrimitiveKind::Arch,
        _ => DummyPrimitiveKind::Cuboid,
    }
}

fn parse_dummy_collider_shape(name: &str) -> DummyColliderShape {
    match name.to_ascii_lowercase().as_str() {
        "none" | "off" => DummyColliderShape::None,
        "box" | "cuboid" => DummyColliderShape::Box,
        "sphere" => DummyColliderShape::Sphere,
        "capsule" => DummyColliderShape::Capsule,
        "cylinder" => DummyColliderShape::Cylinder,
        "auto" | _ => DummyColliderShape::Auto,
    }
}

fn table_f32(t: &mlua::Table, name: &str, default: f32) -> f32 {
    t.get::<f32>(name)
        .or_else(|_| t.get::<i64>(name).map(|v| v as f32))
        .unwrap_or(default)
}

fn table_u32(t: &mlua::Table, name: &str, default: u32) -> u32 {
    t.get::<u32>(name)
        .or_else(|_| t.get::<i64>(name).map(|v| v.max(0) as u32))
        .unwrap_or(default)
}

fn table_bool(t: &mlua::Table, name: &str, default: bool) -> bool {
    t.get::<bool>(name).unwrap_or(default)
}

fn parse_dummy_from_lua(shape: &str, params: Option<mlua::Table>) -> DummyObjectMarker {
    let mut out = DummyObjectMarker {
        kind: parse_dummy_kind(shape),
        ..Default::default()
    };

    let Some(t) = params else { return out };

    if let Ok(size_t) = t.get::<mlua::Table>("size") {
        out.size = table_to_vec3(&size_t);
    } else {
        let uniform = table_f32(&t, "size", out.size[0]);
        if uniform > 0.0 {
            out.size = [uniform, uniform, uniform];
        }
    }

    out.radius = table_f32(&t, "radius", out.radius).max(0.001);
    out.height = table_f32(&t, "height", out.height).max(0.001);
    out.steps = table_u32(&t, "steps", out.steps).max(1);
    out.segments = table_u32(&t, "segments", out.segments).max(3);

    let r = table_f32(&t, "r", out.color[0]).clamp(0.0, 1.0);
    let g = table_f32(&t, "g", out.color[1]).clamp(0.0, 1.0);
    let b = table_f32(&t, "b", out.color[2]).clamp(0.0, 1.0);
    let a = table_f32(&t, "a", out.color[3]).clamp(0.0, 1.0);
    out.color = [r, g, b, a];

    if let Ok(col_t) = t.get::<mlua::Table>("collider") {
        let mut c = out.collider;
        c.enabled = table_bool(&col_t, "enabled", c.enabled);
        c.is_static = table_bool(&col_t, "is_static", c.is_static);
        c.is_trigger = table_bool(&col_t, "is_trigger", c.is_trigger);
        c.stairs = table_bool(&col_t, "stairs", c.stairs);
        c.stairs_slope_invert = table_bool(&col_t, "stairs_slope_invert", c.stairs_slope_invert);
        c.stairs_clearance_y = table_f32(&col_t, "stairs_clearance_y", c.stairs_clearance_y).max(0.0);
        c.friction = table_f32(&col_t, "friction", c.friction).max(0.0);
        c.restitution = table_f32(&col_t, "restitution", c.restitution).max(0.0);
        c.radius = table_f32(&col_t, "radius", c.radius).max(0.001);
        c.height = table_f32(&col_t, "height", c.height).max(0.001);
        if let Ok(sz_t) = col_t.get::<mlua::Table>("size") {
            c.size = table_to_vec3(&sz_t);
        }
        if let Ok(shape_name) = col_t.get::<String>("shape") {
            c.shape = parse_dummy_collider_shape(&shape_name);
        }
        out.collider = c;
    }

    out
}

fn parse_collider_from_lua(params: Option<mlua::Table>) -> DummyColliderDef {
    let mut c = DummyColliderDef::default();
    let Some(t) = params else { return c };

    // Podporuje obě varianty:
    // 1) params = { enabled=..., shape=..., size=..., ... }
    // 2) params = { collider = { ... } }
    let source = t.get::<mlua::Table>("collider").ok().unwrap_or(t);

    c.enabled = table_bool(&source, "enabled", c.enabled);
    c.is_static = table_bool(&source, "is_static", c.is_static);
    c.is_trigger = table_bool(&source, "is_trigger", c.is_trigger);
    c.stairs = table_bool(&source, "stairs", c.stairs);
    c.stairs_slope_invert = table_bool(&source, "stairs_slope_invert", c.stairs_slope_invert);
    c.stairs_clearance_y = table_f32(&source, "stairs_clearance_y", c.stairs_clearance_y).max(0.0);
    c.friction = table_f32(&source, "friction", c.friction).max(0.0);
    c.restitution = table_f32(&source, "restitution", c.restitution).max(0.0);
    c.radius = table_f32(&source, "radius", c.radius).max(0.001);
    c.height = table_f32(&source, "height", c.height).max(0.001);

    if let Ok(sz_t) = source.get::<mlua::Table>("size") {
        c.size = table_to_vec3(&sz_t);
    }

    if let Ok(shape_name) = source.get::<String>("shape") {
        c.shape = parse_dummy_collider_shape(&shape_name);
    }

    c
}

/// Převede Lua player_id (integer, number nebo string) na u64. 
fn lua_value_to_u64(v: &mlua::Value) -> Option<u64> {
    match v {
        mlua::Value::Integer(i) if *i > 0 => Some(*i as u64),
        mlua::Value::Number(f) if *f > 0.0 => Some(*f as u64),
        mlua::Value::String(s) => s.to_str().ok().and_then(|s| s.parse::<u64>().ok()),
        _ => None,
    }
}

/// Převede Lua table parametrů (nebo nil) na Vec<Json> pro DB executor.
fn json_params(v: mlua::Value) -> Vec<Json> {
    match v {
        mlua::Value::Table(t) => {
            let len = t.raw_len();
            (1..=len)
                .filter_map(|i| t.get::<mlua::Value>(i).ok())
                .map(lua_value_to_json)
                .collect()
        }
        mlua::Value::Nil => Vec::new(),
        other => vec![lua_value_to_json(other)],
    }
}

// ---------------------------------------------------------------------------
// Runtime API installer
// ---------------------------------------------------------------------------

/// Converts DbQueryResult to Lua value.
/// RowsAffected → integer, Rows → table of row-tables, Error → (nil, error_string)
fn db_result_to_lua(lua: &Lua, result: DbQueryResult) -> mlua::Value {
    match result {
        DbQueryResult::RowsAffected(n) => mlua::Value::Integer(n as i64),
        DbQueryResult::Rows(rows) => {
            let table = match lua.create_table() {
                Ok(t) => t,
                Err(_) => return mlua::Value::Nil,
            };
            for (i, row) in rows.into_iter().enumerate() {
                let row_table = match lua.create_table() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                for (k, v) in row {
                    let lua_val = json_to_lua_value(lua, v).unwrap_or(mlua::Value::Nil);
                    let _ = row_table.set(k, lua_val);
                }
                let _ = table.set(i + 1, row_table);
            }
            mlua::Value::Table(table)
        }
        DbQueryResult::Error(e) => {
            bevy::log::warn!("[db] query error: {}", e);
            mlua::Value::Nil
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn install_runtime_api(
    lua: &Lua,
    id: &ResourceId,
    side: Side,
    outgoing: &Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    command_handlers: &Rc<RefCell<HashMap<String, Vec<(RegistryKey, bool)>>>>,
    cmd_queue: &CommandQueue,
    local_bus: &LocalEventBus,
    model_cmds: &ModelCommandQueue,
    model_registry: &ModelRegistry,
    model_anims: &ModelAnimationRegistry,
    raycast: &RaycastBridge,
    engine_state: &EngineStateBridge,
    input_bridge: &InputBridge,
    connection: &ConnectionBridge,
    stats_cache: &PlayerStatsCache,
    entity_cache: &EntityStateCache,
    db_bridge: &Option<DbBridge>,
    db_callbacks: &Rc<RefCell<HashMap<u64, RegistryKey>>>,
    db_counter: &Rc<RefCell<u64>>,
    local_stats: &Option<LocalPlayerStats>,
    thread_pool: &ThreadPool,
    draw_buffer: &GuiDrawBuffer,
    ace_registry: &AceRegistry,
    auth_bridge: &AuthBridge,
    crosshair: &CrosshairBridge,
    camera_bridge: &CameraBridge,
    anim_set_cmds: &AnimSetCommandQueue,
    anim_set_registry: &AnimSetRegistry,
) -> Result<(), SandboxError> {
    install_runtime_api_inner(lua, id, side, outgoing, handlers, command_handlers, cmd_queue, local_bus, model_cmds, model_registry, model_anims, raycast, engine_state, input_bridge, connection, stats_cache, entity_cache, db_bridge, db_callbacks, db_counter, local_stats, thread_pool, draw_buffer, ace_registry, auth_bridge, crosshair, camera_bridge, anim_set_cmds, anim_set_registry)
        .map_err(|e| SandboxError::Api { id: id.clone(), source: e })
}

#[allow(clippy::too_many_arguments)]
fn install_runtime_api_inner(
    lua: &Lua,
    id: &ResourceId,
    side: Side,
    outgoing: &Rc<RefCell<Vec<LuaEventOut>>>,
    handlers: &Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    command_handlers: &Rc<RefCell<HashMap<String, Vec<(RegistryKey, bool)>>>>,
    cmd_queue: &CommandQueue,
    local_bus: &LocalEventBus,
    model_cmds: &ModelCommandQueue,
    model_registry: &ModelRegistry,
    model_anims: &ModelAnimationRegistry,
    raycast: &RaycastBridge,
    engine_state: &EngineStateBridge,
    input_bridge: &InputBridge,
    connection: &ConnectionBridge,
    stats_cache: &PlayerStatsCache,
    entity_cache: &EntityStateCache,
    db_bridge: &Option<DbBridge>,
    db_callbacks: &Rc<RefCell<HashMap<u64, RegistryKey>>>,
    db_counter: &Rc<RefCell<u64>>,
    local_stats: &Option<LocalPlayerStats>,
    thread_pool: &ThreadPool,
    draw_buffer: &GuiDrawBuffer,
    ace_registry: &AceRegistry,
    auth_bridge: &AuthBridge,
    crosshair: &CrosshairBridge,
    camera_bridge: &CameraBridge,
    anim_set_cmds: &AnimSetCommandQueue,
    anim_set_registry: &AnimSetRegistry,
) -> mlua::Result<()> {
    let globals = lua.globals();

    // print
    let id_p = id.clone();
    globals.set("print", lua.create_function(move |_, args: MultiValue| {
        let parts: Vec<String> = args.iter().map(|v| format!("{:?}", v)).collect();
        bevy::log::info!("[lua:{}] {}", id_p, parts.join("\t"));
        Ok(())
    })?)?;

    let id_d = id.clone();
    globals.set("log_debug", lua.create_function(move |_, msg: String| {
        bevy::log::debug!("[lua:{}] {}", id_d, msg); Ok(())
    })?)?;
    let id_i = id.clone();
    globals.set("log_info", lua.create_function(move |_, msg: String| {
        bevy::log::info!("[lua:{}] {}", id_i, msg); Ok(())
    })?)?;
    let id_w = id.clone();
    globals.set("log_warn", lua.create_function(move |_, msg: String| {
        bevy::log::warn!("[lua:{}] {}", id_w, msg); Ok(())
    })?)?;

    globals.set("RESOURCE_ID", id.as_str())?;
    globals.set("SIDE", side.label())?;
    globals.set("IS_SERVER", side == Side::Server)?;
    globals.set("IS_CLIENT", side == Side::Client)?;

    // RegisterEvent
    let handlers_for_reg = handlers.clone();
    globals.set("RegisterEvent", lua.create_function(move |lua, (name, f): (String, mlua::Function)| {
        let key = lua.create_registry_value(f)?;
        handlers_for_reg.borrow_mut().entry(name).or_default().push(key);
        Ok(())
    })?)?;

    // RegisterCommand(name, handler, restricted?) — registruje konzolový / chat příkaz.
    // Handler: function(source, args, rawCommand)
    //   source:     0 = konzole/server, player_id = z chatu
    //   args:       table of strings (pozicinální argumenty)
    //   rawCommand: původní vstupní string
    // restricted:   true = vyžaduje ACE "command.<name>" pro hráče (default false)
    let cmd_handlers_for_reg = command_handlers.clone();
    globals.set("RegisterCommand", lua.create_function(move |lua, (name, f, restricted): (String, mlua::Function, Option<bool>)| {
        let key = lua.create_registry_value(f)?;
        let restricted = restricted.unwrap_or(false);
        cmd_handlers_for_reg.borrow_mut()
            .entry(name.to_lowercase())
            .or_default()
            .push((key, restricted));
        Ok(())
    })?)?;

    // TriggerEvent — cross-sandbox same-process bus (Phase 3.8)
    let bus = local_bus.clone();
    let id_e = id.clone();
    globals.set("TriggerEvent", lua.create_function(move |_, (name, payload): (String, mlua::Value)| {
        let bytes = encode_payload(payload);
        bevy::log::trace!("[lua:{}] TriggerEvent '{}'", id_e, name);
        bus.push(name, bytes);
        Ok(())
    })?)?;

    // Side-specific network RPC
    match side {
        Side::Client => {
            let out = outgoing.clone();
            globals.set("TriggerServerEvent", lua.create_function(
                move |_, (name, payload): (String, mlua::Value)| {
                    out.borrow_mut().push(LuaEventOut {
                        direction: LuaEventDirection::ToServer,
                        name,
                        target: None,
                        payload: encode_payload(payload),
                    });
                    Ok(())
                },
            )?)?;
            globals.set("TriggerClientEvent", lua.create_function(|_, _: MultiValue| -> mlua::Result<()> {
                Err(mlua::Error::RuntimeError("TriggerClientEvent is server-only".into()))
            })?)?;
        }
        Side::Server => {
            // TriggerClientEvent(name, target, payload)
            // target: nil/false = broadcast, positive integer = unicast player_id
            let out = outgoing.clone();
            globals.set("TriggerClientEvent", lua.create_function(
                move |_, (name, target, payload): (String, mlua::Value, mlua::Value)| {
                    let target_id: Option<u64> = match &target {
                        mlua::Value::Nil | mlua::Value::Boolean(false) => None,
                        mlua::Value::Integer(i) if *i > 0 => Some(*i as u64),
                        mlua::Value::Number(f) if *f > 0.0 => Some(*f as u64),
                        mlua::Value::String(s) => s
                            .to_str()
                            .ok()
                            .and_then(|v| v.parse::<u64>().ok()),
                        _ => None,
                    };
                    out.borrow_mut().push(LuaEventOut {
                        direction: LuaEventDirection::ToClient,
                        name,
                        target: target_id,
                        payload: encode_payload(payload),
                    });
                    Ok(())
                },
            )?)?;
            globals.set("TriggerServerEvent", lua.create_function(|_, _: MultiValue| -> mlua::Result<()> {
                Err(mlua::Error::RuntimeError("TriggerServerEvent is client-only".into()))
            })?)?;
        }
    }

    // World namespace
    let world = lua.create_table()?;

    let cq = cmd_queue.clone();
    world.set("SpawnLocalObject", lua.create_function(
        move |_, (model, pos_t, rot_t): (String, mlua::Table, mlua::Table)| {
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnLocalObject { handle, model, pos, rot });
            Ok(handle)
        },
    )?)?;

    // World.SpawnNetworkedObject — server-only, replikovana entita (Phase 3.5)
    let cq = cmd_queue.clone();
    world.set("SpawnNetworkedObject", lua.create_function(
        move |_, (model, pos_t, rot_t): (String, mlua::Table, mlua::Table)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError(
                    "World.SpawnNetworkedObject is server-only".into(),
                ));
            }
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnNetworkedObject { handle, model, pos, rot });
            Ok(handle)
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("SpawnLocalDummy", lua.create_function(
        move |_, (shape, params_v, pos_t, rot_t): (String, mlua::Value, mlua::Table, mlua::Table)| {
            let params = match params_v {
                mlua::Value::Table(t) => Some(t),
                _ => None,
            };
            let def = parse_dummy_from_lua(&shape, params);
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnLocalDummy { handle, def, pos, rot });
            Ok(handle)
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("SpawnNetworkedDummy", lua.create_function(
        move |_, (shape, params_v, pos_t, rot_t): (String, mlua::Value, mlua::Table, mlua::Table)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError(
                    "World.SpawnNetworkedDummy is server-only".into(),
                ));
            }
            let params = match params_v {
                mlua::Value::Table(t) => Some(t),
                _ => None,
            };
            let def = parse_dummy_from_lua(&shape, params);
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnNetworkedDummy { handle, def, pos, rot });
            Ok(handle)
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("SpawnLocalCollider", lua.create_function(
        move |_, (params_v, pos_t, rot_t): (mlua::Value, mlua::Table, mlua::Table)| {
            let params = match params_v {
                mlua::Value::Table(t) => Some(t),
                _ => None,
            };
            let collider = parse_collider_from_lua(params);
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnLocalCollider { handle, collider, pos, rot });
            Ok(handle)
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("SpawnNetworkedCollider", lua.create_function(
        move |_, (params_v, pos_t, rot_t): (mlua::Value, mlua::Table, mlua::Table)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError(
                    "World.SpawnNetworkedCollider is server-only".into(),
                ));
            }
            let params = match params_v {
                mlua::Value::Table(t) => Some(t),
                _ => None,
            };
            let collider = parse_collider_from_lua(params);
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            let handle = cq.alloc_handle();
            cq.push(LuaCommand::SpawnNetworkedCollider { handle, collider, pos, rot });
            Ok(handle)
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("AttachWithOffset", lua.create_function(
        move |_, (child_handle, parent_handle, offset_t, rot_t): (u64, u64, mlua::Table, mlua::Table)| {
            let offset = table_to_vec3(&offset_t);
            let rot = table_to_vec3(&rot_t);
            cq.push(LuaCommand::AttachWithOffset {
                child_handle,
                parent_handle,
                offset,
                rot,
            });
            Ok(())
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("DeleteObject", lua.create_function(move |_, handle: u64| {
        cq.push(LuaCommand::DespawnEntity { handle });
        Ok(())
    })?)?;

    let cq = cmd_queue.clone();
    world.set("SetTransform", lua.create_function(
        move |_, (handle, pos_t, rot_t): (u64, mlua::Table, mlua::Table)| {
            let pos = table_to_vec3(&pos_t);
            let rot = table_to_vec3(&rot_t);
            cq.push(LuaCommand::SetTransform { handle, pos, rot });
            Ok(())
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("ApplyDamage", lua.create_function(
        move |_, (target_handle, amount, source_handle): (u64, f32, Option<u64>)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("World.ApplyDamage is server-only".into()));
            }
            cq.push(LuaCommand::ApplyDamage { target_handle, amount, source_handle });
            Ok(())
        },
    )?)?;

    // -- Entity state getters (synchronous read from EntityStateCache) --------

    let ec = entity_cache.clone();
    world.set("IsValid", lua.create_function(move |_, handle: u64| {
        Ok(ec.is_valid(handle))
    })?)?;

    let ec = entity_cache.clone();
    world.set("IsAlive", lua.create_function(move |_, handle: u64| {
        Ok(ec.get(handle).map(|s| s.alive).unwrap_or(false))
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetHealth", lua.create_function(move |_, handle: u64| -> mlua::Result<mlua::Value> {
        Ok(match ec.get(handle).and_then(|s| s.health) {
            Some(v) => mlua::Value::Number(v as f64),
            None => mlua::Value::Nil,
        })
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetModel", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        Ok(match ec.get(handle).and_then(|s| s.model) {
            Some(m) => mlua::Value::String(lua.create_string(m)?),
            None => mlua::Value::Nil,
        })
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetHandlesByModel", lua.create_function(move |lua, model: String| -> mlua::Result<mlua::Table> {
        let handles = ec.handles_by_model(&model);
        let out = lua.create_table()?;
        for (idx, handle) in handles.iter().enumerate() {
            out.set(idx + 1, *handle)?;
        }
        Ok(out)
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetPosition", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let t = lua.create_table()?;
        t.set("x", snap.pos[0])?;
        t.set("y", snap.pos[1])?;
        t.set("z", snap.pos[2])?;
        Ok(mlua::Value::Table(t))
    })?)?;

    // Vrátí rotaci jako Euler XYZ ve stupních — stejný formát jako SetTransform.
    let ec = entity_cache.clone();
    world.set("GetRotation", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let q = Quat::from_array(snap.rot);
        let (ex, ey, ez) = q.to_euler(EulerRot::XYZ);
        let t = lua.create_table()?;
        t.set("x", ex.to_degrees())?;
        t.set("y", ey.to_degrees())?;
        t.set("z", ez.to_degrees())?;
        Ok(mlua::Value::Table(t))
    })?)?;

    // Vrátí rotaci jako kvaternion {x, y, z, w} pro přesné výpočty (bez gimbal locku).
    let ec = entity_cache.clone();
    world.set("GetQuaternion", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let t = lua.create_table()?;
        t.set("x", snap.rot[0])?;
        t.set("y", snap.rot[1])?;
        t.set("z", snap.rot[2])?;
        t.set("w", snap.rot[3])?;
        Ok(mlua::Value::Table(t))
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetScale", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let t = lua.create_table()?;
        t.set("x", snap.scale[0])?;
        t.set("y", snap.scale[1])?;
        t.set("z", snap.scale[2])?;
        Ok(mlua::Value::Table(t))
    })?)?;

    // Vrátí celý transform najednou: {pos={x,y,z}, rot={x,y,z}, scale={x,y,z}}
    let ec = entity_cache.clone();
    world.set("GetTransform", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        let q = Quat::from_array(snap.rot);
        let (ex, ey, ez) = q.to_euler(EulerRot::XYZ);

        let pos_t = lua.create_table()?;
        pos_t.set("x", snap.pos[0])?;
        pos_t.set("y", snap.pos[1])?;
        pos_t.set("z", snap.pos[2])?;

        let rot_t = lua.create_table()?;
        rot_t.set("x", ex.to_degrees())?;
        rot_t.set("y", ey.to_degrees())?;
        rot_t.set("z", ez.to_degrees())?;

        let scale_t = lua.create_table()?;
        scale_t.set("x", snap.scale[0])?;
        scale_t.set("y", snap.scale[1])?;
        scale_t.set("z", snap.scale[2])?;

        let t = lua.create_table()?;
        t.set("pos", pos_t)?;
        t.set("rot", rot_t)?;
        t.set("scale", scale_t)?;
        Ok(mlua::Value::Table(t))
    })?)?;

    // Vrátí world-space transform socketu: {pos={x,y,z}, rot={x,y,z,w}} nebo nil.
    let ec = entity_cache.clone();
    world.set("GetSocketTransform", lua.create_function(
        move |lua, (handle, socket_name): (u64, String)| -> mlua::Result<mlua::Value> {
            let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
            let Some(socket_tf) = snap.sockets.get(&socket_name) else {
                return Ok(mlua::Value::Nil);
            };

            let pos_t = lua.create_table()?;
            pos_t.set("x", socket_tf.pos[0])?;
            pos_t.set("y", socket_tf.pos[1])?;
            pos_t.set("z", socket_tf.pos[2])?;

            let rot_t = lua.create_table()?;
            rot_t.set("x", socket_tf.rot[0])?;
            rot_t.set("y", socket_tf.rot[1])?;
            rot_t.set("z", socket_tf.rot[2])?;
            rot_t.set("w", socket_tf.rot[3])?;

            let t = lua.create_table()?;
            t.set("pos", pos_t)?;
            t.set("rot", rot_t)?;
            Ok(mlua::Value::Table(t))
        },
    )?)?;

    let ec = entity_cache.clone();
    world.set("GetAnimation", lua.create_function(move |lua, handle: u64| -> mlua::Result<mlua::Value> {
        Ok(match ec.get(handle).and_then(|s| s.animation) {
            Some(a) => mlua::Value::String(lua.create_string(a)?),
            None => mlua::Value::Nil,
        })
    })?)?;

    let ec = entity_cache.clone();
    world.set("GetAnimationSpeed", lua.create_function(move |_, handle: u64| -> mlua::Result<f64> {
        Ok(ec.get(handle).map(|s| s.anim_speed as f64).unwrap_or(1.0))
    })?)?;

    // -- Entity state setters (queued commands) --------------------------------

    let cq = cmd_queue.clone();
    world.set("SetModel", lua.create_function(
        move |_, (handle, model): (u64, String)| {
            cq.push(LuaCommand::SetModel { handle, model });
            Ok(())
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("SetPosition", lua.create_function(
        move |_, (handle, pos_t): (u64, mlua::Table)| {
            let pos = table_to_vec3(&pos_t);
            cq.push(LuaCommand::SetPosition { handle, pos });
            Ok(())
        },
    )?)?;

    // SetRotation přijímá Euler XYZ ve stupních — stejný formát jako SetTransform.
    let cq = cmd_queue.clone();
    world.set("SetRotation", lua.create_function(
        move |_, (handle, rot_t): (u64, mlua::Table)| {
            let rot = table_to_vec3(&rot_t);
            cq.push(LuaCommand::SetRotation { handle, rot });
            Ok(())
        },
    )?)?;

    // SetScale přijímá číslo (uniform) nebo tabulku {x, y, z}.
    let cq = cmd_queue.clone();
    world.set("SetScale", lua.create_function(
        move |_, (handle, scale_v): (u64, mlua::Value)| {
            let scale = match &scale_v {
                mlua::Value::Number(f) => [*f as f32; 3],
                mlua::Value::Integer(i) => [*i as f32; 3],
                mlua::Value::Table(t) => table_to_vec3(t),
                _ => [1.0; 3],
            };
            cq.push(LuaCommand::SetScale { handle, scale });
            Ok(())
        },
    )?)?;

    // PlayAnimation podporuje 3 tvary:
    // 1) PlayAnimation(handle, name, blend_time?)
    // 2) PlayAnimation(handle, name, looping?, speed?, blend_time?)
    // 3) PlayAnimation(handle, name, looping?, speed?, blend_time?, flags?)
    let cq = cmd_queue.clone();
    world.set("PlayAnimation", lua.create_function(
        move |_, args: MultiValue| {
            if args.len() < 2 {
                return Err(mlua::Error::RuntimeError(
                    "World.PlayAnimation(handle, name, ...) requires at least 2 arguments".into(),
                ));
            }

            let handle = match &args[0] {
                mlua::Value::Integer(v) if *v >= 0 => *v as u64,
                mlua::Value::Number(v) if *v >= 0.0 => *v as u64,
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "World.PlayAnimation: invalid handle".into(),
                    ))
                }
            };
            let name = match &args[1] {
                mlua::Value::String(s) => s.to_str()?.to_string(),
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "World.PlayAnimation: invalid animation name".into(),
                    ))
                }
            };

            let mut looping = true;
            let mut speed = 1.0_f32;
            let mut blend_time = 0.0_f32;
            let mut flags = 1_u32;

            if args.len() >= 3 {
                match &args[2] {
                    mlua::Value::Boolean(v) => looping = *v,
                    mlua::Value::Integer(v) => blend_time = *v as f32,
                    mlua::Value::Number(v) => blend_time = *v as f32,
                    mlua::Value::Nil => {}
                    _ => {}
                }
            }
            if args.len() >= 4 {
                match &args[3] {
                    mlua::Value::Integer(v) => speed = *v as f32,
                    mlua::Value::Number(v) => speed = *v as f32,
                    mlua::Value::Nil => {}
                    _ => {}
                }
            }
            if args.len() >= 5 {
                match &args[4] {
                    mlua::Value::Integer(v) => blend_time = *v as f32,
                    mlua::Value::Number(v) => blend_time = *v as f32,
                    mlua::Value::Nil => {}
                    _ => {}
                }
            }
            if args.len() >= 6 {
                match &args[5] {
                    mlua::Value::Integer(v) if *v >= 0 => flags = *v as u32,
                    mlua::Value::Number(v) if *v >= 0.0 => flags = *v as u32,
                    mlua::Value::Nil => {}
                    _ => {}
                }
            }

            cq.push(LuaCommand::PlayAnimation {
                handle,
                name,
                looping,
                speed,
                blend_time,
                flags,
            });
            Ok(())
        },
    )?)?;

    let cq = cmd_queue.clone();
    world.set("ApplyAnimSet", lua.create_function(move |_, (handle, path): (u64, String)| {
        cq.push(LuaCommand::ApplyAnimSet { handle, path });
        Ok(())
    })?)?;

    let cq = cmd_queue.clone();
    world.set("StopAnimation", lua.create_function(move |_, handle: u64| {
        cq.push(LuaCommand::StopAnimation { handle });
        Ok(())
    })?)?;

    // World.PlayBlendSpace(handle, blend_space_name, move_x, move_y, speed?, flags?)
    // Přehraje blend space (míchání více klipů podle 2D vektoru pohybu).
    let cq = cmd_queue.clone();
    world.set("PlayBlendSpace", lua.create_function(
        move |_, args: MultiValue| {
            if args.len() < 4 {
                return Err(mlua::Error::RuntimeError(
                    "World.PlayBlendSpace(handle, blend_space_name, move_x, move_y, ...) requires at least 4 arguments".into(),
                ));
            }

            let handle = match &args[0] {
                mlua::Value::Integer(v) if *v >= 0 => *v as u64,
                mlua::Value::Number(v) if *v >= 0.0 => *v as u64,
                _ => return Err(mlua::Error::RuntimeError("PlayBlendSpace: invalid handle".into())),
            };
            let blend_space_name = match &args[1] {
                mlua::Value::String(s) => s.to_str()?.to_string(),
                _ => return Err(mlua::Error::RuntimeError("PlayBlendSpace: invalid blend_space_name".into())),
            };
            let move_x = match &args[2] {
                mlua::Value::Integer(v) => *v as f32,
                mlua::Value::Number(v) => *v as f32,
                _ => return Err(mlua::Error::RuntimeError("PlayBlendSpace: invalid move_x".into())),
            };
            let move_y = match &args[3] {
                mlua::Value::Integer(v) => *v as f32,
                mlua::Value::Number(v) => *v as f32,
                _ => return Err(mlua::Error::RuntimeError("PlayBlendSpace: invalid move_y".into())),
            };

            let mut speed = 1.0_f32;
            let mut flags = 1_u32;

            if args.len() >= 5 {
                match &args[4] {
                    mlua::Value::Integer(v) => speed = *v as f32,
                    mlua::Value::Number(v) => speed = *v as f32,
                    _ => {}
                }
            }
            if args.len() >= 6 {
                match &args[5] {
                    mlua::Value::Integer(v) if *v >= 0 => flags = *v as u32,
                    mlua::Value::Number(v) if *v >= 0.0 => flags = *v as u32,
                    _ => {}
                }
            }

            cq.push(LuaCommand::PlayBlendSpace {
                handle,
                blend_space_name,
                position: [move_x, move_y],
                speed,
                flags,
            });
            Ok(())
        },
    )?)?;

    // -- Phase 5 extensions ---------------------------------------------------

    // World.GetDistance(handle1, handle2) → number|nil
    // Vrátí euklidovskou vzdálenost mezi dvěma entitami v world units.
    // Vrátí nil pokud jedna nebo obě entity neexistují v cache.
    let ec = entity_cache.clone();
    world.set("GetDistance", lua.create_function(move |_, (h1, h2): (u64, u64)| -> mlua::Result<mlua::Value> {
        let (Some(s1), Some(s2)) = (ec.get(h1), ec.get(h2)) else {
            return Ok(mlua::Value::Nil);
        };
        let dx = s1.pos[0] - s2.pos[0];
        let dy = s1.pos[1] - s2.pos[1];
        let dz = s1.pos[2] - s2.pos[2];
        Ok(mlua::Value::Number(((dx * dx + dy * dy + dz * dz) as f64).sqrt()))
    })?)?;

    // World.GetNetworkId(handle) → number|nil
    // Vrátí network ID entity pokud je síťová (NetworkedObjectMarker).
    // Network ID = Lua handle (stejné číslo platí na serveru i klientovi).
    // Vrátí nil pro lokální (non-networked) entity.
    let ec = entity_cache.clone();
    world.set("GetNetworkId", lua.create_function(move |_, handle: u64| -> mlua::Result<mlua::Value> {
        let Some(snap) = ec.get(handle) else { return Ok(mlua::Value::Nil) };
        if snap.is_networked {
            Ok(mlua::Value::Integer(handle as i64))
        } else {
            Ok(mlua::Value::Nil)
        }
    })?)?;

    // World.GetHandleFromNetworkId(net_id) → number|nil
    // Přeloží network ID na Lua handle. Protože network ID == handle, jen ověří
    // existenci entity v cache. Na klientovi funguje po replicaci EntityHandle.
    let ec = entity_cache.clone();
    world.set("GetHandleFromNetworkId", lua.create_function(move |_, net_id: u64| -> mlua::Result<mlua::Value> {
        if ec.is_valid(net_id) {
            Ok(mlua::Value::Integer(net_id as i64))
        } else {
            Ok(mlua::Value::Nil)
        }
    })?)?;

    // World.SetCollisionEnabled(handle, enabled) — zapne/vypne fyzikální kolizi entity.
    // Fyzikální backend (Avian) reaguje na kolizi přes CollisionEnabled komponent.
    let cq = cmd_queue.clone();
    world.set("SetCollisionEnabled", lua.create_function(move |_, (handle, enabled): (u64, bool)| {
        cq.push(LuaCommand::SetCollisionEnabled { handle, enabled });
        Ok(())
    })?)?;

    // World.SetMaterialParam(handle, param, value) — nastaví materiálový parametr.
    // Param: "snow_level" | "dirt_level" | "wetness" (0.0–1.0).
    // Aplikováno na mesh potomky entity systémem apply_material_overrides v core_drawable.
    let cq = cmd_queue.clone();
    world.set("SetMaterialParam", lua.create_function(move |_, (handle, param, value): (u64, String, f32)| {
        cq.push(LuaCommand::SetMaterialParam { handle, param, value });
        Ok(())
    })?)?;

    // World.Attach(child_handle, child_socket, parent_handle, parent_socket)
    let cq = cmd_queue.clone();
    world.set("Attach", lua.create_function(
        move |_, (child_handle, child_socket, parent_handle, parent_socket): (u64, String, u64, String)| {
            cq.push(LuaCommand::Attach {
                child_handle,
                child_socket,
                parent_handle,
                parent_socket,
            });
            Ok(())
        },
    )?)?;

    // World.Detach(child_handle)
    let cq = cmd_queue.clone();
    world.set("Detach", lua.create_function(move |_, child_handle: u64| {
        cq.push(LuaCommand::Detach { child_handle });
        Ok(())
    })?)?;

    // World.EnableIk(handle, blend_weight?) — Phase 4.2 — zapne IK
    let cq = cmd_queue.clone();
    world.set("EnableIk", lua.create_function(
        move |_, (handle, blend_weight_opt): (u64, mlua::Value)| {
            let blend_weight = match blend_weight_opt {
                mlua::Value::Number(n) => n as f32,
                mlua::Value::Integer(i) => i as f32,
                mlua::Value::Nil => 1.0,
                _ => 1.0,
            }.clamp(0.0, 1.0);
            cq.push(LuaCommand::EnableIk { handle, blend_weight });
            Ok(())
        },
    )?)?;

    // World.DisableIk(handle) — Phase 4.2 — vypne IK
    let cq = cmd_queue.clone();
    world.set("DisableIk", lua.create_function(move |_, handle: u64| {
        cq.push(LuaCommand::DisableIk { handle });
        Ok(())
    })?)?;

    globals.set("World", world)?;

    // Engine namespace — Model Registry + Anim Set Registry
    let engine = lua.create_table()?;

    let mc = model_cmds.clone();
    engine.set("RequestModel", lua.create_function(move |_, name: String| {
        mc.push(ModelCommand::Request(name));
        Ok(())
    })?)?;

    let mr = model_registry.clone();
    engine.set("HasModelLoaded", lua.create_function(move |_, name: String| -> mlua::Result<bool> {
        Ok(mr.has_loaded(&name))
    })?)?;

    let ma = model_anims.clone();
    engine.set("GetModelClipCount", lua.create_function(move |_, name: String| -> mlua::Result<u32> {
        Ok(ma.get_clip_count(&name) as u32)
    })?)?;

    let ma = model_anims.clone();
    engine.set("GetModelClipNames", lua.create_function(move |lua, name: String| -> mlua::Result<mlua::Table> {
        let names = ma.get_clip_names(&name);
        let out = lua.create_table()?;
        for (idx, clip_name) in names.iter().enumerate() {
            out.set(idx + 1, clip_name.clone())?;
        }
        Ok(out)
    })?)?;

    let anim_set_cmds_request = anim_set_cmds.clone();
    engine.set("RequestAnimSet", lua.create_function(move |_, path: String| {
        anim_set_cmds_request.push(AnimSetCommand::Request(path));
        Ok(())
    })?)?;

    let anim_set_cmds_release = anim_set_cmds.clone();
    engine.set("SetAnimSetAsNoLongerNeeded", lua.create_function(move |_, path: String| {
        anim_set_cmds_release.push(AnimSetCommand::Release(path));
        Ok(())
    })?)?;

    let anim_set_registry = anim_set_registry.clone();
    engine.set("HasAnimSetLoaded", lua.create_function(move |_, path: String| -> mlua::Result<bool> {
        Ok(anim_set_registry.has_loaded(&path))
    })?)?;

    // Engine.RequestAnimDict(model_name, dict_name) — požádej o load animation dictionary
    // Na klientovi: asynchronně requestuje preload modelu (pokud ne cached).
    // Na serveru: fallback na okamžitý `true`.
    let mc = model_cmds.clone();
    let ma = model_anims.clone();
    engine.set("RequestAnimDict", lua.create_function(move |_, (model_name, dict_name): (String, String)| {
        // Ověř, že dictionary existuje v modelu
        let clips = ma.get_dictionary_clips(&model_name, &dict_name);
        if clips.is_empty() {
            warn!("[Engine.RequestAnimDict] dictionary '{}:{}' not found", model_name, dict_name);
        }
        // Request model load pokud není cached
        mc.push(ModelCommand::Request(model_name));
        Ok(())
    })?)?;

    // Engine.HasAnimDictLoaded(model_name) — vrátí true, pokud je model (a tedy dict) dostupný
    let mr = model_registry.clone();
    engine.set("HasAnimDictLoaded", lua.create_function(move |_, model_name: String| -> mlua::Result<bool> {
        Ok(mr.has_loaded(&model_name))
    })?)?;

    // Engine.GetAnimDictClips(model_name, dict_name) → tabulka clipů | prázdná tabulka
    let ma = model_anims.clone();
    engine.set("GetAnimDictClips", lua.create_function(move |lua, (model_name, dict_name): (String, String)| -> mlua::Result<mlua::Table> {
        let clips = ma.get_dictionary_clips(&model_name, &dict_name);
        let out = lua.create_table()?;
        for (idx, clip_name) in clips.iter().enumerate() {
            out.set(idx + 1, clip_name.clone())?;
        }
        Ok(out)
    })?)?;

    // Engine.GetAnimDictNames(model_name) → tabulka dictionary názvů
    let ma = model_anims.clone();
    engine.set("GetAnimDictNames", lua.create_function(move |lua, model_name: String| -> mlua::Result<mlua::Table> {
        let dicts = ma.get_dictionary_names(&model_name);
        let out = lua.create_table()?;
        for (idx, dict_name) in dicts.iter().enumerate() {
            out.set(idx + 1, dict_name.clone())?;
        }
        Ok(out)
    })?)?;

    let mc = model_cmds.clone();
    engine.set("SetModelAsNoLongerNeeded", lua.create_function(move |_, name: String| {
        mc.push(ModelCommand::Release(name));
        Ok(())
    })?)?;

    // Engine.SetCursorLocked(bool) — ESC menu / UI overlay cursor control
    let esb = engine_state.clone();
    engine.set("SetCursorLocked", lua.create_function(move |_, locked: bool| {
        esb.set_cursor_locked(locked);
        Ok(())
    })?)?;

    // Engine.Quit() — request app exit
    let esb = engine_state.clone();
    engine.set("Quit", lua.create_function(move |_, ()| {
        esb.0.lock().unwrap_or_else(|p| p.into_inner()).quit_requested = true;
        Ok(())
    })?)?;

    // Engine.Disconnect() — request return to lobby
    let esb = engine_state.clone();
    engine.set("Disconnect", lua.create_function(move |_, ()| {
        esb.0.lock().unwrap_or_else(|p| p.into_inner()).disconnect_requested = true;
        Ok(())
    })?)?;

    globals.set("Engine", engine)?;

    // -- Raycast namespace (Phase 3.7) ---------------------------------------
    // Klientsky gameplay system aktualizuje RaycastBridge kazdy frame.
    // Raycast.GetGroundPosition() vraci posledni znamou pozici mysi
    // na Y=0 rovine. Na serveru vzdy vraci {0,0,0}.
    let rc_ns = lua.create_table()?;
    let pos_arc = raycast.0.clone();
    rc_ns.set(
        "GetGroundPosition",
        lua.create_function(move |lua, ()| {
            let pos = *pos_arc.lock().unwrap_or_else(|p| p.into_inner());
            let t = lua.create_table()?;
            t.set("x", pos[0])?;
            t.set("y", pos[1])?;
            t.set("z", pos[2])?;
            Ok(t)
        })?,
    )?;
    // Raycast.GetEntityUnderCrosshair(max_dist?) → {handle, distance} | nil
    // Client: vrací entitu pod středem obrazovky (crosshair raycast) do max_dist metrů.
    // Server: vždy nil.
    let ch_arc = crosshair.0.clone();
    rc_ns.set("GetEntityUnderCrosshair", lua.create_function(move |lua, max_dist: Option<f32>| {
        let max_d = max_dist.unwrap_or(100.0);
        let guard = ch_arc.lock().unwrap_or_else(|p| p.into_inner());
        let Some(hit) = guard.as_ref() else { return Ok(mlua::Value::Nil) };
        if hit.distance > max_d { return Ok(mlua::Value::Nil) }
        let t = lua.create_table()?;
        t.set("handle", hit.handle)?;
        t.set("distance", hit.distance)?;
        Ok(mlua::Value::Table(t))
    })?)?;

    globals.set("Raycast", rc_ns)?;

    // -- Player namespace (Phase 4) -----------------------------------------
    // Server-only: čtení z PlayerStatsCache (Arc), zápisy přes CommandQueue.
    let player_ns = lua.create_table()?;

    // Player.GetStat(player_id, stat_name) -> value|nil
    let sc = stats_cache.clone();
    player_ns.set("GetStat", lua.create_function(
        move |_, (player_id_v, name): (mlua::Value, String)| {
            let pid = lua_value_to_u64(&player_id_v);
            let val = pid.and_then(|id| sc.get(id))
                         .and_then(|snap| snap.stats.get(&name).copied());
            Ok(val)
        },
    )?)?;

    // Player.GetStats(player_id) -> table|nil
    let sc = stats_cache.clone();
    player_ns.set("GetStats", lua.create_function(
        move |lua, player_id_v: mlua::Value| {
            let pid = lua_value_to_u64(&player_id_v);
            let Some(snap) = pid.and_then(|id| sc.get(id)) else {
                return Ok(mlua::Value::Nil);
            };
            let t = lua.create_table()?;
            for (k, v) in &snap.stats {
                t.set(k.as_str(), *v)?;
            }
            Ok(mlua::Value::Table(t))
        },
    )?)?;

    // Player.SetStat(player_id, name, value)  — server only
    let cq = cmd_queue.clone();
    player_ns.set("SetStat", lua.create_function(
        move |_, (player_id_v, name, value): (mlua::Value, String, f64)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Player.SetStat is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            cq.push(LuaCommand::SetStat { player_id, name, value });
            Ok(())
        },
    )?)?;

    // Player.GetHealth(player_id) -> number|nil
    let sc = stats_cache.clone();
    player_ns.set("GetHealth", lua.create_function(
        move |_, player_id_v: mlua::Value| {
            let pid = lua_value_to_u64(&player_id_v);
            let val = pid.and_then(|id| sc.get(id)).map(|snap| snap.health);
            Ok(val)
        },
    )?)?;

    // Player.GetInventory(player_id) -> table|nil
    let sc = stats_cache.clone();
    player_ns.set("GetInventory", lua.create_function(
        move |lua, player_id_v: mlua::Value| {
            let pid = lua_value_to_u64(&player_id_v);
            let Some(snap) = pid.and_then(|id| sc.get(id)) else {
                return Ok(mlua::Value::Nil);
            };
            let t = lua.create_table()?;
            for (k, v) in &snap.inventory {
                t.set(k.as_str(), *v)?;
            }
            Ok(mlua::Value::Table(t))
        },
    )?)?;

    // Player.GetItemCount(player_id, item) -> integer
    let sc = stats_cache.clone();
    player_ns.set("GetItemCount", lua.create_function(
        move |_, (player_id_v, item): (mlua::Value, String)| {
            let pid = lua_value_to_u64(&player_id_v);
            let count = pid.and_then(|id| sc.get(id))
                           .and_then(|snap| snap.inventory.get(&item).copied())
                           .unwrap_or(0);
            Ok(count)
        },
    )?)?;

    // Player.GiveItem(player_id, item, count) — server only
    let cq = cmd_queue.clone();
    player_ns.set("GiveItem", lua.create_function(
        move |_, (player_id_v, item, count): (mlua::Value, String, i32)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Player.GiveItem is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            cq.push(LuaCommand::GiveItem { player_id, item, count });
            Ok(())
        },
    )?)?;

    // Player.TakeItem(player_id, item, count) — server only (alias GiveItem negative)
    let cq = cmd_queue.clone();
    player_ns.set("TakeItem", lua.create_function(
        move |_, (player_id_v, item, count): (mlua::Value, String, u32)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Player.TakeItem is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            cq.push(LuaCommand::GiveItem { player_id, item, count: -(count as i32) });
            Ok(())
        },
    )?)?;

    globals.set("Player", player_ns)?;

    // -- Database namespace (Phase 4) — server only, jen pokud je bridge k dispozici --
    if let Some(bridge) = db_bridge {
        let db_ns = lua.create_table()?;
        let res_id = id.clone();

        // Database.execute(sql, params, callback)
        // callback(rows_affected: integer)  — INSERT/UPDATE/DELETE
        {
            let bridge = bridge.clone();
            let res_id = res_id.clone();
            let cbs = db_callbacks.clone();
            let cnt = db_counter.clone();
            db_ns.set("execute", lua.create_function(
                move |lua, (sql, params_v, cb): (String, mlua::Value, mlua::Function)| {
                    if side != Side::Server {
                        return Err(mlua::Error::RuntimeError("Database.execute is server-only".into()));
                    }
                    let cb_id = { let mut c = cnt.borrow_mut(); *c += 1; *c };
                    let key = lua.create_registry_value(cb)?;
                    cbs.borrow_mut().insert(cb_id, key);
                    let params = json_params(params_v);
                    bridge.executor.execute(
                        sql, params,
                        res_id.clone(), cb_id,
                        bridge.queue.clone(),
                    );
                    Ok(())
                },
            )?)?;
        }

        // Database.query(sql, params, callback)
        // callback(rows: table)  — SELECT
        {
            let bridge = bridge.clone();
            let res_id = res_id.clone();
            let cbs = db_callbacks.clone();
            let cnt = db_counter.clone();
            db_ns.set("query", lua.create_function(
                move |lua, (sql, params_v, cb): (String, mlua::Value, mlua::Function)| {
                    if side != Side::Server {
                        return Err(mlua::Error::RuntimeError("Database.query is server-only".into()));
                    }
                    let cb_id = { let mut c = cnt.borrow_mut(); *c += 1; *c };
                    let key = lua.create_registry_value(cb)?;
                    cbs.borrow_mut().insert(cb_id, key);
                    let params = json_params(params_v);
                    bridge.executor.query(
                        sql, params,
                        res_id.clone(), cb_id,
                        bridge.queue.clone(),
                    );
                    Ok(())
                },
            )?)?;
        }

        // Database.isConnected() -> bool
        {
            let bridge = bridge.clone();
            db_ns.set("isConnected", lua.create_function(
                move |_, ()| Ok(bridge.executor.is_connected()),
            )?)?;
        }

        globals.set("Database", db_ns)?;
    } else {
        // Stub na klientu / bez DB — vrátí runtime error při volání
        let db_ns = lua.create_table()?;
        for fname in &["execute", "query", "isConnected"] {
            let n = fname.to_string();
            db_ns.set(*fname, lua.create_function(move |_, _: MultiValue| -> mlua::Result<mlua::Value> {
                Err(mlua::Error::RuntimeError(format!("Database.{} is not available (no DB configured)", n)))
            })?)?;
        }
        globals.set("Database", db_ns)?;
    }

    // -- Player.GetLocalStats() — client only ---------------------------------
    // Vrátí snapshot HP lokálního hráče aktualizovaný serverem přes PlayerStatsUpdate.
    // Lua resource ho může číst v loopu: while true do ... wait(100) end
    {
        let player_tbl = lua.create_table()?;

        if side == Side::Client {
            if let Some(ls) = local_stats.clone() {
                player_tbl.set("GetLocalStats", lua.create_function(move |lua, ()| {
                    let snap = ls.get();
                    let t = lua.create_table()?;
                    t.set("hp", snap.health)?;
                    t.set("max_hp", snap.max_health)?;
                    Ok(t)
                })?)?;
            } else {
                player_tbl.set("GetLocalStats", lua.create_function(|lua, ()| {
                    let t = lua.create_table()?;
                    t.set("hp", 100.0_f32)?;
                    t.set("max_hp", 100.0_f32)?;
                    Ok(t)
                })?)?;
            }
        } else {
            player_tbl.set("GetLocalStats", lua.create_function(|_, ()| -> mlua::Result<()> {
                Err(mlua::Error::RuntimeError("Player.GetLocalStats is client-only".into()))
            })?)?;
        }

        globals.set("Player", player_tbl)?;
    }

    // -- Threading: CreateThread + Wait ----------------------------------------
    // CreateThread(fn) — spustí coroutinu v příštím ticku.
    // Wait(ms)         — alias pro coroutine.yield(ms); pozastaví thread na ms ms.
    //                    Wait(0) = "pokračuj v příštím frame".
    {
        let co: mlua::Table = globals.get("coroutine")?;
        let yield_fn: mlua::Function = co.get("yield")?;
        globals.set("Wait", yield_fn)?;

        let pool = thread_pool.clone();
        globals.set("CreateThread", lua.create_function(move |lua, f: mlua::Function| {
            let thread = lua.create_thread(f)?;
            let key = lua.create_registry_value(thread)?;
            pool.borrow_mut().push(ThreadEntry { key, wake_at_ms: 0 });
            Ok(())
        })?)?;
    }

    // -- Gui namespace — client only -------------------------------------------
    // Souřadnice: normalizované 0.0–1.0, origin vlevo nahoře.
    // Barvy: r, g, b, a jako 0–255 integers.
    {
        let gui_ns = lua.create_table()?;

        if side == Side::Client {
            let buf = draw_buffer.clone();
            gui_ns.set("DrawRect", lua.create_function(
                move |_, (x, y, w, h, r, g, b, a): (f32, f32, f32, f32, u8, u8, u8, u8)| {
                    buf.push(DrawCommand::Rect { x, y, w, h, color: [r, g, b, a] });
                    Ok(())
                },
            )?)?;

            let buf = draw_buffer.clone();
            gui_ns.set("DrawText", lua.create_function(
                move |_, (text, x, y, scale, r, g, b, a, font_id): (String, f32, f32, f32, u8, u8, u8, u8, Option<String>)| {
                    buf.push(DrawCommand::Text { text, x, y, scale, color: [r, g, b, a], font_id });
                    Ok(())
                },
            )?)?;

            let buf = draw_buffer.clone();
            gui_ns.set("DrawLine", lua.create_function(
                move |_, (x1, y1, x2, y2, r, g, b, a): (f32, f32, f32, f32, u8, u8, u8, u8)| {
                    buf.push(DrawCommand::Line { x1, y1, x2, y2, color: [r, g, b, a] });
                    Ok(())
                },
            )?)?;

            let buf = draw_buffer.clone();
            gui_ns.set("DrawCircle", lua.create_function(
                move |_, (x, y, radius, r, g, b, a): (f32, f32, f32, u8, u8, u8, u8)| {
                    buf.push(DrawCommand::Circle { x, y, radius, color: [r, g, b, a] });
                    Ok(())
                },
            )?)?;

            let buf = draw_buffer.clone();
            gui_ns.set("DrawDisc", lua.create_function(
                move |_, (x, y, radius, r, g, b, a): (f32, f32, f32, u8, u8, u8, u8)| {
                    buf.push(DrawCommand::Disc { x, y, radius, color: [r, g, b, a] });
                    Ok(())
                },
            )?)?;

            let buf = draw_buffer.clone();
            gui_ns.set("DrawSprite", lua.create_function(
                move |_, (image_id, x, y, w, h, r, g, b, a, opts): (
                    String, f32, f32, f32, f32,
                    Option<u8>, Option<u8>, Option<u8>, Option<u8>,
                    Option<mlua::Table>,
                )| {
                    let fit = opts.as_ref()
                        .and_then(|t| t.get::<String>("fit").ok())
                        .map(|s| match s.as_str() {
                            "fit"  => SpriteFit::Fit,
                            "fill" => SpriteFit::Fill,
                            _      => SpriteFit::Stretch,
                        })
                        .unwrap_or_default();

                    let uv = opts.as_ref()
                        .and_then(|t| t.get::<mlua::Table>("uv").ok())
                        .and_then(|uv| {
                            let u0 = uv.get::<f32>(1).ok()?;
                            let v0 = uv.get::<f32>(2).ok()?;
                            let u1 = uv.get::<f32>(3).ok()?;
                            let v1 = uv.get::<f32>(4).ok()?;
                            Some([u0, v0, u1, v1])
                        });

                    let flip_x = opts.as_ref()
                        .and_then(|t| t.get::<bool>("flip_x").ok())
                        .unwrap_or(false);
                    let flip_y = opts.as_ref()
                        .and_then(|t| t.get::<bool>("flip_y").ok())
                        .unwrap_or(false);

                    buf.push(DrawCommand::Sprite {
                        image_id,
                        x, y, w, h,
                        color: [r.unwrap_or(255), g.unwrap_or(255), b.unwrap_or(255), a.unwrap_or(255)],
                        uv,
                        fit,
                        flip_x,
                        flip_y,
                    });
                    Ok(())
                },
            )?)?;
            // Mouse / cursor helpers (client)
            let ib = input_bridge.clone();
            gui_ns.set("GetCursorPos", lua.create_function(move |lua, ()| {
                let snap = ib.0.lock().unwrap_or_else(|p| p.into_inner());
                let t = lua.create_table()?;
                t.set("x", snap.cursor_x)?;
                t.set("y", snap.cursor_y)?;
                Ok(t)
            })?)?;

            let ib = input_bridge.clone();
            gui_ns.set("IsMouseOver", lua.create_function(move |_, (x, y, w, h): (f32, f32, f32, f32)| {
                let snap = ib.0.lock().unwrap_or_else(|p| p.into_inner());
                Ok(snap.cursor_x >= x - w * 0.5 && snap.cursor_x <= x + w * 0.5
                    && snap.cursor_y >= y - h * 0.5 && snap.cursor_y <= y + h * 0.5)
            })?)?;

            let ib = input_bridge.clone();
            gui_ns.set("IsMouseDown", lua.create_function(move |_, btn: Option<String>| {
                let btn = btn.unwrap_or_else(|| "left".to_string());
                let snap = ib.0.lock().unwrap_or_else(|p| p.into_inner());
                Ok(snap.mouse_pressed.contains(&btn.to_lowercase()))
            })?)?;

            let ib = input_bridge.clone();
            gui_ns.set("IsMouseClicked", lua.create_function(move |_, btn: Option<String>| {
                let btn = btn.unwrap_or_else(|| "left".to_string());
                let snap = ib.0.lock().unwrap_or_else(|p| p.into_inner());
                Ok(snap.mouse_just_pressed.contains(&btn.to_lowercase()))
            })?)?;
        } else {
            for fname in &["DrawRect", "DrawText", "DrawLine", "DrawCircle", "DrawSprite", "DrawDisc"] {
                gui_ns.set(*fname, lua.create_function(|_, _: MultiValue| Ok(()))?)?;
            }
            gui_ns.set("GetCursorPos", lua.create_function(|lua, ()| {
                let t = lua.create_table()?;
                t.set("x", 0.0f32)?;
                t.set("y", 0.0f32)?;
                Ok(t)
            })?)?;
            for fname in &["IsMouseOver", "IsMouseDown", "IsMouseClicked"] {
                gui_ns.set(*fname, lua.create_function(|_, _: MultiValue| -> mlua::Result<bool> { Ok(false) })?)?;
            }
        }

        globals.set("Gui", gui_ns)?;

        lua.load(r#"
local _g = Gui

-- DrawRoundedRect: zaoblené rohy přes 2 překrývající se recty + 4 disky na rozích.
-- Pro neprůhledné barvy bez artefaktů; pro alpha < 255 preferuj DrawRect.
function _g.DrawRoundedRect(x, y, w, h, radius, r, g, b, a)
    local rad = math.min(radius or 0, w * 0.5, h * 0.5)
    if rad < 0.0005 then
        _g.DrawRect(x, y, w, h, r, g, b, a)
        return
    end
    local hw = w * 0.5 - rad
    local hh = h * 0.5 - rad
    _g.DrawRect(x, y, w,       h - rad * 2, r, g, b, a)
    _g.DrawRect(x, y, w - rad * 2, h,       r, g, b, a)
    _g.DrawDisc(x - hw, y - hh, rad, r, g, b, a)
    _g.DrawDisc(x + hw, y - hh, rad, r, g, b, a)
    _g.DrawDisc(x - hw, y + hh, rad, r, g, b, a)
    _g.DrawDisc(x + hw, y + hh, rad, r, g, b, a)
end

-- DrawBorder: obrys pomocí 4 tenkých rectů (rohové pixely se překrývají).
function _g.DrawBorder(x, y, w, h, thickness, r, g, b, a)
    local t = thickness
    _g.DrawRect(x,               y - h*0.5 + t*0.5, w,   t,         r, g, b, a)
    _g.DrawRect(x,               y + h*0.5 - t*0.5, w,   t,         r, g, b, a)
    _g.DrawRect(x - w*0.5 + t*0.5, y, t, h - t * 2, r, g, b, a)
    _g.DrawRect(x + w*0.5 - t*0.5, y, t, h - t * 2, r, g, b, a)
end

-- DrawShadow: vrstvený drop-shadow. Volej PŘED vykreslením elementu.
function _g.DrawShadow(x, y, w, h, size, r, g, b, a)
    local n   = 4
    local off = size * 0.4
    local base = a or 60
    for i = 1, n do
        local s  = size * i / n
        local al = math.floor(base * (n - i + 1) / (n * (n + 1) / 2))
        _g.DrawRect(x + off, y + off, w + s * 2, h + s * 2, r or 0, g or 0, b or 0, al)
    end
end

-- Button: convenience s hover/active efektem + zaoblenými rohy.
function _g.Button(x, y, w, h, label, r, g, b, a)
    local hovered = _g.IsMouseOver(x, y, w, h)
    local held    = hovered and _g.IsMouseDown()
    local clicked = hovered and _g.IsMouseClicked()
    local cr, cg, cb = r or 80, g or 80, b or 80
    if held then
        cr = math.max(0,   math.floor(cr * 0.70))
        cg = math.max(0,   math.floor(cg * 0.70))
        cb = math.max(0,   math.floor(cb * 0.70))
    elseif hovered then
        cr = math.min(255, cr + 40)
        cg = math.min(255, cg + 40)
        cb = math.min(255, cb + 40)
    end
    _g.DrawRoundedRect(x, y, w, h, h * 0.20, cr, cg, cb, a or 230)
    if label and label ~= "" then
        local th = 0.018
        _g.DrawText(label, x - w*0.5 + 0.018, y - h*0.5 + (h - th)*0.5, 0.9, 215, 215, 215, 255)
    end
    return clicked
end

-- ── UI framework ─────────────────────────────────────────────────────────────
-- UI.Window(opts) vrátí objekt s metodami :Button, :Label, :Sep, :Open/:Close,
-- :Toggle, :IsOpen, :Render. Volej :Render() každý frame v draw threadu.
-- opts: { title, width, height, x, y }

UI = {}

local _T = {
    bg          = {18,  20,  26,  238},
    bg_header   = {26,  29,  38,  255},
    border      = {52,  58,  74,  160},
    sep         = {48,  54,  68,  135},
    btn         = {40,  46,  58,  215},
    btn_hover   = {60,  68,  85,  240},
    btn_active  = {26,  30,  38,  255},
    btn_danger  = {148, 36,  36,  220},
    btn_accent  = {48,  108, 175, 220},
    text        = {215, 215, 215, 255},
    text_dim    = {128, 128, 138, 185},
    shadow_col  = {0,   0,   0,   50},
    shadow_size = 0.005,
    radius      = 0.008,
    border_w    = 0.0013,
    btn_h       = 0.050,
    btn_gap     = 0.010,
    pad_x       = 0.016,
    pad_y       = 0.018,
    header_h    = 0.052,
    fade_in     = 10.0,
    fade_out    = 15.0,
}

function UI.SetTheme(t)
    for k, v in pairs(t) do _T[k] = v end
end
function UI.Theme() return _T end

function UI.Window(opts)
    local o   = opts or {}
    local W   = o.width  or 0.28
    local H   = o.height
    local CX  = o.x     or 0.50
    local CY  = o.y     or 0.50
    local TTL = o.title  or ""
    local items   = {}
    local fade    = 0.0
    local visible = false
    local win = {}

    function win:Button(lbl, cb, style)
        table.insert(items, {t="btn", label=lbl, cb=cb, style=style or "normal"})
        return self
    end
    function win:Label(txt, dim)
        table.insert(items, {t="lbl", text=txt, dim=dim})
        return self
    end
    function win:Sep()
        table.insert(items, {t="sep"})
        return self
    end
    function win:Open()    visible = true  end
    function win:Close()   visible = false end
    function win:Toggle()  visible = not visible end
    function win:IsOpen()  return visible end
    function win:GetFade() return fade    end

    function win:Render()
        local target = visible and 1.0 or 0.0
        local spd    = visible and _T.fade_in or _T.fade_out
        fade = fade + (target - fade) * spd * 0.016
        fade = math.max(0.0, math.min(1.0, fade))
        if fade < 0.004 then return end
        local f = fade

        -- auto-height z položek
        local ch = _T.header_h + _T.pad_y
        for _, it in ipairs(items) do
            if     it.t == "btn" then ch = ch + _T.btn_h  + _T.btn_gap
            elseif it.t == "lbl" then ch = ch + 0.026     + _T.btn_gap * 0.5
            elseif it.t == "sep" then ch = ch + 0.016
            end
        end
        ch = ch + _T.pad_y
        local h = H or ch

        -- shadow
        local sc = _T.shadow_col
        _g.DrawShadow(CX, CY, W, h, _T.shadow_size,
            sc[1], sc[2], sc[3], math.floor(sc[4] * f))

        -- border (vnější)
        local bw = _T.border_w
        local br = _T.border
        _g.DrawRoundedRect(CX, CY, W + bw*2, h + bw*2, _T.radius + bw,
            br[1], br[2], br[3], math.floor(br[4] * f))

        -- panel fill
        local bg = _T.bg
        _g.DrawRoundedRect(CX, CY, W, h, _T.radius,
            bg[1], bg[2], bg[3], math.floor(bg[4] * f))

        -- header bar
        local top = CY - h * 0.5
        local hcy = top + _T.header_h * 0.5
        local hb  = _T.bg_header
        _g.DrawRoundedRect(CX, hcy, W, _T.header_h, _T.radius,
            hb[1], hb[2], hb[3], math.floor(hb[4] * f))
        -- vyplnit spodní rohy headeru (aby nebyly zaobleny)
        _g.DrawRect(CX, top + _T.header_h - _T.radius * 0.5,
            W, _T.radius * 1.1,
            hb[1], hb[2], hb[3], math.floor(hb[4] * f))

        -- separator pod headerem
        local sy = top + _T.header_h
        local sp = _T.sep
        _g.DrawLine(CX - W*0.5 + 0.005, sy, CX + W*0.5 - 0.005, sy,
            sp[1], sp[2], sp[3], math.floor(sp[4] * f))

        -- titulek
        if TTL ~= "" then
            local tc = _T.text
            _g.DrawText(TTL,
                CX - W*0.5 + _T.pad_x, hcy - 0.009, 0.95,
                tc[1], tc[2], tc[3], math.floor(tc[4] * f))
        end

        -- položky
        local bw2 = W - _T.pad_x * 2
        local iy  = top + _T.header_h + _T.pad_y

        for _, it in ipairs(items) do
            if it.t == "btn" then
                local bcy = iy + _T.btn_h * 0.5
                local hov  = _g.IsMouseOver(CX, bcy, bw2, _T.btn_h)
                local held = hov and _g.IsMouseDown()
                local clk  = hov and _g.IsMouseClicked()
                local bc
                if it.style == "danger" then
                    local d = _T.btn_danger
                    bc = held and {math.floor(d[1]*.65),math.floor(d[2]*.65),math.floor(d[3]*.65),d[4]}
                      or hov  and d
                      or          {math.floor(d[1]*.50),math.floor(d[2]*.50),math.floor(d[3]*.50),d[4]}
                elseif it.style == "accent" then
                    local ac = _T.btn_accent
                    bc = held and {math.floor(ac[1]*.65),math.floor(ac[2]*.65),math.floor(ac[3]*.65),ac[4]}
                      or hov  and ac
                      or          {math.floor(ac[1]*.70),math.floor(ac[2]*.70),math.floor(ac[3]*.70),ac[4]}
                else
                    bc = held and _T.btn_active or hov and _T.btn_hover or _T.btn
                end
                _g.DrawRoundedRect(CX, bcy, bw2, _T.btn_h, _T.radius * 0.75,
                    bc[1], bc[2], bc[3], math.floor((bc[4] or 215) * f))
                local tc2 = _T.text
                _g.DrawText(it.label,
                    CX - bw2*0.5 + _T.pad_x, bcy - 0.008, 0.85,
                    tc2[1], tc2[2], tc2[3], math.floor(tc2[4] * f))
                if clk and it.cb then it.cb() end
                iy = iy + _T.btn_h + _T.btn_gap

            elseif it.t == "lbl" then
                local tc3 = it.dim and _T.text_dim or _T.text
                _g.DrawText(it.text,
                    CX - W*0.5 + _T.pad_x, iy, 0.72,
                    tc3[1], tc3[2], tc3[3], math.floor(tc3[4] * f))
                iy = iy + 0.026 + _T.btn_gap * 0.5

            elseif it.t == "sep" then
                local sy2 = iy + 0.008
                local sp2 = _T.sep
                _g.DrawLine(CX - W*0.5 + _T.pad_x, sy2, CX + W*0.5 - _T.pad_x, sy2,
                    sp2[1], sp2[2], sp2[3], math.floor(sp2[4] * f))
                iy = iy + 0.016
            end
        end
    end

    return win
end
"#).exec()?;
    }

    // -- Input namespace — synchronous key/mouse query (Phase 4) ---------------
    // Input.IsKeyDown("w"), Input.IsKeyJustPressed("space"), etc.
    // Key names: single letter ("a".."z"), digit ("0".."9"), "space", "escape",
    //   "enter", "tab", "backspace", "delete", "up"/"down"/"left"/"right",
    //   "lshift"/"rshift", "lctrl"/"rctrl", "lalt"/"ralt",
    //   "f1".."f12", "num0".."num9".
    // Mouse: "left", "right", "middle".
    // On server all functions return false.
    {
        let input_ns = lua.create_table()?;

        if side == Side::Client {
            let ib = input_bridge.clone();
            input_ns.set("IsKeyDown", lua.create_function(move |_, key: String| {
                Ok(ib.0.lock().unwrap_or_else(|p| p.into_inner()).pressed.contains(&key.to_lowercase()))
            })?)?;

            let ib = input_bridge.clone();
            input_ns.set("IsKeyJustPressed", lua.create_function(move |_, key: String| {
                Ok(ib.0.lock().unwrap_or_else(|p| p.into_inner()).just_pressed.contains(&key.to_lowercase()))
            })?)?;

            let ib = input_bridge.clone();
            input_ns.set("IsKeyJustReleased", lua.create_function(move |_, key: String| {
                Ok(ib.0.lock().unwrap_or_else(|p| p.into_inner()).just_released.contains(&key.to_lowercase()))
            })?)?;

            let ib = input_bridge.clone();
            input_ns.set("IsMouseButtonDown", lua.create_function(move |_, btn: String| {
                Ok(ib.0.lock().unwrap_or_else(|p| p.into_inner()).mouse_pressed.contains(&btn.to_lowercase()))
            })?)?;

            let ib = input_bridge.clone();
            input_ns.set("IsMouseButtonJustPressed", lua.create_function(move |_, btn: String| {
                Ok(ib.0.lock().unwrap_or_else(|p| p.into_inner()).mouse_just_pressed.contains(&btn.to_lowercase()))
            })?)?;

            let ib = input_bridge.clone();
            input_ns.set("IsMouseButtonJustReleased", lua.create_function(move |_, btn: String| {
                Ok(ib.0.lock().unwrap_or_else(|p| p.into_inner()).mouse_just_released.contains(&btn.to_lowercase()))
            })?)?;
        } else {
            for fname in &["IsKeyDown", "IsKeyJustPressed", "IsKeyJustReleased",
                           "IsMouseButtonDown", "IsMouseButtonJustPressed", "IsMouseButtonJustReleased"] {
                input_ns.set(*fname, lua.create_function(|_, _: String| -> mlua::Result<bool> { Ok(false) })?)?;
            }
        }

        globals.set("Input", input_ns)?;
    }

    // -- ACE namespace — FiveM-style permission system -------------------------
    // All mutating functions are server-only; reads (IsAceAllowed, IsPlayerAceAllowed)
    // are available on both sides but will always return false on the client since
    // the registry is never populated there.
    {
        let ace_ns = lua.create_table()?;

        // ACE.AddAce(principal, ace_name, "allow"|"deny")
        let ace_ref = ace_registry.clone();
        ace_ns.set("AddAce", lua.create_function(move |_, (principal, ace_name, mode): (String, String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("ACE.AddAce is server-only".into()));
            }
            let allow = mode.to_lowercase() != "deny";
            ace_ref.add_ace(&principal, &ace_name, allow);
            Ok(())
        })?)?;

        // ACE.RemoveAce(principal, ace_name)
        let ace_ref = ace_registry.clone();
        ace_ns.set("RemoveAce", lua.create_function(move |_, (principal, ace_name): (String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("ACE.RemoveAce is server-only".into()));
            }
            ace_ref.remove_ace(&principal, &ace_name);
            Ok(())
        })?)?;

        // ACE.AddPrincipal(child, parent) — inherit all permissions of parent
        let ace_ref = ace_registry.clone();
        ace_ns.set("AddPrincipal", lua.create_function(move |_, (child, parent): (String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("ACE.AddPrincipal is server-only".into()));
            }
            ace_ref.add_principal(&child, &parent);
            Ok(())
        })?)?;

        // ACE.RemovePrincipal(child, parent)
        let ace_ref = ace_registry.clone();
        ace_ns.set("RemovePrincipal", lua.create_function(move |_, (child, parent): (String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("ACE.RemovePrincipal is server-only".into()));
            }
            ace_ref.remove_principal(&child, &parent);
            Ok(())
        })?)?;

        // ACE.AddPlayerIdentifier(player_id, identifier)
        // identifier format: "ip:1.2.3.4", "discord:123456789", etc.
        let ace_ref = ace_registry.clone();
        ace_ns.set("AddPlayerIdentifier", lua.create_function(move |_, (player_id_v, identifier): (mlua::Value, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("ACE.AddPlayerIdentifier is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            ace_ref.add_identifier(player_id, identifier);
            Ok(())
        })?)?;

        // ACE.RemovePlayer(player_id) — cleans up identifiers and per-player entries on disconnect
        let ace_ref = ace_registry.clone();
        ace_ns.set("RemovePlayer", lua.create_function(move |_, player_id_v: mlua::Value| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("ACE.RemovePlayer is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            ace_ref.remove_player(player_id);
            Ok(())
        })?)?;

        // ACE.GetPlayerIdentifiers(player_id) -> {string, ...}
        let ace_ref = ace_registry.clone();
        ace_ns.set("GetPlayerIdentifiers", lua.create_function(move |lua, player_id_v: mlua::Value| {
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            let ids = ace_ref.get_identifiers(player_id);
            let t = lua.create_table()?;
            for (i, id) in ids.into_iter().enumerate() {
                t.set(i + 1, id)?;
            }
            Ok(t)
        })?)?;

        // ACE.GetPlayerIdentifier(player_id, type) -> string|nil
        // type examples: "ip", "discord", "steam"
        let ace_ref = ace_registry.clone();
        ace_ns.set("GetPlayerIdentifier", lua.create_function(move |lua, (player_id_v, id_type): (mlua::Value, String)| -> mlua::Result<mlua::Value> {
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            match ace_ref.get_identifier(player_id, &id_type) {
                Some(s) => Ok(mlua::Value::String(lua.create_string(s.as_bytes())?)),
                None => Ok(mlua::Value::Nil),
            }
        })?)?;

        // ACE.IsAceAllowed(principal, ace_name) -> bool
        let ace_ref = ace_registry.clone();
        ace_ns.set("IsAceAllowed", lua.create_function(move |_, (principal, ace_name): (String, String)| {
            Ok(ace_ref.is_allowed(&principal, &ace_name))
        })?)?;

        // ACE.IsPlayerAceAllowed(player_id, ace_name) -> bool
        let ace_ref = ace_registry.clone();
        ace_ns.set("IsPlayerAceAllowed", lua.create_function(move |_, (player_id_v, ace_name): (mlua::Value, String)| {
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            Ok(ace_ref.is_player_allowed(player_id, &ace_name))
        })?)?;

        globals.set("ACE", ace_ns)?;

        // FiveM compatibility aliases (global funcs):
        // AddAce/AddPrincipal/RemoveAce/RemovePrincipal/IsAceAllowed/IsPlayerAceAllowed
        // plus principal helper IsPrincipalAceAllowed.

        let ace_ref = ace_registry.clone();
        globals.set("AddAce", lua.create_function(move |_, (principal, ace_name, mode): (String, String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("AddAce is server-only".into()));
            }
            let allow = mode.to_lowercase() != "deny";
            ace_ref.add_ace(&principal, &ace_name, allow);
            Ok(())
        })?)?;

        let ace_ref = ace_registry.clone();
        globals.set("RemoveAce", lua.create_function(move |_, (principal, ace_name): (String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("RemoveAce is server-only".into()));
            }
            ace_ref.remove_ace(&principal, &ace_name);
            Ok(())
        })?)?;

        let ace_ref = ace_registry.clone();
        globals.set("AddPrincipal", lua.create_function(move |_, (child, parent): (String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("AddPrincipal is server-only".into()));
            }
            ace_ref.add_principal(&child, &parent);
            Ok(())
        })?)?;

        let ace_ref = ace_registry.clone();
        globals.set("RemovePrincipal", lua.create_function(move |_, (child, parent): (String, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("RemovePrincipal is server-only".into()));
            }
            ace_ref.remove_principal(&child, &parent);
            Ok(())
        })?)?;

        let ace_ref = ace_registry.clone();
        globals.set("IsAceAllowed", lua.create_function(move |_, (principal, ace_name): (String, String)| {
            Ok(ace_ref.is_allowed(&principal, &ace_name))
        })?)?;

        let ace_ref = ace_registry.clone();
        globals.set("IsPrincipalAceAllowed", lua.create_function(move |_, (principal, ace_name): (String, String)| {
            Ok(ace_ref.is_allowed(&principal, &ace_name))
        })?)?;

        let ace_ref = ace_registry.clone();
        globals.set("IsPlayerAceAllowed", lua.create_function(move |_, (player_id_v, ace_name): (mlua::Value, String)| {
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            Ok(ace_ref.is_player_allowed(player_id, &ace_name))
        })?)?;
    }

    // -- Auth namespace — login/register flow ----------------------------------
    // Client: Auth.SendCredentials, Auth.IsRequired
    // Server: Auth.MarkPlayerAuthenticated, Auth.RejectPlayer
    // Both:   Auth.GetAccountId, Auth.IsAuthenticated
    {
        let auth_ns = lua.create_table()?;

        // Auth.SendCredentials("login"|"register", username, password) — client only.
        // Queues credentials to be sent via lightyear next frame.
        let auth_out = auth_bridge.outgoing.clone();
        auth_ns.set("SendCredentials", lua.create_function(move |_, (action_str, username, password): (String, String, String)| {
            if side != Side::Client {
                return Err(mlua::Error::RuntimeError("Auth.SendCredentials is client-only".into()));
            }
            let action = if action_str.to_lowercase() == "register" { 1u8 } else { 0u8 };
            auth_out.lock().unwrap_or_else(|p| p.into_inner())
                .push(PendingAuthCredentials { action, username, password });
            Ok(())
        })?)?;

        // Auth.IsRequired() -> bool — true when server requires authentication.
        let auth_req = auth_bridge.required.clone();
        auth_ns.set("IsRequired", lua.create_function(move |_, ()| {
            Ok(*auth_req.lock().unwrap_or_else(|p| p.into_inner()))
        })?)?;

        // Auth.MarkPlayerAuthenticated(player_id, account_id) — server only.
        // Called by the core/auth Lua resource after successful DB validation.
        // Removes from PendingAuth, registers ACE identifier, queues AuthResult to client.
        let auth_res = auth_bridge.results.clone();
        let auth_pend = auth_bridge.pending.clone();
        let ace_ref = ace_registry.clone();
        let bus_ref = local_bus.clone();
        auth_ns.set("MarkPlayerAuthenticated", lua.create_function(move |_, (player_id_v, account_id): (mlua::Value, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Auth.MarkPlayerAuthenticated is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            auth_pend.lock().unwrap_or_else(|p| p.into_inner()).remove(&player_id);
            // Register permanent ACE identifier: "user:<account_id>"
            ace_ref.add_identifier(player_id, format!("user:{account_id}"));
            // Queue AuthResult for dispatch back to client
            auth_res.lock().unwrap_or_else(|p| p.into_inner()).push(PendingAuthResult {
                player_id,
                success: true,
                account_id: account_id.clone(),
                error: String::new(),
            });
            // Emit cross-sandbox event so other resources know
            let payload = serde_json::to_vec(&serde_json::json!({
                "player": player_id.to_string(),
                "account_id": account_id,
            })).unwrap_or_default();
            bus_ref.push("auth:playerAuthenticated".to_string(), payload);
            Ok(())
        })?)?;

        // Auth.RejectPlayer(player_id, reason) — server only.
        // Player stays in PendingAuth (can retry). Server queues failure AuthResult.
        let auth_res = auth_bridge.results.clone();
        auth_ns.set("RejectPlayer", lua.create_function(move |_, (player_id_v, reason): (mlua::Value, String)| {
            if side != Side::Server {
                return Err(mlua::Error::RuntimeError("Auth.RejectPlayer is server-only".into()));
            }
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            auth_res.lock().unwrap_or_else(|p| p.into_inner()).push(PendingAuthResult {
                player_id,
                success: false,
                account_id: String::new(),
                error: reason,
            });
            Ok(())
        })?)?;

        // Auth.GetAccountId(player_id) -> string|nil — reads ACE "user" identifier.
        let ace_ref = ace_registry.clone();
        auth_ns.set("GetAccountId", lua.create_function(move |lua, player_id_v: mlua::Value| -> mlua::Result<mlua::Value> {
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            match ace_ref.get_identifier(player_id, "user") {
                Some(s) => Ok(mlua::Value::String(lua.create_string(s.as_bytes())?)),
                None => Ok(mlua::Value::Nil),
            }
        })?)?;

        // Auth.IsAuthenticated(player_id) -> bool — server only.
        let auth_pend = auth_bridge.pending.clone();
        auth_ns.set("IsAuthenticated", lua.create_function(move |_, player_id_v: mlua::Value| {
            let player_id = lua_value_to_u64(&player_id_v).unwrap_or(0);
            Ok(!auth_pend.lock().unwrap_or_else(|p| p.into_inner()).contains(&player_id))
        })?)?;

        globals.set("Auth", auth_ns)?;
    }

    // -- Network namespace — connection / server info (Phase 4) ----------------
    // Network.IsConnected(), Network.GetServerAddress(), Network.GetPing(),
    // Network.GetClientId() (returns string to avoid u64 precision loss in Lua)
    {
        let net_ns = lua.create_table()?;

        let cb = connection.clone();
        net_ns.set("IsConnected", lua.create_function(move |_, ()| {
            Ok(cb.0.lock().unwrap_or_else(|p| p.into_inner()).connected)
        })?)?;

        let cb = connection.clone();
        net_ns.set("GetServerAddress", lua.create_function(move |_, ()| -> mlua::Result<String> {
            Ok(cb.0.lock().unwrap_or_else(|p| p.into_inner()).server_addr.clone())
        })?)?;

        let cb = connection.clone();
        net_ns.set("GetPing", lua.create_function(move |_, ()| -> mlua::Result<u32> {
            Ok(cb.0.lock().unwrap_or_else(|p| p.into_inner()).ping_ms)
        })?)?;

        let cb = connection.clone();
        net_ns.set("GetClientId", lua.create_function(move |_, ()| -> mlua::Result<String> {
            Ok(cb.0.lock().unwrap_or_else(|p| p.into_inner()).client_id.to_string())
        })?)?;

        globals.set("Network", net_ns)?;
    }

    // -- Camera namespace — klientský systém kamer (Phase 5) ------------------
    // Na serveru jsou funkce no-op / vracejí nil; namespace existuje na obou stranách.
    {
        let cam_ns = lua.create_table()?;

        // Camera.Create(id, opts?) -> id
        // opts = { fov = 60.0 }
        {
            let cb = camera_bridge.clone();
            cam_ns.set("Create", lua.create_function(move |_, (id, opts): (String, Option<mlua::Table>)| -> mlua::Result<String> {
                if side == Side::Client {
                    let fov = opts.as_ref().and_then(|t| t.get::<f32>("fov").ok());
                    cb.create(id.clone(), fov);
                }
                Ok(id)
            })?)?;
        }

        // Camera.Delete(id)
        {
            let cb = camera_bridge.clone();
            cam_ns.set("Delete", lua.create_function(move |_, id: String| {
                if side == Side::Client { cb.delete(&id); }
                Ok(())
            })?)?;
        }

        // Camera.SetActive(id | nil) — nil vrátí zpět na player kameru
        {
            let cb = camera_bridge.clone();
            cam_ns.set("SetActive", lua.create_function(move |_, id: Option<String>| {
                if side == Side::Client { cb.set_active(id); }
                Ok(())
            })?)?;
        }

        // Camera.GetActive() -> id | nil
        {
            let cb = camera_bridge.clone();
            cam_ns.set("GetActive", lua.create_function(move |_, ()| -> mlua::Result<Option<String>> {
                if side != Side::Client { return Ok(None); }
                Ok(cb.get_active_id())
            })?)?;
        }

        // Camera.AttachToEntity(id, entity_handle, offset?, look_at_entity?)
        // offset = {x,y,z} world-space offset od entity pozice
        // look_at_entity = true → kamera míří na entitu; false (default) → mouse look
        {
            let cb = camera_bridge.clone();
            cam_ns.set("AttachToEntity", lua.create_function(move |_,
                (cam_id, handle, offset_v, look_at): (String, u64, Option<mlua::Table>, Option<bool>)| {
                if side == Side::Client {
                    let offset = offset_v.as_ref().map(|t| table_to_vec3(t)).unwrap_or([0.0; 3]);
                    cb.set_attachment(&cam_id, CameraAttachment::Entity {
                        handle, offset, look_at: look_at.unwrap_or(false),
                    });
                }
                Ok(())
            })?)?;
        }

        // Camera.AttachToBone(id, entity_handle, bone_name, offset?)
        // Kamera zdědí transformaci kosti (pozice + rotace); offset v bone-space.
        {
            let cb = camera_bridge.clone();
            cam_ns.set("AttachToBone", lua.create_function(move |_,
                (cam_id, handle, bone, offset_v): (String, u64, String, Option<mlua::Table>)| {
                if side == Side::Client {
                    let offset = offset_v.as_ref().map(|t| table_to_vec3(t)).unwrap_or([0.0; 3]);
                    cb.set_attachment(&cam_id, CameraAttachment::Bone { handle, bone, offset });
                }
                Ok(())
            })?)?;
        }

        // Camera.AttachToPosition(id, pos, look_at?)
        // pos = {x,y,z} světová pozice; look_at = {x,y,z} bod pohledu (nil = mouse look)
        {
            let cb = camera_bridge.clone();
            cam_ns.set("AttachToPosition", lua.create_function(move |_,
                (cam_id, pos_t, look_at_v): (String, mlua::Table, Option<mlua::Table>)| {
                if side == Side::Client {
                    let pos = table_to_vec3(&pos_t);
                    let look_at = look_at_v.as_ref().map(|t| table_to_vec3(t));
                    cb.set_attachment(&cam_id, CameraAttachment::Position { pos, look_at });
                }
                Ok(())
            })?)?;
        }

        // Camera.SetFOV(id, fov_degrees)
        {
            let cb = camera_bridge.clone();
            cam_ns.set("SetFOV", lua.create_function(move |_, (cam_id, fov): (String, f32)| {
                if side == Side::Client { cb.set_fov(&cam_id, fov); }
                Ok(())
            })?)?;
        }

        // Camera.SetMode(mode)  "first_person" | "third_person"
        {
            let cb = camera_bridge.clone();
            cam_ns.set("SetMode", lua.create_function(move |_, mode: String| {
                if side == Side::Client { cb.set_first_person(mode == "first_person"); }
                Ok(())
            })?)?;
        }

        // Camera.GetMode() -> "first_person" | "third_person" | custom_camera_id
        {
            let cb = camera_bridge.clone();
            cam_ns.set("GetMode", lua.create_function(move |_, ()| -> mlua::Result<String> {
                if side != Side::Client { return Ok("third_person".to_string()); }
                if let Some(id) = cb.get_active_id() { return Ok(id); }
                Ok(if cb.is_first_person() { "first_person" } else { "third_person" }.to_string())
            })?)?;
        }

        globals.set("Camera", cam_ns)?;
    }

    Ok(())
}
