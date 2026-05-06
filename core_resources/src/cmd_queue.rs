//! Phase 3.2 — Command Queue: bezpečný Lua → ECS most.
//!
//! Lua sandbox nesmí přímo mutovat ECS svět (`mlua` je `!Send`, sandbox běží
//! na main threadu v `NonSend` resource). Místo toho Lua vkládá záměry do
//! sdíleného `CommandQueue` bufferu. Bevy systém `process_lua_commands`
//! v `PostUpdate` frontu vybere a bezpečně aplikuje příkazy na ECS svět.
//!
//! Phase 4 přidává:
//! * `Stats` a `Inventory` ECS komponenty.
//! * `PlayerEntityMap` — mapuje client_id → Entity (udržuje core_net).
//! * `PlayerStatsCache` — Arc<Mutex> snapshot pro synchronní Lua čtení.
//! * `LuaCommand::SetStat`, `GiveItem`, `TakeItem` — mutace přes frontu.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use core_shared::{Health, PlayerMarker};

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
    /// Spustí animaci na entitě. Phase 4 napojí na Bevy AnimationPlayer.
    PlayAnimation {
        handle: u64,
        name: String,
        looping: bool,
        speed: f32,
    },
    /// Zastaví aktuální animaci entity.
    StopAnimation {
        handle: u64,
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

/// Embeddovaný handle na všech entitách spawnutých přes Lua bridge.
/// Umožňuje `sync_entity_state_cache` iterovat přes všechny Lua entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct EntityHandle(pub u64);

/// Kanonické jméno modelu entity — aktualizované přes `SetModel` příkaz.
/// Oddělené od markerů pro snadnější mutaci bez znalosti typu objektu.
#[derive(Component, Debug, Clone)]
pub struct ModelName(pub String);

/// Animační stav entity. Phase 4 propojí tuto komponentu s Bevy AnimationPlayer.
#[derive(Component, Debug, Clone)]
pub struct AnimationState {
    pub current: Option<String>,
    pub speed: f32,
    pub looping: bool,
    pub paused: bool,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self { current: None, speed: 1.0, looping: true, paused: false }
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
    mut player_stats: Query<(&PlayerMarker, &mut Stats, &mut Inventory)>,
) {
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

            LuaCommand::PlayAnimation { handle, name, looping, speed } => {
                if let Some(entity) = world_state.entity_for(handle) {
                    commands.entity(entity).insert(AnimationState {
                        current: Some(name),
                        speed,
                        looping,
                        paused: false,
                    });
                } else {
                    warn!("[cmd_queue] PlayAnimation: unknown handle {}", handle);
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
    )>,
    cache: Res<EntityStateCache>,
) {
    let mut lock = cache.0.lock().unwrap_or_else(|p| p.into_inner());
    lock.clear();
    for (handle, transform, model, health, anim) in &query {
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
        };
        lock.insert(handle.0, snapshot);
    }
}
