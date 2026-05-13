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

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use bevy::reflect::Reflect;
use core_shared::{Health, NetTransform, PlayerMarker};
use serde::{Deserialize, Serialize};

use crate::model_registry::AnimSetCommandQueue;

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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct NpcPathWaypoint {
    pub target: Vec3,
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
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.rng_state as f64 / (u32::MAX as f64)) as f32
    }
}

/// Vlastník NPC — client_id hráče, který simuluje toto NPC.
/// `None` = žádný hráč v okolí, NPC je zmrazeno.
/// Replikováno klientům přes lightyear.
#[derive(Component, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NpcOwner(pub Option<u64>);

const NPC_OWNERSHIP_RADIUS: f32 = 200.0;
const NPC_OWNERSHIP_ASSIGN_INTERVAL: f32 = 2.0;

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
    mut timer: Local<f32>,
    mut npcs: Query<(&Transform, &mut NpcOwner), With<NpcAgent>>,
    players: Query<(&Transform, &PlayerMarker)>,
) {
    *timer += time.delta_secs();
    if *timer < NPC_OWNERSHIP_ASSIGN_INTERVAL {
        return;
    }
    *timer = 0.0;

    for (npc_tf, mut owner) in &mut npcs {
        // Ověř, zda stávající owner stále existuje a je v dosahu.
        let owner_still_valid = owner.0.map(|id| {
            players.iter().any(|(ptf, pm)| {
                pm.client_id == id
                    && ptf.translation.distance(npc_tf.translation) <= NPC_OWNERSHIP_RADIUS
            })
        }).unwrap_or(false);

        if owner_still_valid {
            continue; // Owner platný → nic neměníme.
        }

        // Owner chybí nebo vypadl — najdeme nejbližšího hráče.
        let nearest = players
            .iter()
            .filter_map(|(ptf, pm)| {
                let dist = ptf.translation.distance(npc_tf.translation);
                if dist <= NPC_OWNERSHIP_RADIUS { Some((dist, pm.client_id)) } else { None }
            })
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let new_owner = nearest.map(|(_, id)| id);
        if owner.0 != new_owner {
            if let Some(id) = new_owner {
                debug!("[npc_owner] NPC at {:?} → client {}", npc_tf.translation, id);
            } else {
                debug!("[npc_owner] NPC at {:?} → frozen (no player nearby)", npc_tf.translation);
            }
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

/// Snapshot stavu hráče — synchronizovaný každý FixedUpdate tick
/// systémem `sync_stats_cache` v `core_net/sim.rs`.
/// Lua sandbox ho čte synchronně (bez latence).
#[derive(Debug, Clone, Default)]
pub struct StatsSnapshot {
    pub stats: HashMap<String, f64>,
    pub inventory: HashMap<String, u32>,
    pub health: f32,
    pub max_health: f32,
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
}

/// Mapa client_id → Entity. Udržovaná observery v `core_net/sim.rs`
/// při Add/Remove PlayerMarker. Umožňuje `process_lua_commands` rychlé
/// vyhledání entity hráče bez iterace celého světa.
#[derive(Resource, Default)]
pub struct PlayerEntityMap {
    pub map: HashMap<u64, Entity>,
}

/// Stats lokálního hráče na klientu — aktualizovány serverem přes `PlayerStatsUpdate`.
/// Arc<Mutex> sdíleno se sandbox closurami pro synchronní čtení z `Player.GetLocalStats()`.
#[derive(Resource, Clone, Default)]
pub struct LocalPlayerStats(pub Arc<Mutex<StatsSnapshot>>);

impl LocalPlayerStats {
    pub fn update_health(&self, hp: f32, max_hp: f32) {
        let mut snap = self.0.lock().unwrap_or_else(|p| p.into_inner());
        snap.health = hp;
        snap.max_health = max_hp;
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
    player_map: Res<PlayerEntityMap>,
    mut commands: Commands,
    mut damage_events: MessageWriter<PendingDamageEvent>,
    transforms: Query<&Transform>,
    globals: Query<&GlobalTransform>,
    socket_maps: Query<&AdsSocketMap>,
    anim_set_cmds: Res<AnimSetCommandQueue>,
    mut attached_anim_sets: Query<&mut AttachedAnimSets>,
    mut player_stats: Query<(&PlayerMarker, &mut Stats, &mut Inventory)>,
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
                let mut entity_builder = commands.spawn((
                    NpcPedMarker,
                    NetworkedObjectMarker { model: model.clone() },
                    ModelName(model.clone()),
                    EntityHandle(handle),
                    NpcAgent::new(handle, spawn_translation),
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
                    } else {
                        let mut agent = NpcAgent::new(handle, home);
                        agent.goal = std::mem::replace(&mut goal, NpcMoveGoal::Idle);
                        commands.entity(entity).insert(agent);
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
                    } else {
                        let mut agent = NpcAgent::new(handle, home);
                        agent.goal = goal;
                        commands.entity(entity).insert(agent);
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
                    } else {
                        let mut agent = NpcAgent::new(handle, home);
                        agent.goal = goal;
                        commands.entity(entity).insert(agent);
                    }
                } else {
                    warn!("[cmd_queue] NpcGoToEntity: unknown handle {}", handle);
                }
            }

            LuaCommand::NpcStop { handle } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    if let Ok(mut agent) = npc_agents.get_mut(entity) {
                        agent.goal = NpcMoveGoal::Idle;
                    }
                } else {
                    warn!("[cmd_queue] NpcStop: unknown handle {}", handle);
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
                if let Some(&entity) = player_map.map.get(&player_id) {
                    if let Ok((_, mut stats, _)) = player_stats.get_mut(entity) {
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
                if let Some(&entity) = player_map.map.get(&player_id) {
                    if let Ok((_, _, mut inv)) = player_stats.get_mut(entity) {
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
        } else {
            commands.entity(entity).insert(new_params);
        }
    }
}

pub fn tick_npc_agents(
    time: Res<Time<Fixed>>,
    world_state: Res<LuaWorldState>,
    mut npcs: Query<(&EntityHandle, &mut Transform, Option<&mut NetTransform>, &mut NpcAgent, Option<&NpcOwner>)>,
    globals: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (_handle, mut transform, net_tf_opt, mut agent, owner) in &mut npcs {
        // Frozen: žádný hráč v okolí, nesimulujeme pohyb.
        if let Some(o) = owner {
            if o.0.is_none() {
                continue;
            }
        }
        let mut stop_distance = agent.arrive_distance.max(0.01);
        let mut complete_goal = false;

        let goal_snapshot = agent.goal.clone();
        let target_pos = match goal_snapshot {
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
                        stop_distance = stop.max(agent.arrive_distance).max(0.01);
                        t.translation()
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

                        if agent.patrol_to_target {
                            patrol
                        } else {
                            agent.home
                        }
                    }
                    NpcWanderKind::Orbit => {
                        let sign = if clockwise { -1.0 } else { 1.0 };
                        agent.orbit_angle = (agent.orbit_angle + sign * orbit_angular_speed.max(0.05) * dt)
                            .rem_euclid(TAU);
                        stop_distance = (radius * 0.15).max(agent.arrive_distance).max(0.1);
                        Vec3::new(
                            agent.home.x + radius * agent.orbit_angle.cos(),
                            transform.translation.y,
                            agent.home.z + radius * agent.orbit_angle.sin(),
                        )
                    }
                }
            }
        };

        let to_target = Vec2::new(
            target_pos.x - transform.translation.x,
            target_pos.z - transform.translation.z,
        );
        let dist = to_target.length();

        if dist <= stop_distance {
            if matches!(goal_snapshot, NpcMoveGoal::GoToCoord { .. } | NpcMoveGoal::GoToEntity { .. }) {
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

        if complete_goal {
            agent.goal = NpcMoveGoal::Idle;
        }

        if let Some(mut net_tf) = net_tf_opt {
            net_tf.translation = transform.translation;
            net_tf.rotation = transform.rotation;
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
