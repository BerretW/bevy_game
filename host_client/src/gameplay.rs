//! Phase 3 — klientska gameplay vrstva.
//!
//! Phase 3.7: `RaycastBridge` se aktualizuje kazdy frame z pozice mysi.
//! Lua sandbox cte pres `Raycast.GetGroundPosition()`.

use bevy::input::mouse::MouseMotion;
use bevy::ecs::message::MessageReader;
use bevy::prelude::*;
use bevy::transform::components::TransformTreeChanged;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use avian3d::prelude::*;
use core_net::{ClientHandshakeState, HandshakeStatus};
use bevy::asset::AssetPath;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use bevy_gltf::{
    GltfExtras,
    GltfAssetLabel,
    GltfMaterialExtras,
    GltfMaterialName,
    GltfMeshExtras,
    GltfMeshName,
    GltfSceneExtras,
};
use core_net::{player_action, InputChannel, PlayerInput};
use core_resources::{
    AnimationState, AttachedAnimSets, CameraAttachment, ConnectionInfo, CrosshairHit,
    DummyObjectMarker, DummyPrimitiveKind, EntityHandle, GameBridges, InputSnapshot,
    LocalEventBus, LocalObjectMarker, LuaWorldState, ModelAnimationRegistry, ModelName,
    ModelRegistry, StairsCollider, process_lua_commands, sync_entity_state_cache,
};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::Predicted;

use crate::config::ClientConfigResource;
use crate::native_assets::{AdmHandleCache, PedAdsAnimIndex};
use crate::drawable::AdmSceneRoot;
use crate::AppState;
use crate::drawable::{GltfHandleCache, PedPhysicsDef, PedPhysicsRegistry};

const THIRD_PERSON_DISTANCE: f32 = 5.5;
const FIRST_PERSON_EYE_HEIGHT: f32 = 1.7;
const MAX_PITCH_RAD: f32 = 1.25;
const MOUSE_SENS_SCALE: f32 = 0.0025;
const POSITION_SMOOTHING_RATE: f32 = 14.0;
const ROTATION_SMOOTHING_RATE: f32 = 18.0;
/// Výchozí FOV kamery v radiánech (60°) — Bevy default.
const DEFAULT_CAMERA_FOV: f32 = std::f32::consts::FRAC_PI_3;

#[derive(Resource, Clone, Copy)]
pub struct LocalClientId(pub u64);

#[derive(Resource, Clone, Copy)]
struct CameraLookState {
    yaw: f32,
    pitch: f32,
}

impl Default for CameraLookState {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: -0.2,
        }
    }
}

#[derive(Component)]
struct MainGameplayCamera;

#[derive(Component)]
struct PlayerVisualAttached;

#[derive(Component)]
struct LocalObjectVisualAttached;

#[derive(Component)]
struct DummyObjectVisualAttached;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocomotionState {
    Idle,
    Walk,
    Run,
    Sprint,
}

impl Default for LocomotionState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerAnimState {
    Idle,
    Walk,
    Run,
    Sprint,
    Jump,
    Fall,
    Land,
}

impl PlayerAnimState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Walk => "walk",
            Self::Run => "run",
            Self::Sprint => "sprint",
            Self::Jump => "jump",
            Self::Fall => "fall",
            Self::Land => "land",
        }
    }
}

impl Default for PlayerAnimState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Component, Default)]
struct AutoPlayerAnimMemory {
    was_grounded: bool,
    land_until: f32,
    airborne_since: Option<f32>,
    move_intent_until: f32,
    filtered_horiz_speed: f32,
    locomotion: LocomotionState,
    anim_state: PlayerAnimState,
}

#[derive(Resource, Default)]
struct LuaAnimationGraphCache {
    // (model_name, clip_index) -> (graph_handle, node_index)
    map: HashMap<(String, u32), (Handle<AnimationGraph>, AnimationNodeIndex)>,
}

#[derive(Resource, Default)]
struct ModelAnimationDiscoveryCache {
    adm: HashMap<String, Handle<crate::drawable::AdmScene>>,
    gltf: HashMap<String, Handle<bevy::gltf::Gltf>>,
}

pub struct ClientGameplayPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        // GLTF SceneRoot spawn vyzaduje reflektovane registrace komponent,
        // jinak scene_spawner panicne na "unregistered type".
        app.register_type::<Transform>()
            .register_type::<GlobalTransform>()
            .register_type::<Visibility>()
            .register_type::<InheritedVisibility>()
            .register_type::<ViewVisibility>()
            .register_type::<TransformTreeChanged>()
            .register_type::<Mesh3d>()
            .register_type::<MeshMaterial3d<StandardMaterial>>()
            .register_type::<bevy::camera::primitives::Aabb>()
            .register_type::<bevy::mesh::skinning::SkinnedMesh>()
            .register_type::<GltfExtras>()
            .register_type::<GltfSceneExtras>()
            .register_type::<GltfMeshExtras>()
            .register_type::<GltfMeshName>()
            .register_type::<GltfMaterialExtras>()
            .register_type::<GltfMaterialName>()
            .register_type::<bevy::ecs::hierarchy::ChildOf>()
            .register_type::<bevy::ecs::hierarchy::Children>()
            .register_type::<Name>();

        app.init_resource::<CameraLookState>();
        app.init_resource::<LuaAnimationGraphCache>();
        app.init_resource::<ModelAnimationDiscoveryCache>();
        // Scéna a kamera se nastavují až při vstupu do InGame, ne na Startup
        app.add_systems(OnEnter(AppState::InGame), (setup_scene_and_camera, reset_engine_state));
        app.add_systems(OnExit(AppState::InGame), reset_connection_bridge);
        app.add_systems(
            Update,
            (
                toggle_camera_mode,
                update_camera_look_from_mouse,
                apply_cursor_mode,
                attach_player_model_to_new_players,
                prefer_predicted_player_visuals,
                sync_net_transform_to_render,
                update_camera_follow,
                update_raycast_bridge,
                update_crosshair_entity,
                update_input_bridge,
                update_connection_bridge,
                update_local_player_visibility,
                publish_input_state_to_lua,
                publish_stairs_state_to_lua,
                attach_mesh_to_local_objects,
                attach_mesh_to_dummy_objects,
                update_player_state_driven_animations,
                update_model_animation_registry,
                apply_lua_animation_state,
                handle_engine_cmds,
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
        app.add_systems(
            FixedUpdate,
            (
                apply_player_movement,
                collect_and_send_input,
            ).chain().run_if(in_state(AppState::InGame)),
        );
        // Registrace replicated entity handles v LuaWorldState.
        // Musí běžet po process_lua_commands (lokální spawny) ale před sync_entity_state_cache
        // (aby replicated entity byly v cache ve stejném framu).
        app.add_systems(
            PostUpdate,
            sync_replicated_entity_handles
                .after(process_lua_commands)
                .before(sync_entity_state_cache),
        );
        // app.add_systems(Update, debug_player_movement.run_if(on_timer(std::time::Duration::from_secs(2))));
    }
}

/// Registruje entity replicated přes lightyear v `LuaWorldState`.
/// Zachytí všechny entity s nově přidaným `EntityHandle` komponentem —
/// pro lokální spawn jsou již registrované přes `process_lua_commands`,
/// pro replicated entity ze serveru je toto jediný bod registrace.
fn sync_replicated_entity_handles(
    new_handles: Query<(Entity, &EntityHandle), Added<EntityHandle>>,
    mut world_state: ResMut<LuaWorldState>,
) {
    for (entity, handle) in &new_handles {
        if world_state.entity_for(handle.0).is_none() {
            world_state.register(handle.0, entity);
            debug!("[gameplay] registered replicated entity handle={} entity={:?}", handle.0, entity);
        }
    }
}

