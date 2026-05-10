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
use avian3d::math::Dir;
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
use core_resources::{ConnectionInfo, GameBridges, InputSnapshot, LocalEventBus, LocalObjectMarker, ModelName, ModelRegistry};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::Predicted;

use crate::config::ClientConfigResource;
use crate::physics::StaticWorldCollider;
use crate::native_assets::AdmHandleCache;
use crate::drawable::{AdmSceneRoot, DisableDrawableCollisions, DrawableCollision, CollisionShape, CollisionMaterial};
use crate::AppState;

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
            collect_and_send_input.run_if(in_state(AppState::InGame)),
        );
        // app.add_systems(Update, debug_player_movement.run_if(on_timer(std::time::Duration::from_secs(2))));
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

        // Hráč dostane capsule collider pro fyziku
        let player_collider = DrawableCollision {
            shape: CollisionShape::Capsule,
            half_extents: None,
            radius: Some(0.4),
            height: Some(1.7),
            mass: 80.0,
            is_static: false,
            climbable: false,
            ladder: false,
            material: CollisionMaterial::Concrete,
            friction: 0.0,
            restitution: 0.0,
            tags: vec![],
            lock_translation: Some([false, false, false]),
            lock_rotation: Some([false, true, true]),
        };

        commands
            .entity(entity)
            .insert((
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Visible,
                InheritedVisibility::default(),
                ViewVisibility::default(),
                PlayerVisualAttached,
                player_collider,
            ))
            .with_children(|p| {
                p.spawn((
                    AdmSceneRoot(model.0.clone()),
                    DisableDrawableCollisions,
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
    // Preferuj server-confirmed stav, fallback na local predicted NetTransform.
    for (mut local, _marker, predicted_net, confirmed_net) in predicted_q.iter_mut() {
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
    fixed_time: Res<Time<Fixed>>,
    local_client_id: Option<Res<LocalClientId>>,
    spatial_query: SpatialQuery,
    static_world_colliders: Query<(), With<StaticWorldCollider>>,
    predicted_players: Query<(&PlayerMarker, &NetTransform), With<Predicted>>,
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

    // Klientske WASD je v "camera-local" prostoru.
    // Server sim ale ocekava world-space move_dir, takze vektor
    // pred odeslanim otocime podle aktualni yaw kamery.
    let yaw_rad = look.yaw;
    let forward_x = yaw_rad.sin();
    let forward_z = yaw_rad.cos();
    let right_x = yaw_rad.cos();
    let right_z = -yaw_rad.sin();
    let world_move_x = right_x * move_x + forward_x * move_y;
    let world_move_z = right_z * move_x + forward_z * move_y;

    let mut clamped_move_x = world_move_x;
    let mut clamped_move_z = world_move_z;

    if (world_move_x != 0.0 || world_move_z != 0.0) && local_client_id.is_some() {
        let crouching = keys.pressed(bindings.crouch);
        let sprinting = keys.pressed(bindings.sprint) && !crouching;

        let mut move_speed = core_net::sim::PLAYER_MOVE_SPEED;
        if sprinting {
            move_speed *= core_net::sim::PLAYER_SPRINT_MULTIPLIER;
        }
        if crouching {
            move_speed *= core_net::sim::PLAYER_CROUCH_MULTIPLIER;
        }

        if let Some(local_id) = local_client_id {
            let player_pos = predicted_players
                .iter()
                .find(|(marker, _)| marker.client_id == local_id.0)
                .map(|(_, t)| t.translation);

            if let Some(player_pos) = player_pos {
                let move_dir = Vec3::new(world_move_x, 0.0, world_move_z);
                let max_move_distance = move_speed * fixed_time.delta_secs();
                let desired_move_delta = move_dir * max_move_distance;
                let resolved_move_delta = resolve_movement_with_colliders(
                    player_pos,
                    desired_move_delta,
                    &spatial_query,
                    &static_world_colliders,
                );

                if max_move_distance > 0.00001 {
                    clamped_move_x = (resolved_move_delta.x / max_move_distance).clamp(-1.0, 1.0);
                    clamped_move_z = (resolved_move_delta.z / max_move_distance).clamp(-1.0, 1.0);
                }
            }
        }
    }

    let mut actions = 0u32;
    if mouse.pressed(mouse_b.attack_primary) { actions |= player_action::PRIMARY_FIRE; }
    if mouse.pressed(mouse_b.attack_secondary) { actions |= player_action::SECONDARY_FIRE; }
    if keys.pressed(bindings.reload) { actions |= player_action::RELOAD; }
    if keys.pressed(bindings.jump) { actions |= player_action::JUMP; }
    if keys.pressed(bindings.crouch) { actions |= player_action::CROUCH; }
    if keys.pressed(bindings.sprint) { actions |= player_action::SPRINT; }
    if keys.pressed(bindings.interact) { actions |= player_action::INTERACT; }
    if keys.pressed(bindings.use_item) { actions |= player_action::USE_ITEM; }

    // Yaw posilame primo z kamery (stabilni mouse-delta kontrola).
    let yaw = look.yaw.to_degrees();

    let input = PlayerInput {
        move_dir: [clamped_move_x, clamped_move_z],
        look: [yaw, 0.0],
        actions,
        client_tick: *tick,
    };

    for mut sender in senders.iter_mut() {
        let _ = sender.send::<InputChannel>(input.clone());
    }
}

fn resolve_movement_with_colliders(
    player_pos: Vec3,
    desired_move_delta: Vec3,
    spatial_query: &SpatialQuery,
    static_world_colliders: &Query<(), With<StaticWorldCollider>>,
) -> Vec3 {
    const SKIN_WIDTH: f32 = 0.02;

    if desired_move_delta.length_squared() <= 0.0000001 {
        return Vec3::ZERO;
    }

    // Hráč aproximován kapslí: síťová pozice je u nohou, proto offset středu nahoru.
    let player_shape = Collider::capsule(0.35, 1.1);
    let shape_origin = Vec3::new(player_pos.x, player_pos.y + 0.9, player_pos.z);

    let Some((move_dir, move_distance)) = Dir::new_and_length(desired_move_delta).ok() else {
        return Vec3::ZERO;
    };

    let cast_cfg = ShapeCastConfig::from_max_distance(move_distance);
    let cast_filter = SpatialQueryFilter::default();

    let first_hit = spatial_query.cast_shape_predicate(
        &player_shape,
        shape_origin,
        Quat::IDENTITY,
        move_dir,
        &cast_cfg,
        &cast_filter,
        &|entity| static_world_colliders.get(entity).is_ok(),
    );

    let Some(hit) = first_hit else {
        return desired_move_delta;
    };

    let first_move = move_dir.as_vec3() * (hit.distance - SKIN_WIDTH).max(0.0);
    let remaining = desired_move_delta - first_move;
    if remaining.length_squared() <= 0.0000001 {
        return first_move;
    }

    // Jednoduchy wall-slide: odstran komponentu smerem do kolizni normaly.
    let wall_normal = Vec3::new(hit.normal1.x, 0.0, hit.normal1.z).normalize_or_zero();
    if wall_normal.length_squared() <= 0.0000001 {
        return first_move;
    }

    let slide_delta = remaining - wall_normal * remaining.dot(wall_normal);
    let Some((slide_dir, slide_distance)) = Dir::new_and_length(slide_delta).ok() else {
        return first_move;
    };

    let slide_origin = shape_origin + first_move;
    let slide_cfg = ShapeCastConfig::from_max_distance(slide_distance);
    let slide_hit = spatial_query.cast_shape_predicate(
        &player_shape,
        slide_origin,
        Quat::IDENTITY,
        slide_dir,
        &slide_cfg,
        &cast_filter,
        &|entity| static_world_colliders.get(entity).is_ok(),
    );

    let slide_move = if let Some(hit) = slide_hit {
        slide_dir.as_vec3() * (hit.distance - SKIN_WIDTH).max(0.0)
    } else {
        slide_delta
    };

    first_move + slide_move
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
