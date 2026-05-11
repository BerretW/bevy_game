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
use std::collections::HashSet;
use bevy_gltf::{
    GltfExtras,
    GltfMaterialExtras,
    GltfMaterialName,
    GltfMeshExtras,
    GltfMeshName,
    GltfSceneExtras,
};
use core_net::{player_action, InputChannel, PlayerInput};
use core_resources::{ConnectionInfo, CrosshairHit, EntityHandle, GameBridges, InputSnapshot, LocalEventBus, LocalObjectMarker, LuaWorldState, ModelName, ModelRegistry, process_lua_commands, sync_entity_state_cache};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::Predicted;

use crate::config::ClientConfigResource;
use crate::native_assets::AdmHandleCache;
use crate::drawable::AdmSceneRoot;
use crate::AppState;
use crate::drawable::{PedPhysicsDef, PedPhysicsRegistry};

const THIRD_PERSON_DISTANCE: f32 = 5.5;
const FIRST_PERSON_EYE_HEIGHT: f32 = 1.7;
const PLAYER_MODEL_ASSET_PATH: &str = "models/player.adm";
const MAX_PITCH_RAD: f32 = 1.25;
const MOUSE_SENS_SCALE: f32 = 0.0025;
const POSITION_SMOOTHING_RATE: f32 = 14.0;
const ROTATION_SMOOTHING_RATE: f32 = 18.0;

#[derive(Resource, Clone, Copy)]
pub struct LocalClientId(pub u64);

#[derive(Resource, Clone)]
struct PlayerModelHandle(Handle<crate::drawable::AdmScene>);

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
enum CameraMode {
    FirstPerson,
    ThirdPerson,
}

#[derive(Resource, Clone, Copy)]
struct CameraModeState {
    mode: CameraMode,
}

impl Default for CameraModeState {
    fn default() -> Self {
        Self {
            mode: CameraMode::ThirdPerson,
        }
    }
}

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

        app.init_resource::<CameraModeState>();
        app.init_resource::<CameraLookState>();
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
                attach_mesh_to_local_objects,
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

    // Načti ped profil pro "player" — fallback na konstanty pokud ještě není ready.
    let ped = ped_reg.0.get("player").and_then(|h| ped_assets.get(h));

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

        let has_ground_contact = shape_hits
            .map(|hits| {
                hits.iter().any(|hit| {
                    // normal2 míří od druhého collideru směrem k casteru.
                    // Pro "zem pod hráčem" chceme výrazně upward normálu.
                    (-hit.normal2).dot(Vec3::Y) > 0.35
                })
            })
            .unwrap_or(false);

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


fn setup_scene_and_camera(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
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

    commands.insert_resource(PlayerModelHandle(asset_server.load::<crate::drawable::AdmScene>(PLAYER_MODEL_ASSET_PATH)));

    info!(
        "[gameplay/client] 3D scene ready (camera toggle: F6, player model: {})",
        PLAYER_MODEL_ASSET_PATH
    );
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

fn toggle_camera_mode(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<CameraModeState>) {
    if !keys.just_pressed(KeyCode::F6) {
        return;
    }
    mode.mode = match mode.mode {
        CameraMode::FirstPerson => CameraMode::ThirdPerson,
        CameraMode::ThirdPerson => CameraMode::FirstPerson,
    };
    info!("[gameplay/client] camera mode -> {:?}", mode.mode);
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
    mode: Res<CameraModeState>,
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
    let _ = mode.mode;
}

fn attach_player_model_to_new_players(
    mut commands: Commands,
    model: Res<PlayerModelHandle>,
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

        commands
            .entity(entity)
            .insert((
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Visible,
                InheritedVisibility::default(),
                ViewVisibility::default(),
                PlayerVisualAttached,
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
                p.spawn((
                    AdmSceneRoot(model.0.clone()),
                    // Bez DisableDrawableCollisions — COL_player z manifestu
                    // se stane compound coliderem parent RigidBody.
                    ModelName("player".to_string()),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                    GlobalTransform::default(),
                    Visibility::default(),
                    InheritedVisibility::default(),
                    ViewVisibility::default(),
                ));
            });

        info!(
            "[gameplay/client] ADM model attached to player {:?} (client_id={})",
            entity, marker.client_id
        );
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

fn update_camera_follow(
    local_client_id: Option<Res<LocalClientId>>,
    mode: Res<CameraModeState>,
    look: Res<CameraLookState>,
    predicted_players: Query<
        (&Transform, &PlayerMarker),
        (With<Predicted>, Without<MainGameplayCamera>),
    >,
    mut cam_q: Query<&mut Transform, With<MainGameplayCamera>>,
) {
    let Some(local_client_id) = local_client_id else {
        return;
    };
    let Ok(mut cam_transform) = cam_q.single_mut() else {
        return;
    };

    let mut player_pos: Option<Vec3> = None;
    for (tfm, marker) in predicted_players.iter() {
        if marker.client_id == local_client_id.0 {
            player_pos = Some(Vec3::new(tfm.translation.x, 0.0, tfm.translation.z));
            break;
        }
    }

    let Some(player_pos) = player_pos else {
        return;
    };

    let cp = look.pitch.cos();
    let mut forward = Vec3::new(look.yaw.sin() * cp, look.pitch.sin(), look.yaw.cos() * cp);
    if forward.length_squared() < 0.0001 {
        forward = Vec3::Z;
    } else {
        forward = forward.normalize();
    }

    let focus = player_pos + Vec3::new(0.0, FIRST_PERSON_EYE_HEIGHT, 0.0);

    match mode.mode {
        CameraMode::FirstPerson => {
            let eye = focus;
            cam_transform.translation = eye;
            cam_transform.look_at(eye + forward, Vec3::Y);
        }
        CameraMode::ThirdPerson => {
            let eye = focus - forward * THIRD_PERSON_DISTANCE;
            cam_transform.translation = eye;
            cam_transform.look_at(focus, Vec3::Y);
        }
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
    mode: Res<CameraModeState>,
    mut players: Query<(&PlayerMarker, &mut Visibility), With<PlayerVisualAttached>>,
) {
    let Some(local_client_id) = local_client_id else {
        return;
    };
    for (marker, mut vis) in players.iter_mut() {
        if marker.client_id == local_client_id.0 {
            // Lokalni model nechame viditelny i v 1st person, at je jasne,
            // ze se replikuje a pohybuje.
            *vis = Visibility::Visible;
        }
    }

    // Pouzij `mode` aby system stale reagoval na zmenu a nevznikal warning.
    let _ = mode.mode;
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

    let filter = SpatialQueryFilter::default();
    let hit = spatial_query.cast_ray(origin, dir, 100.0, true, &filter);
    let Some(hit) = hit else {
        bridges.crosshair.set(None);
        return;
    };

    // Projdi hierarchii nahoru — collider může být na child entitě, EntityHandle na rootu.
    let mut current = hit.entity;
    let root_with_handle = loop {
        if let Ok(h) = handle_q.get(current) {
            break Some((current, h.0));
        }
        if let Ok(child_of) = child_of_q.get(current) {
            current = child_of.parent();
        } else {
            break None;
        }
    };

    let Some((root_entity, handle)) = root_with_handle else {
        bridges.crosshair.set(None);
        return;
    };

    // Ignoruj hráčské entity.
    if player_q.get(root_entity).unwrap_or(false) {
        bridges.crosshair.set(None);
        return;
    }

    bridges.crosshair.set(Some(CrosshairHit { handle, distance: hit.distance }));
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