const PLAYER_MOVE_SPEED: f32 = 5.0;
const PLAYER_SPRINT_MULT: f32 = 1.8;
const PLAYER_CROUCH_MULT: f32 = 0.5;
const PLAYER_JUMP_VEL: f32 = 6.0;
/// Fallback konstanty — použijí se pokud player.ped.toml není ještě načten.
const GROUND_VEL_THRESHOLD: f32 = 0.25;
const GROUND_CASTER_RADIUS: f32 = 0.28;
const GROUND_CASTER_MAX_DISTANCE: f32 = 0.12;

/// (player.ped.toml) pokud je načten — jinak se použijí fallback konstanty.
fn apply_player_movement(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    look: Res<CameraLookState>,
    local_client_id: Option<Res<LocalClientId>>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    mut players: Query<(&PlayerMarker, &mut LinearVelocity, Option<&ShapeHits>), With<Predicted>>,
) {
    let Some(lid) = local_client_id else { return };
    let bindings = &cfg.0.input.keys;

    // Načti preferovaný ped profil (primárně "player", fallback na první dostupný).
    let ped = resolve_default_ped_profile(&ped_reg, &ped_assets);

    let mut move_x = 0.0f32;
    let mut move_y = 0.0f32;
    if keys.pressed(bindings.move_forward)  { move_y += 1.0; }
    if keys.pressed(bindings.move_backward) { move_y -= 1.0; }
    if keys.pressed(bindings.move_right)    { move_x -= 1.0; }
    if keys.pressed(bindings.move_left)     { move_x += 1.0; }

    let sprint = keys.pressed(bindings.sprint);
    let crouch = keys.pressed(bindings.crouch);
    let jump   = keys.pressed(bindings.jump);

    // Rychlost pohybu z ped profilu nebo fallback
    let (run_speed, sprint_speed, crouch_speed, movement_smoothing) = if let Some(p) = ped {
        (
            p.movement.run_speed,
            p.movement.sprint_speed,
            p.movement.crouch_speed,
            p.movement.movement_smoothing,
        )
    } else {
        (
            PLAYER_MOVE_SPEED,
            PLAYER_MOVE_SPEED * PLAYER_SPRINT_MULT,
            PLAYER_MOVE_SPEED * PLAYER_CROUCH_MULT,
            0.0,
        )
    };
    let speed = if crouch { crouch_speed } else if sprint { sprint_speed } else { run_speed };

    // Parametry skoku z ped profilu nebo fallback
    let (jump_impulse, grounded_thresh, double_jump_enabled) = if let Some(p) = ped {
        (p.jump.impulse, p.jump.grounded_vel_threshold, p.jump.double_jump)
    } else {
        (PLAYER_JUMP_VEL, GROUND_VEL_THRESHOLD, false)
    };

    // Rotace vstupního vektoru podle yaw kamery (world-space).
    let yaw = look.yaw;
    let world_x = yaw.cos() * move_x + yaw.sin() * move_y;
    let world_z = -yaw.sin() * move_x + yaw.cos() * move_y;

    let mag2 = world_x * world_x + world_z * world_z;
    let (world_x, world_z) = if mag2 > 1.0 {
        let inv = mag2.sqrt().recip();
        (world_x * inv, world_z * inv)
    } else {
        (world_x, world_z)
    };

    for (marker, mut vel, shape_hits) in players.iter_mut() {
        if marker.client_id != lid.0 { continue; }
        let target_x = world_x * speed;
        let target_z = world_z * speed;

        if movement_smoothing <= 0.0 {
            vel.x = target_x;
            vel.z = target_z;
        } else {
            // Exponenciální smoothing: vyšší rate = rychlejší dorovnání.
            let alpha = (1.0 - (-movement_smoothing * time.delta_secs()).exp()).clamp(0.0, 1.0);
            vel.x = vel.x + (target_x - vel.x) * alpha;
            vel.z = vel.z + (target_z - vel.z) * alpha;
        }

        let has_ground_contact = has_ground_contact(shape_hits);

        let can_jump = if double_jump_enabled {
            // Legacy fallback: když je double-jump zapnutý, ponecháme i velocity gate.
            has_ground_contact || vel.y.abs() < grounded_thresh
        } else {
            // Požadavek: bez double-jump musí být fyzický kontakt pod hráčem.
            has_ground_contact
        };

        if jump && can_jump {
            vel.y = jump_impulse;
        }
    }
}

fn has_ground_contact(shape_hits: Option<&ShapeHits>) -> bool {
    shape_hits
        .map(|hits| {
            hits.iter().any(|hit| {
                // normal2 míří od druhého collideru směrem k casteru.
                // Pro "zem pod hráčem" chceme výrazně upward normálu.
                (-hit.normal2).dot(Vec3::Y) > 0.35
            })
        })
        .unwrap_or(false)
}

fn resolve_default_ped_profile<'a>(
    ped_reg: &'a PedPhysicsRegistry,
    ped_assets: &'a Assets<PedPhysicsDef>,
) -> Option<&'a PedPhysicsDef> {
    ped_reg
        .0
        .get("player")
        .and_then(|h| ped_assets.get(h))
        .or_else(|| ped_reg.0.values().find_map(|h| ped_assets.get(h)))
}

fn resolve_ped_profile_for_model<'a>(
    model_name: &str,
    ped_reg: &'a PedPhysicsRegistry,
    ped_assets: &'a Assets<PedPhysicsDef>,
) -> Option<&'a PedPhysicsDef> {
    ped_reg
        .0
        .values()
        .find_map(|h| {
            let ped = ped_assets.get(h)?;
            if ped.identity.model == model_name {
                Some(ped)
            } else {
                None
            }
        })
        .or_else(|| resolve_default_ped_profile(ped_reg, ped_assets))
}

fn setup_scene_and_camera(
    mut commands: Commands,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-5.0, 6.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        MainGameplayCamera,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 15000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(
            EulerRot::XYZ,
            -1.0,
            -0.8,
            0.0,
        )),
    ));

    info!("[gameplay/client] 3D scene ready (camera toggle: F6)");
}

/// Aktualizuje `RaycastBridge` podle aktualni pozice mysi.
/// Tato pozice se pouziva v Lua `Raycast.GetGroundPosition()`.
fn update_raycast_bridge(
    camera_q: Query<&GlobalTransform, With<MainGameplayCamera>>,
    bridges: Res<GameBridges>,
) {
    let raycast = &bridges.raycast;
    let Ok(cam_transform) = camera_q.single() else { return };
    let origin = cam_transform.translation();
    let dir = cam_transform.forward();

    let dir_y = dir.y;
    if dir_y.abs() < 0.0001 {
        return;
    }

    // Prusecik pohledu kamery s rovinou zeme Y=0.
    let t = -origin.y / dir_y;
    if t <= 0.0 {
        return;
    }

    let hit = origin + dir * t;
    raycast.set_pos([hit.x, 0.0, hit.z]);
}

fn toggle_camera_mode(keys: Res<ButtonInput<KeyCode>>, bridges: Res<GameBridges>) {
    if !keys.just_pressed(KeyCode::F6) {
        return;
    }
    let new_first = !bridges.camera.is_first_person();
    bridges.camera.set_first_person(new_first);
    info!("[gameplay/client] camera mode -> {}", if new_first { "first_person" } else { "third_person" });
}

fn update_camera_look_from_mouse(
    mut motions: MessageReader<MouseMotion>,
    cfg: Res<ClientConfigResource>,
    bridges: Res<GameBridges>,
    mut look: ResMut<CameraLookState>,
) {
    // Don't rotate camera while Lua has the cursor unlocked (e.g. ESC menu open)
    if !bridges.engine.cursor_locked() {
        for _ in motions.read() {}
        return;
    }

    let mut delta = Vec2::ZERO;
    for m in motions.read() {
        delta += m.delta;
    }

    if delta == Vec2::ZERO {
        return;
    }

    let sens = cfg.0.input.mouse_sensitivity * MOUSE_SENS_SCALE;
    let invert_y = if cfg.0.input.invert_y { 1.0 } else { -1.0 };

    look.yaw = (look.yaw - delta.x * sens).rem_euclid(std::f32::consts::TAU);
    look.pitch = (look.pitch + delta.y * sens * invert_y)
        .clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);
}

