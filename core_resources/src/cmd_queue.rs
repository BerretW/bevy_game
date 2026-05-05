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
use core_shared::PlayerMarker;

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
    /// Damage intent — jen server side; klient dostane runtime error z Lua API.
    ApplyDamage {
        target_handle: u64,
        amount: f32,
        source_handle: Option<u64>,
    },
    /// Phase 3.5 — replikovana entita spawnovana serverem, klienti ji dostanou
    /// pres lightyear replication. Server-only.
    SpawnNetworkedObject {
        handle: u64,
        model: String,
        pos: [f32; 3],
        rot: [f32; 3],
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

/// Marker component na replikovanych entitách (`World.SpawnNetworkedObject`).
/// Phase 3.5 — lightyear Replicate je pridan pri process_lua_commands.
#[derive(Component, Debug, Clone)]
pub struct NetworkedObjectMarker {
    pub model: String,
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

// ---------------------------------------------------------------------------
// Bevy systém
// ---------------------------------------------------------------------------

/// Zpracuje všechny pending `LuaCommand`y.
/// Přidán do `PostUpdate`, aby měl k dispozici příkazy z celého Update frame.
pub fn process_lua_commands(
    cmd_queue: Res<CommandQueue>,
    mut world_state: ResMut<LuaWorldState>,
    player_map: Res<PlayerEntityMap>,
    mut commands: Commands,
    mut damage_events: MessageWriter<PendingDamageEvent>,
    mut player_stats: Query<(&PlayerMarker, &mut Stats, &mut Inventory)>,
) {
    for cmd in cmd_queue.drain() {
        match cmd {
            LuaCommand::SpawnLocalObject { handle, model, pos, rot } => {
                let entity = commands
                    .spawn((
                        LocalObjectMarker { model: model.clone() },
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
                    commands.entity(entity).insert(Transform {
                        translation: Vec3::new(pos[0], pos[1], pos[2]),
                        rotation: Quat::from_euler(
                            EulerRot::XYZ,
                            rot[0].to_radians(),
                            rot[1].to_radians(),
                            rot[2].to_radians(),
                        ),
                        scale: Vec3::ONE,
                    });
                } else {
                    warn!("[cmd_queue] SetTransform: unknown handle {}", handle);
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
                // Spawneme entitu s NetworkedObjectMarker. core_net observer
                // (Add<NetworkedObjectMarker>) automaticky prida lightyear Replicate.
                let entity = commands
                    .spawn((
                        NetworkedObjectMarker { model: model.clone() },
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
