//! `ResourcesPlugin` — Bevy plugin, ktery drzi VFS, watcher a sandbox registry.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::cmd_queue::{
    advance_npc_scenario_time, assign_npc_owners, process_lua_commands, run_npc_population_director,
    sync_entity_state_cache, CommandQueue, EntityStateCache, sync_npc_brains_to_agents,
    sync_npc_scenario_runtime, tick_npc_agents, NpcAiLodConfig, NpcScenarioClockConfig,
    NpcPopulationDirectorConfig, NpcScenarioRegistry, NpcScenarioTime, EnvironmentLightConfig,
    LocalPlayerStats, LuaWorldState, PendingDamageEvent, PlayerEntityMap, PlayerStatsCache,
};
use crate::db_bridge::{DatabaseBridgeResource, DbBridge, DbCallbackQueue};
use crate::model_registry::{
    process_anim_set_commands, process_model_commands, refresh_anim_set_load_states,
    refresh_model_load_states, AnimSetCommandQueue, AnimSetRegistry,
    ModelAnimationRegistry, ModelCommandQueue, ModelRegistry,
};
use crate::npc_brain::NpcBrainRegistry;
use crate::resolver::resolve_load_order;
use crate::gui::{FontLoadQueue, FontLoadRequest, GuiDrawBuffer, ImageLoadQueue, ImageLoadRequest};
use crate::sandbox::{GameBridges, LocalEventBus, LuaSandbox};
use crate::weapons::{AmmoRegistry, AttachmentRegistry, HitboxRegistry, MaterialRegistry, WeaponRegistry};

/// All passthrough resources for `rebuild()` bundled into one SystemParam.
/// Keeps `initial_load` / `hot_reload_on_dirty` well under Bevy's 16-param limit.
#[derive(SystemParam)]
struct RebuildParams<'w> {
    bridges:       Res<'w, GameBridges>,
    cmd_queue:     Res<'w, CommandQueue>,
    local_bus:     Res<'w, LocalEventBus>,
    model_cmds:    Res<'w, ModelCommandQueue>,
    anim_set_cmds: Res<'w, AnimSetCommandQueue>,
    model_anims:   Res<'w, ModelAnimationRegistry>,
    anim_set_reg:  Res<'w, AnimSetRegistry>,
    stats_cache:   Res<'w, PlayerStatsCache>,
    entity_cache:  Res<'w, EntityStateCache>,
    db_bridge_res: Res<'w, DatabaseBridgeResource>,
    local_stats:   Res<'w, LocalPlayerStats>,
    draw_buffer:   Res<'w, GuiDrawBuffer>,
    font_queue:    Res<'w, FontLoadQueue>,
    image_queue:   Res<'w, ImageLoadQueue>,
    allowlist:     Res<'w, ServerResourceAllowlist>,
    weapon_registry: Res<'w, WeaponRegistry>,
    ammo_registry: Res<'w, AmmoRegistry>,
    attachment_registry: Res<'w, AttachmentRegistry>,
    material_registry: Res<'w, MaterialRegistry>,
    hitbox_registry: Res<'w, HitboxRegistry>,
}
use crate::types::{ResourceId, Side};
use crate::vfs::Vfs;
use crate::watcher::{drain_watcher, ResourcesDirty, VfsWatcher};

#[derive(Clone)]
pub struct ResourcesPlugin {
    pub root: PathBuf,
    pub side: Side,
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

#[derive(Default)]
pub struct SandboxRegistry {
    pub sandboxes: HashMap<ResourceId, LuaSandbox>,
    pub order: Vec<ResourceId>,
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct ResourcesSide(pub Side);

/// Množina resource ID + očekávané blake3 hashe souborů povolených serverem.
///
/// Na serverové straně zůstává `ids = None` (= načíst vše z `resources/`).
/// Na klientské straně ji nastavuje `ClientHandshakePlugin` z `ServerHello` manifestu.
/// Dokud není nastavena (`None`), klient nenačítá žádné sandboxy —
/// po handshaku se vždy spustí hot-reload s korektní sadou.
///
/// `file_hashes`: `resource_id → rel_path → blake3` — ověřuje se před načtením každého sandboxu.
#[derive(Resource, Default, Clone)]
pub struct ServerResourceAllowlist {
    pub ids: Option<HashSet<ResourceId>>,
    pub file_hashes: HashMap<ResourceId, HashMap<String, [u8; 32]>>,
}


impl Plugin for ResourcesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Vfs::new(&self.root));
        app.insert_resource(ResourcesSide(self.side));
        app.insert_non_send_resource(SandboxRegistry::default());
        app.add_message::<ResourcesDirty>();