fn apply_cursor_mode(
    bridges: Res<GameBridges>,
    handshake: Res<ClientHandshakeState>,
    mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor_q.single_mut() else { return };

    // Auth UI needs a free cursor — always unlock while waiting for login.
    if handshake.status == HandshakeStatus::AwaitingAuth {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
        return;
    }

    let locked = bridges.engine.cursor_locked();
    cursor.visible = !locked;
    cursor.grab_mode = if locked { CursorGrabMode::Locked } else { CursorGrabMode::None };
}

fn attach_player_model_to_new_players(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    ped_anim_index: Res<PedAdsAnimIndex>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    predicted_players: Query<&PlayerMarker, With<Predicted>>,
    new_players: Query<
        (Entity, &PlayerMarker, Option<&Predicted>),
        (With<NetTransform>, Without<PlayerVisualAttached>),
    >,
) {
    let predicted_ids: HashSet<u64> = predicted_players
        .iter()
        .map(|m| m.client_id)
        .collect();

    for (entity, marker, predicted) in new_players.iter() {
        // Pokud existuje predicted entita pro stejneho hrace,
        // vizual attachujeme jen na ni (zabrani statickym duplikatum).
        if predicted.is_none() && predicted_ids.contains(&marker.client_id) {
            continue;
        }

        let model_name = resolve_default_ped_profile(&ped_reg, &ped_assets)
            .map(|p| p.identity.model.as_str())
            .unwrap_or("player");
        let attached_anim_sets = ped_anim_index
            .0
            .get(model_name)
            .cloned()
            .unwrap_or_default();
        let model_path = format!("models/{}.adm", model_name);
        let model_handle = asset_server.load::<crate::drawable::AdmScene>(model_path);

        commands
            .entity(entity)
            .insert((
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Visible,
                InheritedVisibility::default(),
                ViewVisibility::default(),
                PlayerVisualAttached,
                AutoPlayerAnimMemory::default(),
                // FiveM-style: RigidBody přímo na root entitě hráče.
                // COL_player z player.drawable bude compound collider tohoto těla.
                RigidBody::Dynamic,
                LockedAxes::new()
                    .lock_rotation_x()
                    .lock_rotation_y()
                    .lock_rotation_z(),
                // Nulové tření s Multiply combine rule zabraňuje "wall-stick"
                // při kontaktu se stěnou během skoku.
                Friction::ZERO.with_combine_rule(CoefficientCombine::Multiply),
                // Ground-check pro jump gating (double_jump=false vyžaduje kontakt se zemí).
                ShapeCaster::new(
                    Collider::sphere(GROUND_CASTER_RADIUS),
                    Vec3::ZERO,
                    Quat::IDENTITY,
                    Dir3::NEG_Y,
                )
                .with_max_distance(GROUND_CASTER_MAX_DISTANCE)
                .with_max_hits(4),
            ))
            .with_children(|p| {
                let mut child = p.spawn((
                    AdmSceneRoot(model_handle.clone()),
                    // Bez DisableDrawableCollisions — COL_player z manifestu
                    // se stane compound coliderem parent RigidBody.
                    ModelName(model_name.to_string()),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                    GlobalTransform::default(),
                    Visibility::default(),
                    InheritedVisibility::default(),
                    ViewVisibility::default(),
                ));
                
                // Přidej počáteční idle animaci z player.ped.toml — 
                // zajistí, že model hraje animaci hned po spawnu, místo aby zůstal v t-pose
                if let Some(ped) = resolve_ped_profile_for_model(model_name, &ped_reg, &ped_assets) {
                    let idle_clip = ped.animations.idle.clone();
                    if !idle_clip.is_empty() {
                        child.insert(AnimationState {
                            current: Some(idle_clip),
                            speed: 1.0,
                            looping: true,
                            paused: false,
                            blend_time: 0.0,
                            flags: 1,
                        });
                    }
                }
                
                if !attached_anim_sets.is_empty() {
                    child.insert(AttachedAnimSets {
                        sets: attached_anim_sets.clone(),
                    });
                }
            });

        info!(
            "[gameplay/client] ADM model attached to player {:?} (client_id={})",
            entity, marker.client_id
        );
    }
}

fn update_player_state_driven_animations(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    local_bus: Res<LocalEventBus>,
    local_client_id: Option<Res<LocalClientId>>,
    mut commands: Commands,
    mut model_roots: Query<
        (
            Entity,
            &AdmSceneRoot,
            &ModelName,
            &bevy::ecs::hierarchy::ChildOf,
            Option<&mut AnimationState>,
        ),
        With<AdmSceneRoot>,
    >,
    mut players: Query<(
        &PlayerMarker,
        Option<&EntityHandle>,
        &LinearVelocity,
        Option<&ShapeHits>,
        &mut AutoPlayerAnimMemory,
    )>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let bindings = &cfg.0.input.keys;
    let has_move_input =
        keys.pressed(bindings.move_forward)
            || keys.pressed(bindings.move_backward)
            || keys.pressed(bindings.move_left)
            || keys.pressed(bindings.move_right);

    for (model_entity, adm_root, model_name, child_of, anim_state) in &mut model_roots {
        let parent = child_of.parent();
        let Ok((marker, handle, vel, shape_hits, mut memory)) = players.get_mut(parent) else {
            continue;
        };

        let Some(ped) = resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets) else {
            continue;
        };

        let raw_grounded = has_ground_contact(shape_hits);
        if raw_grounded {
            memory.airborne_since = None;
        } else if memory.airborne_since.is_none() {
            memory.airborne_since = Some(now);
        }

        // Ground grace okno tlumí 1-2 frame výpadky ShapeCaster kontaktu.
        let grounded_grace = 0.10;
        let grounded = raw_grounded
            || memory
                .airborne_since
                .map(|t| now - t <= grounded_grace)
                .unwrap_or(false);

        if grounded && !memory.was_grounded && vel.y < -0.10 {
            memory.land_until = now + 0.18;
        }
        memory.was_grounded = grounded;

        let horiz_speed = Vec2::new(vel.x, vel.z).length();
        // Potlačí frame-to-frame jitter velocity a zklidní animační rozhodování.
        let speed_alpha = (1.0 - (-10.0 * dt).exp()).clamp(0.0, 1.0);
        memory.filtered_horiz_speed += (horiz_speed - memory.filtered_horiz_speed) * speed_alpha;

        let idle_threshold = ped.movement.idle_threshold.max(0.01);
        let walk_threshold = (ped.movement.run_speed * 0.60).max(idle_threshold + 0.05);
        let sprint_threshold = (ped.movement.run_speed + ped.movement.sprint_speed) * 0.5;

        // Pokud lokální hráč drží movement input, krátce podrž locomotion stav,
        // i když fyzická rychlost spadne při drhnutí o collider.
        let is_local_player = local_client_id
            .as_ref()
            .map(|id| id.0 == marker.client_id)
            .unwrap_or(false);
        if is_local_player && has_move_input {
            memory.move_intent_until = now + 0.22;
        }
        let movement_intent_active = is_local_player && now < memory.move_intent_until;

        // Hysteréze: oddělené entry/exit prahy, aby Idle byl dosažitelný i při low-speed jitteru.
        let idle_exit = idle_threshold + 0.06;
        let walk_up = walk_threshold + 0.16;
        let walk_down = (walk_threshold - 0.16).max(idle_exit + 0.02);
        let sprint_up = sprint_threshold + 0.20;
        let sprint_down = (sprint_threshold - 0.20).max(walk_up + 0.02);

        memory.locomotion = match memory.locomotion {
            LocomotionState::Idle => {
                if memory.filtered_horiz_speed > idle_exit || (movement_intent_active && grounded) {
                    LocomotionState::Walk
                } else {
                    LocomotionState::Idle
                }
            }
            LocomotionState::Walk => {
                if memory.filtered_horiz_speed <= idle_threshold && !movement_intent_active {
                    LocomotionState::Idle
                } else if memory.filtered_horiz_speed >= walk_up {
                    LocomotionState::Run
                } else {
                    LocomotionState::Walk
                }
            }
            LocomotionState::Run => {
                if memory.filtered_horiz_speed <= walk_down {
                    LocomotionState::Walk
                } else if memory.filtered_horiz_speed >= sprint_up {
                    LocomotionState::Sprint
                } else {
                    LocomotionState::Run
                }
            }
            LocomotionState::Sprint => {
                if memory.filtered_horiz_speed <= sprint_down {
                    LocomotionState::Run
                } else {
                    LocomotionState::Sprint
                }
            }
        };

        let (desired_clip, looping, next_anim_state) = if !grounded {
            if vel.y < -0.2 {
                (&ped.animations.fall_loop, true, PlayerAnimState::Fall)
            } else {
                (&ped.animations.jump_loop, true, PlayerAnimState::Jump)
            }
        } else if now < memory.land_until {
            (&ped.animations.land, false, PlayerAnimState::Land)
        } else {
            match memory.locomotion {
                LocomotionState::Idle if movement_intent_active => {
                    (&ped.animations.walk, true, PlayerAnimState::Walk)
                }
                LocomotionState::Idle => (&ped.animations.idle, true, PlayerAnimState::Idle),
                LocomotionState::Walk => (&ped.animations.walk, true, PlayerAnimState::Walk),
                LocomotionState::Run => (&ped.animations.run, true, PlayerAnimState::Run),
                LocomotionState::Sprint => (&ped.animations.sprint, true, PlayerAnimState::Sprint),
            }
        };

        if desired_clip.is_empty() {
            continue;
        }
        let clip = desired_clip.clone();

        let blend_time = match next_anim_state {
            PlayerAnimState::Land => 0.08,
            PlayerAnimState::Jump | PlayerAnimState::Fall => 0.10,
            _ => 0.16,
        };

        if memory.anim_state != next_anim_state {
            let payload = serde_json::to_vec(&serde_json::json!({
                "client_id": marker.client_id,
                "handle": handle.map(|h| h.0),
                "state": next_anim_state.as_str(),
                "clip": clip,
                "grounded": grounded,
                "speed": memory.filtered_horiz_speed,
                "is_local": is_local_player,
                "move_intent": movement_intent_active,
            }))
            .unwrap_or_default();
            local_bus.push("player:anim_state".to_string(), payload);
            memory.anim_state = next_anim_state;
        }

        match anim_state {
            Some(mut state) => {
                if state.current.as_deref() != Some(clip.as_str()) || state.looping != looping {
                    state.current = Some(clip.clone());
                    state.looping = looping;
                    state.paused = false;
                    state.speed = 1.0;
                    state.blend_time = blend_time;
                    state.flags = 1;
                }
            }
            None => {
                commands.entity(model_entity).insert(AnimationState {
                    current: Some(clip),
                    speed: 1.0,
                    looping,
                    paused: false,
                    blend_time,
                    flags: 1,
                });
            }
        }
    }
}

