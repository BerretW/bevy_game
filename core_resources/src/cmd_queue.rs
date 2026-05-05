//! Phase 3.2 — Command Queue: bezpečný Lua → ECS most.
//!
//! Lua sandbox nesmí přímo mutovat ECS svět (`mlua` je `!Send`, sandbox běží
//! na main threadu v `NonSend` resource). Místo toho Lua vkládá záměry do
//! sdíleného `CommandQueue` bufferu. Bevy systém `process_lua_commands`
//! v `PostUpdate` frontu vybere a bezpečně aplikuje příkazy na ECS svět.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

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
/// Phase 3.4 Model Registry bude tento komponent číst pro načtení meshe.
#[derive(Component, Debug, Clone)]
pub struct LocalObjectMarker {
    pub model: String,
}

/// Event emitovaný při zpracování `World.ApplyDamage`.
/// Phase 3.3 combat systémy se na tento event přihlásí přes `EventReader`.
#[derive(Event, Debug, Clone)]
pub struct PendingDamageEvent {
    pub target: Entity,
    pub amount: f32,
    pub source: Option<Entity>,
}

// ---------------------------------------------------------------------------
// Bevy systém
// ---------------------------------------------------------------------------

/// Zpracuje všechny pending `LuaCommand`y.
/// Přidán do `PostUpdate`, aby měl k dispozici příkazy z celého Update frame.
pub fn process_lua_commands(
    cmd_queue: Res<CommandQueue>,
    mut world_state: ResMut<LuaWorldState>,
    mut commands: Commands,
    mut damage_events: EventWriter<PendingDamageEvent>,
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
        }
    }
}
