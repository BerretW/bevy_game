/// Komponenta pro explicitní override ped physics profilu na entitě (NPC).
#[derive(Debug, Clone, Component, Reflect, Serialize, Deserialize, PartialEq, Eq)]
pub struct PedProfileOverride(pub String);
// Phase 3.2 — Command Queue: bezpečný Lua → ECS most.
//
// Lua sandbox nesmí přímo mutovat ECS svět (`mlua` je `!Send`, sandbox běží
// na main threadu v `NonSend` resource). Místo toho Lua vkládá záměry do
// sdíleného `CommandQueue` bufferu. Bevy systém `process_lua_commands`
// v `PostUpdate` frontu vybere a bezpečně aplikuje příkazy na ECS svět.
//
// Phase 4 přidává:
// * `Stats` a `Inventory` ECS komponenty.
// * `PlayerEntityMap` — mapuje client_id → Entity (udržuje core_net).
// * `PlayerStatsCache` — Arc<Mutex> snapshot pro synchronní Lua čtení.
// * `LuaCommand::SetStat`, `GiveItem`, `TakeItem` — mutace přes frontu.

use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bevy::ecs::hierarchy::ChildOf;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::reflect::Reflect;
use core_shared::{Health, NetTransform, PlayerMarker};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as Json};

use crate::npc_brain::{
    NpcBrainDef, NpcBrainRegistry, NpcBrainState, NpcBrainTarget, NpcTaskKind,
    ReplicatedNpcBrain,
};
use crate::plugin::ResourcesSide;
use crate::model_registry::AnimSetCommandQueue;
use crate::types::Side;
use crate::weapons::WeaponRegistry;

// ---------------------------------------------------------------------------
// LuaCommand — záměry, které Lua enqueuje přes World.* API
// ---------------------------------------------------------------------------

/// Příkazy, které Lua může zapsat do fronty.
/// Rust server validuje a zpracuje každý FixedUpdate / PostUpdate tick.
#[derive(Debug, Clone)]
pub enum LuaCommand {
    SpawnLocalObject {
        handle: u64,
        model: String,
        pos: [f32; 3],
        /// Euler XYZ ve stupních.
        rot: [f32; 3],
    },
    DespawnEntity {
        handle: u64,
    },
    SetTransform {
        handle: u64,
        pos: [f32; 3],
        rot: [f32; 3],
    },
    /// Nastaví pouze pozici entity (zachová rotaci a scale).
    SetPosition {
        handle: u64,
        pos: [f32; 3],
    },
    /// Nastaví pouze rotaci entity jako Euler XYZ ve stupních (zachová pozici a scale).
    SetRotation {
        handle: u64,
        rot: [f32; 3],
    },
    /// Nastaví scale entity. Uniform nebo per-axis.
    SetScale {
        handle: u64,
        scale: [f32; 3],
    },
    /// Nastaví per-entitní shader profile na drawable materiálu.
    /// Profile jsou interpretované samotným shaderem (např. "debug_stripes").
    SetEntityShaderProfile {
        handle: u64,
        profile: String,
    },
    /// Změní jméno modelu entity (Phase 4: swap meshe).
    SetModel {
        handle: u64,
        model: String,
    },
    /// Damage intent — jen server side; klient dostane runtime error z Lua API.
    ApplyDamage {
        target_handle: u64,
        amount: f32,
        source_handle: Option<u64>,
    },
    /// Phase 3.5 — replikovaná entita spawnovaná serverem, klienti ji dostanou
    /// přes lightyear replication. Server-only.
    SpawnNetworkedObject {
        handle: u64,
        model: String,
        pos: [f32; 3],
        rot: [f32; 3],
    },
    /// Server-only: replikovaná NPC entita (model + NPC marker).
    /// Server-only: replikovaná NPC entita (model + NPC marker + volitelný ped profil).
    SpawnNetworkedNpc {
        handle: u64,
        model: String,
        pos: [f32; 3],
        rot: [f32; 3],
        ped_profile: Option<String>,
    },
    /// Parametrický dummy objekt bez závislosti na asset modelu.
    SpawnLocalDummy {
        handle: u64,
        def: DummyObjectMarker,
        pos: [f32; 3],
        rot: [f32; 3],
    },
    /// Server-only parametrický dummy objekt replikovaný na klienty.
    SpawnNetworkedDummy {
        handle: u64,
        def: DummyObjectMarker,
        pos: [f32; 3],
        rot: [f32; 3],
    },
    /// Samostatný collider objekt bez vizuální reprezentace.
    SpawnLocalCollider {
        handle: u64,
        collider: DummyColliderDef,
        pos: [f32; 3],
        rot: [f32; 3],
    },
    /// Server-only samostatný collider objekt replikovaný na klienty.
    SpawnNetworkedCollider {
        handle: u64,
        collider: DummyColliderDef,
        pos: [f32; 3],
        rot: [f32; 3],
    },
    /// Spustí animaci na entitě. Phase 4 napojí na Bevy AnimationPlayer.
    PlayAnimation {
        handle: u64,
        name: String,
        looping: bool,
        speed: f32,
        blend_time: f32,
        flags: u32,
    },
    /// Přidá nebo nastaví anim-set pro entitu.
    ApplyAnimSet {
        handle: u64,
        path: String,
    },
    /// Zastaví aktuální animaci entity.
    StopAnimation {
        handle: u64,
    },
    /// Phase 4 — Spustí blend space (míchání více klipů podle 2D vektoru pohybu).
    PlayBlendSpace {
        handle: u64,
        blend_space_name: String,
        position: [f32; 2],  // 2D vektor (x, y) nebo 1D (x, 0)
        speed: f32,
        flags: u32,
    },
    /// Phase 4.2 — Zapne IK na entitě s danou váhou blendování (0-1).
    EnableIk {
        handle: u64,
        blend_weight: f32,
    },
    /// Phase 4.2 — Vypne IK na entitě.
    DisableIk {
        handle: u64,
    },
    /// Phase 4.3 — Zapne Root Motion na ADM entitě.
    EnableRootMotion {
        handle: u64,
        /// Jméno root bonu. Pokud None, použije výchozí "DEF_hips".
        root_bone_name: Option<String>,
        /// Pokud true, ignoruj Y-složku delty (hráč používá gravitaci/fyziku pro Y).
        lock_y: bool,
    },
    /// Phase 4.3 — Vypne Root Motion na ADM entitě.
    DisableRootMotion {
        handle: u64,
    },
    /// AI: nastaví základní parametry NPC agenta na entitě.
    NpcConfigure {
        handle: u64,
        move_speed: Option<f32>,
        arrive_distance: Option<f32>,
        turn_speed: Option<f32>,
    },
    /// AI: nastaví NPC do wander módu.
    NpcWander {
        handle: u64,
        kind: NpcWanderKind,
        radius: f32,
        retarget_sec: f32,
        orbit_angular_speed: f32,
        patrol_point: Option<[f32; 3]>,
        clockwise: bool,
    },
    /// AI: pohyb NPC k pevné world-space pozici.
    NpcGoToCoord {
        handle: u64,
        target: [f32; 3],
        stop_distance: f32,
    },
    /// AI: pohyb NPC k jiné entitě (podle jejího handle).
    NpcGoToEntity {
        handle: u64,
        target_handle: u64,
        stop_distance: f32,
    },
    /// AI: zastaví aktivní movement goal NPC.
    NpcStop {
        handle: u64,
    },
    RegisterNpcBrain {
        brain_id: String,
        def: NpcBrainDef,
    },
    RegisterNpcScenario {
        scenario_id: String,
        def: NpcScenarioDef,
    },
    ConfigureNpcScenarioClock {
        config: NpcScenarioClockConfigPatch,
    },
    ConfigureNpcAiLod {
        config: NpcAiLodConfigPatch,
    },
    ConfigureNpcPopulationDirector {
        config: NpcPopulationDirectorConfigPatch,
    },
    SetNpcScenarioTime {
        hour_of_day: f32,
    },
    ConfigureEnvironmentLight {
        config: EnvironmentLightConfigPatch,
    },
    SetEnvironmentTime {
        hour_of_day: f32,
    },
    SetNpcBrain {
        handle: u64,
        brain_id: String,
    },
    SetNpcTask {
        handle: u64,
        task: NpcTaskKind,
        scenario_id: Option<String>,
        target_handle: Option<u64>,
        target_pos: Option<[f32; 3]>,
        params: Json,
    },
    /// Připojí child entitu k parent entitě přes dvojici socketů.
    Attach {
        child_handle: u64,
        child_socket: String,
        parent_handle: u64,
        parent_socket: String,
    },
    /// Připojí child entitu k parent přes lokální offset od pivotu parenta.
    AttachWithOffset {
        child_handle: u64,
        parent_handle: u64,
        offset: [f32; 3],
        rot: [f32; 3],
    },
    /// Odpojí child entitu z hierarchie a zachová world-space transform.
    Detach {
        child_handle: u64,
    },
    /// Phase 4 — nastav libovolný stat hráče (server only).
    SetStat {
        player_id: u64,
        name: String,
        value: f64,
    },
    /// Phase 4 — přidej nebo uber předměty z inventáře (server only).
    /// Kladné `count` = dát, záporné = vzít; pod 0 se orizne na 0.
    GiveItem {
        player_id: u64,
        item: String,
        count: i32,
    },
    /// Phase 5 — nastaví nebo vymaže equipped weapon v daném slotu hráče.
    SetEquippedWeapon {
        player_id: u64,
        slot: u8,
        equipped: Option<EquippedWeapon>,
    },
    /// Phase 5 — nastaví rezervu munice pro konkrétní ammo typ.
    SetAmmoReserve {
        player_id: u64,
        ammo_type_id: String,
        count: u32,
    },
    /// Phase 5 — přepne aktivní weapon slot hráče.
    SetActiveWeaponSlot {
        player_id: u64,
        slot: u8,
    },
    /// Phase 5 — nastaví fire mode vybavené zbraně ve slotu nebo v aktivním slotu.
    SetWeaponFireMode {
        player_id: u64,
        slot: Option<u8>,
        fire_mode: String,
    },
    /// Phase 5 — vynutí přebití aktivní zbraně z ammo reserve.
    ForceReload {
        player_id: u64,
    },
    /// Phase 5 — nastaví nebo vymaže armor piece hráče.
    SetPlayerArmor {
        player_id: u64,
        slot: String,
        armor: Option<ArmorPiece>,
    },
    /// Phase 5 — nastaví aktuální/max HP hráče.
    SetPlayerHealth {
        player_id: u64,
        current: f32,
        max: Option<f32>,
    },
    /// Phase 5 — zapne nebo vypne kolizi entity a jejích potomků.
    /// Fyzikální backend reaguje přes `CollisionEnabled` komponent.
    SetCollisionEnabled {
        handle: u64,
        enabled: bool,
    },
    /// Phase 5 — nastaví jeden materiálový parametr na entitě.
    /// Param: "snow_level" | "dirt_level" | "wetness" (float 0..1)
    ///        "snow_height" | "wet_height" (world-space Y cutoff; 0 = vypnuto).
    SetMaterialParam {
        handle: u64,
        param: String,
        value: f32,
    },
}

// ---------------------------------------------------------------------------
// CommandQueue — sdílený buffer (Arc<Mutex<...>>)
// ---------------------------------------------------------------------------

/// Fronta příkazů sdílená mezi Lua sandboxy a Bevy systémem.
///
/// `Clone` je levný (kopíruje Arc pointer). Každý `LuaSandbox` dostane
/// svůj vlastní klon; všechny zapisují do stejného vektoru.
#[derive(Resource, Clone)]
pub struct CommandQueue {
    inner: Arc<Mutex<Vec<LuaCommand>>>,
    /// Monotonicky rostoucí counter pro přidělování handles.
    /// Handle 0 je vyhrazený jako null sentinel.
    counter: Arc<AtomicU64>,
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::new())),
            counter: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl CommandQueue {
    /// Přidělí unikátní handle. Volá se synchronně z Lua closure —
    /// Lua dostane číslo zpět ještě před zpracováním příkazu.
    pub fn alloc_handle(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    pub fn push(&self, cmd: LuaCommand) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(cmd);
    }

    /// Vybere všechny pending příkazy atomicky. Volá se z Bevy systému.
    pub fn drain(&self) -> Vec<LuaCommand> {
        std::mem::take(
            &mut *self.inner.lock().unwrap_or_else(|p| p.into_inner()),
        )
    }
}

// ---------------------------------------------------------------------------
// LuaWorldState — handle → Entity mapa
// ---------------------------------------------------------------------------

/// Mapuje Lua handles na Bevy Entity. Přetrvává přes hot-reload sandboxů.
#[derive(Resource, Default)]
pub struct LuaWorldState {
    handle_map: HashMap<u64, Entity>,
}

impl LuaWorldState {
    pub fn register(&mut self, handle: u64, entity: Entity) {
        self.handle_map.insert(handle, entity);
    }

    pub fn entity_for(&self, handle: u64) -> Option<Entity> {
        self.handle_map.get(&handle).copied()
    }

    pub fn remove(&mut self, handle: u64) -> Option<Entity> {
        self.handle_map.remove(&handle)
    }
}

// ---------------------------------------------------------------------------
// ECS typy výstupu
// ---------------------------------------------------------------------------

/// Marker component na entitách spawnutých přes `World.SpawnLocalObject`.
#[derive(Component, Debug, Clone)]
pub struct LocalObjectMarker {
    pub model: String,
}

/// Marker component na replikovaných entitách (`World.SpawnNetworkedObject`).
/// Phase 3.5 — lightyear Replicate je přidán při process_lua_commands.
#[derive(Component, Debug, Clone)]
pub struct NetworkedObjectMarker {
    pub model: String,
}

/// Marker na síťových entitách spawnutých jako NPC.
/// Klient používá tento marker pro vytvoření capsule collideru a NPC vizuálního setupu.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcPedMarker;

/// Embeddovaný handle na všech entitách spawnutých přes Lua bridge.
/// Replikovaný klientům přes lightyear — client identifikuje síťové entity
/// stejným u64 klíčem jako server.
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EntityHandle(pub u64);