fn sync_net_transform_to_render(
    time: Res<Time>,
    local_client_id: Option<Res<LocalClientId>>,
    mut predicted_q: Query<
        (
            &mut Transform,
            &PlayerMarker,
            &NetTransform,
            Option<&Confirmed<NetTransform>>,
        ),
        With<Predicted>,
    >,
) {
    let local_id = local_client_id.map(|r| r.0);
    // Preferuj server-confirmed stav, fallback na local predicted NetTransform.
    for (mut local, marker, predicted_net, confirmed_net) in predicted_q.iter_mut() {
        // FiveM-style: lokálního hráče neovlivňujeme — jeho Transform řídí Avian fyzika.
        if Some(marker.client_id) == local_id {
            // Rotaci ale stále synchronizujeme ze serveru (yaw modelu)
            let src = confirmed_net.map(|c| &c.0).unwrap_or(predicted_net);
            let dt = time.delta_secs();
            let rot_alpha = (1.0 - (-ROTATION_SMOOTHING_RATE * dt).exp()).clamp(0.0, 1.0);
            local.rotation = local.rotation.slerp(src.rotation, rot_alpha);
            continue;
        }
        let src = confirmed_net.map(|c| &c.0).unwrap_or(predicted_net);
        let target_pos = Vec3::new(src.translation.x, src.translation.y, src.translation.z);
        let dt = time.delta_secs();
        let pos_alpha = (1.0 - (-POSITION_SMOOTHING_RATE * dt).exp()).clamp(0.0, 1.0);
        let rot_alpha = (1.0 - (-ROTATION_SMOOTHING_RATE * dt).exp()).clamp(0.0, 1.0);

        local.translation = local.translation.lerp(target_pos, pos_alpha);
        local.rotation = local.rotation.slerp(src.rotation, rot_alpha);
    }
}

/// Prochází hierarchii dětí a hledá entitu se jménem `bone`.
/// Depth guard zabraňuje nadměrné rekurzi na komplexních modelech.
fn find_bone_entity(
    root: Entity,
    bone: &str,
    children_q: &Query<&Children>,
    name_q: &Query<&Name>,
    depth: u8,
) -> Option<Entity> {
    if depth == 0 { return None; }
    let Ok(children) = children_q.get(root) else { return None };
    for child in children.iter() {
        if name_q.get(child).map(|n| n.as_str() == bone).unwrap_or(false) {
            return Some(child);
        }
        if let Some(found) = find_bone_entity(child, bone, children_q, name_q, depth - 1) {
            return Some(found);
        }
    }
    None
}

fn update_camera_follow(
    local_client_id: Option<Res<LocalClientId>>,
    look: Res<CameraLookState>,
    bridges: Res<GameBridges>,
    world_state: Res<LuaWorldState>,
    predicted_players: Query<
        (&Transform, &PlayerMarker),
        (With<Predicted>, Without<MainGameplayCamera>),
    >,
    entity_q: Query<&GlobalTransform, Without<MainGameplayCamera>>,
    children_q: Query<&Children>,
    name_q: Query<&Name>,
    mut cam_q: Query<(&mut Transform, &mut Projection), With<MainGameplayCamera>>,
) {
    let Ok((mut cam_transform, mut projection)) = cam_q.single_mut() else { return };

    // Forward vector z mouse look state (yaw + pitch)
    let cp = look.pitch.cos();
    let mut forward = Vec3::new(look.yaw.sin() * cp, look.pitch.sin(), look.yaw.cos() * cp);
    if forward.length_squared() < 0.0001 { forward = Vec3::Z; } else { forward = forward.normalize(); }

    // Nastav FOV podle aktivního rigu nebo reset na default.
    let target_fov = bridges.camera.get_active_rig()
        .and_then(|r| r.fov)
        .map(|deg| deg.to_radians())
        .unwrap_or(DEFAULT_CAMERA_FOV);
    if let Projection::Perspective(p) = projection.as_mut() {
        p.fov = target_fov;
    }

    // --- Custom camera rig ---
    if let Some(rig) = bridges.camera.get_active_rig() {
        match &rig.attachment {
            CameraAttachment::Position { pos, look_at } => {
                cam_transform.translation = Vec3::from(*pos);
                if let Some(target) = look_at {
                    cam_transform.look_at(Vec3::from(*target), Vec3::Y);
                } else {
                    let eye = cam_transform.translation;
                    cam_transform.look_at(eye + forward, Vec3::Y);
                }
            }
            CameraAttachment::Entity { handle, offset, look_at } => {
                if let Some(entity) = world_state.entity_for(*handle) {
                    if let Ok(et) = entity_q.get(entity) {
                        let entity_pos = et.translation();
                        let cam_pos = entity_pos + Vec3::from(*offset);
                        cam_transform.translation = cam_pos;
                        if *look_at {
                            cam_transform.look_at(entity_pos, Vec3::Y);
                        } else {
                            cam_transform.look_at(cam_pos + forward, Vec3::Y);
                        }
                    }
                }
            }
            CameraAttachment::Bone { handle, bone, offset } => {
                if let Some(entity) = world_state.entity_for(*handle) {
                    if let Some(bone_ent) = find_bone_entity(entity, bone, &children_q, &name_q, 8) {
                        if let Ok(bt) = entity_q.get(bone_ent) {
                            let (_, bone_rot, bone_pos) = bt.to_scale_rotation_translation();
                            cam_transform.translation = bone_pos + bone_rot * Vec3::from(*offset);
                            cam_transform.rotation = bone_rot;
                        }
                    }
                }
            }
        }
        return;
    }

    // --- Výchozí player kamera ---
    let Some(local_client_id) = local_client_id else { return };
    let mut player_pos: Option<Vec3> = None;
    for (tfm, marker) in predicted_players.iter() {
        if marker.client_id == local_client_id.0 {
            player_pos = Some(Vec3::new(tfm.translation.x, 0.0, tfm.translation.z));
            break;
        }
    }
    let Some(player_pos) = player_pos else { return };

    let focus = player_pos + Vec3::new(0.0, FIRST_PERSON_EYE_HEIGHT, 0.0);

    if bridges.camera.is_first_person() {
        cam_transform.translation = focus;
        cam_transform.look_at(focus + forward, Vec3::Y);
    } else {
        let eye = focus - forward * THIRD_PERSON_DISTANCE;
        cam_transform.translation = eye;
        cam_transform.look_at(focus, Vec3::Y);
    }
}

