use bevy::ecs::message::MessageReader;
use bevy::input::mouse::MouseMotion;
use bevy::light::GlobalAmbientLight;
use bevy::pbr::DistanceFog;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use core_net::{ClientHandshakeState, HandshakeStatus};
use core_resources::{CameraAttachment, GameBridges, LuaWorldState};
use core_shared::PlayerMarker;
use lightyear::prelude::Predicted;

use super::{LocalClientId, resolve_default_ped_profile};
use crate::config::ClientConfigResource;
use crate::drawable::{PedPhysicsDef, PedPhysicsRegistry};

const THIRD_PERSON_DISTANCE: f32 = 5.5;
const MAX_PITCH_RAD: f32 = 1.25;
const MOUSE_SENS_SCALE: f32 = 0.0025;
const DEFAULT_CAMERA_FOV: f32 = std::f32::consts::FRAC_PI_3;
/// FOV while aiming down sights (ADS), in radians (~60 degrees).
const ADS_CAMERA_FOV: f32 = std::f32::consts::FRAC_PI_3 * (60.0 / 60.0) * (60.0 / 90.0);
/// Speed of FOV interpolation when entering/exiting ADS.
const ADS_FOV_LERP_SPEED: f32 = 10.0;
/// Maximum camera lean angle in radians (~8 degrees).
const MAX_LEAN_ANGLE: f32 = 0.1396;
/// Speed of lean interpolation.
const LEAN_LERP_SPEED: f32 = 8.0;
/// Trauma decay rate per second.
const TRAUMA_DECAY_RATE: f32 = 2.0;
/// Maximum shake angle offset in radians for trauma = 1.
const SHAKE_MAX_ANGLE: f32 = 0.05;
/// Head bob frequency in radians/second while running.
const HEAD_BOB_FREQUENCY: f32 = 9.0;
/// Head bob amplitude in metres while running.
const HEAD_BOB_AMPLITUDE: f32 = 0.045;
/// Minimum horizontal speed (m/s) to activate head bob.
#[allow(dead_code)]
const HEAD_BOB_MIN_SPEED: f32 = 0.3;

/// Camera shake state. Add trauma (0-1) to trigger shake; it decays automatically.
#[derive(Component, Default)]
pub(super) struct CameraShake {
    /// Current trauma level [0, 1]. Set this to add shake; it decays automatically.
    pub(super) trauma: f32,
    /// Current noise-derived yaw/pitch offset applied this frame.
    pub(super) shake_offset: Vec2,
    /// Internal phase accumulator for shake noise.
    shake_time: f32,
}

/// Head-bob state attached to the gameplay camera.
#[derive(Component, Default)]
pub(super) struct HeadBob {
    /// Phase accumulator (advances while player is moving on ground).
    pub(super) timer: f32,
    /// Current vertical bob amplitude [0, HEAD_BOB_AMPLITUDE]. Blends in/out.
    pub(super) amplitude: f32,
    /// Bob frequency (rad/s). Defaults to HEAD_BOB_FREQUENCY.
    pub(super) frequency: f32,
}

/// Per-camera ADS and lean state.
#[derive(Component)]
pub(super) struct CameraFovLean {
    /// Current interpolated FOV (radians).
    pub(super) current_fov: f32,
    /// Target FOV: ADS_CAMERA_FOV when aiming, DEFAULT_CAMERA_FOV otherwise.
    pub(super) ads_fov_target: f32,
    /// Current lean roll (radians). Positive = lean right.
    pub(super) current_lean: f32,
    /// Target lean roll set from Q/E key input.
    pub(super) target_lean: f32,
}

impl Default for CameraFovLean {
    fn default() -> Self {
        Self {
            current_fov: DEFAULT_CAMERA_FOV,
            ads_fov_target: DEFAULT_CAMERA_FOV,
            current_lean: 0.0,
            target_lean: 0.0,
        }
    }
}

#[derive(Resource, Clone, Copy)]
pub(super) struct CameraLookState {
    pub(super) yaw: f32,
    pub(super) pitch: f32,
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
pub(super) struct MainGameplayCamera;

pub(super) fn setup_scene_and_camera(mut commands: Commands) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 0.0,
        ..default()
    });

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-5.0, 6.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        DistanceFog::default(),
        MainGameplayCamera,
        CameraShake::default(),
        HeadBob {
            timer: 0.0,
            amplitude: 0.0,
            frequency: HEAD_BOB_FREQUENCY,
        },
        CameraFovLean::default(),
    ));

    info!("[gameplay/client] 3D scene ready (camera toggle: F6)");
}