        // Phase 3.2 - Command Queue
        app.init_resource::<CommandQueue>();
        app.init_resource::<LuaWorldState>();
        app.init_resource::<WeaponRegistry>();
        app.init_resource::<AmmoRegistry>();
        app.init_resource::<AttachmentRegistry>();
        app.init_resource::<MaterialRegistry>();
        app.init_resource::<HitboxRegistry>();
        app.init_resource::<NpcBrainRegistry>();
        app.init_resource::<NpcScenarioRegistry>();
        app.init_resource::<NpcScenarioClockConfig>();
        app.init_resource::<NpcPopulationDirectorConfig>();
        app.init_resource::<NpcScenarioTime>();
        app.init_resource::<NpcAiLodConfig>();
            app.init_resource::<EnvironmentLightConfig>();
            app.add_message::<PendingDamageEvent>();
        app.add_systems(PostUpdate, process_lua_commands);
        app.add_systems(FixedUpdate, (sync_npc_scenario_runtime, sync_npc_brains_to_agents, tick_npc_agents).chain());
        app.add_systems(Update, (advance_npc_scenario_time, run_npc_population_director, assign_npc_owners).chain());

        // Phase 3.4 - Model Registry
        app.init_resource::<ModelRegistry>();
        app.init_resource::<ModelCommandQueue>();
        app.init_resource::<ModelAnimationRegistry>();
        app.init_resource::<AnimSetRegistry>();
        app.init_resource::<AnimSetCommandQueue>();
        app.add_systems(
            PostUpdate,
            (
                process_model_commands,
                refresh_model_load_states,
                process_anim_set_commands,
                refresh_anim_set_load_states,
            )
                .chain(),
        );

        // Phase 3.8 - Cross-sandbox local event bus
        app.init_resource::<LocalEventBus>();
        app.add_systems(PostUpdate, dispatch_local_events);

        // Phase 3.7 / Phase 4 — all Arc bridges in one resource
        app.init_resource::<GameBridges>();

        // Entity state cache — synchronní čtení stavu Lua entit
        app.init_resource::<EntityStateCache>();
        app.add_systems(PostUpdate, sync_entity_state_cache
            .after(process_lua_commands)
            .before(dispatch_local_events));

        // Player stats cache + entity map + DB callback queue
        app.init_resource::<PlayerStatsCache>();
        app.init_resource::<PlayerEntityMap>();
        app.init_resource::<DbCallbackQueue>();
        app.init_resource::<DatabaseBridgeResource>();
        app.add_systems(PostUpdate, dispatch_db_callbacks);

        // Stats lokálního hráče (klient čte přes Player.GetLocalStats())
        app.init_resource::<LocalPlayerStats>();

        // GUI draw buffer — Lua thready pushují příkazy; GuiRenderPlugin drainuje
        app.init_resource::<GuiDrawBuffer>();
        app.init_resource::<FontLoadQueue>();
        app.init_resource::<ImageLoadQueue>();

        // Server resource allowlist — klient sem dostane seznam povolených IDs po handshaku
        app.init_resource::<ServerResourceAllowlist>();

        // Console / chat příkazy — dispatch do sandboxů (PostUpdate)
        app.add_systems(PostUpdate, dispatch_console_commands);

        // Tick Lua threadů v PreUpdate (před process_lua_commands)
        app.add_systems(PreUpdate, tick_lua_threads);

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
                        "[core_resources] watcher disabled — failed to start: {e}"
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
    mut model_registry: ResMut<ModelRegistry>,
    p: RebuildParams,
) {
    rebuild(
        &mut vfs, side.0, &mut registry,
        p.cmd_queue.clone(), p.local_bus.clone(), p.model_cmds.clone(),
        p.anim_set_cmds.clone(),
        &mut model_registry,
        p.model_anims.clone(),
        p.anim_set_reg.clone(),
        p.bridges.clone(),
        p.stats_cache.clone(), p.entity_cache.clone(),
        p.db_bridge_res.0.clone(), p.local_stats.clone(), p.draw_buffer.clone(),
        Some((p.font_queue.clone(), p.image_queue.clone())),
        p.weapon_registry.clone(), p.ammo_registry.clone(), p.attachment_registry.clone(), p.material_registry.clone(), p.hitbox_registry.clone(),
        &p.allowlist,
    );
}