fn prefer_predicted_player_visuals(
    predicted_ids_q: Query<&PlayerMarker, With<Predicted>>,
    mut fallback_visuals_q: Query<
        (&PlayerMarker, &mut Visibility),
        (With<PlayerVisualAttached>, Without<Predicted>),
    >,
) {
    let predicted_ids: HashSet<u64> = predicted_ids_q
        .iter()
        .map(|m| m.client_id)
        .collect();

    for (marker, mut visibility) in fallback_visuals_q.iter_mut() {
        *visibility = if predicted_ids.contains(&marker.client_id) {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}

fn update_local_player_visibility(
    local_client_id: Option<Res<LocalClientId>>,
    mut players: Query<(&PlayerMarker, &mut Visibility), With<PlayerVisualAttached>>,
) {
    let Some(local_client_id) = local_client_id else { return; };
    for (marker, mut vis) in players.iter_mut() {
        if marker.client_id == local_client_id.0 {
            *vis = Visibility::Visible;
        }
    }
}

fn collect_and_send_input(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    mouse: Res<ButtonInput<MouseButton>>,
    local_client_id: Option<Res<LocalClientId>>,
    // FiveM-style: čteme Transform (fyzikální pozici), nikoli NetTransform
    predicted_players: Query<(&PlayerMarker, &Transform), With<Predicted>>,
    look: Res<CameraLookState>,
    mut senders: Query<&mut MessageSender<PlayerInput>>,
    mut tick: Local<u32>,
) {
    *tick = tick.wrapping_add(1);

    let bindings = &cfg.0.input.keys;
    let mouse_b = &cfg.0.input.mouse;

    let mut move_x = 0.0_f32;
    let mut move_y = 0.0_f32;
    if keys.pressed(bindings.move_forward) { move_y += 1.0; }
    if keys.pressed(bindings.move_backward) { move_y -= 1.0; }
    if keys.pressed(bindings.move_right) { move_x -= 1.0; }
    if keys.pressed(bindings.move_left) { move_x += 1.0; }

    let mag2 = move_x * move_x + move_y * move_y;
    if mag2 > 1.0 {
        let inv = mag2.sqrt().recip();
        move_x *= inv;
        move_y *= inv;
    }

    // World-space WASD rotovaný podle yaw kamery
    let yaw_rad = look.yaw;
    let world_move_x = yaw_rad.cos() * move_x + yaw_rad.sin() * move_y;
    let world_move_z = -yaw_rad.sin() * move_x + yaw_rad.cos() * move_y;

    let mut actions = 0u32;
    if mouse.pressed(mouse_b.attack_primary) { actions |= player_action::PRIMARY_FIRE; }
    if mouse.pressed(mouse_b.attack_secondary) { actions |= player_action::SECONDARY_FIRE; }
    if keys.pressed(bindings.reload) { actions |= player_action::RELOAD; }
    if keys.pressed(bindings.jump) { actions |= player_action::JUMP; }
    if keys.pressed(bindings.crouch) { actions |= player_action::CROUCH; }
    if keys.pressed(bindings.sprint) { actions |= player_action::SPRINT; }
    if keys.pressed(bindings.interact) { actions |= player_action::INTERACT; }
    if keys.pressed(bindings.use_item) { actions |= player_action::USE_ITEM; }

    // Aktuální fyzikální pozice lokálního hráče (z Avian Transform, ne NetTransform)
    let physics_pos = local_client_id.as_ref().and_then(|lid| {
        predicted_players
            .iter()
            .find(|(m, _)| m.client_id == lid.0)
            .map(|(_, t)| t.translation)
    }).unwrap_or(Vec3::ZERO);

    let yaw = look.yaw.to_degrees();

    let input = PlayerInput {
        move_dir: [world_move_x, world_move_z],
        look: [yaw, 0.0],
        actions,
        client_tick: *tick,
        position: [physics_pos.x, physics_pos.y, physics_pos.z],
    };

    for mut sender in senders.iter_mut() {
        let _ = sender.send::<InputChannel>(input.clone());
    }
}


/// Publikuje stav inputu do Lua local event busu jako `input:state`.
/// Resource skripty tak mohou robustne cist klavesove vstupy bez vazby
/// na Rust struktury `ButtonInput<KeyCode>`.
fn publish_input_state_to_lua(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    local_bus: Res<LocalEventBus>,
) {
    let bindings = &cfg.0.input.keys;

    let mut move_x = 0.0_f32;
    let mut move_y = 0.0_f32;
    if keys.pressed(bindings.move_forward) { move_y += 1.0; }
    if keys.pressed(bindings.move_backward) { move_y -= 1.0; }
    if keys.pressed(bindings.move_right) { move_x += 1.0; }
    if keys.pressed(bindings.move_left) { move_x -= 1.0; }

    let payload = serde_json::to_vec(&serde_json::json!({
        "move": {
            "x": move_x,
            "y": move_y,
        },
        "keys": {
            "move_forward": keys.pressed(bindings.move_forward),
            "move_backward": keys.pressed(bindings.move_backward),
            "move_left": keys.pressed(bindings.move_left),
            "move_right": keys.pressed(bindings.move_right),
            "jump": keys.pressed(bindings.jump),
            "sprint": keys.pressed(bindings.sprint),
            "crouch": keys.pressed(bindings.crouch),
            "interact": keys.pressed(bindings.interact),
        },
        "keys_just": {
            "options_menu": keys.just_pressed(KeyCode::Escape),
        }
    }))
    .unwrap_or_default();

    local_bus.push("input:state".to_string(), payload);
}

/// Publikuje stav detekce STAIRS trigger collideru pod lokálním hráčem.
/// Event: `stairs:state`
fn publish_stairs_state_to_lua(
    local_bus: Res<LocalEventBus>,
    local_client_id: Option<Res<LocalClientId>>,
    players: Query<(&PlayerMarker, &Transform, Option<&LinearVelocity>, Option<&ShapeHits>), With<Predicted>>,
    stairs_q: Query<(), With<StairsCollider>>,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    spatial_query: SpatialQuery,
) {
    let Some(lid) = local_client_id.as_ref() else { return; };

    let Some((_, tf, vel, shape_hits)) = players.iter().find(|(marker, _, _, _)| marker.client_id == lid.0) else {
        return;
    };

    let is_stairs_or_parent = |entity: Entity| -> bool {
        let mut current = entity;
        loop {
            if stairs_q.get(current).is_ok() {
                return true;
            }
            match child_of_q.get(current) {
                Ok(co) => current = co.parent(),
                Err(_) => return false,
            }
        }
    };

    let origin = tf.translation + Vec3::new(0.0, 0.20, 0.0);
    let dir = Dir3::NEG_Y;
    let max_dist = 2.2;
    let filter = SpatialQueryFilter::default();
    let hit = spatial_query.cast_ray(origin, dir, max_dist, true, &filter);

    let on_stairs = hit.as_ref().map(|h| is_stairs_or_parent(h.entity)).unwrap_or(false);
    let hit_distance = hit.as_ref().map(|h| h.distance).unwrap_or(-1.0);
    let hit_pos = hit
        .as_ref()
        .map(|h| origin + Vec3::new(0.0, -h.distance, 0.0));
    let grounded = has_ground_contact(shape_hits);
    let vy = vel.map(|v| v.y).unwrap_or(0.0);

    let payload = serde_json::to_vec(&serde_json::json!({
        "on_stairs": on_stairs,
        "reacting": on_stairs && grounded,
        "grounded": grounded,
        "hit_distance": hit_distance,
        "hit_pos": hit_pos.map(|p| serde_json::json!({
            "x": p.x,
            "y": p.y,
            "z": p.z,
        })),
        "player": {
            "x": tf.translation.x,
            "y": tf.translation.y,
            "z": tf.translation.z,
            "vy": vy,
        }
    }))
    .unwrap_or_default();

    local_bus.push("stairs:state".to_string(), payload);
}

// ---------------------------------------------------------------------------
// KeyCode / MouseButton → canonical Lua name
// ---------------------------------------------------------------------------

fn keycode_name(k: &KeyCode) -> String {
    match k {
        KeyCode::KeyA => "a",  KeyCode::KeyB => "b",  KeyCode::KeyC => "c",
        KeyCode::KeyD => "d",  KeyCode::KeyE => "e",  KeyCode::KeyF => "f",
        KeyCode::KeyG => "g",  KeyCode::KeyH => "h",  KeyCode::KeyI => "i",
        KeyCode::KeyJ => "j",  KeyCode::KeyK => "k",  KeyCode::KeyL => "l",
        KeyCode::KeyM => "m",  KeyCode::KeyN => "n",  KeyCode::KeyO => "o",
        KeyCode::KeyP => "p",  KeyCode::KeyQ => "q",  KeyCode::KeyR => "r",
        KeyCode::KeyS => "s",  KeyCode::KeyT => "t",  KeyCode::KeyU => "u",
        KeyCode::KeyV => "v",  KeyCode::KeyW => "w",  KeyCode::KeyX => "x",
        KeyCode::KeyY => "y",  KeyCode::KeyZ => "z",
        KeyCode::Digit0 => "0", KeyCode::Digit1 => "1", KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3", KeyCode::Digit4 => "4", KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6", KeyCode::Digit7 => "7", KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::Numpad0 => "num0", KeyCode::Numpad1 => "num1", KeyCode::Numpad2 => "num2",
        KeyCode::Numpad3 => "num3", KeyCode::Numpad4 => "num4", KeyCode::Numpad5 => "num5",
        KeyCode::Numpad6 => "num6", KeyCode::Numpad7 => "num7", KeyCode::Numpad8 => "num8",
        KeyCode::Numpad9 => "num9",
        KeyCode::Space        => "space",
        KeyCode::Escape       => "escape",
        KeyCode::Enter        => "enter",
        KeyCode::NumpadEnter  => "enter",
        KeyCode::Tab          => "tab",
        KeyCode::Backspace    => "backspace",
        KeyCode::Delete       => "delete",
        KeyCode::Insert       => "insert",
        KeyCode::Home         => "home",
        KeyCode::End          => "end",
        KeyCode::PageUp       => "pageup",
        KeyCode::PageDown     => "pagedown",
        KeyCode::ArrowUp      => "up",
        KeyCode::ArrowDown    => "down",
        KeyCode::ArrowLeft    => "left",
        KeyCode::ArrowRight   => "right",
        KeyCode::ShiftLeft    => "lshift",
        KeyCode::ShiftRight   => "rshift",
        KeyCode::ControlLeft  => "lctrl",
        KeyCode::ControlRight => "rctrl",
        KeyCode::AltLeft      => "lalt",
        KeyCode::AltRight     => "ralt",
        KeyCode::SuperLeft | KeyCode::SuperRight => "super",
        KeyCode::CapsLock     => "capslock",
        KeyCode::F1  => "f1",  KeyCode::F2  => "f2",  KeyCode::F3  => "f3",
        KeyCode::F4  => "f4",  KeyCode::F5  => "f5",  KeyCode::F6  => "f6",
        KeyCode::F7  => "f7",  KeyCode::F8  => "f8",  KeyCode::F9  => "f9",
        KeyCode::F10 => "f10", KeyCode::F11 => "f11", KeyCode::F12 => "f12",
        _ => return format!("{:?}", k).to_lowercase(),
    }.to_string()
}

fn mousebutton_name(btn: &MouseButton) -> String {
    match btn {
        MouseButton::Left   => "left".to_string(),
        MouseButton::Right  => "right".to_string(),
        MouseButton::Middle => "middle".to_string(),
        MouseButton::Other(n) => format!("mouse{n}"),
        _ => format!("{:?}", btn).to_lowercase(),
    }
}

/// Updates InputBridge each frame so Lua can query key/mouse state synchronously.
fn update_input_bridge(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    bridges: Res<GameBridges>,
) {
    let (cursor_x, cursor_y) = window_q
        .single()
        .ok()
        .and_then(|w| w.cursor_position().map(|p| (p.x / w.width(), p.y / w.height())))
        .unwrap_or((0.0, 0.0));

    bridges.input.update(InputSnapshot {
        pressed:             keys.get_pressed().map(keycode_name).collect::<HashSet<_>>(),
        just_pressed:        keys.get_just_pressed().map(keycode_name).collect::<HashSet<_>>(),
        just_released:       keys.get_just_released().map(keycode_name).collect::<HashSet<_>>(),
        mouse_pressed:       mouse.get_pressed().map(mousebutton_name).collect::<HashSet<_>>(),
        mouse_just_pressed:  mouse.get_just_pressed().map(mousebutton_name).collect::<HashSet<_>>(),
        mouse_just_released: mouse.get_just_released().map(mousebutton_name).collect::<HashSet<_>>(),
        cursor_x,
        cursor_y,
    });
}

/// Updates ConnectionBridge every frame while in InGame.
fn update_connection_bridge(
    cfg: Res<core_net::ClientNetConfig>,
    local_client: Option<Res<LocalClientId>>,
    bridges: Res<GameBridges>,
) {
    bridges.connection.set(ConnectionInfo {
        connected: true,
        server_addr: cfg.server.to_string(),
        ping_ms: 0, // TODO: lightyear RTT diagnostics (Phase 5)
        client_id: local_client.as_deref().map_or(0, |lc| lc.0),
    });
}

/// Resets ConnectionBridge when leaving InGame (disconnect / lobby).
fn reset_connection_bridge(bridges: Res<GameBridges>) {
    bridges.connection.set_disconnected();
}

/// Reset EngineStateBridge when entering InGame so ESC menu starts closed.
fn reset_engine_state(bridges: Res<GameBridges>) {
    bridges.engine.reset();
}

/// Poll EngineStateBridge for quit / disconnect requests from Lua.
fn handle_engine_cmds(
    bridges: Res<GameBridges>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if bridges.engine.take_disconnect() {
        next_state.set(AppState::Lobby);
    }
    if bridges.engine.take_quit() {
        std::process::exit(0);
    }
}

/// Vrhá paprsek z kamery dopředu, hledá první entitu s `EntityHandle` v hierarchii.
/// Výsledek (handle, vzdálenost) zapíše do `CrosshairBridge` pro Lua `Raycast.GetEntityUnderCrosshair()`.
/// Hráčské entity (s `PlayerMarker`) jsou ignorovány.
fn update_crosshair_entity(
    camera_q: Query<&GlobalTransform, With<MainGameplayCamera>>,
    spatial_query: SpatialQuery,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    handle_q: Query<&EntityHandle>,
    player_q: Query<Has<core_shared::PlayerMarker>>,
    bridges: Res<GameBridges>,
) {
    let Ok(cam) = camera_q.single() else {
        bridges.crosshair.set(None);
        return;
    };
    let origin = cam.translation();
    let dir = cam.forward();

    // Predikát: přeskoč entity jejichž kořen má PlayerMarker (vlastní hráčovo tělo).
    // Hráčův collider leží na child entitě → musíme projet hierarchii nahoru.
    let not_player = |entity: Entity| -> bool {
        let mut current = entity;
        loop {
            if player_q.get(current).unwrap_or(false) { return false; }
            match child_of_q.get(current) {
                Ok(co) => current = co.parent(),
                Err(_)  => return true,
            }
        }
    };

    let filter = SpatialQueryFilter::default();
    let hit = spatial_query.cast_ray_predicate(origin, dir, 100.0, true, &filter, &not_player);
    let Some(hit) = hit else {
        bridges.crosshair.set(None);
        return;
    };

    // Projdi hierarchii nahoru — collider může být na child entitě, EntityHandle na rootu.
    let mut current = hit.entity;
    let root_with_handle = loop {
        if let Ok(h) = handle_q.get(current) {
            break Some(h.0);
        }
        if let Ok(child_of) = child_of_q.get(current) {
            current = child_of.parent();
        } else {
            break None;
        }
    };

    match root_with_handle {
        Some(handle) => bridges.crosshair.set(Some(CrosshairHit { handle, distance: hit.distance })),
        None         => bridges.crosshair.set(None),
    }
}

/// Přidá vizuál na lokální objekty spawnuté přes `World.SpawnLocalObject`.
/// Rozlišuje .adm (AdmSceneRoot) a .glb/.gltf (SceneRoot #Scene0).
fn attach_mesh_to_local_objects(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    model_registry: Res<ModelRegistry>,
    adm_cache: Res<AdmHandleCache>,
    new_objs: Query<
        (Entity, &LocalObjectMarker),
        (With<LocalObjectMarker>, Without<LocalObjectVisualAttached>),
    >,
) {
    for (entity, marker) in new_objs.iter() {
        commands.entity(entity).insert((
            GlobalTransform::default(),
            Visibility::Visible,
            InheritedVisibility::default(),
            ViewVisibility::default(),
            LocalObjectVisualAttached,
        ));

        if let Some(path) = model_registry.path(&marker.model) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "adm" {
                // ADM: načti přes AdmSceneRoot — DrawablePlugin ho spawne do potomků.
                let handle = adm_cache.0.get(&marker.model).cloned().unwrap_or_else(|| {
                    let bevy_path = path.to_string_lossy().replace('\\', "/");
                    asset_server.load(bevy_path)
                });
                commands.entity(entity).insert((
                    AdmSceneRoot(handle),
                    ModelName(marker.model.clone()),
                ));
            } else {
                // GLB / GLTF: standardní #Scene0 label
                let scene_path = AssetPath::from_path_buf(path.clone()).with_label("Scene0");
                let scene: Handle<Scene> = asset_server.load_override(scene_path);
                commands.entity(entity).with_children(|p| {
                    p.spawn((SceneRoot(scene), Transform::default()));
                });
            }
            continue;
        }

        // Fallback: neznámý model = zelená kostka (viditelné debugovatelné chování).
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::new(0.9, 0.9, 0.9))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.2, 0.85, 0.3),
                ..default()
            })),
        ));
        warn!(
            "[gameplay/client] LocalObject '{}' not found in ModelRegistry; using fallback cube",
            marker.model
        );
    }
}

