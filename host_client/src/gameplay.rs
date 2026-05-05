//! Phase 3 — klientska gameplay vrstva.
//!
//! Phase 3.7: `RaycastBridge` se aktualizuje kazdy frame z pozice mysi.
//! Lua sandbox cte pres `Raycast.GetGroundPosition()`.

use bevy::math::primitives::{Capsule3d, Cuboid, Plane3d};
use bevy::input::mouse::MouseMotion;
use bevy::ecs::message::MessageReader;
use bevy::prelude::*;
use bevy::transform::components::TransformTreeChanged;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
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
use core_resources::{LocalEventBus, LocalObjectMarker, ModelRegistry, RaycastBridge};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::Predicted;

use crate::config::ClientConfigResource;

const THIRD_PERSON_DISTANCE: f32 = 5.5;
const FIRST_PERSON_EYE_HEIGHT: f32 = 1.7;
const PLAYER_MODEL_ASSET_PATH: &str = "models/player.glb#Scene0";
const MAX_PITCH_RAD: f32 = 1.25;
const MOUSE_SENS_SCALE: f32 = 0.0025;
const POSITION_SMOOTHING_RATE: f32 = 14.0;
const ROTATION_SMOOTHING_RATE: f32 = 18.0;

#[derive(Resource, Clone, Copy)]
pub struct LocalClientId(pub u64);

#[derive(Resource, Clone)]
struct PlayerModelHandle(Handle<Scene>);

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
        app.add_systems(Startup, setup_scene_and_camera);
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
                update_local_player_visibility,
                publish_input_state_to_lua,
                attach_mesh_to_local_objects,
            )
                .chain(),
        );
        app.add_systems(FixedUpdate, collect_and_send_input);
        // app.add_systems(Update, debug_player_movement.run_if(on_timer(std::time::Duration::from_secs(2))));
    }
}

fn setup_scene_and_camera(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(200.0, 200.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.16, 0.12),
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));

    // Orientacni body ve svete: je hned videt, jestli se hybe hrac nebo scena.
    let marker_mesh = meshes.add(Cuboid::new(1.2, 2.8, 1.2));
    let marker_positions = [
        (Vec3::new(0.0, 1.4, 0.0), Color::srgb(1.0, 0.2, 0.2)),
        (Vec3::new(10.0, 1.4, 0.0), Color::srgb(1.0, 0.85, 0.2)),
        (Vec3::new(-10.0, 1.4, 0.0), Color::srgb(0.2, 0.85, 1.0)),
        (Vec3::new(0.0, 1.4, 10.0), Color::srgb(0.2, 1.0, 0.35)),
        (Vec3::new(0.0, 1.4, -10.0), Color::srgb(1.0, 0.35, 0.9)),
        (Vec3::new(18.0, 1.4, 18.0), Color::srgb(1.0, 0.55, 0.15)),
        (Vec3::new(-18.0, 1.4, -18.0), Color::srgb(0.65, 0.4, 1.0)),
    ];
    for (pos, color) in marker_positions {
        commands.spawn((
            Mesh3d(marker_mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                emissive: color.into(),
                perceptual_roughness: 0.45,
                ..default()
            })),
            Transform::from_translation(pos),
        ));
    }

    // Delsi referencni objekty kolem os X/Z.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(30.0, 0.3, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.2, 0.2),
            emissive: Color::srgb(0.25, 0.05, 0.05).into(),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.15, 0.0),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.0, 0.3, 30.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.75, 0.95),
            emissive: Color::srgb(0.05, 0.15, 0.2).into(),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.15, 0.0),
    ));

    commands.insert_resource(PlayerModelHandle(asset_server.load(PLAYER_MODEL_ASSET_PATH)));

    info!(
        "[gameplay/client] 3D scene ready (camera toggle: F6, player model: {})",
        PLAYER_MODEL_ASSET_PATH
    );
}

/// Aktualizuje `RaycastBridge` podle aktualni pozice mysi.
/// Tato pozice se pouziva v Lua `Raycast.GetGroundPosition()`.
fn update_raycast_bridge(
    camera_q: Query<&GlobalTransform, With<MainGameplayCamera>>,
    raycast: Res<RaycastBridge>,
) {
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
    mut look: ResMut<CameraLookState>,
) {
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
    mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor_q.single_mut() else { return };

    // V obou rezimech drzi gameplay relativni ovladani mysi.
    cursor.visible = false;
    cursor.grab_mode = CursorGrabMode::Locked;

    // Pouzij `mode` aby system stale reagoval na zmenu a nevznikal warning.
    let _ = mode.mode;
}

fn attach_player_model_to_new_players(
    mut commands: Commands,
    model: Res<PlayerModelHandle>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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

        let hue = (marker.client_id as f32 * 47.0).rem_euclid(360.0);
        let color = Color::hsl(hue, 0.7, 0.6);

        commands
            .entity(entity)
            .insert((
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Visible,
                InheritedVisibility::default(),
                ViewVisibility::default(),
                PlayerVisualAttached,
            ))
            .with_children(|p| {
                p.spawn((
                    SceneRoot(model.0.clone()),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                ));

                // Fallback mesh viditelny i kdyz model neni dostupny.
                p.spawn((
                    Mesh3d(meshes.add(Capsule3d::new(0.35, 1.0))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: color,
                        ..default()
                    })),
                    Transform::from_xyz(0.0, 1.0, 0.0),
                ));
            });

        info!(
            "[gameplay/client] model attached to player {:?} (client_id={})",
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
        move_dir: [world_move_x, world_move_z],
        look: [yaw, 0.0],
        actions,
        client_tick: *tick,
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
        }
    }))
    .unwrap_or_default();

    local_bus.push("input:state".to_string(), payload);
}

/// Prida Sprite na lokalni objekty spawnute pres `World.SpawnLocalObject`.
/// Bez vizualu by byly neviditelne.
fn attach_mesh_to_local_objects(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    model_registry: Res<ModelRegistry>,
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
            let scene_path = AssetPath::from_path_buf(path.clone()).with_label("Scene0");
            let scene: Handle<Scene> = asset_server.load_override(scene_path);
            commands.entity(entity).with_children(|p| {
                p.spawn((
                    SceneRoot(scene),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                ));
            });
            continue;
        }

        // Fallback: neznamy model = default kostka (debugitelne chovani).
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::new(0.9, 0.9, 0.9))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.2, 0.85, 0.3),
                ..default()
            })),
            Transform::from_xyz(0.0, 0.45, 0.0),
        ));
        warn!(
            "[gameplay/client] LocalObject '{}' not found in ModelRegistry; using fallback cube",
            marker.model
        );
    }
}