fn hot_reload_on_dirty(
    mut events: MessageReader<ResourcesDirty>,
    mut vfs: ResMut<Vfs>,
    side: Res<ResourcesSide>,
    mut registry: NonSendMut<SandboxRegistry>,
    mut model_registry: ResMut<ModelRegistry>,
    p: RebuildParams,
) {
    if events.is_empty() {
        return;
    }
    let count = events.read().count();
    info!(
        "[core_resources] filesystem change ({count} event{}), reloading",
        if count == 1 { "" } else { "s" }
    );
    rebuild(
        &mut vfs, side.0, &mut registry,
        p.cmd_queue.clone(), p.local_bus.clone(), p.model_cmds.clone(),
        p.anim_set_cmds.clone(),
        &mut model_registry,
        p.model_anims.clone(),
        p.anim_set_reg.clone(),
        p.bridges.clone(),
        p.stats_cache.clone(), p.entity_cache.clone(),
        p.db_bridge_res.0.clone(), p.local_stats.clone(), p.draw_buffer.clone(),
        Some((p.font_queue.clone(), p.image_queue.clone())),
        p.weapon_registry.clone(), p.ammo_registry.clone(), p.attachment_registry.clone(), p.material_registry.clone(), p.hitbox_registry.clone(),
        &p.allowlist,
    );
}

