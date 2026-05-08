use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

/// Marker pro orbit kameru.
#[derive(Component)]
pub struct OrbitCamera;

/// Stav orbit kamery.
#[derive(Resource)]
pub struct OrbitState {
    pub yaw:      f32,
    pub pitch:    f32,
    pub distance: f32,
    pub focus:    Vec3,
}

impl Default for OrbitState {
    fn default() -> Self {
        Self { yaw: -0.5, pitch: 0.4, distance: 4.0, focus: Vec3::ZERO }
    }
}

/// Orbit pravým tlačítkem, pan středním, zoom kolečkem.
pub fn orbit_camera(
    mut state: ResMut<OrbitState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut camera: Query<&mut Transform, With<OrbitCamera>>,
) {
    let Ok(mut transform) = camera.single_mut() else { return };

    let orbit = mouse_buttons.pressed(MouseButton::Right);
    let pan   = mouse_buttons.pressed(MouseButton::Middle);

    if orbit {
        state.yaw   -= mouse_motion.delta.x * 0.005;
        state.pitch  = (state.pitch - mouse_motion.delta.y * 0.005).clamp(-1.5, 1.5);
    }
    if pan {
        let right: Vec3 = *transform.right();
        let up: Vec3    = *transform.up();
        let speed = state.distance * 0.0008;
        state.focus -= right * mouse_motion.delta.x * speed;
        state.focus += up   * mouse_motion.delta.y * speed;
    }

    let scroll_delta = scroll.delta.y + scroll.delta.x;
    if scroll_delta.abs() > 0.0 {
        state.distance = (state.distance - scroll_delta * 0.4).max(0.05);
    }

    let rot    = Quat::from_euler(EulerRot::YXZ, state.yaw, state.pitch, 0.0);
    let offset = rot * Vec3::new(0.0, 0.0, state.distance);
    transform.translation = state.focus + offset;
    transform.look_at(state.focus, Vec3::Y);
}