fn attach_mesh_to_dummy_objects(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    new_objs: Query<
        (Entity, &DummyObjectMarker),
        (With<DummyObjectMarker>, Without<DummyObjectVisualAttached>),
    >,
) {
    for (entity, marker) in new_objs.iter() {
        commands.entity(entity).insert((
            GlobalTransform::default(),
            Visibility::Visible,
            InheritedVisibility::default(),
            ViewVisibility::default(),
            DummyObjectVisualAttached,
        ));

        let base_material = materials.add(StandardMaterial {
            base_color: Color::srgba(
                marker.color[0].clamp(0.0, 1.0),
                marker.color[1].clamp(0.0, 1.0),
                marker.color[2].clamp(0.0, 1.0),
                marker.color[3].clamp(0.0, 1.0),
            ),
            ..default()
        });

        match marker.kind {
            DummyPrimitiveKind::Cuboid => {
                let sx = marker.size[0].max(0.01);
                let sy = marker.size[1].max(0.01);
                let sz = marker.size[2].max(0.01);
                commands.entity(entity).insert((
                    Mesh3d(meshes.add(Cuboid::new(sx, sy, sz))),
                    MeshMaterial3d(base_material.clone()),
                ));
            }
            DummyPrimitiveKind::Cube => {
                let s = marker.size[0].max(0.01);
                commands.entity(entity).insert((
                    Mesh3d(meshes.add(Cuboid::new(s, s, s))),
                    MeshMaterial3d(base_material.clone()),
                ));
            }
            DummyPrimitiveKind::Sphere => {
                let r = marker.radius.max(0.01);
                commands.entity(entity).insert((
                    Mesh3d(meshes.add(Sphere::new(r))),
                    MeshMaterial3d(base_material.clone()),
                ));
            }
            DummyPrimitiveKind::Stairs => {
                let width = marker.size[0].max(0.05);
                let total_height = marker.height.max(0.05);
                let total_depth = marker.size[2].max(0.05);
                let steps = marker.steps.max(1);
                let step_h = total_height / steps as f32;
                let step_d = total_depth / steps as f32;

                commands.entity(entity).with_children(|p| {
                    for i in 0..steps {
                        let y = -total_height * 0.5 + step_h * (i as f32 + 0.5);
                        let z = -total_depth * 0.5 + step_d * (i as f32 + 0.5);
                        p.spawn((
                            Mesh3d(meshes.add(Cuboid::new(width, step_h.max(0.01), step_d.max(0.01)))),
                            MeshMaterial3d(base_material.clone()),
                            Transform::from_xyz(0.0, y, z),
                            GlobalTransform::default(),
                            Visibility::Visible,
                            InheritedVisibility::default(),
                            ViewVisibility::default(),
                        ));
                    }
                });
            }
            DummyPrimitiveKind::Arch => {
                let width = marker.size[0].max(0.05);
                let outer_r = marker.radius.max(0.1);
                let thickness = marker.size[2].max(0.05);
                let segments = marker.segments.max(3);
                let inner_r = (outer_r - thickness).max(0.02);

                commands.entity(entity).with_children(|p| {
                    for i in 0..segments {
                        let t0 = (i as f32 / segments as f32) * std::f32::consts::PI;
                        let t1 = ((i + 1) as f32 / segments as f32) * std::f32::consts::PI;
                        let a = (t0 + t1) * 0.5;
                        let center_r = (outer_r + inner_r) * 0.5;
                        let seg_len = (t1 - t0).abs() * center_r;
                        let y = a.sin() * center_r;
                        let z = a.cos() * center_r;
                        let rot = Quat::from_rotation_x(a);

                        p.spawn((
                            Mesh3d(meshes.add(Cuboid::new(width, thickness, seg_len.max(0.02)))),
                            MeshMaterial3d(base_material.clone()),
                            Transform {
                                translation: Vec3::new(0.0, y, z),
                                rotation: rot,
                                scale: Vec3::ONE,
                            },
                            GlobalTransform::default(),
                            Visibility::Visible,
                            InheritedVisibility::default(),
                            ViewVisibility::default(),
                        ));
                    }
                });
            }
        }
    }
}