/// Kanonické jméno modelu entity — aktualizované přes `SetModel` příkaz.
/// Replikované klientům, aby věděli, jaký model načíst.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelName(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DummyPrimitiveKind {
    Cuboid,
    Sphere,
    Cube,
    Stairs,
    Arch,
    PointLight,
    SpotLight,
    DirectionalLight,
    FogVolume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLightKind {
    Point,
    Spot,
    Directional,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RuntimeLightDef {
    pub enabled: bool,
    pub kind: RuntimeLightKind,
    pub color: [f32; 4],
    pub intensity: f32,
    pub illuminance: f32,
    pub range: f32,
    pub radius: f32,
    pub shadows_enabled: bool,
    pub inner_angle_deg: f32,
    pub outer_angle_deg: f32,
}

impl Default for RuntimeLightDef {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: RuntimeLightKind::Point,
            color: [1.0, 1.0, 1.0, 1.0],
            intensity: 1_000_000.0,
            illuminance: 10_000.0,
            range: 20.0,
            radius: 0.0,
            shadows_enabled: true,
            inner_angle_deg: 22.5,
            outer_angle_deg: 45.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RuntimeFogVolumeDef {
    pub enabled: bool,
    pub color: [f32; 4],
    pub density_factor: f32,
    pub absorption: f32,
    pub scattering: f32,
    pub scattering_asymmetry: f32,
    pub light_tint: [f32; 4],
    pub light_intensity: f32,
}

impl Default for RuntimeFogVolumeDef {
    fn default() -> Self {
        Self {
            enabled: true,
            color: [0.85, 0.88, 0.92, 0.72],
            density_factor: 0.16,
            absorption: 0.24,
            scattering: 0.40,
            scattering_asymmetry: 0.60,
            light_tint: [1.0, 1.0, 1.0, 1.0],
            light_intensity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DummyColliderShape {
    Auto,
    None,
    Box,
    Sphere,
    Capsule,
    Cylinder,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DummyColliderDef {
    pub enabled: bool,
    pub shape: DummyColliderShape,
    pub size: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub is_static: bool,
    pub is_trigger: bool,
    pub stairs: bool,
    pub stairs_slope_invert: bool,
    pub stairs_clearance_y: f32,
    pub friction: f32,
    pub restitution: f32,
}

impl Default for DummyColliderDef {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: DummyColliderShape::Auto,
            size: [1.0, 1.0, 1.0],
            radius: 0.5,
            height: 1.0,
            is_static: true,
            is_trigger: false,
            stairs: false,
            stairs_slope_invert: false,
            stairs_clearance_y: 0.0,
            friction: 0.8,
            restitution: 0.0,
        }
    }
}

/// Marker na collideru/entitě, která se má chovat jako schodiště trigger.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StairsCollider;

/// Parametrický dummy objekt generovaný runtime systémem bez model assetu.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DummyObjectMarker {
    pub kind: DummyPrimitiveKind,
    pub size: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub steps: u32,
    pub segments: u32,
    pub color: [f32; 4],
    pub collider: DummyColliderDef,
    pub light: Option<RuntimeLightDef>,
    pub fog_volume: Option<RuntimeFogVolumeDef>,
}

/// Samostatný collider objekt bez render mesh.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColliderObjectMarker {
    pub collider: DummyColliderDef,
}

impl Default for DummyObjectMarker {
    fn default() -> Self {
        Self {
            kind: DummyPrimitiveKind::Cuboid,
            size: [1.0, 1.0, 1.0],
            radius: 0.5,
            height: 1.0,
            steps: 4,
            segments: 12,
            color: [0.65, 0.75, 0.9, 1.0],
            collider: DummyColliderDef::default(),
            light: None,
            fog_volume: None,
        }
    }
}

/// Seznam anim-setů připojených k entitě.
/// Runtime v core_drawable z nich bude později vyhledávat clipy podle názvu.
#[derive(Component, Debug, Clone, Default)]
pub struct AttachedAnimSets {
    pub sets: Vec<String>,
}

/// Lua-řízené zapnutí/vypnutí kolizí entity.
/// Fyzikální backend (Avian) reaguje na `Changed<CollisionEnabled>` a
/// přidává/odebírá `ColliderDisabled` na všech potomcích s kolizemi.
#[derive(Component, Debug, Clone, Copy)]
pub struct CollisionEnabled(pub bool);

/// Per-entitní materiálové přepisy nastavované z Lua přes `World.SetMaterialParam`.
/// `core_drawable` je aplikuje na mesh potomky v systému `apply_material_overrides`.
/// Pole `None` = neměnit daný parametr.
#[derive(Component, Debug, Clone, Default)]
pub struct LuaMaterialOverride {
    /// Množství sněhu (0.0–1.0) — `DrawableParams.weather.x`.
    pub snow_level: Option<f32>,
    /// Množství nečistot (0.0–1.0) — `DrawableParams.weather.y`.
    pub dirt_level: Option<f32>,
    /// Vlhkost povrchu (0.0–1.0) — `DrawableParams.weather.z`.
    pub wetness: Option<f32>,
    /// World-space Y cutoff pro sníh — pod touto hranicí sníh mizí (`flags.z`).
    /// 0.0 = bez omezení výšky (sníh všude kde je snow_level > 0).
    pub snow_height: Option<f32>,
    /// World-space Y cutoff pro vlhkost — nad touto hranicí vlhkost mizí (`flags.w`).
    /// 0.0 = bez omezení výšky (vlhkost všude kde je wetness > 0).
    pub wet_height: Option<f32>,
    /// Volitelný shader profile pro konkrétní entitu.
    /// Prázdný string = explicitní reset na default profil.
    pub shader_profile: Option<String>,
}

/// Mapa socketů dostupných na root entitě modelu.
/// Klíč je canonical socket name (např. `SOC_R_Hand_Weapon`), hodnota je ECS entita socket uzlu.
#[derive(Component, Debug, Clone, Default)]
pub struct AdsSocketMap(pub HashMap<String, Entity>);

/// Marker aktivního socket attachmentu pro debug a synchronní dotazy.
#[derive(Component, Debug, Clone)]
pub struct SocketAttachment {
    pub child_socket: String,
    pub parent_handle: u64,
    pub parent_socket: String,
}

/// Animační stav entity. Phase 4 propojí tuto komponentu s Bevy AnimationPlayer.
#[derive(Component, Debug, Clone)]
pub struct AnimationState {
    pub current: Option<String>,
    pub speed: f32,
    pub looping: bool,
    pub paused: bool,
    pub blend_time: f32,
    pub flags: u32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self { current: None, speed: 1.0, looping: true, paused: false, blend_time: 0.0, flags: 1 }
    }
}

/// Phase 4.2 — Marker: IK je povolen pro tuto entitu.
#[derive(Component, Debug, Copy, Clone, Reflect)]
pub struct IkEnabledComponent {
    pub blend_weight: f32,  // 0-1: jak moc se aplikuje IK vs originální pozice
}

impl Default for IkEnabledComponent {
    fn default() -> Self {
        Self {
            blend_weight: 1.0,
        }
    }
}

/// Phase 4.3 — Root Motion stav pro ADM entity.
/// Sleduje předchozí pozici root bonu a extrahuje delta pohyb z animace.
/// Typicky se vkládá na `AdmSceneRoot` entitu; systém `extract_root_motion`
/// pak aplikuje delta na parent entity (hráč, vozidlo, …).
#[derive(Component, Debug, Clone)]
pub struct RootMotionState {
    /// Jméno root bonu v `AdmNodeEntityMap`. Typicky "DEF_hips", "Root" nebo "Armature".
    pub root_bone_name: String,
    /// Světová pozice root bonu z minulého framu (None = první frame).
    pub prev_root_world_pos: Option<Vec3>,
    /// Delta pohybu vypočítaná v aktuálním framu — připravená k aplikaci na parent.
    pub accumulated_delta: Vec3,
    /// Pokud `true`, ignoruj Y-složku delty (gravitace řídí výšku hráče).
    pub lock_y: bool,
}

impl Default for RootMotionState {
    fn default() -> Self {
        Self {
            root_bone_name: "DEF_hips".to_string(),
            prev_root_world_pos: None,
            accumulated_delta: Vec3::ZERO,
            lock_y: true,
        }
    }
}

/// Runtime stav blend space — aktivní klípy a jejich váhy.
#[derive(Component, Debug, Clone)]
pub struct BlendSpaceState {
    pub blend_space_name: String,
    pub position: Vec2,  // 2D vektor (x, y) pro evaluaci
    /// Aktivní klípy a jejich váhy: (clip_name, weight)
    pub active_clips: Vec<(String, f32)>,
    pub speed: f32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcWanderKind {
    Random,
    Patrol,
    Orbit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NpcMoveGoal {
    Idle,
    GoToCoord {
        target: Vec3,
        stop_distance: f32,
    },
    GoToEntity {
        target_handle: u64,
        stop_distance: f32,
    },
    Wander {
        kind: NpcWanderKind,
        radius: f32,
        retarget_sec: f32,
        orbit_angular_speed: f32,
        patrol_point: Option<Vec3>,
        clockwise: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NpcPathWaypoint {
    pub target: Vec3,
}

#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplicatedNpcSteering {
    pub home: Vec3,
    pub wander_target: Vec3,
    pub wander_timer: f32,
    pub orbit_angle: f32,
    pub patrol_to_target: bool,
    pub current_path: Vec<NpcPathWaypoint>,
    pub waypoint_index: usize,
    pub map_id: String,
    pub last_nav_target: Option<Vec3>,
    pub entity_target_position: Option<Vec3>,
    pub entity_target_velocity: Vec3,
    pub formation_offset: Vec3,
    pub avoidance_offset: Vec3,
    pub avoidance_timer: f32,
}

impl Default for ReplicatedNpcSteering {
    fn default() -> Self {
        Self {
            home: Vec3::ZERO,
            wander_target: Vec3::ZERO,
            wander_timer: 0.0,
            orbit_angle: 0.0,
            patrol_to_target: true,
            current_path: Vec::new(),
            waypoint_index: 0,
            map_id: String::new(),
            last_nav_target: None,
            entity_target_position: None,
            entity_target_velocity: Vec3::ZERO,
            formation_offset: Vec3::ZERO,
            avoidance_offset: Vec3::ZERO,
            avoidance_timer: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcAiLodLevel {
    Full,
    Reduced,
    Background,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcAiLodState {
    pub level: NpcAiLodLevel,
}

impl Default for NpcAiLodState {
    fn default() -> Self {
        Self {
            level: NpcAiLodLevel::Full,
        }
    }
}

#[derive(Resource, Debug, Clone)]
pub struct NpcAiLodConfig {
    pub full_radius: f32,
    pub reduced_radius: f32,
    pub reduced_tick_interval: f32,
    pub full_budget_per_player: usize,
    pub reduced_budget_per_player: usize,
    pub zone_size: f32,
    pub full_budget_per_zone: usize,
    pub reduced_budget_per_zone: usize,
}

impl Default for NpcAiLodConfig {
    fn default() -> Self {
        Self {
            full_radius: 110.0,
            reduced_radius: NPC_OWNERSHIP_RELEASE_RADIUS,
            reduced_tick_interval: 0.25,
            full_budget_per_player: 24,
            reduced_budget_per_player: 48,
            zone_size: 160.0,
            full_budget_per_zone: 32,
            reduced_budget_per_zone: 72,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NpcAiLodConfigPatch {
    #[serde(default)]
    pub full_radius: Option<f32>,
    #[serde(default)]
    pub reduced_radius: Option<f32>,
    #[serde(default)]
    pub reduced_tick_interval: Option<f32>,
    #[serde(default)]
    pub full_budget_per_player: Option<usize>,
    #[serde(default)]
    pub reduced_budget_per_player: Option<usize>,
    #[serde(default)]
    pub zone_size: Option<f32>,
    #[serde(default)]
    pub full_budget_per_zone: Option<usize>,
    #[serde(default)]
    pub reduced_budget_per_zone: Option<usize>,
}

fn default_json_object() -> Json {
    Json::Object(JsonMap::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpcScenarioDef {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub task: Option<NpcTaskKind>,
    #[serde(default)]
    pub target_pos: Option<Vec3>,
    #[serde(default)]
    pub active_from_hour: Option<f32>,
    #[serde(default)]
    pub active_until_hour: Option<f32>,
    #[serde(default)]
    pub max_occupants: Option<usize>,
    #[serde(default)]
    pub lod_priority: u8,
    #[serde(default)]
    pub auto_assign: bool,
    #[serde(default)]
    pub assignment_radius: Option<f32>,
    #[serde(default)]
    pub release_distance: Option<f32>,
    #[serde(default)]
    pub required_tags: Vec<String>,
    #[serde(default)]
    pub preferred_brain_kind: Option<crate::npc_brain::NpcBrainKind>,
    #[serde(default = "default_json_object")]
    pub params: Json,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct NpcScenarioRegistry {
    pub defs: HashMap<String, NpcScenarioDef>,
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct NpcScenarioTime {
    pub hour_of_day: f32,
}

impl Default for NpcScenarioTime {
    fn default() -> Self {
        Self { hour_of_day: 12.0 }
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct NpcScenarioClockConfig {
    pub auto_advance: bool,
    pub day_length_seconds: f32,
}

impl Default for NpcScenarioClockConfig {
    fn default() -> Self {
        Self {
            auto_advance: true,
            day_length_seconds: 1200.0,
        }
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct EnvironmentLightConfig {
    pub enabled: bool,
    pub shadows_enabled: bool,
    pub color: [f32; 4],
    pub illuminance: f32,
    pub ambient_enabled: bool,
    pub ambient_color: [f32; 4],
    pub ambient_brightness: f32,
    pub hour_of_day: f32,
    pub azimuth_deg: f32,
    pub max_elevation_deg: f32,
    pub fog_enabled: bool,
    pub fog_color: [f32; 4],
    pub fog_directional_light_color: [f32; 4],
    pub fog_directional_light_exponent: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub fog_follow_streaming_boundary: bool,
    pub fog_boundary_inner_distance: f32,
    pub fog_boundary_outer_distance: f32,
    pub volumetric_fog_enabled: bool,
    pub volumetric_fog_ambient_color: [f32; 4],
    pub volumetric_fog_ambient_intensity: f32,
    pub volumetric_fog_jitter: f32,
    pub volumetric_fog_step_count: u32,
}

impl Default for EnvironmentLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            shadows_enabled: true,
            color: [1.0, 0.97, 0.92, 1.0],
            illuminance: 15000.0,
            ambient_enabled: false,
            ambient_color: [0.55, 0.60, 0.70, 1.0],
            ambient_brightness: 0.0,
            hour_of_day: 12.0,
            azimuth_deg: -45.0,
            max_elevation_deg: 75.0,
            fog_enabled: false,
            fog_color: [0.62, 0.70, 0.78, 0.55],
            fog_directional_light_color: [1.0, 0.94, 0.88, 0.30],
            fog_directional_light_exponent: 20.0,
            fog_start: 180.0,
            fog_end: 320.0,
            fog_follow_streaming_boundary: false,
            fog_boundary_inner_distance: 140.0,
            fog_boundary_outer_distance: 35.0,
            volumetric_fog_enabled: false,
            volumetric_fog_ambient_color: [0.55, 0.60, 0.70, 1.0],
            volumetric_fog_ambient_intensity: 0.06,
            volumetric_fog_jitter: 0.02,
            volumetric_fog_step_count: 48,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvironmentLightConfigPatch {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub shadows_enabled: Option<bool>,
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    #[serde(default)]
    pub illuminance: Option<f32>,
    #[serde(default)]
    pub ambient_enabled: Option<bool>,
    #[serde(default)]
    pub ambient_color: Option<[f32; 4]>,
    #[serde(default)]
    pub ambient_brightness: Option<f32>,
    #[serde(default)]
    pub hour_of_day: Option<f32>,
    #[serde(default)]
    pub azimuth_deg: Option<f32>,
    #[serde(default)]
    pub max_elevation_deg: Option<f32>,
    #[serde(default)]
    pub fog_enabled: Option<bool>,
    #[serde(default)]
    pub fog_color: Option<[f32; 4]>,
    #[serde(default)]
    pub fog_directional_light_color: Option<[f32; 4]>,
    #[serde(default)]
    pub fog_directional_light_exponent: Option<f32>,
    #[serde(default)]
    pub fog_start: Option<f32>,
    #[serde(default)]
    pub fog_end: Option<f32>,
    #[serde(default)]
    pub fog_follow_streaming_boundary: Option<bool>,
    #[serde(default)]
    pub fog_boundary_inner_distance: Option<f32>,
    #[serde(default)]
    pub fog_boundary_outer_distance: Option<f32>,
    #[serde(default)]
    pub volumetric_fog_enabled: Option<bool>,
    #[serde(default)]
    pub volumetric_fog_ambient_color: Option<[f32; 4]>,
    #[serde(default)]
    pub volumetric_fog_ambient_intensity: Option<f32>,
    #[serde(default)]
    pub volumetric_fog_jitter: Option<f32>,
    #[serde(default)]
    pub volumetric_fog_step_count: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NpcScenarioClockConfigPatch {
    #[serde(default)]
    pub auto_advance: Option<bool>,
    #[serde(default)]
    pub day_length_seconds: Option<f32>,
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct NpcPopulationDirectorConfig {
    pub default_assignment_radius: f32,
    pub release_distance_multiplier: f32,
    pub default_release_distance: f32,
}

impl Default for NpcPopulationDirectorConfig {
    fn default() -> Self {
        Self {
            default_assignment_radius: 96.0,
            release_distance_multiplier: 1.5,
            default_release_distance: 96.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NpcPopulationDirectorConfigPatch {
    #[serde(default)]
    pub default_assignment_radius: Option<f32>,
    #[serde(default)]
    pub release_distance_multiplier: Option<f32>,
    #[serde(default)]
    pub default_release_distance: Option<f32>,
}

impl NpcScenarioRegistry {
    pub fn upsert(&mut self, mut def: NpcScenarioDef) {
        if def.id.trim().is_empty() {
            return;
        }
        if !matches!(def.params, Json::Object(_)) {
            def.params = default_json_object();
        }
        self.defs.insert(def.id.clone(), def);
    }

    pub fn get(&self, scenario_id: &str) -> Option<&NpcScenarioDef> {
        self.defs.get(scenario_id)
    }
}

#[derive(SystemParam)]
pub struct NpcRuntimeRegistries<'w> {
    pub npc_brains: ResMut<'w, NpcBrainRegistry>,
    pub npc_scenarios: ResMut<'w, NpcScenarioRegistry>,
    pub npc_scenario_clock: ResMut<'w, NpcScenarioClockConfig>,
    pub npc_ai_lod: ResMut<'w, NpcAiLodConfig>,
    pub npc_population_director: ResMut<'w, NpcPopulationDirectorConfig>,
    pub environment_light: ResMut<'w, EnvironmentLightConfig>,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct NpcScenarioRuntimeState {
    pub active: bool,
    pub occupancy_granted: bool,
    pub occupancy_slot: Option<usize>,
    pub lod_priority: u8,
}

impl Default for NpcScenarioRuntimeState {
    fn default() -> Self {
        Self {
            active: true,
            occupancy_granted: true,
            occupancy_slot: None,
            lod_priority: 0,
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct NpcPopulationAssignment {
    pub scenario_id: String,
}

#[derive(Component, Debug, Clone)]
pub struct NpcAgent {
    pub move_speed: f32,
    pub arrive_distance: f32,
    pub turn_speed: f32,
    pub home: Vec3,
    pub goal: NpcMoveGoal,
    pub wander_target: Vec3,
    pub wander_timer: f32,
    pub orbit_angle: f32,
    pub patrol_to_target: bool,
    pub rng_state: u32,
    pub current_path: Vec<NpcPathWaypoint>,
    pub waypoint_index: usize,
    pub map_id: String,
    pub last_nav_target: Option<Vec3>,
    pub nav_repath_timer: f32,
    pub entity_target_position: Option<Vec3>,
    pub entity_target_velocity: Vec3,
    pub formation_offset: Vec3,
    pub avoidance_offset: Vec3,
    pub avoidance_timer: f32,
}

impl NpcAgent {
    pub fn new(handle: u64, home: Vec3) -> Self {
        let seed = (handle as u32)
            .wrapping_mul(747_796_405)
            .wrapping_add(2_891_336_453);
        Self {
            move_speed: 2.5,
            arrive_distance: 0.2,
            turn_speed: 10.0,
            home,
            goal: NpcMoveGoal::Idle,
            wander_target: home,
            wander_timer: 0.0,
            orbit_angle: 0.0,
            patrol_to_target: true,
            rng_state: seed,
            current_path: Vec::new(),
            waypoint_index: 0,
            map_id: String::new(),
            last_nav_target: None,
            nav_repath_timer: 0.0,
            entity_target_position: None,
            entity_target_velocity: Vec3::ZERO,
            formation_offset: Vec3::ZERO,
            avoidance_offset: Vec3::ZERO,
            avoidance_timer: 0.0,
        }
    }

    pub fn reset_navigation_state(&mut self) {
        self.current_path.clear();
        self.waypoint_index = 0;
        self.map_id.clear();
        self.last_nav_target = None;
        self.nav_repath_timer = 0.0;
        self.entity_target_position = None;
        self.entity_target_velocity = Vec3::ZERO;
        self.formation_offset = Vec3::ZERO;
        self.avoidance_offset = Vec3::ZERO;
        self.avoidance_timer = 0.0;
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.rng_state as f64 / (u32::MAX as f64)) as f32
    }
}

fn json_number(value: f32) -> Json {
    serde_json::Number::from_f64(value as f64)
        .map(Json::Number)
        .unwrap_or(Json::Null)
}

fn active_brain_id_for_entity(
    entity: Entity,
    npc_brain_states: &Query<&NpcBrainState>,
    npc_brains: &NpcBrainRegistry,
) -> String {
    npc_brain_states
        .get(entity)
        .map(|state| npc_brains.canonical_brain_id(&state.brain_id))
        .unwrap_or_else(|_| npc_brains.canonical_brain_id("core/human"))
}

fn brain_from_goal(brain_id: String, goal: &NpcMoveGoal) -> ReplicatedNpcBrain {
    match goal {
        NpcMoveGoal::Idle => ReplicatedNpcBrain::new(brain_id, NpcTaskKind::Idle),
        NpcMoveGoal::GoToCoord { target, stop_distance } => {
            let mut params = JsonMap::new();
            params.insert("stop_distance".to_string(), json_number(*stop_distance));
            ReplicatedNpcBrain::new(brain_id, NpcTaskKind::Investigate)
                .with_target(NpcBrainTarget::Position(*target))
                .with_params(Json::Object(params))
        }
        NpcMoveGoal::GoToEntity { target_handle, stop_distance } => {
            let mut params = JsonMap::new();
            params.insert("stop_distance".to_string(), json_number(*stop_distance));
            ReplicatedNpcBrain::new(brain_id, NpcTaskKind::FollowTarget)
                .with_target(NpcBrainTarget::Entity(*target_handle))
                .with_params(Json::Object(params))
        }
        NpcMoveGoal::Wander { kind, radius, retarget_sec, orbit_angular_speed, patrol_point, clockwise } => {
            let mut params = JsonMap::new();
            let kind_name = match kind {
                NpcWanderKind::Random => "random",
                NpcWanderKind::Patrol => "patrol",
                NpcWanderKind::Orbit => "orbit",
            };
            params.insert("wander_kind".to_string(), Json::String(kind_name.to_string()));
            params.insert("radius".to_string(), json_number(*radius));
            params.insert("retarget_sec".to_string(), json_number(*retarget_sec));
            params.insert("orbit_angular_speed".to_string(), json_number(*orbit_angular_speed));
            params.insert("clockwise".to_string(), Json::Bool(*clockwise));
            if let Some(point) = patrol_point {
                params.insert(
                    "patrol_point".to_string(),
                    Json::Array(vec![json_number(point.x), json_number(point.y), json_number(point.z)]),
                );
            }
            let task = match kind {
                NpcWanderKind::Random => NpcTaskKind::WanderZone,
                NpcWanderKind::Patrol => NpcTaskKind::PatrolRoute,
                NpcWanderKind::Orbit => NpcTaskKind::Ambient,
            };
            ReplicatedNpcBrain::new(brain_id, task).with_params(Json::Object(params))
        }
    }
}

fn decode_vec3_json(value: &Json) -> Option<Vec3> {
    let Json::Array(parts) = value else { return None; };
    if parts.len() != 3 {
        return None;
    }
    Some(Vec3::new(
        parts[0].as_f64()? as f32,
        parts[1].as_f64()? as f32,
        parts[2].as_f64()? as f32,
    ))
}

fn merge_json_objects(base: &Json, overlay: &Json) -> Json {
    let mut merged = match base {
        Json::Object(map) => map.clone(),
        _ => JsonMap::new(),
    };

    if let Json::Object(overlay_map) = overlay {
        for (key, value) in overlay_map {
            merged.insert(key.clone(), value.clone());
        }
    }

    Json::Object(merged)
}

fn apply_scenario_to_brain(
    scenario_registry: &NpcScenarioRegistry,
    brain: &ReplicatedNpcBrain,
) -> ReplicatedNpcBrain {
    let Some(scenario_id) = brain.scenario_id.as_deref() else {
        return brain.clone();
    };
    let Some(scenario) = scenario_registry.get(scenario_id) else {
        return brain.clone();
    };

    let mut effective = brain.clone();
    effective.params = merge_json_objects(&scenario.params, &effective.params);

    if matches!(effective.target, NpcBrainTarget::None) {
        if let Some(target_pos) = scenario.target_pos {
            effective.target = NpcBrainTarget::Position(target_pos);
        }
    }

    if matches!(effective.task, NpcTaskKind::UseScenarioPoint) {
        effective.task = scenario.task.unwrap_or(NpcTaskKind::Investigate);
    }

    effective
}

fn scenario_is_active(def: &NpcScenarioDef, hour_of_day: f32) -> bool {
    match (def.active_from_hour, def.active_until_hour) {
        (Some(start), Some(end)) => {
            let hour = hour_of_day.rem_euclid(24.0);
            let start = start.rem_euclid(24.0);
            let end = end.rem_euclid(24.0);
            if start <= end {
                hour >= start && hour <= end
            } else {
                hour >= start || hour <= end
            }
        }
        _ => true,
    }
}

fn npc_supports_scenario(
    brain_registry: &NpcBrainRegistry,
    brain_state: Option<&NpcBrainState>,
    brain: &ReplicatedNpcBrain,
    scenario: &NpcScenarioDef,
) -> bool {
    let brain_id = brain_state
        .map(|state| state.brain_id.as_str())
        .unwrap_or(brain.brain_id.as_str());
    let brain_def = brain_registry.resolve_or_fallback(brain_id);

    if let Some(required_kind) = scenario.preferred_brain_kind {
        if brain_def.kind != required_kind {
            return false;
        }
    }

    scenario.required_tags.iter().all(|required_tag| {
        brain_def
            .scenario_tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(required_tag))
    })
}

fn release_population_assignment(
    brain_registry: &NpcBrainRegistry,
    brain_state: Option<&NpcBrainState>,
    brain: &mut ReplicatedNpcBrain,
) {
    let brain_id = brain_state
        .map(|state| state.brain_id.as_str())
        .unwrap_or(brain.brain_id.as_str());
    let brain_def = brain_registry.resolve_or_fallback(brain_id);
    brain.scenario_id = None;
    brain.target = NpcBrainTarget::None;
    brain.task = brain_def.default_task;
    brain.params = default_json_object();
}

fn task_priority(task: NpcTaskKind) -> i32 {
    match task {
        NpcTaskKind::Combat => 90,
        NpcTaskKind::ChaseTarget => 80,
        NpcTaskKind::Flee => 75,
        NpcTaskKind::FollowTarget => 65,
        NpcTaskKind::Investigate => 55,
        NpcTaskKind::UseScenarioPoint => 35,
        NpcTaskKind::DriveRoute | NpcTaskKind::FlyRoute | NpcTaskKind::SwimRoute => 30,
        NpcTaskKind::PatrolRoute => 24,
        NpcTaskKind::WanderZone | NpcTaskKind::Ambient => 16,
        NpcTaskKind::Idle => 0,
    }
}

fn brain_kind_priority(kind: crate::npc_brain::NpcBrainKind) -> i32 {
    match kind {
        crate::npc_brain::NpcBrainKind::Human => 8,
        crate::npc_brain::NpcBrainKind::Vehicle => 7,
        crate::npc_brain::NpcBrainKind::Animal => 5,
        crate::npc_brain::NpcBrainKind::Bird => 3,
        crate::npc_brain::NpcBrainKind::Fish => 2,
    }
}

fn npc_lod_priority_score(
    brain_registry: &NpcBrainRegistry,
    scenario_runtime: Option<&NpcScenarioRuntimeState>,
    brain_state: Option<&NpcBrainState>,
    brain: &ReplicatedNpcBrain,
) -> i32 {
    let brain_id = brain_state
        .map(|state| state.brain_id.as_str())
        .unwrap_or(brain.brain_id.as_str());
    let def = brain_registry.resolve_or_fallback(brain_id);
    let scenario_bonus = scenario_runtime
        .map(|runtime| runtime.lod_priority as i32 * 20)
        .unwrap_or(0);
    let occupancy_bonus = scenario_runtime
        .map(|runtime| if runtime.occupancy_granted { 8 } else { -12 })
        .unwrap_or(0);

    scenario_bonus + occupancy_bonus + task_priority(brain.task) + brain_kind_priority(def.kind)
}

pub fn sync_npc_scenario_runtime(
    scenario_time: Res<NpcScenarioTime>,
    scenario_registry: Res<NpcScenarioRegistry>,
    mut commands: Commands,
    npcs: Query<(Entity, &ReplicatedNpcBrain, &Transform)>,
) {
    let mut occupants_by_scenario: HashMap<String, Vec<(Entity, f32)>> = HashMap::new();

    for (entity, brain, transform) in &npcs {
        let Some(scenario_id) = brain.scenario_id.as_deref() else {
            continue;
        };
        let Some(scenario) = scenario_registry.get(scenario_id) else {
            continue;
        };
        if let Some(max_occupants) = scenario.max_occupants {
            if max_occupants > 0 {
                let distance = scenario
                    .target_pos
                    .map(|target| target.distance(transform.translation))
                    .unwrap_or(0.0);
                occupants_by_scenario
                    .entry(scenario_id.to_string())
                    .or_default()
                    .push((entity, distance));
            }
        }
    }

    let mut occupancy_by_entity: HashMap<Entity, (bool, Option<usize>)> = HashMap::new();
    for (scenario_id, occupants) in &mut occupants_by_scenario {
        let Some(scenario) = scenario_registry.get(scenario_id) else {
            continue;
        };
        let Some(max_occupants) = scenario.max_occupants else {
            continue;
        };
        occupants.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (idx, (entity, _)) in occupants.iter().enumerate() {
            occupancy_by_entity.insert(*entity, (idx < max_occupants, (idx < max_occupants).then_some(idx)));
        }
    }

    for (entity, brain, _) in &npcs {
        let mut runtime = NpcScenarioRuntimeState::default();
        if let Some(scenario_id) = brain.scenario_id.as_deref() {
            if let Some(scenario) = scenario_registry.get(scenario_id) {
                runtime.active = scenario_is_active(scenario, scenario_time.hour_of_day);
                runtime.lod_priority = scenario.lod_priority;
                if scenario.max_occupants.unwrap_or(0) > 0 {
                    let (granted, slot) = occupancy_by_entity
                        .get(&entity)
                        .copied()
                        .unwrap_or((false, None));
                    runtime.occupancy_granted = granted;
                    runtime.occupancy_slot = slot;
                }
            }
        }
        commands.entity(entity).insert(runtime);
    }
}

pub fn advance_npc_scenario_time(
    time: Res<Time>,
    side: Res<ResourcesSide>,
    clock: Res<NpcScenarioClockConfig>,
    mut scenario_time: ResMut<NpcScenarioTime>,
) {
    if !matches!(side.0, Side::Server) || !clock.auto_advance {
        return;
    }
    let delta_hours = time.delta_secs() * (24.0 / clock.day_length_seconds.max(1.0));
    scenario_time.hour_of_day = (scenario_time.hour_of_day + delta_hours).rem_euclid(24.0);
}

pub fn run_npc_population_director(
    side: Res<ResourcesSide>,
    scenario_time: Res<NpcScenarioTime>,
    director_config: Res<NpcPopulationDirectorConfig>,
    scenario_registry: Res<NpcScenarioRegistry>,
    brain_registry: Res<NpcBrainRegistry>,
    mut commands: Commands,
    mut npcs: Query<(
        Entity,
        &Transform,
        &mut ReplicatedNpcBrain,
        Option<&NpcBrainState>,
        Option<&NpcPopulationAssignment>,
    )>,
) {
    if !matches!(side.0, Side::Server) {
        return;
    }

    let mut occupant_counts: HashMap<String, usize> = HashMap::new();
    for (_, _, brain, _, _) in &npcs {
        if let Some(scenario_id) = brain.scenario_id.as_ref() {
            *occupant_counts.entry(scenario_id.clone()).or_default() += 1;
        }
    }

    for (entity, transform, mut brain, brain_state, assignment) in &mut npcs {
        let Some(assignment) = assignment else {
            continue;
        };
        let should_release = match scenario_registry.get(&assignment.scenario_id) {
            Some(scenario) => {
                let active = scenario_is_active(scenario, scenario_time.hour_of_day);
                let in_range = scenario.target_pos.map(|target| {
                    let release_distance = scenario
                        .release_distance
                        .or(scenario.assignment_radius.map(|radius| radius * director_config.release_distance_multiplier))
                        .unwrap_or(director_config.default_release_distance);
                    target.distance(transform.translation) <= release_distance
                }).unwrap_or(true);
                !active || !in_range || !npc_supports_scenario(&brain_registry, brain_state, &brain, scenario)
            }
            None => true,
        };

        if should_release {
            if let Some(count) = occupant_counts.get_mut(&assignment.scenario_id) {
                *count = count.saturating_sub(1);
            }
            release_population_assignment(&brain_registry, brain_state, &mut brain);
            commands.entity(entity).remove::<NpcPopulationAssignment>();
        }
    }

    let mut scenarios_to_fill: Vec<&NpcScenarioDef> = scenario_registry
        .defs
        .values()
        .filter(|scenario| {
            scenario.auto_assign
                && scenario.target_pos.is_some()
                && scenario_is_active(scenario, scenario_time.hour_of_day)
                && occupant_counts.get(&scenario.id).copied().unwrap_or(0)
                    < scenario.max_occupants.unwrap_or(1)
        })
        .collect();
    scenarios_to_fill.sort_by(|a, b| b.lod_priority.cmp(&a.lod_priority));

    for scenario in scenarios_to_fill {
        let target = scenario.target_pos.unwrap_or(Vec3::ZERO);
        let max_occupants = scenario.max_occupants.unwrap_or(1);

        while occupant_counts.get(&scenario.id).copied().unwrap_or(0) < max_occupants {
            let mut best_candidate: Option<(Entity, f32)> = None;

            for (entity, transform, brain, brain_state, assignment) in &npcs {
                if assignment.is_some() || brain.scenario_id.is_some() {
                    continue;
                }
                if !matches!(brain.task, NpcTaskKind::Idle | NpcTaskKind::Ambient) {
                    continue;
                }
                if !npc_supports_scenario(&brain_registry, brain_state, brain, scenario) {
                    continue;
                }
                let distance = target.distance(transform.translation);
                let assignment_radius = scenario
                    .assignment_radius
                    .unwrap_or(director_config.default_assignment_radius);
                if distance > assignment_radius {
                    continue;
                }
                match best_candidate {
                    Some((_, best_distance)) if distance >= best_distance => {}
                    _ => best_candidate = Some((entity, distance)),
                }
            }

            let Some((entity, _)) = best_candidate else {
                break;
            };

            if let Ok((_, _, mut brain, _, _)) = npcs.get_mut(entity) {
                brain.task = NpcTaskKind::UseScenarioPoint;
                brain.scenario_id = Some(scenario.id.clone());
                brain.target = NpcBrainTarget::None;
                brain.params = default_json_object();
                commands.entity(entity).insert(NpcPopulationAssignment {
                    scenario_id: scenario.id.clone(),
                });
                *occupant_counts.entry(scenario.id.clone()).or_default() += 1;
            } else {
                break;
            }
        }
    }
}

fn brain_to_goal(brain: &ReplicatedNpcBrain) -> NpcMoveGoal {
    let stop_distance = brain
        .params
        .get("stop_distance")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.35);

    match brain.task {
        NpcTaskKind::Idle => NpcMoveGoal::Idle,
        NpcTaskKind::Ambient | NpcTaskKind::WanderZone | NpcTaskKind::PatrolRoute => {
            let kind = match brain.params.get("wander_kind").and_then(|v| v.as_str()) {
                Some("patrol") => NpcWanderKind::Patrol,
                Some("orbit") => NpcWanderKind::Orbit,
                _ => NpcWanderKind::Random,
            };
            let patrol_point = brain.params.get("patrol_point").and_then(decode_vec3_json);
            NpcMoveGoal::Wander {
                kind,
                radius: brain.params.get("radius").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(4.0),
                retarget_sec: brain.params.get("retarget_sec").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(2.5),
                orbit_angular_speed: brain.params.get("orbit_angular_speed").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(0.8),
                patrol_point,
                clockwise: brain.params.get("clockwise").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        }
        NpcTaskKind::UseScenarioPoint
        | NpcTaskKind::Investigate
        | NpcTaskKind::DriveRoute
        | NpcTaskKind::FlyRoute
        | NpcTaskKind::SwimRoute
        | NpcTaskKind::Flee => match &brain.target {
            NpcBrainTarget::Position(target) => NpcMoveGoal::GoToCoord {
                target: *target,
                stop_distance,
            },
            NpcBrainTarget::Entity(target_handle) => NpcMoveGoal::GoToEntity {
                target_handle: *target_handle,
                stop_distance,
            },
            NpcBrainTarget::None => NpcMoveGoal::Idle,
        },
        NpcTaskKind::FollowTarget | NpcTaskKind::ChaseTarget | NpcTaskKind::Combat => match &brain.target {
            NpcBrainTarget::Entity(target_handle) => NpcMoveGoal::GoToEntity {
                target_handle: *target_handle,
                stop_distance: stop_distance.max(1.1),
            },
            NpcBrainTarget::Position(target) => NpcMoveGoal::GoToCoord {
                target: *target,
                stop_distance,
            },
            NpcBrainTarget::None => NpcMoveGoal::Idle,
        },
    }
}

pub fn apply_replicated_npc_brain(
    brain_registry: &NpcBrainRegistry,
    scenario_registry: &NpcScenarioRegistry,
    brain: &ReplicatedNpcBrain,
    state: &mut NpcBrainState,
    agent: &mut NpcAgent,
) {
    let canonical_brain_id = brain_registry.canonical_brain_id(&brain.brain_id);
    let def = brain_registry.resolve_or_fallback(&canonical_brain_id);
    let mut effective_brain = apply_scenario_to_brain(scenario_registry, brain);
    let effective_task = if def.allowed_tasks.contains(&effective_brain.task) {
        effective_brain.task
    } else {
        def.default_task
    };

    state.brain_id = canonical_brain_id.clone();
    agent.move_speed = def.motion.cruise_speed.max(0.0);
    agent.turn_speed = def.motion.turn_speed.max(0.0);
    agent.arrive_distance = def.motion.brake_distance.max(0.01);

    effective_brain.brain_id = canonical_brain_id;
    effective_brain.task = effective_task;
    let desired_goal = brain_to_goal(&effective_brain);
    if agent.goal != desired_goal {
        agent.goal = desired_goal;
        agent.reset_navigation_state();
    }
}

pub fn snapshot_npc_steering(agent: &NpcAgent) -> ReplicatedNpcSteering {
    ReplicatedNpcSteering {
        home: agent.home,
        wander_target: agent.wander_target,
        wander_timer: agent.wander_timer,
        orbit_angle: agent.orbit_angle,
        patrol_to_target: agent.patrol_to_target,
        current_path: agent.current_path.clone(),
        waypoint_index: agent.waypoint_index,
        map_id: agent.map_id.clone(),
        last_nav_target: agent.last_nav_target,
        entity_target_position: agent.entity_target_position,
        entity_target_velocity: agent.entity_target_velocity,
        formation_offset: agent.formation_offset,
        avoidance_offset: agent.avoidance_offset,
        avoidance_timer: agent.avoidance_timer,
    }
}

pub fn apply_replicated_npc_steering(agent: &mut NpcAgent, steering: &ReplicatedNpcSteering) {
    agent.home = steering.home;
    agent.wander_target = steering.wander_target;
    agent.wander_timer = steering.wander_timer;
    agent.orbit_angle = steering.orbit_angle;
    agent.patrol_to_target = steering.patrol_to_target;
    agent.current_path = steering.current_path.clone();
    agent.waypoint_index = steering.waypoint_index.min(agent.current_path.len());
    agent.map_id = steering.map_id.clone();
    agent.last_nav_target = steering.last_nav_target;
    agent.entity_target_position = steering.entity_target_position;
    agent.entity_target_velocity = steering.entity_target_velocity;
    agent.formation_offset = steering.formation_offset;
    agent.avoidance_offset = steering.avoidance_offset;
    agent.avoidance_timer = steering.avoidance_timer;
}

fn npc_zone_key(position: Vec3, zone_size: f32) -> (i32, i32) {
    let size = zone_size.max(1.0);
    (
        (position.x / size).floor() as i32,
        (position.z / size).floor() as i32,
    )
}

fn lod_allows_tick(
    handle: u64,
    level: NpcAiLodLevel,
    config: &NpcAiLodConfig,
    elapsed_secs: f32,
    fixed_dt: f32,
) -> bool {
    match level {
        NpcAiLodLevel::Full => true,
        NpcAiLodLevel::Background => false,
        NpcAiLodLevel::Reduced => {
            let interval_ticks = (config.reduced_tick_interval / fixed_dt)
                .round()
                .max(1.0) as u64;
            let tick = (elapsed_secs / fixed_dt).round().max(0.0) as u64;
            tick % interval_ticks == handle % interval_ticks
        }
    }
}

pub fn sync_npc_brains_to_agents(
    brain_registry: Res<NpcBrainRegistry>,
    scenario_registry: Res<NpcScenarioRegistry>,
    mut npcs: Query<(&ReplicatedNpcBrain, Option<&NpcScenarioRuntimeState>, &mut NpcBrainState, &mut NpcAgent)>,
) {
    for (brain, runtime, mut state, mut agent) in &mut npcs {
        let mut effective_brain = brain.clone();
        if let Some(runtime) = runtime {
            if !runtime.active || !runtime.occupancy_granted {
                effective_brain.task = NpcTaskKind::Idle;
                effective_brain.target = NpcBrainTarget::None;
            }
        }
        apply_replicated_npc_brain(&brain_registry, &scenario_registry, &effective_brain, &mut state, &mut agent);
    }
}

/// Vlastník NPC — client_id hráče, který simuluje toto NPC.
/// `None` = žádný hráč v okolí, NPC je zmrazeno.
/// Replikováno klientům přes lightyear.
#[derive(Component, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NpcOwner(pub Option<u64>);

/// Server-side lease metadata pro ownership handoff.
/// Nereplikuje se; slouží jen k hysteresis a cooldown rozhodování.
#[derive(Component, Debug, Clone)]
pub struct NpcOwnershipLease {
    pub last_owner: Option<u64>,
    pub last_handoff_at: f32,
}

impl Default for NpcOwnershipLease {
    fn default() -> Self {
        Self {
            last_owner: None,
            last_handoff_at: -10_000.0,
        }
    }
}

/// Poslední validní client-owned transform update, který server přijal.
/// Slouží pro fallback na server simulaci, pokud owner umlkne.
#[derive(Component, Debug, Clone)]
pub struct NpcLastClientUpdate {
    pub client_id: u64,
    pub received_at: f32,
}

impl Default for NpcLastClientUpdate {
    fn default() -> Self {
        Self {
            client_id: 0,
            received_at: -10_000.0,
        }
    }
}

const NPC_OWNERSHIP_RADIUS: f32 = 200.0;
const NPC_OWNERSHIP_RELEASE_RADIUS: f32 = 230.0;
const NPC_OWNERSHIP_HANDOFF_ADVANTAGE: f32 = 20.0;
const NPC_OWNERSHIP_HANDOFF_COOLDOWN: f32 = 6.0;
const NPC_OWNERSHIP_ASSIGN_INTERVAL: f32 = 2.0;
pub const NPC_CLIENT_UPDATE_TIMEOUT_SECS: f32 = 5.0;

/// Přiřazuje vlastnictví NPC nejbližšímu hráči v `NPC_OWNERSHIP_RADIUS`.
/// Spouštěno periodicky (každé 2 s) pouze na serveru; na klientovi query
/// vrátí 0 výsledků, protože NpcAgent není replikovaný.
///
/// Logika:
/// - NPC s platným ownerem (hráč existuje a je v dosahu) → beze změny.
/// - NPC bez ownera nebo s ownerem co vypadl (odpojil se / opustil dosah)
///   → hledáme nejbližšího hráče; pokud nikdo není, NPC zůstane/bude zmrazeno.
pub fn assign_npc_owners(
    time: Res<Time>,
    lod_config: Res<NpcAiLodConfig>,
    mut timer: Local<f32>,
    brain_registry: Res<NpcBrainRegistry>,
    mut npcs: Query<(
        Entity,
        &Transform,
        &ReplicatedNpcBrain,
        Option<&NpcScenarioRuntimeState>,
        Option<&NpcBrainState>,
        &mut NpcOwner,
        &mut NpcOwnershipLease,
        &mut NpcAiLodState,
    ), With<NpcAgent>>,
    players: Query<(&NetTransform, &PlayerMarker)>,
) {
    *timer += time.delta_secs();
    if *timer < NPC_OWNERSHIP_ASSIGN_INTERVAL {
        return;
    }
    *timer = 0.0;

    let now = time.elapsed_secs();
    let player_entries: Vec<(u64, Vec3)> = players
        .iter()
        .map(|(tf, marker)| (marker.client_id, tf.translation))
        .collect();

    let mut desired_lod_by_entity: HashMap<Entity, NpcAiLodLevel> = HashMap::new();
    let mut controlling_player_by_entity: HashMap<Entity, u64> = HashMap::new();
    let mut full_candidates: HashMap<u64, Vec<(Entity, f32, i32)>> = HashMap::new();
    let mut active_candidates: HashMap<u64, Vec<(Entity, f32, i32)>> = HashMap::new();
    let mut zone_candidates: HashMap<(i32, i32), Vec<(Entity, f32, i32)>> = HashMap::new();

    for (entity, npc_tf, brain, scenario_runtime, brain_state, _owner, _lease, _lod_state) in &mut npcs {
        let nearest_player = player_entries
            .iter()
            .map(|(client_id, pos)| (*client_id, pos.distance(npc_tf.translation)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let priority = npc_lod_priority_score(&brain_registry, scenario_runtime, brain_state, brain);

        let base_lod = match nearest_player {
            Some((_, distance)) if distance <= lod_config.full_radius => NpcAiLodLevel::Full,
            Some((_, distance)) if distance <= lod_config.reduced_radius => NpcAiLodLevel::Reduced,
            _ => NpcAiLodLevel::Background,
        };
        desired_lod_by_entity.insert(entity, base_lod);

        if let Some((client_id, distance)) = nearest_player {
            if !matches!(base_lod, NpcAiLodLevel::Background) {
                controlling_player_by_entity.insert(entity, client_id);
                active_candidates.entry(client_id).or_default().push((entity, distance, priority));
                zone_candidates
                    .entry(npc_zone_key(npc_tf.translation, lod_config.zone_size))
                    .or_default()
                    .push((entity, distance, priority));
                if matches!(base_lod, NpcAiLodLevel::Full) {
                    full_candidates.entry(client_id).or_default().push((entity, distance, priority));
                }
            }
        }
    }

    for candidates in full_candidates.values_mut() {
        candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
        for (idx, (entity, _, _)) in candidates.iter().enumerate() {
            if idx >= lod_config.full_budget_per_player {
                desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Reduced);
            }
        }
    }

    let total_active_budget = lod_config
        .full_budget_per_player
        .saturating_add(lod_config.reduced_budget_per_player);
    for candidates in active_candidates.values_mut() {
        candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
        for (idx, (entity, _, _)) in candidates.iter().enumerate() {
            if idx >= total_active_budget {
                desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Background);
            }
        }
    }

    let total_zone_budget = lod_config
        .full_budget_per_zone
        .saturating_add(lod_config.reduced_budget_per_zone);
    for candidates in zone_candidates.values_mut() {
        candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
        for (idx, (entity, _, _)) in candidates.iter().enumerate() {
            if idx >= total_zone_budget {
                desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Background);
            } else if idx >= lod_config.full_budget_per_zone {
                if matches!(desired_lod_by_entity.get(entity), Some(NpcAiLodLevel::Full)) {
                    desired_lod_by_entity.insert(*entity, NpcAiLodLevel::Reduced);
                }
            }
        }
    }

    for (entity, npc_tf, _brain, _scenario_runtime, _brain_state, mut owner, mut lease, mut lod_state) in &mut npcs {
        lod_state.level = desired_lod_by_entity
            .get(&entity)
            .copied()
            .unwrap_or(NpcAiLodLevel::Background);

        if matches!(lod_state.level, NpcAiLodLevel::Background) {
            owner.0 = None;
            continue;
        }

        let assigned_player = controlling_player_by_entity.get(&entity).copied();
        let current_owner_distance = owner.0.and_then(|owner_id| {
            player_entries.iter()
                .find_map(|(client_id, pos)| {
                    if *client_id == owner_id {
                        Some(pos.distance(npc_tf.translation))
                    } else {
                        None
                    }
                })
        });

        let nearest = assigned_player.and_then(|client_id| {
            player_entries.iter().find_map(|(candidate_id, pos)| {
                if *candidate_id != client_id {
                    return None;
                }
                let dist = pos.distance(npc_tf.translation);
                if dist <= NPC_OWNERSHIP_RADIUS {
                    Some((dist, client_id))
                } else {
                    None
                }
            })
        });

        let owner_still_valid = current_owner_distance
            .map(|distance| distance <= NPC_OWNERSHIP_RELEASE_RADIUS)
            .unwrap_or(false);
        let cooldown_active = (now - lease.last_handoff_at) < NPC_OWNERSHIP_HANDOFF_COOLDOWN;

        let new_owner = if owner_still_valid {
            match (owner.0, current_owner_distance, nearest) {
                (Some(current_owner), Some(current_dist), Some((candidate_dist, candidate_id)))
                    if candidate_id != current_owner
                        && !cooldown_active
                        && candidate_dist + NPC_OWNERSHIP_HANDOFF_ADVANTAGE < current_dist =>
                {
                    Some(candidate_id)
                }
                (current_owner, _, _) => current_owner,
            }
        } else {
            nearest.map(|(_, id)| id)
        };

        if owner.0 != new_owner {
            if let Some(id) = new_owner {
                debug!(
                    "[npc_owner] NPC at {:?} → client {} (prev={:?}, cooldown_active={})",
                    npc_tf.translation,
                    id,
                    owner.0,
                    cooldown_active
                );
            } else {
                debug!("[npc_owner] NPC at {:?} → frozen (no player nearby)", npc_tf.translation);
            }
            lease.last_owner = owner.0;
            lease.last_handoff_at = now;
            owner.0 = new_owner;
        }
    }
}

/// Message emitovaná při zpracování `World.ApplyDamage`.
/// Phase 3.3 combat systémy ji čtou přes `MessageReader<PendingDamageEvent>`.
#[derive(Message, Debug, Clone)]
pub struct PendingDamageEvent {
    pub target: Entity,
    pub amount: f32,
    pub source: Option<Entity>,
}

// ---------------------------------------------------------------------------
// EntitySnapshot + EntityStateCache — synchronní čtení entity stavu z Lua
// ---------------------------------------------------------------------------

/// Snapshot stavu jedné Lua entity pro synchronní čtení z Lua sandboxu.
/// Aktualizováno každý frame systémem `sync_entity_state_cache`.
#[derive(Debug, Clone, Default)]
pub struct EntitySnapshot {
    pub pos: [f32; 3],
    /// Kvaternion [x, y, z, w].
    pub rot: [f32; 4],
    pub scale: [f32; 3],
    pub model: Option<String>,
    /// `true` pokud entita existuje a health > 0 (nebo nemá Health komponentu).
    pub alive: bool,
    pub health: Option<f32>,
    pub max_health: Option<f32>,
    pub animation: Option<String>,
    pub anim_speed: f32,
    pub anim_looping: bool,
    pub anim_paused: bool,
    /// `true` pokud je entita síťová (má `NetworkedObjectMarker`).
    pub is_networked: bool,
    /// `true` pokud má entita aktivní kolizi (výchozí `true` bez `CollisionEnabled`).
    pub collision_enabled: bool,
    /// Runtime socket transforms ve world-space.
    pub sockets: HashMap<String, SocketTransformSnapshot>,
}

/// Snapshot jednoho socketu pro synchronní čtení z Lua.
#[derive(Debug, Clone, Default)]
pub struct SocketTransformSnapshot {
    pub pos: [f32; 3],
    /// Kvaternion [x, y, z, w].
    pub rot: [f32; 4],
}

/// Sdílená cache stavu entit — handle → EntitySnapshot.
/// Aktualizovaná systémem `sync_entity_state_cache` v PostUpdate.
/// Lua sandbox čte synchronně bez latence.
#[derive(Resource, Clone, Default)]
pub struct EntityStateCache(pub Arc<Mutex<HashMap<u64, EntitySnapshot>>>);

impl EntityStateCache {
    pub fn get(&self, handle: u64) -> Option<EntitySnapshot> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).get(&handle).cloned()
    }

    pub fn is_valid(&self, handle: u64) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&handle)
    }

    /// Vrátí všechny handles entit, které aktuálně používají daný model.
    pub fn handles_by_model(&self, model_name: &str) -> Vec<u64> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(|(handle, snapshot)| {
                if snapshot.model.as_deref() == Some(model_name) {
                    Some(*handle)
                } else {
                    None
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Phase 4 — Stats, Inventory, PlayerEntityMap, PlayerStatsCache
// ---------------------------------------------------------------------------

/// Herní statistiky hráče (XP, gold, level, …). Definice hodnot plně
/// v Lua — Rust drží jen HashMap<String, f64>.
#[derive(Component, Debug, Clone, Default)]
pub struct Stats(pub HashMap<String, f64>);

/// Inventář hráče — item_id → počet kusů.
#[derive(Component, Debug, Clone, Default)]
pub struct Inventory(pub HashMap<String, u32>);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EquippedWeapon {
    pub weapon_id: String,
    pub ammo_in_mag: u32,
    pub ammo_type_id: String,
    pub fire_mode: String,
    #[serde(default)]
    pub attachments: Vec<String>,
}

#[derive(Component, Debug, Clone)]
pub struct WeaponSlots(pub [Option<EquippedWeapon>; 4]);

impl Default for WeaponSlots {
    fn default() -> Self {
        Self([None, None, None, None])
    }
}

impl WeaponSlots {
    pub fn get(&self, slot: u8) -> Option<&EquippedWeapon> {
        self.0.get(slot as usize).and_then(|entry| entry.as_ref())
    }

    pub fn get_mut(&mut self, slot: u8) -> Option<&mut EquippedWeapon> {
        self.0.get_mut(slot as usize).and_then(|entry| entry.as_mut())
    }

    pub fn set(&mut self, slot: u8, equipped: Option<EquippedWeapon>) -> bool {
        let Some(dst) = self.0.get_mut(slot as usize) else {
            return false;
        };
        *dst = equipped;
        true
    }
}

#[derive(Component, Debug, Clone, Default)]
pub struct AmmoReserve(pub HashMap<String, u32>);

#[derive(Component, Debug, Clone, Default)]
pub struct ActiveWeaponSlot(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmorClass {
    I,
    Ii,
    Iiia,
    Iii,
    Iv,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmorPiece {
    pub class: ArmorClass,
    pub durability: f32,
    pub max_durability: f32,
}

impl ArmorPiece {
    pub fn clamped(mut self) -> Self {
        self.max_durability = self.max_durability.max(0.0);
        self.durability = self.durability.clamp(0.0, self.max_durability);
        self
    }
}

#[derive(Component, Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArmorComponent {
    pub helmet: Option<ArmorPiece>,
    pub vest: Option<ArmorPiece>,
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct PlayerHitbox(pub String);

impl Default for PlayerHitbox {
    fn default() -> Self {
        Self("player_default".to_string())
    }
}

#[derive(Component, Debug, Clone, Default)]
pub struct FireState {
    pub cooldown_remaining: f32,
    pub shot_interval: f32,
    pub trigger_held: bool,
}

#[derive(Component, Debug, Clone, Default)]
pub struct ReloadState {
    pub remaining: f32,
    pub duration: f32,
    pub slot: u8,
}

#[derive(Component, Debug, Clone, Default)]
pub struct WeaponSwapState {
    pub remaining: f32,
    pub duration: f32,
    pub target_slot: u8,
}

const DEFAULT_WEAPON_SWAP_SECS: f32 = 0.25;
const MIN_WEAPON_ACTION_SECS: f32 = 0.05;

fn weapon_swap_duration_for_slots(
    weapon_registry: &WeaponRegistry,
    slots: &WeaponSlots,
    active_slot: u8,
    target_slot: u8,
) -> f32 {
    slots
        .get(target_slot)
        .and_then(|weapon| weapon_registry.get(&weapon.weapon_id))
        .map(|weapon| weapon.ads_time_sec)
        .filter(|duration| *duration > 0.0)
        .or_else(|| {
            slots.get(active_slot)
                .and_then(|weapon| weapon_registry.get(&weapon.weapon_id))
                .map(|weapon| weapon.ads_time_sec)
                .filter(|duration| *duration > 0.0)
        })
        .unwrap_or(DEFAULT_WEAPON_SWAP_SECS)
        .max(MIN_WEAPON_ACTION_SECS)
}

fn reload_duration_for_slot(
    weapon_registry: &WeaponRegistry,
    slots: &WeaponSlots,
    reserve: &AmmoReserve,
    active_slot: u8,
) -> Option<f32> {
    let equipped = slots.get(active_slot)?;
    let weapon_def = weapon_registry.get(&equipped.weapon_id)?;
    let mag_capacity = weapon_def.mag_capacity;
    if mag_capacity == 0 || equipped.ammo_in_mag >= mag_capacity {
        return None;
    }

    let ammo_id = if equipped.ammo_type_id.trim().is_empty() {
        weapon_def.default_ammo.clone()
    } else {
        equipped.ammo_type_id.clone()
    };
    if ammo_id.trim().is_empty() || reserve.0.get(&ammo_id).copied().unwrap_or(0) == 0 {
        return None;
    }

    let duration = if equipped.ammo_in_mag == 0 {
        weapon_def.reload_empty_sec
    } else {
        weapon_def.reload_tactical_sec
    };

    Some(duration.max(MIN_WEAPON_ACTION_SECS))
}

/// Snapshot stavu hráče — synchronizovaný každý FixedUpdate tick
/// systémem `sync_stats_cache` v `core_net/sim.rs`.
/// Lua sandbox ho čte synchronně (bez latence).
#[derive(Debug, Clone, Default)]
pub struct StatsSnapshot {
    pub stats: HashMap<String, f64>,
    pub inventory: HashMap<String, u32>,
    pub health: f32,
    pub max_health: f32,
    pub armor: ArmorComponent,
    pub weapon_slots: Vec<Option<EquippedWeapon>>,
    pub ammo_reserve: HashMap<String, u32>,
    pub active_weapon_slot: u8,
    pub fire_cooldown_remaining: f32,
    pub fire_trigger_held: bool,
    pub reload_remaining: f32,
    pub reload_duration: f32,
    pub weapon_swap_remaining: f32,
    pub weapon_swap_duration: f32,
    pub weapon_swap_target_slot: Option<u8>,
}

/// Sdílená cache: client_id → StatsSnapshot. Arc<Mutex> umožňuje
/// Lua closurám číst bez přístupu do ECS světa.
#[derive(Resource, Clone, Default)]
pub struct PlayerStatsCache(pub Arc<Mutex<HashMap<u64, StatsSnapshot>>>);

impl PlayerStatsCache {
    pub fn update(&self, client_id: u64, snapshot: StatsSnapshot) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(client_id, snapshot);
    }

    pub fn remove(&self, client_id: u64) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&client_id);
    }

    pub fn get(&self, client_id: u64) -> Option<StatsSnapshot> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&client_id)
            .cloned()
    }

    pub fn retain_ids(&self, keep: &HashSet<u64>) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|client_id, _| keep.contains(client_id));
    }
}

/// Mapa client_id → Entity. Udržovaná observery v `core_net/sim.rs`
/// při Add/Remove PlayerMarker. Umožňuje `process_lua_commands` rychlé
/// vyhledání entity hráče bez iterace celého světa.
#[derive(Resource, Default)]
pub struct PlayerEntityMap {
    pub map: HashMap<u64, Entity>,
}

#[derive(SystemParam)]
pub struct PlayerRuntimeState<'w> {
    player_map: Res<'w, PlayerEntityMap>,
    weapon_registry: Res<'w, WeaponRegistry>,
}

/// Stats lokálního hráče na klientu — aktualizovány serverem přes `PlayerStatsUpdate`.
/// Arc<Mutex> sdíleno se sandbox closurami pro synchronní čtení z `Player.GetLocalStats()`.
#[derive(Resource, Clone, Default)]
pub struct LocalPlayerStats(pub Arc<Mutex<StatsSnapshot>>);

impl LocalPlayerStats {
    pub fn update_snapshot(&self, snapshot: StatsSnapshot) {
        let mut snap = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *snap = snapshot;
    }

    pub fn get(&self) -> StatsSnapshot {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

// ---------------------------------------------------------------------------
// Bevy systémy
// ---------------------------------------------------------------------------

/// Zpracuje všechny pending `LuaCommand`y.
/// Přidán do `PostUpdate`, aby měl k dispozici příkazy z celého Update frame.
pub fn process_lua_commands(
    cmd_queue: Res<CommandQueue>,
    mut world_state: ResMut<LuaWorldState>,
    mut npc_registries: NpcRuntimeRegistries,
    player_runtime: PlayerRuntimeState,
    npc_brain_states: Query<&NpcBrainState>,
    npc_replicated_brains: Query<&ReplicatedNpcBrain>,
    mut commands: Commands,
    mut damage_events: MessageWriter<PendingDamageEvent>,
    transforms: Query<&Transform>,
    globals: Query<&GlobalTransform>,
    socket_maps: Query<&AdsSocketMap>,
    anim_set_cmds: Res<AnimSetCommandQueue>,
    mut attached_anim_sets: Query<&mut AttachedAnimSets>,
    mut player_stats: Query<(
        &PlayerMarker,
        &mut Health,
        &mut Stats,
        &mut Inventory,
        &mut ArmorComponent,
        &mut WeaponSlots,
        &mut AmmoReserve,
        &mut ActiveWeaponSlot,
        &mut FireState,
        &mut ReloadState,
        &mut WeaponSwapState,
    )>,
    mut mat_overrides: Query<&mut LuaMaterialOverride>,
    mut npc_agents: Query<&mut NpcAgent>,
) {
    let mut pending_mat: HashMap<u64, LuaMaterialOverride> = HashMap::new();

    for cmd in cmd_queue.drain() {
        match cmd {
            LuaCommand::SpawnLocalObject { handle, model, pos, rot } => {
                let entity = commands
                    .spawn((
                        LocalObjectMarker { model: model.clone() },
                        ModelName(model.clone()),
                        EntityHandle(handle),
                        Transform {
                            translation: Vec3::new(pos[0], pos[1], pos[2]),
                            rotation: Quat::from_euler(
                                EulerRot::XYZ,
                                rot[0].to_radians(),
                                rot[1].to_radians(),
                                rot[2].to_radians(),
                            ),
                            scale: Vec3::ONE,
                        },
                    ))
                    .id();
                world_state.register(handle, entity);
                debug!(
                    "[cmd_queue] spawned local object '{}' (handle={}, entity={:?})",
                    model, handle, entity
                );
            }

            LuaCommand::DespawnEntity { handle } => {
                if let Some(entity) = world_state.remove(handle) {
                    commands.entity(entity).despawn();
                    debug!("[cmd_queue] despawned {:?} (handle={})", entity, handle);
                } else {
                    warn!("[cmd_queue] DespawnEntity: unknown handle {}", handle);
                }
            }

            LuaCommand::SetTransform { handle, pos, rot } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let scale = transforms.get(entity).map(|t| t.scale).unwrap_or(Vec3::ONE);
                    commands.entity(entity).insert(Transform {
                        translation: Vec3::new(pos[0], pos[1], pos[2]),
                        rotation: Quat::from_euler(
                            EulerRot::XYZ,
                            rot[0].to_radians(),
                            rot[1].to_radians(),
                            rot[2].to_radians(),
                        ),
                        scale,
                    });
                } else {
                    warn!("[cmd_queue] SetTransform: unknown handle {}", handle);
                }
            }

            LuaCommand::SetPosition { handle, pos } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let (rot, scale) = transforms.get(entity)
                        .map(|t| (t.rotation, t.scale))
                        .unwrap_or((Quat::IDENTITY, Vec3::ONE));
                    commands.entity(entity).insert(Transform {
                        translation: Vec3::new(pos[0], pos[1], pos[2]),
                        rotation: rot,
                        scale,
                    });
                } else {
                    warn!("[cmd_queue] SetPosition: unknown handle {}", handle);
                }
            }

            LuaCommand::SetRotation { handle, rot } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let (translation, scale) = transforms.get(entity)
                        .map(|t| (t.translation, t.scale))
                        .unwrap_or((Vec3::ZERO, Vec3::ONE));
                    commands.entity(entity).insert(Transform {
                        translation,
                        rotation: Quat::from_euler(
                            EulerRot::XYZ,
                            rot[0].to_radians(),
                            rot[1].to_radians(),
                            rot[2].to_radians(),
                        ),
                        scale,
                    });
                } else {
                    warn!("[cmd_queue] SetRotation: unknown handle {}", handle);
                }
            }

            LuaCommand::SetScale { handle, scale } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let (translation, rotation) = transforms.get(entity)
                        .map(|t| (t.translation, t.rotation))
                        .unwrap_or((Vec3::ZERO, Quat::IDENTITY));
                    commands.entity(entity).insert(Transform {
                        translation,
                        rotation,
                        scale: Vec3::new(scale[0], scale[1], scale[2]),
                    });
                } else {
                    warn!("[cmd_queue] SetScale: unknown handle {}", handle);
                }
            }

            LuaCommand::SetModel { handle, model } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(ModelName(model));
                } else {
                    warn!("[cmd_queue] SetModel: unknown handle {}", handle);
                }
            }

            LuaCommand::ApplyDamage { target_handle, amount, source_handle } => {
                match world_state.entity_for(target_handle) {
                    Some(target) => {
                        let source = source_handle
                            .and_then(|h| world_state.entity_for(h));
                        damage_events.write(PendingDamageEvent { target, amount, source });
                    }
                    None => warn!(
                        "[cmd_queue] ApplyDamage: unknown target handle {}",
                        target_handle
                    ),
                }
            }

            LuaCommand::SpawnNetworkedObject { handle, model, pos, rot } => {
                let entity = commands
                    .spawn((
                        NetworkedObjectMarker { model: model.clone() },
                        ModelName(model.clone()),
                        EntityHandle(handle),
                        Transform {
                            translation: Vec3::new(pos[0], pos[1], pos[2]),
                            rotation: Quat::from_euler(
                                EulerRot::XYZ,
                                rot[0].to_radians(),
                                rot[1].to_radians(),
                                rot[2].to_radians(),
                            ),
                            scale: Vec3::ONE,
                        },
                    ))
                    .id();
                world_state.register(handle, entity);
                info!(
                    "[cmd_queue] queued networked object '{}' (handle={}, entity={:?})",
                    model, handle, entity
                );
            }

            LuaCommand::SpawnNetworkedNpc { handle, model, pos, rot, ped_profile } => {
                let spawn_translation = Vec3::new(pos[0], pos[1], pos[2]);
                let spawn_rotation = Quat::from_euler(
                    EulerRot::XYZ,
                    rot[0].to_radians(),
                    rot[1].to_radians(),
                    rot[2].to_radians(),
                );
                let agent = NpcAgent::new(handle, spawn_translation);
                let steering = snapshot_npc_steering(&agent);
                let mut entity_builder = commands.spawn((
                    NpcPedMarker,
                    NetworkedObjectMarker { model: model.clone() },
                    ModelName(model.clone()),
                    EntityHandle(handle),
                    NpcOwner::default(),
                    NpcOwnershipLease::default(),
                    NpcAiLodState::default(),
                    NpcLastClientUpdate::default(),
                    NpcBrainState::default(),
                    ReplicatedNpcBrain::default(),
                    agent,
                    steering,
                    NetTransform {
                        translation: spawn_translation,
                        rotation: spawn_rotation,
                    },
                    Transform {
                        translation: spawn_translation,
                        rotation: spawn_rotation,
                        scale: Vec3::ONE,
                    },
                ));
                if let Some(profile) = ped_profile.clone() {
                    entity_builder.insert(PedProfileOverride(profile));
                }
                let entity = entity_builder.id();
                world_state.register(handle, entity);
                info!(
                    "[cmd_queue] queued networked npc '{}' (handle={}, entity={:?}, ped_profile={:?})",
                    model, handle, entity, ped_profile
                );
            }

            LuaCommand::SpawnLocalDummy { handle, def, pos, rot } => {
                let entity = commands
                    .spawn((
                        def,
                        EntityHandle(handle),
                        Transform {
                            translation: Vec3::new(pos[0], pos[1], pos[2]),
                            rotation: Quat::from_euler(
                                EulerRot::XYZ,
                                rot[0].to_radians(),
                                rot[1].to_radians(),
                                rot[2].to_radians(),
                            ),
                            scale: Vec3::ONE,
                        },
                    ))
                    .id();
                world_state.register(handle, entity);
                debug!(
                    "[cmd_queue] spawned local dummy (handle={}, entity={:?})",
                    handle, entity
                );
            }

            LuaCommand::SpawnNetworkedDummy { handle, def, pos, rot } => {
                let entity = commands
                    .spawn((
                        def,
                        NetworkedObjectMarker {
                            model: "__dummy__".to_string(),
                        },
                        EntityHandle(handle),
                        Transform {
                            translation: Vec3::new(pos[0], pos[1], pos[2]),
                            rotation: Quat::from_euler(
                                EulerRot::XYZ,
                                rot[0].to_radians(),
                                rot[1].to_radians(),
                                rot[2].to_radians(),
                            ),
                            scale: Vec3::ONE,
                        },
                    ))
                    .id();
                world_state.register(handle, entity);
                info!(
                    "[cmd_queue] queued networked dummy (handle={}, entity={:?})",
                    handle, entity
                );
            }

            LuaCommand::SpawnLocalCollider { handle, collider, pos, rot } => {
                let entity = commands
                    .spawn((
                        ColliderObjectMarker { collider },
                        EntityHandle(handle),
                        Transform {
                            translation: Vec3::new(pos[0], pos[1], pos[2]),
                            rotation: Quat::from_euler(
                                EulerRot::XYZ,
                                rot[0].to_radians(),
                                rot[1].to_radians(),
                                rot[2].to_radians(),
                            ),
                            scale: Vec3::ONE,
                        },
                    ))
                    .id();
                world_state.register(handle, entity);
                debug!(
                    "[cmd_queue] spawned local collider (handle={}, entity={:?})",
                    handle, entity
                );
            }

            LuaCommand::SpawnNetworkedCollider { handle, collider, pos, rot } => {
                let entity = commands
                    .spawn((
                        ColliderObjectMarker { collider },
                        NetworkedObjectMarker {
                            model: "__collider__".to_string(),
                        },
                        EntityHandle(handle),
                        Transform {
                            translation: Vec3::new(pos[0], pos[1], pos[2]),
                            rotation: Quat::from_euler(
                                EulerRot::XYZ,
                                rot[0].to_radians(),
                                rot[1].to_radians(),
                                rot[2].to_radians(),
                            ),
                            scale: Vec3::ONE,
                        },
                    ))
                    .id();
                world_state.register(handle, entity);
                info!(
                    "[cmd_queue] queued networked collider (handle={}, entity={:?})",
                    handle, entity
                );
            }

            LuaCommand::PlayAnimation { handle, name, looping, speed, blend_time, flags } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(AnimationState {
                        current: Some(name),
                        speed,
                        looping,
                        paused: false,
                        blend_time,
                        flags,
                    });
                } else {
                    warn!("[cmd_queue] PlayAnimation: unknown handle {}", handle);
                }
            }

            LuaCommand::ApplyAnimSet { handle, path } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    anim_set_cmds.push(crate::model_registry::AnimSetCommand::Request(path.clone()));
                    if let Ok(mut attached) = attached_anim_sets.get_mut(entity) {
                        if !attached.sets.iter().any(|existing| existing == &path) {
                            attached.sets.push(path);
                        }
                    } else {
                        commands.entity(entity).insert(AttachedAnimSets { sets: vec![path] });
                    }
                } else {
                    warn!("[cmd_queue] ApplyAnimSet: unknown handle {}", handle);
                }
            }

            LuaCommand::StopAnimation { handle } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(AnimationState {
                        current: None,
                        ..Default::default()
                    });
                } else {
                    warn!("[cmd_queue] StopAnimation: unknown handle {}", handle);
                }
            }

            LuaCommand::PlayBlendSpace { handle, blend_space_name, position, speed, flags } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(BlendSpaceState {
                        blend_space_name,
                        position: Vec2::new(position[0], position[1]),
                        active_clips: Vec::new(),  // Vyplní se v apply_adm_animations
                        speed,
                        flags,
                    });
                    // Také odstraníme AnimationState, pokud byl přítomen
                    commands.entity(entity).remove::<AnimationState>();
                } else {
                    warn!("[cmd_queue] PlayBlendSpace: unknown handle {}", handle);
                }
            }

            LuaCommand::EnableIk { handle, blend_weight } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(IkEnabledComponent {
                        blend_weight: blend_weight.clamp(0.0, 1.0),
                    });
                    debug!("[cmd_queue] EnableIk handle={} blend_weight={}", handle, blend_weight);
                } else {
                    warn!("[cmd_queue] EnableIk: unknown handle {}", handle);
                }
            }

            LuaCommand::DisableIk { handle } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).remove::<IkEnabledComponent>();
                    debug!("[cmd_queue] DisableIk handle={}", handle);
                } else {
                    warn!("[cmd_queue] DisableIk: unknown handle {}", handle);
                }
            }

            LuaCommand::EnableRootMotion { handle, root_bone_name, lock_y } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(RootMotionState {
                        root_bone_name: root_bone_name.unwrap_or_else(|| "DEF_hips".to_string()),
                        prev_root_world_pos: None,
                        accumulated_delta: Vec3::ZERO,
                        lock_y,
                    });
                    debug!("[cmd_queue] EnableRootMotion handle={} lock_y={}", handle, lock_y);
                } else {
                    warn!("[cmd_queue] EnableRootMotion: unknown handle {}", handle);
                }
            }

            LuaCommand::DisableRootMotion { handle } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).remove::<RootMotionState>();
                    debug!("[cmd_queue] DisableRootMotion handle={}", handle);
                } else {
                    warn!("[cmd_queue] DisableRootMotion: unknown handle {}", handle);
                }
            }

            LuaCommand::NpcConfigure {
                handle,
                move_speed,
                arrive_distance,
                turn_speed,
            } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    if let Ok(mut agent) = npc_agents.get_mut(entity) {
                        if let Some(v) = move_speed {
                            agent.move_speed = v.max(0.0);
                        }
                        if let Some(v) = arrive_distance {
                            agent.arrive_distance = v.max(0.01);
                        }
                        if let Some(v) = turn_speed {
                            agent.turn_speed = v.max(0.0);
                        }
                    } else {
                        let home = transforms
                            .get(entity)
                            .map(|t| t.translation)
                            .unwrap_or(Vec3::ZERO);
                        let mut agent = NpcAgent::new(handle, home);
                        if let Some(v) = move_speed {
                            agent.move_speed = v.max(0.0);
                        }
                        if let Some(v) = arrive_distance {
                            agent.arrive_distance = v.max(0.01);
                        }
                        if let Some(v) = turn_speed {
                            agent.turn_speed = v.max(0.0);
                        }
                        commands.entity(entity).insert(agent);
                    }
                } else {
                    warn!("[cmd_queue] NpcConfigure: unknown handle {}", handle);
                }
            }

            LuaCommand::NpcWander {
                handle,
                kind,
                radius,
                retarget_sec,
                orbit_angular_speed,
                patrol_point,
                clockwise,
            } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let home = transforms
                        .get(entity)
                        .map(|t| t.translation)
                        .unwrap_or(Vec3::ZERO);

                    let mut goal = NpcMoveGoal::Wander {
                        kind,
                        radius: radius.max(0.1),
                        retarget_sec: retarget_sec.max(0.05),
                        orbit_angular_speed: orbit_angular_speed.max(0.05),
                        patrol_point: patrol_point
                            .map(|p| Vec3::new(p[0], p[1], p[2])),
                        clockwise,
                    };

                    if let Ok(mut agent) = npc_agents.get_mut(entity) {
                        if matches!(agent.goal, NpcMoveGoal::Idle) {
                            agent.home = home;
                        }
                        agent.goal = goal;
                        agent.wander_timer = 0.0;
                        agent.reset_navigation_state();
                        commands.entity(entity).insert(brain_from_goal(
                            active_brain_id_for_entity(entity, &npc_brain_states, &npc_registries.npc_brains),
                            &agent.goal,
                        ));
                    } else {
                        let mut agent = NpcAgent::new(handle, home);
                        agent.goal = std::mem::replace(&mut goal, NpcMoveGoal::Idle);
                        let brain = brain_from_goal(
                            active_brain_id_for_entity(entity, &npc_brain_states, &npc_registries.npc_brains),
                            &agent.goal,
                        );
                        commands.entity(entity).insert((agent, brain, NpcBrainState::default()));
                    }
                } else {
                    warn!("[cmd_queue] NpcWander: unknown handle {}", handle);
                }
            }

            LuaCommand::NpcGoToCoord {
                handle,
                target,
                stop_distance,
            } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let home = transforms
                        .get(entity)
                        .map(|t| t.translation)
                        .unwrap_or(Vec3::ZERO);
                    let goal = NpcMoveGoal::GoToCoord {
                        target: Vec3::new(target[0], target[1], target[2]),
                        stop_distance: stop_distance.max(0.01),
                    };
                    if let Ok(mut agent) = npc_agents.get_mut(entity) {
                        if matches!(agent.goal, NpcMoveGoal::Idle) {
                            agent.home = home;
                        }
                        agent.goal = goal;
                        agent.reset_navigation_state();
                        commands.entity(entity).insert(brain_from_goal(
                            active_brain_id_for_entity(entity, &npc_brain_states, &npc_registries.npc_brains),
                            &agent.goal,
                        ));
                    } else {
                        let mut agent = NpcAgent::new(handle, home);
                        agent.goal = goal;
                        let brain = brain_from_goal(
                            active_brain_id_for_entity(entity, &npc_brain_states, &npc_registries.npc_brains),
                            &agent.goal,
                        );
                        commands.entity(entity).insert((agent, brain, NpcBrainState::default()));
                    }
                } else {
                    warn!("[cmd_queue] NpcGoToCoord: unknown handle {}", handle);
                }
            }

            LuaCommand::NpcGoToEntity {
                handle,
                target_handle,
                stop_distance,
            } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let home = transforms
                        .get(entity)
                        .map(|t| t.translation)
                        .unwrap_or(Vec3::ZERO);
                    let goal = NpcMoveGoal::GoToEntity {
                        target_handle,
                        stop_distance: stop_distance.max(0.01),
                    };
                    if let Ok(mut agent) = npc_agents.get_mut(entity) {
                        if matches!(agent.goal, NpcMoveGoal::Idle) {
                            agent.home = home;
                        }
                        agent.goal = goal;
                        agent.reset_navigation_state();
                        commands.entity(entity).insert(brain_from_goal(
                            active_brain_id_for_entity(entity, &npc_brain_states, &npc_registries.npc_brains),
                            &agent.goal,
                        ));
                    } else {
                        let mut agent = NpcAgent::new(handle, home);
                        agent.goal = goal;
                        let brain = brain_from_goal(
                            active_brain_id_for_entity(entity, &npc_brain_states, &npc_registries.npc_brains),
                            &agent.goal,
                        );
                        commands.entity(entity).insert((agent, brain, NpcBrainState::default()));
                    }
                } else {
                    warn!("[cmd_queue] NpcGoToEntity: unknown handle {}", handle);
                }
            }

            LuaCommand::NpcStop { handle } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    if let Ok(mut agent) = npc_agents.get_mut(entity) {
                        agent.goal = NpcMoveGoal::Idle;
                        agent.reset_navigation_state();
                        commands.entity(entity).insert(ReplicatedNpcBrain::default());
                    }
                } else {
                    warn!("[cmd_queue] NpcStop: unknown handle {}", handle);
                }
            }

            LuaCommand::RegisterNpcBrain { brain_id, mut def } => {
                if def.id.trim().is_empty() {
                    def.id = brain_id.clone();
                }
                npc_registries.npc_brains.upsert(def);
                info!("[npc_brain] registered/upserted '{}'", brain_id);
            }

            LuaCommand::SetNpcBrain { handle, brain_id } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let resolved = npc_registries.npc_brains.canonical_brain_id(&brain_id);
                    if resolved != brain_id {
                        warn!(
                            "[npc_brain] unknown brain '{}' for handle {} — falling back to '{}'",
                            brain_id,
                            handle,
                            resolved,
                        );
                    }
                    commands.entity(entity).insert(NpcBrainState::new(resolved.clone()));
                    let mut replicated = npc_replicated_brains
                        .get(entity)
                        .cloned()
                        .unwrap_or_default();
                    replicated.brain_id = resolved;
                    commands.entity(entity).insert(replicated);
                } else {
                    warn!("[cmd_queue] SetNpcBrain: unknown handle {}", handle);
                }
            }

            LuaCommand::RegisterNpcScenario { scenario_id, mut def } => {
                if def.id.trim().is_empty() {
                    def.id = scenario_id.clone();
                }
                npc_registries.npc_scenarios.upsert(def);
            }

            LuaCommand::ConfigureNpcScenarioClock { config } => {
                if let Some(auto_advance) = config.auto_advance {
                    npc_registries.npc_scenario_clock.auto_advance = auto_advance;
                }
                if let Some(day_length_seconds) = config.day_length_seconds {
                    npc_registries.npc_scenario_clock.day_length_seconds = day_length_seconds.max(1.0);
                }
            }

            LuaCommand::ConfigureNpcAiLod { config } => {
                if let Some(value) = config.full_radius {
                    npc_registries.npc_ai_lod.full_radius = value.max(1.0);
                }
                if let Some(value) = config.reduced_radius {
                    npc_registries.npc_ai_lod.reduced_radius = value.max(1.0);
                }
                if let Some(value) = config.reduced_tick_interval {
                    npc_registries.npc_ai_lod.reduced_tick_interval = value.max(0.01);
                }
                if let Some(value) = config.full_budget_per_player {
                    npc_registries.npc_ai_lod.full_budget_per_player = value;
                }
                if let Some(value) = config.reduced_budget_per_player {
                    npc_registries.npc_ai_lod.reduced_budget_per_player = value;
                }
                if let Some(value) = config.zone_size {
                    npc_registries.npc_ai_lod.zone_size = value.max(1.0);
                }
                if let Some(value) = config.full_budget_per_zone {
                    npc_registries.npc_ai_lod.full_budget_per_zone = value;
                }
                if let Some(value) = config.reduced_budget_per_zone {
                    npc_registries.npc_ai_lod.reduced_budget_per_zone = value;
                }
            }

            LuaCommand::ConfigureNpcPopulationDirector { config } => {
                if let Some(value) = config.default_assignment_radius {
                    npc_registries.npc_population_director.default_assignment_radius = value.max(1.0);
                }
                if let Some(value) = config.release_distance_multiplier {
                    npc_registries.npc_population_director.release_distance_multiplier = value.max(1.0);
                }
                if let Some(value) = config.default_release_distance {
                    npc_registries.npc_population_director.default_release_distance = value.max(1.0);
                }
            }

            LuaCommand::SetNpcScenarioTime { hour_of_day } => {
                commands.insert_resource(NpcScenarioTime {
                    hour_of_day: hour_of_day.rem_euclid(24.0),
                });
            }

            LuaCommand::ConfigureEnvironmentLight { config } => {
                if let Some(value) = config.enabled {
                    npc_registries.environment_light.enabled = value;
                }
                if let Some(value) = config.shadows_enabled {
                    npc_registries.environment_light.shadows_enabled = value;
                }
                if let Some(value) = config.color {
                    npc_registries.environment_light.color = value;
                }
                if let Some(value) = config.illuminance {
                    npc_registries.environment_light.illuminance = value.max(0.0);
                }
                if let Some(value) = config.ambient_enabled {
                    npc_registries.environment_light.ambient_enabled = value;
                }
                if let Some(value) = config.ambient_color {
                    npc_registries.environment_light.ambient_color = value;
                }
                if let Some(value) = config.ambient_brightness {
                    npc_registries.environment_light.ambient_brightness = value.max(0.0);
                }
                if let Some(value) = config.hour_of_day {
                    npc_registries.environment_light.hour_of_day = value.rem_euclid(24.0);
                }
                if let Some(value) = config.azimuth_deg {
                    npc_registries.environment_light.azimuth_deg = value;
                }
                if let Some(value) = config.max_elevation_deg {
                    npc_registries.environment_light.max_elevation_deg = value.clamp(0.0, 89.0);
                }
                if let Some(value) = config.fog_enabled {
                    npc_registries.environment_light.fog_enabled = value;
                }
                if let Some(value) = config.fog_color {
                    npc_registries.environment_light.fog_color = value;
                }
                if let Some(value) = config.fog_directional_light_color {
                    npc_registries.environment_light.fog_directional_light_color = value;
                }
                if let Some(value) = config.fog_directional_light_exponent {
                    npc_registries.environment_light.fog_directional_light_exponent = value.max(0.0);
                }
                if let Some(value) = config.fog_start {
                    npc_registries.environment_light.fog_start = value.max(0.0);
                }
                if let Some(value) = config.fog_end {
                    npc_registries.environment_light.fog_end = value.max(0.0);
                }
                if let Some(value) = config.fog_follow_streaming_boundary {
                    npc_registries.environment_light.fog_follow_streaming_boundary = value;
                }
                if let Some(value) = config.fog_boundary_inner_distance {
                    npc_registries.environment_light.fog_boundary_inner_distance = value.max(0.0);
                }
                if let Some(value) = config.fog_boundary_outer_distance {
                    npc_registries.environment_light.fog_boundary_outer_distance = value.max(0.0);
                }
                if npc_registries.environment_light.fog_end < npc_registries.environment_light.fog_start {
                    npc_registries.environment_light.fog_end = npc_registries.environment_light.fog_start;
                }
                if let Some(value) = config.volumetric_fog_enabled {
                    npc_registries.environment_light.volumetric_fog_enabled = value;
                }
                if let Some(value) = config.volumetric_fog_ambient_color {
                    npc_registries.environment_light.volumetric_fog_ambient_color = value;
                }
                if let Some(value) = config.volumetric_fog_ambient_intensity {
                    npc_registries.environment_light.volumetric_fog_ambient_intensity = value.max(0.0);
                }
                if let Some(value) = config.volumetric_fog_jitter {
                    npc_registries.environment_light.volumetric_fog_jitter = value.max(0.0);
                }
                if let Some(value) = config.volumetric_fog_step_count {
                    npc_registries.environment_light.volumetric_fog_step_count = value.max(1);
                }
            }

            LuaCommand::SetEnvironmentTime { hour_of_day } => {
                npc_registries.environment_light.hour_of_day = hour_of_day.rem_euclid(24.0);
            }

            LuaCommand::SetNpcTask {
                handle,
                task,
                scenario_id,
                target_handle,
                target_pos,
                params,
            } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    let brain_id = npc_brain_states
                        .get(entity)
                        .map(|state| npc_registries.npc_brains.canonical_brain_id(&state.brain_id))
                        .unwrap_or_else(|_| npc_registries.npc_brains.canonical_brain_id("core/human"));
                    let target = match (target_handle, target_pos) {
                        (Some(target_handle), _) => NpcBrainTarget::Entity(target_handle),
                        (_, Some(target_pos)) => NpcBrainTarget::Position(Vec3::new(target_pos[0], target_pos[1], target_pos[2])),
                        _ => NpcBrainTarget::None,
                    };
                    commands.entity(entity).insert(ReplicatedNpcBrain {
                        brain_id,
                        task,
                        scenario_id,
                        target,
                        params,
                    });
                } else {
                    warn!("[cmd_queue] SetNpcTask: unknown handle {}", handle);
                }
            }

            LuaCommand::Attach { child_handle, child_socket, parent_handle, parent_socket } => {
                let Some(child_entity) = world_state.entity_for(child_handle) else {
                    warn!("[cmd_queue] Attach: unknown child handle {}", child_handle);
                    continue;
                };
                let Some(parent_entity) = world_state.entity_for(parent_handle) else {
                    warn!("[cmd_queue] Attach: unknown parent handle {}", parent_handle);
                    continue;
                };

                let Some(child_map) = socket_maps.get(child_entity).ok() else {
                    warn!("[cmd_queue] Attach: child handle {} has no AdsSocketMap", child_handle);
                    continue;
                };
                let Some(parent_map) = socket_maps.get(parent_entity).ok() else {
                    warn!("[cmd_queue] Attach: parent handle {} has no AdsSocketMap", parent_handle);
                    continue;
                };

                let Some(&child_socket_entity) = child_map.0.get(&child_socket) else {
                    warn!(
                        "[cmd_queue] Attach: child socket '{}' not found on handle {}",
                        child_socket, child_handle
                    );
                    continue;
                };
                let Some(&parent_socket_entity) = parent_map.0.get(&parent_socket) else {
                    warn!(
                        "[cmd_queue] Attach: parent socket '{}' not found on handle {}",
                        parent_socket, parent_handle
                    );
                    continue;
                };

                let Ok(child_root_world) = globals.get(child_entity) else {
                    warn!("[cmd_queue] Attach: missing GlobalTransform for child handle {}", child_handle);
                    continue;
                };
                let Ok(parent_root_world) = globals.get(parent_entity) else {
                    warn!("[cmd_queue] Attach: missing GlobalTransform for parent handle {}", parent_handle);
                    continue;
                };
                let Ok(child_socket_world) = globals.get(child_socket_entity) else {
                    warn!("[cmd_queue] Attach: missing GlobalTransform for child socket '{}'", child_socket);
                    continue;
                };
                let Ok(parent_socket_world) = globals.get(parent_socket_entity) else {
                    warn!("[cmd_queue] Attach: missing GlobalTransform for parent socket '{}'", parent_socket);
                    continue;
                };

                let child_socket_local =
                    child_root_world.to_matrix().inverse() * child_socket_world.to_matrix();
                let target_child_world =
                    parent_socket_world.to_matrix() * child_socket_local.inverse();
                let target_child_local =
                    parent_root_world.to_matrix().inverse() * target_child_world;

                let (scale, rotation, translation) =
                    target_child_local.to_scale_rotation_translation();

                commands.entity(child_entity).insert((
                    ChildOf(parent_entity),
                    Transform {
                        translation,
                        rotation,
                        scale,
                    },
                    SocketAttachment {
                        child_socket,
                        parent_handle,
                        parent_socket,
                    },
                ));
            }

            LuaCommand::AttachWithOffset { child_handle, parent_handle, offset, rot } => {
                let Some(child_entity) = world_state.entity_for(child_handle) else {
                    warn!("[cmd_queue] AttachWithOffset: unknown child handle {}", child_handle);
                    continue;
                };
                let Some(parent_entity) = world_state.entity_for(parent_handle) else {
                    warn!("[cmd_queue] AttachWithOffset: unknown parent handle {}", parent_handle);
                    continue;
                };

                let current_scale = transforms
                    .get(child_entity)
                    .map(|t| t.scale)
                    .unwrap_or(Vec3::ONE);

                commands.entity(child_entity).insert((
                    ChildOf(parent_entity),
                    Transform {
                        translation: Vec3::new(offset[0], offset[1], offset[2]),
                        rotation: Quat::from_euler(
                            EulerRot::XYZ,
                            rot[0].to_radians(),
                            rot[1].to_radians(),
                            rot[2].to_radians(),
                        ),
                        scale: current_scale,
                    },
                ));
                commands.entity(child_entity).remove::<SocketAttachment>();
            }

            LuaCommand::Detach { child_handle } => {
                let Some(child_entity) = world_state.entity_for(child_handle) else {
                    warn!("[cmd_queue] Detach: unknown child handle {}", child_handle);
                    continue;
                };
                let Ok(world_tf) = globals.get(child_entity) else {
                    warn!("[cmd_queue] Detach: missing GlobalTransform for child handle {}", child_handle);
                    continue;
                };

                let (scale, rotation, translation) = world_tf.to_matrix().to_scale_rotation_translation();
                commands.entity(child_entity).insert(Transform {
                    translation,
                    rotation,
                    scale,
                });
                commands.entity(child_entity).remove::<ChildOf>();
                commands.entity(child_entity).remove::<SocketAttachment>();
            }

            LuaCommand::SetStat { player_id, name, value } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, mut stats, _, _, _, _, _, _, _, _)) = player_stats.get_mut(entity) {
                        stats.0.insert(name.clone(), value);
                        debug!("[cmd_queue] SetStat player={} {}={}", player_id, name, value);
                    } else {
                        warn!("[cmd_queue] SetStat: player {} has no Stats component", player_id);
                    }
                } else {
                    warn!("[cmd_queue] SetStat: unknown player_id {}", player_id);
                }
            }

            LuaCommand::GiveItem { player_id, item, count } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, _, mut inv, _, _, _, _, _, _, _)) = player_stats.get_mut(entity) {
                        let entry = inv.0.entry(item.clone()).or_insert(0);
                        if count >= 0 {
                            *entry = entry.saturating_add(count as u32);
                        } else {
                            let take = (-count) as u32;
                            *entry = entry.saturating_sub(take);
                        }
                        debug!(
                            "[cmd_queue] GiveItem player={} item={} count={} -> new={}",
                            player_id, item, count, inv.0[&item]
                        );
                    } else {
                        warn!("[cmd_queue] GiveItem: player {} has no Inventory component", player_id);
                    }
                } else {
                    warn!("[cmd_queue] GiveItem: unknown player_id {}", player_id);
                }
            }

            LuaCommand::SetEquippedWeapon {
                player_id,
                slot,
                equipped,
            } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, _, _, _, mut slots, _, _, _, _, _)) = player_stats.get_mut(entity) {
                        let equipped = equipped.map(|mut equipped| {
                            if equipped.fire_mode.trim().is_empty() {
                                if let Some(weapon_def) = player_runtime.weapon_registry.get(&equipped.weapon_id) {
                                    if !weapon_def.default_fire_mode.trim().is_empty() {
                                        equipped.fire_mode = weapon_def.default_fire_mode;
                                    } else if let Some(first) = weapon_def.fire_modes.first() {
                                        equipped.fire_mode = first.clone();
                                    }
                                }
                            }
                            equipped
                        });
                        if slots.set(slot, equipped.clone()) {
                            debug!(
                                "[cmd_queue] SetEquippedWeapon player={} slot={} weapon={:?}",
                                player_id,
                                slot,
                                equipped.as_ref().map(|value| value.weapon_id.as_str())
                            );
                        } else {
                            warn!("[cmd_queue] SetEquippedWeapon: invalid slot {}", slot);
                        }
                    } else {
                        warn!("[cmd_queue] SetEquippedWeapon: player {} missing WeaponSlots", player_id);
                    }
                } else {
                    warn!("[cmd_queue] SetEquippedWeapon: unknown player_id {}", player_id);
                }
            }

            LuaCommand::SetAmmoReserve {
                player_id,
                ammo_type_id,
                count,
            } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, _, _, _, _, mut reserve, _, _, _, _)) = player_stats.get_mut(entity) {
                        reserve.0.insert(ammo_type_id.clone(), count);
                        debug!(
                            "[cmd_queue] SetAmmoReserve player={} ammo={} count={}",
                            player_id,
                            ammo_type_id,
                            count
                        );
                    } else {
                        warn!("[cmd_queue] SetAmmoReserve: player {} missing AmmoReserve", player_id);
                    }
                } else {
                    warn!("[cmd_queue] SetAmmoReserve: unknown player_id {}", player_id);
                }
            }

            LuaCommand::SetActiveWeaponSlot { player_id, slot } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, _, _, _, slots, _, active_slot, mut fire_state, mut reload_state, mut weapon_swap)) = player_stats.get_mut(entity) {
                        if slot < 4 {
                            *fire_state = FireState::default();
                            if reload_state.remaining > 0.0 {
                                *reload_state = ReloadState::default();
                            }
                            if slot == active_slot.0 {
                                *weapon_swap = WeaponSwapState::default();
                            } else {
                                let duration = weapon_swap_duration_for_slots(
                                    &player_runtime.weapon_registry,
                                    &slots,
                                    active_slot.0,
                                    slot,
                                );
                                *weapon_swap = WeaponSwapState {
                                    remaining: duration,
                                    duration,
                                    target_slot: slot,
                                };
                            }
                            debug!("[cmd_queue] SetActiveWeaponSlot player={} slot={} pending={}", player_id, slot, weapon_swap.remaining);
                        } else {
                            warn!("[cmd_queue] SetActiveWeaponSlot: invalid slot {}", slot);
                        }
                    } else {
                        warn!("[cmd_queue] SetActiveWeaponSlot: player {} missing ActiveWeaponSlot", player_id);
                    }
                } else {
                    warn!("[cmd_queue] SetActiveWeaponSlot: unknown player_id {}", player_id);
                }
            }

            LuaCommand::SetWeaponFireMode {
                player_id,
                slot,
                fire_mode,
            } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, _, _, _, mut slots, _, active_slot, mut fire_state, _, _)) = player_stats.get_mut(entity) {
                        let target_slot = slot.unwrap_or(active_slot.0);
                        if let Some(equipped) = slots.get_mut(target_slot) {
                            equipped.fire_mode = fire_mode.clone();
                            *fire_state = FireState::default();
                            debug!(
                                "[cmd_queue] SetWeaponFireMode player={} slot={} mode={}",
                                player_id,
                                target_slot,
                                fire_mode
                            );
                        } else {
                            warn!("[cmd_queue] SetWeaponFireMode: missing equipped weapon in slot {}", target_slot);
                        }
                    } else {
                        warn!("[cmd_queue] SetWeaponFireMode: player {} missing weapon state", player_id);
                    }
                } else {
                    warn!("[cmd_queue] SetWeaponFireMode: unknown player_id {}", player_id);
                }
            }

            LuaCommand::ForceReload { player_id } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, _, _, _, slots, reserve, active_slot, mut fire_state, mut reload_state, mut weapon_swap)) = player_stats.get_mut(entity) {
                        *fire_state = FireState::default();
                        *weapon_swap = WeaponSwapState::default();
                        if let Some(duration) = reload_duration_for_slot(
                            &player_runtime.weapon_registry,
                            &slots,
                            &reserve,
                            active_slot.0,
                        ) {
                            *reload_state = ReloadState {
                                remaining: duration,
                                duration,
                                slot: active_slot.0,
                            };
                            debug!(
                                "[cmd_queue] ForceReload player={} slot={} pending={}",
                                player_id,
                                active_slot.0,
                                duration
                            );
                        } else {
                            *reload_state = ReloadState::default();
                        }
                    } else {
                        warn!("[cmd_queue] ForceReload: player {} missing weapon state", player_id);
                    }
                } else {
                    warn!("[cmd_queue] ForceReload: unknown player_id {}", player_id);
                }
            }

            LuaCommand::SetPlayerArmor {
                player_id,
                slot,
                armor,
            } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, _, _, _, mut armor_component, _, _, _, _, _, _)) = player_stats.get_mut(entity) {
                        let target = match slot.as_str() {
                            "helmet" => &mut armor_component.helmet,
                            "vest" => &mut armor_component.vest,
                            _ => {
                                warn!("[cmd_queue] SetPlayerArmor: unsupported slot '{}'", slot);
                                continue;
                            }
                        };
                        *target = armor.map(ArmorPiece::clamped);
                        debug!("[cmd_queue] SetPlayerArmor player={} slot={} equipped={}", player_id, slot, target.is_some());
                    } else {
                        warn!("[cmd_queue] SetPlayerArmor: player {} missing ArmorComponent", player_id);
                    }
                } else {
                    warn!("[cmd_queue] SetPlayerArmor: unknown player_id {}", player_id);
                }
            }

            LuaCommand::SetPlayerHealth { player_id, current, max } => {
                if let Some(&entity) = player_runtime.player_map.map.get(&player_id) {
                    if let Ok((_, mut health, _, _, _, _, _, _, _, _, _)) = player_stats.get_mut(entity) {
                        if let Some(max_health) = max {
                            health.max = max_health.max(1.0);
                        }
                        health.current = current.clamp(0.0, health.max.max(1.0));
                        debug!(
                            "[cmd_queue] SetPlayerHealth player={} current={} max={}",
                            player_id,
                            health.current,
                            health.max
                        );
                    } else {
                        warn!("[cmd_queue] SetPlayerHealth: player {} missing Health", player_id);
                    }
                } else {
                    warn!("[cmd_queue] SetPlayerHealth: unknown player_id {}", player_id);
                }
            }

            LuaCommand::SetCollisionEnabled { handle, enabled } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(CollisionEnabled(enabled));
                    debug!("[cmd_queue] SetCollisionEnabled handle={} enabled={}", handle, enabled);
                } else {
                    warn!("[cmd_queue] SetCollisionEnabled: unknown handle {}", handle);
                }
            }

            LuaCommand::SetMaterialParam { handle, param, value } => {
                let entry = pending_mat.entry(handle).or_default();
                match param.as_str() {
                    "snow_level"  => entry.snow_level  = Some(value),
                    "dirt_level"  => entry.dirt_level  = Some(value),
                    "wetness"     => entry.wetness     = Some(value),
                    "snow_height" => entry.snow_height = Some(value),
                    "wet_height"  => entry.wet_height  = Some(value),
                    other => warn!("[cmd_queue] SetMaterialParam: unknown param '{}'", other),
                }
            }

            LuaCommand::SetEntityShaderProfile { handle, profile } => {
                let entry = pending_mat.entry(handle).or_default();
                entry.shader_profile = Some(profile);
            }
        }
    }

    // Aplikuj nahromaděné materiálové přepisy po zpracování všech příkazů.
    // Pokud entita již má LuaMaterialOverride, jen přepíšeme změněná pole (merge).
    // Jinak se komponent přidá jako nový (deferred přes Commands).
    for (handle, new_params) in pending_mat {
        let Some(entity) = world_state.entity_for(handle) else {
            warn!("[cmd_queue] SetMaterialParam: unknown handle {}", handle);
            continue;
        };
        if let Ok(mut existing) = mat_overrides.get_mut(entity) {
            if let Some(v) = new_params.snow_level  { existing.snow_level  = Some(v); }
            if let Some(v) = new_params.dirt_level  { existing.dirt_level  = Some(v); }
            if let Some(v) = new_params.wetness     { existing.wetness     = Some(v); }
            if let Some(v) = new_params.snow_height { existing.snow_height = Some(v); }
            if let Some(v) = new_params.wet_height  { existing.wet_height  = Some(v); }
            if let Some(v) = new_params.shader_profile { existing.shader_profile = Some(v); }
        } else {
            commands.entity(entity).insert(new_params);
        }
    }
}