pub(super) fn update_raycast_bridge(
    camera_q: Query<&GlobalTransform, With<MainGameplayCamera>>,
    bridges: Res<GameBridges>,
) {
    let raycast = &bridges.raycast;
    let Ok(cam_transform) = camera_q.single() else {
        return;
    };
    let origin = cam_transform.translation();
    let dir = cam_transform.forward();

    let dir_y = dir.y;
    if dir_y.abs() < 0.0001 {
        return;
    }

    let t = -origin.y / dir_y;
    if t <= 0.0 {
        return;
    }

    let hit = origin + dir * t;
    raycast.set_pos([hit.x, 0.0, hit.z]);
}

pub(super) fn toggle_camera_mode(keys: Res<ButtonInput<KeyCode>>, bridges: Res<GameBridges>) {
    if !keys.just_pressed(KeyCode::F6) {
        return;
    }
    let new_first = !bridges.camera.is_first_person();
    bridges.camera.set_first_person(new_first);
    info!(
        "[gameplay/client] camera mode -> {}",
        if new_first { "first_person" } else { "third_person" }
    );
}

pub(super) fn update_camera_look_from_mouse(
    mut motions: MessageReader<MouseMotion>,
    cfg: Res<ClientConfigResource>,
    bridges: Res<GameBridges>,
    mut look: ResMut<CameraLookState>,
) {
    if !bridges.engine.cursor_locked() {
        for _ in motions.read() {}
        return;
    }

    let mut delta = Vec2::ZERO;
    for motion in motions.read() {
        delta += motion.delta;
    }

    if delta == Vec2::ZERO {
        return;
    }

    let sens = cfg.0.input.mouse_sensitivity * MOUSE_SENS_SCALE;
    let invert_y = if cfg.0.input.invert_y { 1.0 } else { -1.0 };

    look.yaw = (look.yaw - delta.x * sens).rem_euclid(std::f32::consts::TAU);
    look.pitch = (look.pitch + delta.y * sens * invert_y).clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);
}