fn rebuild(
    vfs: &mut Vfs,
    side: Side,
    registry: &mut SandboxRegistry,
    cmd_queue: CommandQueue,
    local_bus: LocalEventBus,
    model_cmds: ModelCommandQueue,
    anim_set_cmds: AnimSetCommandQueue,
    model_registry: &mut ModelRegistry,
    model_anims: ModelAnimationRegistry,
    anim_set_reg: AnimSetRegistry,
    bridges: GameBridges,
    stats_cache: PlayerStatsCache,
    entity_cache: EntityStateCache,
    db_bridge: Option<DbBridge>,
    local_stats: LocalPlayerStats,
    draw_buffer: GuiDrawBuffer,
    load_queues: Option<(FontLoadQueue, ImageLoadQueue)>,
    weapon_registry: WeaponRegistry,
    ammo_registry: AmmoRegistry,
    attachment_registry: AttachmentRegistry,
    material_registry: MaterialRegistry,
    hitbox_registry: HitboxRegistry,
    allowlist: &ServerResourceAllowlist,
) {
    let report = vfs.rescan();
    for err in &report.errors {
        error!("[core_resources] scan error: {err}");
    }
    info!(
        "[core_resources] scanned {} resource(s) from {}",
        report.loaded.len(),
        vfs.root().display()
    );

    let stream_models = vfs.scan_stream_models();
    model_registry.rebuild_from_scan(stream_models);

    // Na klientovi nenačítáme sandboxy dokud server nepošle manifest se seznamem
    // povolených resources (allowlist.ids == None = handshake ještě neproběhl).
    if side == Side::Client && allowlist.ids.is_none() {
        info!("[core_resources] client: allowlist not set yet (pending handshake), skipping sandbox load");
        registry.sandboxes.clear();
        registry.order.clear();
        return;
    }

    let mut order = match resolve_load_order(vfs.manifests()) {
        Ok(o) => o,
        Err(e) => {
            error!("[core_resources] dependency resolution failed: {e}");
            return;
        }
    };

    // Filtr: na klientovi načíst jen resources, které server poslal v ServerHello
    if let Some(ref allowed) = allowlist.ids {
        let before = order.len();
        order.retain(|id| allowed.contains(id));
        let dropped = before - order.len();
        if dropped > 0 {
            info!("[core_resources] allowlist filtered {dropped} stale/local resource(s)");
        }
    }

    registry.sandboxes.clear();
    registry.order.clear();

    for id in &order {
        let manifest = match vfs.get(id) {
            Some(m) => m,
            None => continue,
        };

        // Ověř hash každého souboru patřícího do tohoto resource.
        // Klient tak nemůže použít lokálně modifikované nebo poškozené soubory.
        if let Some(expected) = allowlist.file_hashes.get(id) {
            if !verify_resource_files(manifest, expected) {
                error!(
                    "[core_resources] hash mismatch for '{}' — skipping (files may be corrupt or locally modified)",
                    id
                );
                continue;
            }
        }

        // Enqueue font/image loads for the client renderer (initial load only).
        if side == Side::Client {
            if let Some((ref font_queue, ref image_queue)) = load_queues {
                for fd in &manifest.fonts {
                    let abs = manifest.root.join(&fd.path);
                    font_queue.push(FontLoadRequest {
                        font_id: fd.id.clone(),
                        abs_path: abs.to_string_lossy().into_owned(),
                    });
                }
                info!("[core_resources] '{}': {} image(s) declared, {} font(s) declared",
                    id, manifest.images.len(), manifest.fonts.len());
                for imd in &manifest.images {
                    let abs = manifest.root.join(&imd.path);
                    info!("[core_resources] enqueueing image '{}' -> '{}'", imd.id, abs.display());
                    image_queue.push(ImageLoadRequest {
                        image_id: imd.id.clone(),
                        abs_path: abs.to_string_lossy().into_owned(),
                    });
                }
            }
        }

        let ls = if side == Side::Client { Some(local_stats.clone()) } else { None };
        match LuaSandbox::create(
            manifest,
            side,
            cmd_queue.clone(),
            local_bus.clone(),
            model_cmds.clone(),
            model_registry.clone(),
            model_anims.clone(),
            bridges.raycast.clone(),
            bridges.engine.clone(),
            bridges.input.clone(),
            bridges.connection.clone(),
            stats_cache.clone(),
            entity_cache.clone(),
            db_bridge.clone(),
            ls,
            draw_buffer.clone(),
            bridges.ace.clone(),
            bridges.auth.clone(),
            bridges.crosshair.clone(),
            bridges.camera.clone(),
            anim_set_cmds.clone(),
            anim_set_reg.clone(),
            weapon_registry.clone(),
            ammo_registry.clone(),
            attachment_registry.clone(),
            material_registry.clone(),
            hitbox_registry.clone(),
        ) {
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

/// Phase 3.8 — Drain LocalEventBus a dispatch do vsech sandboxu.
fn dispatch_local_events(
    local_bus: Res<LocalEventBus>,
    registry: NonSend<SandboxRegistry>,
) {
    let events = local_bus.drain();
    if events.is_empty() {
        return;
    }
    for evt in &events {
        let mut delivered = 0usize;
        for sandbox in registry.sandboxes.values() {
            match sandbox.dispatch_incoming(&evt.name, &evt.payload, None) {
                Ok(n) => delivered += n,
                Err(e) => warn!(
                    "[local_bus] handler error in '{}' for event '{}': {}",
                    sandbox.id, evt.name, e
                ),
            }
        }
        if delivered == 0 {
            trace!(
                "[local_bus] no handler for '{}' (payload {} bytes)",
                evt.name,
                evt.payload.len()
            );
        }
    }
}

/// PreUpdate — resumuje Lua thready jejichž wait timer vypršel.
fn tick_lua_threads(
    time: Res<Time>,
    mut registry: NonSendMut<SandboxRegistry>,
) {
    let delta_ms = (time.delta_secs() * 1000.0) as u64;
    for sandbox in registry.sandboxes.values_mut() {
        sandbox.tick_threads(delta_ms);
    }
}

/// Drain DbCallbackQueue a dispatch výsledků do příslušných sandboxů.
fn dispatch_db_callbacks(
    db_queue: Res<DbCallbackQueue>,
    registry: NonSend<SandboxRegistry>,
) {
    let pending = db_queue.drain();
    if pending.is_empty() {
        return;
    }
    for entry in pending {
        if let Some(sandbox) = registry.sandboxes.get(&entry.resource_id) {
            sandbox.invoke_db_callback(entry.callback_id, entry.result);
        } else {
            warn!(
                "[db_callbacks] sandbox '{}' not found for callback {}",
                entry.resource_id, entry.callback_id
            );
        }
    }
}

/// Drainuje `LuaCmdDispatch` a dispatchuje příkazy do všech sandboxů.
/// Příkaz je zpracován prvním sandboxem, který má registrovaný handler.
/// Pokud žádný sandbox příkaz nezná, loguje se info zpráva (viditelná v konzoli).
fn dispatch_console_commands(
    bridges: Res<GameBridges>,
    registry: NonSend<SandboxRegistry>,
) {
    let pending = bridges.cmd_dispatch.drain();
    if pending.is_empty() {
        return;
    }
    for cmd in &pending {
        let mut total = 0usize;
        for sandbox in registry.sandboxes.values() {
            match sandbox.dispatch_command(cmd) {
                Ok(n) => total += n,
                Err(e) => warn!(
                    "[console] error in '{}' for command '{}': {}",
                    sandbox.id, cmd.name, e
                ),
            }
        }
        if total == 0 {
            info!("[console] unknown command: '{}'", cmd.raw);
        }
    }
}

/// Ověří, že všechny soubory resource na disku odpovídají serverem očekávaným blake3 hashům.
/// Vrátí `false` při jakémkoliv nesouladu nebo chybějícím souboru.
fn verify_resource_files(
    manifest: &crate::manifest::Manifest,
    expected: &HashMap<String, [u8; 32]>,
) -> bool {
    for (rel_path, &expected_hash) in expected {
        let abs = manifest.root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        match std::fs::read(&abs) {
            Ok(bytes) => {
                let actual = *blake3::hash(&bytes).as_bytes();
                if actual != expected_hash {
                    warn!(
                        "[core_resources] hash mismatch: {} (expected {:?}, got {:?})",
                        abs.display(), &expected_hash[..4], &actual[..4]
                    );
                    return false;
                }
            }
            Err(e) => {
                warn!("[core_resources] cannot read '{}': {}", abs.display(), e);
                return false;
            }
        }
    }
    true
}