pub fn tick_npc_agents(
    time: Res<Time<Fixed>>,
    lod_config: Res<NpcAiLodConfig>,
    side: Res<ResourcesSide>,
    world_state: Res<LuaWorldState>,
    mut npcs: Query<(
        &EntityHandle,
        &mut Transform,
        Option<&mut NetTransform>,
        &mut NpcAgent,
        Option<&mut ReplicatedNpcSteering>,
        Option<&NpcOwner>,
        Option<&NpcLastClientUpdate>,
        Option<&NpcAiLodState>,
    )>,
    globals: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let now = time.elapsed_secs();

    for (handle, mut transform, net_tf_opt, mut agent, steering_opt, owner, last_client_update, lod_state) in &mut npcs {
        let lod_level = lod_state.map(|lod| lod.level).unwrap_or(NpcAiLodLevel::Full);
        if !lod_allows_tick(handle.0, lod_level, &lod_config, now, dt) {
            if let Some(mut steering) = steering_opt {
                *steering = snapshot_npc_steering(&agent);
            }
            continue;
        }

        // Frozen: žádný hráč v okolí, nesimulujeme pohyb.
        if let Some(o) = owner {
            if o.0.is_none() {
                if matches!(lod_level, NpcAiLodLevel::Background) {
                    continue;
                }
            }
            if matches!(side.0, Side::Server) {
                let client_owned_is_fresh = match (o.0, last_client_update) {
                    (Some(owner_id), Some(last_update)) => {
                        last_update.client_id == owner_id
                            && (now - last_update.received_at) <= NPC_CLIENT_UPDATE_TIMEOUT_SECS
                    }
                    _ => false,
                };

                if client_owned_is_fresh {
                    continue;
                }
            }
        }

        if matches!(agent.goal, NpcMoveGoal::Idle) {
            agent.reset_navigation_state();
            continue;
        }

        let mut stop_distance = agent.arrive_distance.max(0.01);
        let mut complete_goal = false;
        let mut advance_waypoint = false;

        if agent.avoidance_timer > 0.0 {
            agent.avoidance_timer = (agent.avoidance_timer - dt).max(0.0);
            if agent.avoidance_timer <= 0.0 {
                agent.avoidance_offset = Vec3::ZERO;
            }
        }

        let goal_snapshot = agent.goal.clone();
        let mut target_pos = if let Some(waypoint) = agent.current_path.get(agent.waypoint_index) {
            stop_distance = stop_distance.max(0.1);
            waypoint.target
        } else {
            match goal_snapshot {
            NpcMoveGoal::Idle => continue,
            NpcMoveGoal::GoToCoord { target, stop_distance: stop } => {
                stop_distance = stop.max(agent.arrive_distance).max(0.01);
                target
            }
            NpcMoveGoal::GoToEntity {
                target_handle,
                stop_distance: stop,
            } => {
                if let Some(target_entity) = world_state.entity_for(target_handle) {
                    if let Ok(t) = globals.get(target_entity) {
                        let target_translation = t.translation();
                        if let Some(previous) = agent.entity_target_position.replace(target_translation) {
                            let observed_velocity = Vec3::new(
                                (target_translation.x - previous.x) / dt,
                                0.0,
                                (target_translation.z - previous.z) / dt,
                            );
                            if observed_velocity.is_finite() {
                                agent.entity_target_velocity = agent.entity_target_velocity.lerp(observed_velocity, 0.35);
                            }
                        }

                        stop_distance = stop.max(agent.arrive_distance).max(0.01);
                        if stop_distance >= 1.35 {
                            let max_offset = stop_distance.min(4.0).max(0.75);
                            if agent.formation_offset.length_squared() <= 0.0001 {
                                let relative = Vec3::new(
                                    transform.translation.x - target_translation.x,
                                    0.0,
                                    transform.translation.z - target_translation.z,
                                );
                                if relative.length_squared() > 0.01 {
                                    agent.formation_offset = relative.clamp_length_max(max_offset);
                                } else {
                                    agent.formation_offset = Vec3::new(max_offset * 0.6, 0.0, 0.0);
                                }
                            } else {
                                agent.formation_offset = agent.formation_offset.clamp_length_max(max_offset);
                            }
                            target_translation + agent.formation_offset
                        } else {
                            agent.formation_offset = Vec3::ZERO;
                            let pursuit_lead = Vec3::new(
                                agent.entity_target_velocity.x,
                                0.0,
                                agent.entity_target_velocity.z,
                            )
                            .clamp_length_max(stop_distance.max(1.0) * 1.5)
                                * 0.25;
                            target_translation + pursuit_lead
                        }
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            NpcMoveGoal::Wander {
                kind,
                radius,
                retarget_sec,
                orbit_angular_speed,
                patrol_point,
                clockwise,
            } => {
                let radius = radius.max(0.1);
                let retarget_sec = retarget_sec.max(0.05);

                match kind {
                    NpcWanderKind::Random => {
                        agent.wander_timer -= dt;
                        let dist_to_curr = Vec2::new(
                            transform.translation.x - agent.wander_target.x,
                            transform.translation.z - agent.wander_target.z,
                        )
                        .length();
                        if agent.wander_timer <= 0.0 || dist_to_curr <= stop_distance {
                            let a = agent.next_rand01() * TAU;
                            let d = radius * (0.35 + 0.65 * agent.next_rand01());
                            agent.wander_target = Vec3::new(
                                agent.home.x + a.cos() * d,
                                transform.translation.y,
                                agent.home.z + a.sin() * d,
                            );
                            agent.wander_timer = retarget_sec;
                        }
                        agent.wander_target
                    }
                    NpcWanderKind::Patrol => {
                        let patrol = patrol_point.unwrap_or_else(|| {
                            agent.home + Vec3::new(radius, 0.0, 0.0)
                        });
                        let curr_target = if agent.patrol_to_target {
                            patrol
                        } else {
                            agent.home
                        };

                        let d = Vec2::new(
                            transform.translation.x - curr_target.x,
                            transform.translation.z - curr_target.z,
                        )
                        .length();
                        if d <= stop_distance {
                            agent.patrol_to_target = !agent.patrol_to_target;
                        }

                        agent.wander_target = if agent.patrol_to_target {
                            patrol
                        } else {
                            agent.home
                        };
                        agent.wander_target
                    }
                    NpcWanderKind::Orbit => {
                        let sign = if clockwise { -1.0 } else { 1.0 };
                        agent.orbit_angle = (agent.orbit_angle + sign * orbit_angular_speed.max(0.05) * dt)
                            .rem_euclid(TAU);
                        stop_distance = (radius * 0.15).max(agent.arrive_distance).max(0.1);
                        agent.wander_target = Vec3::new(
                            agent.home.x + radius * agent.orbit_angle.cos(),
                            transform.translation.y,
                            agent.home.z + radius * agent.orbit_angle.sin(),
                        );
                        agent.wander_target
                    }
                }
            }
        }};

        if agent.avoidance_timer > 0.0 {
            target_pos += agent.avoidance_offset;
        }

        let to_target = Vec2::new(
            target_pos.x - transform.translation.x,
            target_pos.z - transform.translation.z,
        );
        let dist = to_target.length();

        if dist <= stop_distance {
            if agent.waypoint_index < agent.current_path.len() {
                advance_waypoint = true;
            } else if matches!(goal_snapshot, NpcMoveGoal::GoToCoord { .. } | NpcMoveGoal::GoToEntity { .. }) {
                complete_goal = true;
            }
        } else {
            let dir = to_target / dist;
            let step = (agent.move_speed.max(0.0) * dt).min(dist - stop_distance);
            transform.translation.x += dir.x * step;
            transform.translation.z += dir.y * step;

            let desired_yaw = dir.x.atan2(dir.y);
            let desired_rot = Quat::from_rotation_y(desired_yaw);
            let t = (agent.turn_speed.max(0.0) * dt).clamp(0.0, 1.0);
            transform.rotation = transform.rotation.slerp(desired_rot, t);
        }

        if advance_waypoint {
            agent.waypoint_index += 1;
            if agent.waypoint_index >= agent.current_path.len() {
                agent.current_path.clear();
                agent.waypoint_index = 0;
                if matches!(goal_snapshot, NpcMoveGoal::GoToCoord { .. } | NpcMoveGoal::GoToEntity { .. }) {
                    complete_goal = true;
                }
            }
        }

        if complete_goal {
            agent.goal = NpcMoveGoal::Idle;
        }

        if let Some(mut net_tf) = net_tf_opt {
            net_tf.translation = transform.translation;
            net_tf.rotation = transform.rotation;
        }
        if let Some(mut steering) = steering_opt {
            *steering = snapshot_npc_steering(&agent);
        }
    }
}

/// Aktualizuje `EntityStateCache` podle aktuálního ECS stavu všech Lua entit.
/// Běží v PostUpdate po `process_lua_commands`, takže cache vidí výsledky
/// tohoto framu. Lua handlery čtou cache synchronně při dispatch_local_events.
pub fn sync_entity_state_cache(
    query: Query<(
        &EntityHandle,
        &Transform,
        Option<&ModelName>,
        Option<&Health>,
        Option<&AnimationState>,
        Has<NetworkedObjectMarker>,
        Option<&CollisionEnabled>,
        Option<&AdsSocketMap>,
    )>,
    globals: Query<&GlobalTransform>,
    cache: Res<EntityStateCache>,
) {
    let mut lock = cache.0.lock().unwrap_or_else(|p| p.into_inner());
    lock.clear();
    for (handle, transform, model, health, anim, is_networked, collision, socket_map) in &query {
        let mut sockets = HashMap::new();
        if let Some(socket_map) = socket_map {
            for (name, socket_entity) in &socket_map.0 {
                if let Ok(tf) = globals.get(*socket_entity) {
                    sockets.insert(
                        name.clone(),
                        SocketTransformSnapshot {
                            pos: tf.translation().to_array(),
                            rot: tf.rotation().to_array(),
                        },
                    );
                }
            }
        }

        let snapshot = EntitySnapshot {
            pos: transform.translation.to_array(),
            rot: [
                transform.rotation.x,
                transform.rotation.y,
                transform.rotation.z,
                transform.rotation.w,
            ],
            scale: transform.scale.to_array(),
            model: model.map(|m| m.0.clone()),
            alive: health.map(|h| !h.is_dead()).unwrap_or(true),
            health: health.map(|h| h.current),
            max_health: health.map(|h| h.max),
            animation: anim.and_then(|a| a.current.clone()),
            anim_speed: anim.map(|a| a.speed).unwrap_or(1.0),
            anim_looping: anim.map(|a| a.looping).unwrap_or(true),
            anim_paused: anim.map(|a| a.paused).unwrap_or(false),
            is_networked,
            collision_enabled: collision.map(|c| c.0).unwrap_or(true),
            sockets,
        };
        lock.insert(handle.0, snapshot);
    }
}
