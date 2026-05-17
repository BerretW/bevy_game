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