pub(super) fn apply_cursor_mode(
    bridges: Res<GameBridges>,
    handshake: Res<ClientHandshakeState>,
    mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor_q.single_mut() else {
        return;
    };

    if handshake.status == HandshakeStatus::AwaitingAuth {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
        return;
    }

    let locked = bridges.engine.cursor_locked();
    cursor.visible = !locked;
    cursor.grab_mode = if locked {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
}

pub(super) fn find_bone_entity(
    root: Entity,
    bone: &str,
    children_q: &Query<&Children>,
    name_q: &Query<&Name>,
    depth: u8,
) -> Option<Entity> {
    if depth == 0 {
        return None;
    }
    let Ok(children) = children_q.get(root) else {
        return None;
    };
    for child in children.iter() {
        if name_q.get(child).map(|name| name.as_str() == bone).unwrap_or(false) {
            return Some(child);
        }
        if let Some(found) = find_bone_entity(child, bone, children_q, name_q, depth - 1) {
            return Some(found);
        }
    }
    None
}

pub(super) fn update_camera_follow(
    local_client_id: Option<Res<LocalClientId>>,
    look: Res<CameraLookState>,
    bridges: Res<GameBridges>,
    world_state: Res<LuaWorldState>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    predicted_players: Query<(&Transform, &PlayerMarker), (With<Predicted>, Without<MainGameplayCamera>)>,
    entity_q: Query<&GlobalTransform, Without<MainGameplayCamera>>,
    children_q: Query<&Children>,
    name_q: Query<&Name>,
    mut cam_q: Query<(&mut Transform, &mut Projection), With<MainGameplayCamera>>,
) {
    let Ok((mut cam_transform, mut projection)) = cam_q.single_mut() else {
        return;
    };

    let cp = look.pitch.cos();
    let mut forward = Vec3::new(look.yaw.sin() * cp, look.pitch.sin(), look.yaw.cos() * cp);
    if forward.length_squared() < 0.0001 {
        forward = Vec3::Z;
    } else {
        forward = forward.normalize();
    }

    let target_fov = bridges
        .camera
        .get_active_rig()
        .and_then(|rig| rig.fov)
        .map(|deg| deg.to_radians())
        .unwrap_or(DEFAULT_CAMERA_FOV);
    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = target_fov;
    }

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
            CameraAttachment::Entity {
                handle,
                offset,
                look_at,
            } => {
                if let Some(entity) = world_state.entity_for(*handle) {
                    if let Ok(entity_transform) = entity_q.get(entity) {
                        let entity_pos = entity_transform.translation();
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
            CameraAttachment::Bone {
                handle,
                bone,
                offset,
            } => {
                if let Some(entity) = world_state.entity_for(*handle) {
                    if let Some(bone_ent) = find_bone_entity(entity, bone, &children_q, &name_q, 8) {
                        if let Ok(bone_transform) = entity_q.get(bone_ent) {
                            let (_, bone_rot, bone_pos) =
                                bone_transform.to_scale_rotation_translation();
                            cam_transform.translation = bone_pos + bone_rot * Vec3::from(*offset);
                            cam_transform.rotation = bone_rot;
                        }
                    }
                }
            }
        }
        return;
    }

    let Some(local_client_id) = local_client_id else {
        return;
    };
    let mut player_pos = None;
    for (transform, marker) in predicted_players.iter() {
        if marker.client_id == local_client_id.0 {
            player_pos = Some(transform.translation);
            break;
        }
    }
    let Some(player_pos) = player_pos else {
        return;
    };

    let eye_height = resolve_default_ped_profile(&ped_reg, &ped_assets)
        .map(|ped| ped.capsule.eye_height)
        .unwrap_or(1.65);
    let focus = player_pos + Vec3::new(0.0, eye_height, 0.0);

    if bridges.camera.is_first_person() {
        cam_transform.translation = focus;
        cam_transform.look_at(focus + forward, Vec3::Y);
    } else {
        let eye = focus - forward * THIRD_PERSON_DISTANCE;
        cam_transform.translation = eye;
        cam_transform.look_at(focus, Vec3::Y);
    }
}

// ---------------------------------------------------------------------------
// Camera shake system
// ---------------------------------------------------------------------------

/// Decays trauma and applies a sine-noise shake offset (yaw/pitch) to the
/// camera. Add trauma to `CameraShake.trauma` from other systems to trigger.
pub(super) fn update_camera_shake(
    time: Res<Time>,
    mut cam_q: Query<&mut CameraShake, With<MainGameplayCamera>>,
) {
    let dt = time.delta_secs();
    let Ok(mut shake) = cam_q.single_mut() else {
        return;
    };

    // Decay trauma over time.
    shake.trauma = (shake.trauma - TRAUMA_DECAY_RATE * dt).max(0.0);
    shake.shake_time += dt;

    let intensity = shake.trauma * shake.trauma; // trauma^2 for natural rolloff
    if intensity < 0.0001 {
        shake.shake_offset = Vec2::ZERO;
        return;
    }

    // Pseudo-random noise via fast sine harmonics (no external crate needed).
    let t = shake.shake_time;
    let noise_yaw = (t * 37.1).sin() * 0.6
        + (t * 83.7).sin() * 0.3
        + (t * 151.3).sin() * 0.1;
    let noise_pitch = (t * 41.3).sin() * 0.6
        + (t * 97.1).sin() * 0.3
        + (t * 173.9).sin() * 0.1;

    let offset_yaw = noise_yaw * intensity * SHAKE_MAX_ANGLE;
    let offset_pitch = noise_pitch * intensity * SHAKE_MAX_ANGLE;
    // Store only — applied in update_fov_and_lean so shake, lean, and look compose cleanly.
    shake.shake_offset = Vec2::new(offset_yaw, offset_pitch);
}

/// Adds a small burst of camera trauma when the primary fire button is pressed.
/// Trauma decays automatically in `update_camera_shake`.
pub(super) fn add_fire_trauma(
    mouse: Res<ButtonInput<MouseButton>>,
    cfg: Res<ClientConfigResource>,
    mut cam_q: Query<&mut CameraShake, With<MainGameplayCamera>>,
) {
    let fire_btn = cfg.0.input.mouse.attack_primary;
    if !mouse.just_pressed(fire_btn) {
        return;
    }
    if let Ok(mut shake) = cam_q.single_mut() {
        shake.trauma = (shake.trauma + 0.18).min(1.0);
    }
}

// ---------------------------------------------------------------------------
// Head bob system
// ---------------------------------------------------------------------------

/// Applies a vertical sine offset to the camera while the local player exists
/// in the world, blending amplitude smoothly in and out.
pub(super) fn update_head_bob(
    time: Res<Time>,
    local_client_id: Option<Res<LocalClientId>>,
    players: Query<(&PlayerMarker, &super::movement::PlayerMovementState), With<Predicted>>,
    mut cam_q: Query<(&mut HeadBob, &mut Transform), With<MainGameplayCamera>>,
) {
    let dt = time.delta_secs();
    let Ok((mut bob, mut cam_transform)) = cam_q.single_mut() else {
        return;
    };

    // Gate head bob on actual movement speed — never bobs when standing still.
    let local_speed = local_client_id.as_ref()
        .and_then(|lid| {
            players.iter()
                .find(|(marker, _)| marker.client_id == lid.0)
                .map(|(_, mvt)| mvt.velocity_xz.length())
        })
        .unwrap_or(0.0);

    let amplitude_target = if local_speed > 0.5 { HEAD_BOB_AMPLITUDE } else { 0.0 };
    let blend_speed = if amplitude_target > bob.amplitude { 6.0 } else { 3.0 };
    bob.amplitude += (amplitude_target - bob.amplitude) * (blend_speed * dt).min(1.0);

    if bob.amplitude < 0.001 {
        return;
    }

    let freq = if bob.frequency < 0.001 { HEAD_BOB_FREQUENCY } else { bob.frequency };
    bob.timer += dt * freq;

    let vertical_offset = bob.timer.sin() * bob.amplitude;
    cam_transform.translation.y += vertical_offset;
}

// ---------------------------------------------------------------------------
// ADS FOV + camera lean system
// ---------------------------------------------------------------------------

/// Smoothly interpolates FOV between default, movement-kick and ADS values,
/// and applies a lean roll (Q = left, E = right) as a local-Z rotation.
/// Must run AFTER `update_camera_follow` and `apply_player_movement` so both
/// the Lua rig FOV and the movement FOV kick are already written.
pub(super) fn update_fov_and_lean(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cfg: Res<ClientConfigResource>,
    fov_state: Res<super::movement::MovementFovState>,
    look: Res<CameraLookState>,
    bridges: Res<GameBridges>,
    mut cam_q: Query<
        (&mut CameraFovLean, &mut Transform, &mut Projection, &CameraShake),
        With<MainGameplayCamera>,
    >,
) {
    let dt = time.delta_secs();
    let Ok((mut fov_lean, mut cam_transform, mut projection, shake)) = cam_q.single_mut() else {
        return;
    };

    // ADS: use configured aim key; right mouse button also accepted.
    let aim_key = cfg.0.input.keys.aim;
    let is_ads = keys.pressed(aim_key) || mouse.pressed(MouseButton::Right);

    // Base FOV: use the movement-kick value (sprint / slide / default) unless ADS.
    // ADS always narrows to ADS_CAMERA_FOV regardless of movement state.
    fov_lean.ads_fov_target = if is_ads {
        ADS_CAMERA_FOV
    } else {
        fov_state.current_fov_rad
    };
    fov_lean.current_fov +=
        (fov_lean.ads_fov_target - fov_lean.current_fov) * (ADS_FOV_LERP_SPEED * dt).min(1.0);

    if let Projection::Perspective(perspective) = projection.as_mut() {
        perspective.fov = fov_lean.current_fov;
    }

    // Lean: Q = lean left (negative roll), E = lean right (positive roll).
    let lean_left = keys.pressed(KeyCode::KeyQ);
    let lean_right = keys.pressed(KeyCode::KeyE);
    fov_lean.target_lean = match (lean_left, lean_right) {
        (true, false) => -MAX_LEAN_ANGLE,
        (false, true) => MAX_LEAN_ANGLE,
        _ => 0.0,
    };
    fov_lean.current_lean +=
        (fov_lean.target_lean - fov_lean.current_lean) * (LEAN_LERP_SPEED * dt).min(1.0);

    // Compose final camera rotation from authoritative CameraLookState + shake + lean.
    // This avoids euler decomposition drift from operating on an already-modified quaternion.
    let no_active_rig = bridges.camera.get_active_rig().is_none();
    if no_active_rig && bridges.camera.is_first_person() {
        // First-person free camera: compose directly from look state so mouse feel is crisp.
        cam_transform.rotation = Quat::from_euler(
            EulerRot::YXZ,
            look.yaw + shake.shake_offset.x,
            look.pitch + shake.shake_offset.y,
            fov_lean.current_lean,
        );
    } else if fov_lean.current_lean.abs() > 0.0001 || shake.shake_offset.length_squared() > 1e-6 {
        // Third-person / Lua rig: camera direction set by update_camera_follow; just add shake
        // and lean on top without overriding the look-at direction entirely.
        let (yaw, pitch, _) = cam_transform.rotation.to_euler(EulerRot::YXZ);
        cam_transform.rotation = Quat::from_euler(
            EulerRot::YXZ,
            yaw + shake.shake_offset.x,
            pitch + shake.shake_offset.y,
            fov_lean.current_lean,
        );
    }
}