fn parse_animation_index(selector: &str) -> Option<u32> {
    if let Ok(idx) = selector.parse::<u32>() {
        return Some(idx);
    }

    if let Some(rest) = selector.strip_prefix("clip:") {
        return rest.parse::<u32>().ok();
    }
    if let Some(rest) = selector.strip_prefix("anim:") {
        return rest.parse::<u32>().ok();
    }

    None
}

fn extract_gltf_clip_names(gltf: &bevy::gltf::Gltf) -> Vec<String> {
    let Some(source) = gltf.source.as_ref() else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for anim in source.animations() {
        if let Some(name) = anim.name() {
            names.push(name.to_string());
        } else {
            names.push(format!("clip:{}", anim.index()));
        }
    }

    names
}

fn update_model_animation_registry(
    model_registry: Res<ModelRegistry>,
    anim_registry: Res<ModelAnimationRegistry>,
    asset_server: Res<AssetServer>,
    adm_cache: Res<AdmHandleCache>,
    gltf_cache: Res<GltfHandleCache>,
    adm_assets: Res<Assets<crate::drawable::AdmScene>>,
    gltf_assets: Res<Assets<bevy::gltf::Gltf>>,
    anim_set_assets: Res<Assets<crate::drawable::AnimationSet>>,
    ped_anim_index: Res<crate::native_assets::PedAdsAnimIndex>,
    ped_anim_handles: Res<crate::native_assets::PedAdsAnimHandleCache>,
    mut discovery_cache: ResMut<ModelAnimationDiscoveryCache>,
) {
    let mut keep = HashSet::new();

    for (model_name, model_path) in model_registry.entries() {
        keep.insert(model_name.clone());
        let ext = model_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        if ext == "adm" {
            let handle = if let Some(h) = adm_cache.0.get(&model_name) {
                h.clone()
            } else if let Some(h) = discovery_cache.adm.get(&model_name) {
                h.clone()
            } else {
                let bevy_path = model_path.to_string_lossy().replace('\\', "/");
                let h: Handle<crate::drawable::AdmScene> = asset_server.load(bevy_path);
                discovery_cache.adm.insert(model_name.clone(), h.clone());
                h
            };

            if adm_assets.get(&handle).is_some() {
                // ADMv6: metadata animací pro ADM modely bereme pouze z připojených .ads_anim setů.
                // Embedded klipy v .adm se už nepoužívají.
                let mut all_clip_names: Vec<String> = Vec::new();
                let mut all_dicts: Vec<core_resources::ModelAnimationDictionary> = Vec::new();

                if let Some(anim_paths) = ped_anim_index.0.get(&model_name) {
                    for path in anim_paths {
                        if let Some(handle) = ped_anim_handles.0.get(path) {
                            if let Some(anim_set) = anim_set_assets.get(handle) {
                                for clip in &anim_set.clips {
                                    if !all_clip_names.contains(&clip.name) {
                                        all_clip_names.push(clip.name.clone());
                                    }
                                }

                                for dict_set in &anim_set.dictionaries {
                                    let mut found = false;
                                    for existing in &mut all_dicts {
                                        if existing.name == dict_set.name {
                                            for clip_name in &dict_set.clip_names {
                                                if !existing.clip_names.contains(clip_name) {
                                                    existing.clip_names.push(clip_name.clone());
                                                }
                                            }
                                            found = true;
                                            break;
                                        }
                                    }
                                    if !found {
                                        all_dicts.push(core_resources::ModelAnimationDictionary {
                                            name: dict_set.name.clone(),
                                            clip_names: dict_set.clip_names.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                // Přepiš cache i prázdnými daty, aby nezůstávaly stale legacy hodnoty.
                anim_registry.set_clip_names(&model_name, all_clip_names);
                anim_registry.set_animation_dictionaries(&model_name, all_dicts);
            }
            continue;
        }

        if ext == "glb" || ext == "gltf" {
            let handle = if let Some((h, _)) = gltf_cache.0.get(&model_name) {
                h.clone()
            } else if let Some(h) = discovery_cache.gltf.get(&model_name) {
                h.clone()
            } else {
                let bevy_path = model_path.to_string_lossy().replace('\\', "/");
                let h: Handle<bevy::gltf::Gltf> = asset_server.load_with_settings(
                    bevy_path,
                    |settings: &mut bevy::gltf::GltfLoaderSettings| {
                        settings.include_source = true;
                    },
                );
                discovery_cache.gltf.insert(model_name.clone(), h.clone());
                h
            };

            if let Some(gltf) = gltf_assets.get(&handle) {
                anim_registry.set_clip_names(&model_name, extract_gltf_clip_names(gltf));
            }
        }
    }

    discovery_cache.adm.retain(|name, _| keep.contains(name));
    discovery_cache.gltf.retain(|name, _| keep.contains(name));
    anim_registry.retain_models(&keep);
}

fn resolve_animation_graph(
    model_name: &str,
    animation_selector: &str,
    model_registry: &ModelRegistry,
    asset_server: &AssetServer,
    graphs: &mut Assets<AnimationGraph>,
    cache: &mut LuaAnimationGraphCache,
) -> Option<(Handle<AnimationGraph>, AnimationNodeIndex)> {
    let clip_index = parse_animation_index(animation_selector)?;

    let key = (model_name.to_string(), clip_index);
    if let Some(cached) = cache.map.get(&key) {
        return Some(cached.clone());
    }

    let model_path = model_registry.path(model_name)?;
    let ext = model_path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    if ext != "glb" && ext != "gltf" {
        return None;
    }

    let model_asset = model_path.to_string_lossy().replace('\\', "/");
    let clip = asset_server.load(GltfAssetLabel::Animation(clip_index as usize).from_asset(model_asset));
    let (graph, node) = AnimationGraph::from_clip(clip);
    let graph_handle = graphs.add(graph);
    cache.map.insert(key, (graph_handle.clone(), node));
    Some((graph_handle, node))
}

fn apply_lua_animation_state(
    mut commands: Commands,
    model_registry: Res<ModelRegistry>,
    asset_server: Res<AssetServer>,
    mut graph_assets: ResMut<Assets<AnimationGraph>>,
    mut graph_cache: ResMut<LuaAnimationGraphCache>,
    anim_roots: Query<(Entity, &AnimationState, &ModelName)>,
    children_q: Query<&Children>,
    mut players: Query<(Entity, &mut AnimationPlayer, Option<&AnimationGraphHandle>)>,
) {
    for (root, state, model_name) in &anim_roots {
        let Some(clip_name) = state.current.as_ref() else { continue };

        let Some((graph_handle, node)) = resolve_animation_graph(
            &model_name.0,
            clip_name,
            &model_registry,
            &asset_server,
            &mut graph_assets,
            &mut graph_cache,
        ) else {
            continue;
        };

        for entity in children_q.iter_descendants(root) {
            let Ok((player_entity, mut player, graph_handle_comp)) = players.get_mut(entity) else {
                continue;
            };

            if graph_handle_comp.map(|h| h.0 != graph_handle).unwrap_or(true) {
                commands
                    .entity(player_entity)
                    .insert(AnimationGraphHandle(graph_handle.clone()));
            }

            let playing = player.play(node);
            if state.looping {
                playing.repeat();
            }

            if let Some(active) = player.animation_mut(node) {
                active.set_speed(state.speed);
                if state.paused {
                    active.pause();
                } else {
                    active.resume();
                }
            }

            if state.blend_time > 0.0 {
                // Zatím bez full AnimationTransitions graph workflow.
                // Placeholder: opětovné přehrání vyvolá blend path v default playeru.
                let _ = Duration::from_secs_f32(state.blend_time);
            }
        }
    }
}
