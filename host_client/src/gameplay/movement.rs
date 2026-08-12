use avian3d::prelude::*;
use bevy::prelude::*;
use core_net::{player_action, InputChannel, PlayerInput};
use core_shared::PlayerMarker;
use lightyear::prelude::*;
use lightyear::prelude::Predicted;

use super::{LocalClientId, camera::CameraLookState, resolve_default_ped_profile};
use crate::config::{ClientConfigResource, Keybindings};
use crate::drawable::{PedPhysicsDef, PedPhysicsRegistry};
use core_resources::LocalEventBus;

// ---------------------------------------------------------------------------
// Ground contact constants
// ---------------------------------------------------------------------------

/// Hystereze pro detekci ground kontaktu (v metrech).
const GROUND_CONTACT_DIST_ENTER: f32 = 0.06;
const GROUND_CONTACT_DIST_EXIT: f32 = 0.11;

// ---------------------------------------------------------------------------
// Movement constants / fallbacks
// ---------------------------------------------------------------------------

const PLAYER_MOVE_SPEED: f32 = 5.0;
const PLAYER_SPRINT_MULT: f32 = 1.8;
const PLAYER_CROUCH_MULT: f32 = 0.5;
const PLAYER_JUMP_VEL: f32 = 6.0;
const GROUND_VEL_THRESHOLD: f32 = 0.25;
/// cos(~49°) — surfaces steeper than this are not walkable.
const GROUND_NORMAL_Y_MIN: f32 = 0.65;
/// Grace period after losing ground contact (coyote time).
const COYOTE_TIME_SECS: f32 = 0.15;

// ---------------------------------------------------------------------------
// Cyberpunk controller tuning
// ---------------------------------------------------------------------------

/// How quickly horizontal velocity ramps up toward target (per-second factor).
const ACCEL_GROUND: f32 = 14.0;
/// How quickly horizontal velocity decelerates when no input (per-second factor).
const DECEL_GROUND: f32 = 8.0;
/// Acceleration multiplier while airborne.
const ACCEL_AIR_MULT: f32 = 0.35;

/// Slide: initial speed boost over current sprint speed.
const SLIDE_BOOST_MULT: f32 = 1.5;
/// Slide: friction deceleration (m/s per second).
const SLIDE_FRICTION: f32 = 4.0;
/// Slide: total duration in seconds.
const SLIDE_DURATION: f32 = 0.8;

/// Dodge: impulse speed (m/s).
const DODGE_SPEED: f32 = 12.0;
/// Dodge: duration in seconds.
const DODGE_DURATION: f32 = 0.2;
/// Dodge: cooldown in seconds.
const DODGE_COOLDOWN: f32 = 1.5;
/// Dodge: maximum vertical velocity to allow a dodge.
const DODGE_MAX_AIR_VEL: f32 = 3.0;

/// FOV for normal movement (degrees).
const FOV_DEFAULT_DEG: f32 = 75.0;
/// FOV while sprinting (degrees).
const FOV_SPRINT_DEG: f32 = 95.0;
/// FOV at peak slide (degrees).
const FOV_SLIDE_DEG: f32 = 100.0;
/// Lerp speed for FOV transitions (per second).
const FOV_LERP_SPEED: f32 = 6.0;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub(super) struct MovementGroundState {
    pub(super) last_grounded_time: f32,
}

/// Tracks current camera FOV so that update_camera_follow can smoothly lerp it.
#[derive(Resource)]
pub(super) struct MovementFovState {
    /// Smoothed FOV in radians written each frame by apply_player_movement.
    pub(super) current_fov_rad: f32,
}

impl Default for MovementFovState {
    fn default() -> Self {
        Self {
            current_fov_rad: FOV_DEFAULT_DEG.to_radians(),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-player movement state component
// ---------------------------------------------------------------------------

/// Persistent per-player state for Cyberpunk-style movement mechanics.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerMovementState {
    /// Smoothed horizontal velocity (world-space XZ, metres/second).
    pub velocity_xz: Vec2,
    pub is_sprinting: bool,
    pub is_crouching: bool,
    /// True while sliding.
    pub is_sliding: bool,
    /// Countdown timer for slide (seconds remaining).
    pub slide_timer: f32,
    /// Slide direction (world-space unit vector XZ).
    pub slide_dir: Vec2,
    /// Countdown timer for dodge impulse (seconds remaining).
    pub dodge_timer: f32,
    /// Dodge direction (world-space unit XZ).
    pub dodge_dir: Vec2,
    /// Cooldown timer for dodge (seconds until next allowed).
    pub dodge_cooldown: f32,
    /// Tracks last time a direction key was first pressed (for double-tap detection).
    pub last_dir_press_time: [f32; 4], // W, S, A, D
    /// Whether the direction key was already pressed in the previous frame.
    pub dir_was_pressed: [bool; 4],
    /// Ground contact last frame.
    pub ground_contact: bool,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Main movement system — Cyberpunk 2077-style feel.
///
/// Responsibilities:
/// - Smooth velocity lerp with separate accel / decel rates
/// - Sprint, crouch, slide, dodge
/// - Jump + coyote time
/// - FOV kick written to `MovementFovState`
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_player_movement(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    look: Res<CameraLookState>,
    local_client_id: Option<Res<LocalClientId>>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    mut ground_state: ResMut<MovementGroundState>,
    mut fov_state: ResMut<MovementFovState>,
    mut players: Query<
        (
            &PlayerMarker,
            &mut LinearVelocity,
            Option<&ShapeHits>,
            &mut PlayerMovementState,
        ),
        With<Predicted>,
    >,
) {
    let Some(local_id) = local_client_id.map(|r| r.0) else {
        return;
    };

    let dt = time.delta_secs().max(0.0001);
    let now = time.elapsed_secs();
    let bindings = &cfg.0.input.keys;

    let ped = resolve_default_ped_profile(&ped_reg, &ped_assets);
    let run_speed = ped.map(|p| p.movement.run_speed).unwrap_or(PLAYER_MOVE_SPEED);
    let sprint_speed = ped
        .map(|p| p.movement.sprint_speed)
        .unwrap_or(PLAYER_MOVE_SPEED * PLAYER_SPRINT_MULT);
    let crouch_speed = ped
        .map(|p| p.movement.crouch_speed)
        .unwrap_or(PLAYER_MOVE_SPEED * PLAYER_CROUCH_MULT);
    let jump_impulse = ped.map(|p| p.jump.impulse).unwrap_or(PLAYER_JUMP_VEL);
    let double_jump_enabled = ped.map(|p| p.jump.double_jump).unwrap_or(false);
    let max_fall_speed = ped.map(|p| p.jump.max_fall_speed).unwrap_or(28.0_f32);

    // Raw directional input and rotation
    let raw_input = normalized_move_input(bindings, &keys);
    let (world_x, world_z) = rotate_input_by_yaw(raw_input, look.yaw);
    let world_dir = Vec2::new(world_x, world_z); // normalised world-space move dir

    let sprint_held = keys.pressed(bindings.sprint);
    let crouch_held = keys.pressed(bindings.crouch);
    let jump_just = keys.just_pressed(bindings.jump);

    for (marker, mut vel, shape_hits, mut mv) in players.iter_mut() {
        if marker.client_id != local_id {
            continue;
        }

        // ----------------------------------------------------------------
        // Ground detection with coyote time
        // ----------------------------------------------------------------
        let raw_grounded = has_ground_contact(shape_hits);
        let prev_grounded = mv.ground_contact;
        if raw_grounded {
            ground_state.last_grounded_time = now;
        }
        let grounded = raw_grounded || (now - ground_state.last_grounded_time) < COYOTE_TIME_SECS;
        let ground_normal = get_ground_normal(shape_hits);
        mv.ground_contact = grounded;

        if prev_grounded && !grounded {
            info!("[movement] grounded LOST (coyote started)");
        }
        if !prev_grounded && grounded {
            info!("[movement] grounded GAINED");
        }

        let coyote_remaining = (COYOTE_TIME_SECS - (now - ground_state.last_grounded_time)).max(0.0);
        debug!("[movement] vel=({:.3},{:.3},{:.3}) grounded={} coyote={:.3}s", vel.x, vel.y, vel.z, grounded, coyote_remaining);

        // ----------------------------------------------------------------
        // Gravity
        // ----------------------------------------------------------------
        if grounded {
            if vel.y < 0.0 {
                vel.y = -(9.81_f32 * dt);
            }
        } else {
            vel.y = (vel.y - 9.81_f32 * dt).max(-max_fall_speed);
        }

        // ----------------------------------------------------------------
        // Jump
        // ----------------------------------------------------------------
        let grounded_vel_thresh = ped
            .map(|p| p.jump.grounded_vel_threshold)
            .unwrap_or(GROUND_VEL_THRESHOLD);
        let can_jump = if double_jump_enabled {
            grounded || vel.y.abs() < grounded_vel_thresh
        } else {
            grounded
        };
        if jump_just && can_jump {
            vel.y = jump_impulse;
            info!("[movement] state -> JUMP vel_y={:.2}", vel.y);
            // Cancel slide on jump
            mv.is_sliding = false;
            mv.slide_timer = 0.0;
        }

        // ----------------------------------------------------------------
        // Dodge detection (double-tap WASD or Space + direction)
        // ----------------------------------------------------------------
        if mv.dodge_cooldown > 0.0 {
            mv.dodge_cooldown -= dt;
        }
        if mv.dodge_timer > 0.0 {
            mv.dodge_timer -= dt;
        }

        let dir_keys = [
            bindings.move_forward,
            bindings.move_backward,
            bindings.move_left,
            bindings.move_right,
        ];
        let dir_vecs_raw: [(f32, f32); 4] = [
            rotate_input_by_yaw(Vec2::new(0.0, 1.0), look.yaw),
            rotate_input_by_yaw(Vec2::new(0.0, -1.0), look.yaw),
            rotate_input_by_yaw(Vec2::new(1.0, 0.0), look.yaw),
            rotate_input_by_yaw(Vec2::new(-1.0, 0.0), look.yaw),
        ];

        // Double-tap dodge: key just pressed AND was released & pressed again within 0.3s
        const DOUBLE_TAP_WINDOW: f32 = 0.3;
        if mv.dodge_cooldown <= 0.0 && mv.dodge_timer <= 0.0 {
            let airborne_ok = !grounded && vel.y.abs() < DODGE_MAX_AIR_VEL;
            let can_dodge = grounded || airborne_ok;

            for (i, &key) in dir_keys.iter().enumerate() {
                let just_pressed = keys.just_pressed(key);
                if just_pressed {
                    let elapsed_since_last = now - mv.last_dir_press_time[i];
                    if elapsed_since_last < DOUBLE_TAP_WINDOW && mv.last_dir_press_time[i] > 0.0 {
                        // Double-tap detected
                        if can_dodge && raw_input.length_squared() > 0.0 {
                            let (dx, dz) = dir_vecs_raw[i];
                            let dir = Vec2::new(dx, dz);
                            let dir_n = if dir.length_squared() > 0.0001 {
                                dir.normalize()
                            } else {
                                world_dir
                            };
                            mv.dodge_dir = dir_n;
                            mv.dodge_timer = DODGE_DURATION;
                            mv.dodge_cooldown = DODGE_COOLDOWN;
                            mv.is_sliding = false;
                            mv.slide_timer = 0.0;
                            info!("[movement] state -> DODGE");
                        }
                    }
                    mv.last_dir_press_time[i] = now;
                }
                mv.dir_was_pressed[i] = keys.pressed(key);
            }
        } else {
            // Still update timestamps so we don't false-trigger after cooldown
            for (i, &key) in dir_keys.iter().enumerate() {
                if keys.just_pressed(key) {
                    mv.last_dir_press_time[i] = now;
                }
                mv.dir_was_pressed[i] = keys.pressed(key);
            }
        }

        // ----------------------------------------------------------------
        // Slide detection: sprint + crouch just pressed while on ground
        // ----------------------------------------------------------------
        let crouch_just = keys.just_pressed(bindings.crouch);
        if grounded
            && sprint_held
            && crouch_just
            && !mv.is_sliding
            && mv.dodge_timer <= 0.0
        {
            let speed_xz = Vec2::new(vel.x, vel.z).length();
            if speed_xz > run_speed * 0.5 {
                let dir = if speed_xz > 0.01 {
                    Vec2::new(vel.x, vel.z).normalize()
                } else {
                    world_dir
                };
                mv.slide_dir = dir;
                mv.slide_timer = SLIDE_DURATION;
                mv.is_sliding = true;
                mv.is_crouching = true;
                info!("[movement] state -> SLIDE duration={:.2}s", SLIDE_DURATION);
                // Initial boost
                let boost_speed = speed_xz * SLIDE_BOOST_MULT;
                vel.x = mv.slide_dir.x * boost_speed;
                vel.z = mv.slide_dir.y * boost_speed;
                mv.velocity_xz = Vec2::new(vel.x, vel.z);
            }
        }

        // Cancel slide if we leave the ground or timer expired
        if mv.is_sliding && (!grounded || mv.slide_timer <= 0.0) {
            mv.is_sliding = false;
            mv.slide_timer = 0.0;
            // Transition to crouch after slide finishes on ground
            if grounded {
                mv.is_crouching = true;
            }
        }

        // Update crouch state (not sliding)
        let was_crouching = mv.is_crouching;
        let was_sprinting = mv.is_sprinting;
        if !mv.is_sliding {
            mv.is_crouching = crouch_held;
        }
        mv.is_sprinting = sprint_held && !mv.is_crouching && !mv.is_sliding;
        if !was_sprinting && mv.is_sprinting {
            info!("[movement] state -> SPRINT");
        }
        if !was_crouching && mv.is_crouching {
            info!("[movement] state -> CROUCH");
        }

        // ----------------------------------------------------------------
        // Horizontal velocity — Cyberpunk smooth lerp
        // ----------------------------------------------------------------
        if mv.dodge_timer > 0.0 {
            // Dodge: override velocity with impulse
            let t = mv.dodge_timer / DODGE_DURATION; // 1 → 0
            let impulse_speed = DODGE_SPEED * t;
            let target_xz = mv.dodge_dir * impulse_speed;
            vel.x = target_xz.x;
            vel.z = target_xz.y;
            // Kill vertical velocity spike
            if !grounded {
                vel.y = vel.y.max(-2.0).min(2.0);
            }
            mv.velocity_xz = Vec2::new(vel.x, vel.z);
        } else if mv.is_sliding {
            // Slide: friction deceleration along slide_dir
            mv.slide_timer -= dt;
            let current_speed = Vec2::new(vel.x, vel.z).length();
            let new_speed = (current_speed - SLIDE_FRICTION * dt).max(0.0);
            let slide_xz = mv.slide_dir * new_speed;
            vel.x = slide_xz.x;
            vel.z = slide_xz.y;
            mv.velocity_xz = Vec2::new(vel.x, vel.z);
        } else {
            // Normal movement: smooth accel / decel
            let target_speed = if mv.is_crouching {
                crouch_speed
            } else if mv.is_sprinting {
                sprint_speed
            } else {
                run_speed
            };

            let target_xz = world_dir * target_speed;

            // Use different rates for ground vs air
            let (accel, decel) = if grounded {
                (ACCEL_GROUND, DECEL_GROUND)
            } else {
                (ACCEL_GROUND * ACCEL_AIR_MULT, DECEL_GROUND * ACCEL_AIR_MULT)
            };

            let is_moving = raw_input.length_squared() > 0.001;
            let rate = if is_moving { accel } else { decel };

            let alpha = (1.0 - (-rate * dt).exp()).clamp(0.0, 1.0);
            mv.velocity_xz = mv.velocity_xz + (Vec2::new(target_xz.x, target_xz.y) - mv.velocity_xz) * alpha;

            // Project onto slope if on angled ground
            let desired = Vec3::new(mv.velocity_xz.x, 0.0, mv.velocity_xz.y);
            let projected = if grounded && ground_normal.y < 0.9999 {
                desired - ground_normal * desired.dot(ground_normal)
            } else {
                desired
            };

            vel.x = projected.x;
            vel.z = projected.z;

            if grounded && ground_normal.y < 0.997 && !jump_just {
                vel.y = projected.y;
            }
        }

        // ----------------------------------------------------------------
        // FOV kick
        // ----------------------------------------------------------------
        let target_fov_deg = if mv.is_sliding {
            FOV_SLIDE_DEG
        } else if mv.is_sprinting {
            FOV_SPRINT_DEG
        } else {
            FOV_DEFAULT_DEG
        };
        let target_fov_rad = target_fov_deg.to_radians();
        let fov_alpha = (1.0 - (-FOV_LERP_SPEED * dt).exp()).clamp(0.0, 1.0);
        fov_state.current_fov_rad = fov_state.current_fov_rad
            + (target_fov_rad - fov_state.current_fov_rad) * fov_alpha;
    }
}

/// Damps spurious upward Y spikes written by Avian contact resolution.
pub(super) fn post_physics_vel_y_damp(
    mut ground_state: ResMut<MovementGroundState>,
    time: Res<Time>,
    local_client_id: Option<Res<LocalClientId>>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    mut players: Query<
        (&PlayerMarker, &mut LinearVelocity, Option<&ShapeHits>),
        With<Predicted>,
    >,
) {
    let Some(lid) = local_client_id else {
        return;
    };
    let now = time.elapsed_secs();

    let jump_impulse = resolve_default_ped_profile(&ped_reg, &ped_assets)
        .map(|p| p.jump.impulse)
        .unwrap_or(PLAYER_JUMP_VEL);

    for (marker, mut vel, shape_hits) in players.iter_mut() {
        if marker.client_id != lid.0 {
            continue;
        }

        let was_grounded = (now - ground_state.last_grounded_time) < COYOTE_TIME_SECS;
        let raw_grounded = has_ground_contact_with_state(shape_hits, was_grounded);
        if raw_grounded {
            ground_state.last_grounded_time = now;
        }
        let recently_grounded = (now - ground_state.last_grounded_time) < COYOTE_TIME_SECS;

        let ground_normal = get_ground_normal(shape_hits);
        let on_slope = ground_normal.y < 0.997;
        if recently_grounded && !on_slope && vel.y > 0.05 && vel.y < jump_impulse * 0.90 {
            let before = vel.y;
            vel.y = 0.0;
            debug!("[movement] vel_y damp: {:.3} -> {:.3}", before, vel.y);
        }
    }
}

pub(super) fn collect_and_send_input(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    mouse: Res<ButtonInput<MouseButton>>,
    local_client_id: Option<Res<LocalClientId>>,
    predicted_players: Query<(&PlayerMarker, &Transform), With<Predicted>>,
    look: Res<CameraLookState>,
    mut senders: Query<&mut MessageSender<PlayerInput>>,
    mut tick: Local<u32>,
) {
    *tick = tick.wrapping_add(1);

    let bindings = &cfg.0.input.keys;
    let mouse_b = &cfg.0.input.mouse;
    let input = normalized_move_input(bindings, &keys);
    let (world_move_x, world_move_z) = rotate_input_by_yaw(input, look.yaw);

    debug!(
        "[movement] input keys: forward={} back={} left={} right={} sprint={} crouch={} jump={}",
        keys.pressed(bindings.move_forward),
        keys.pressed(bindings.move_backward),
        keys.pressed(bindings.move_left),
        keys.pressed(bindings.move_right),
        keys.pressed(bindings.sprint),
        keys.pressed(bindings.crouch),
        keys.pressed(bindings.jump),
    );

    let mut actions = 0u32;
    if mouse.pressed(mouse_b.attack_primary) {
        actions |= player_action::PRIMARY_FIRE;
    }
    if mouse.pressed(mouse_b.attack_secondary) {
        actions |= player_action::SECONDARY_FIRE;
    }
    if keys.pressed(bindings.reload) {
        actions |= player_action::RELOAD;
    }
    if keys.pressed(bindings.jump) {
        actions |= player_action::JUMP;
    }
    if keys.pressed(bindings.crouch) {
        actions |= player_action::CROUCH;
    }
    if keys.pressed(bindings.sprint) {
        actions |= player_action::SPRINT;
    }
    if keys.pressed(bindings.interact) {
        actions |= player_action::INTERACT;
    }
    if keys.pressed(bindings.use_item) {
        actions |= player_action::USE_ITEM;
    }
    if keys.pressed(bindings.aim) {
        actions |= player_action::ADS;
    }
    if keys.pressed(bindings.weapon_1) {
        actions |= player_action::WEAPON_SLOT_1;
    }
    if keys.pressed(bindings.weapon_2) {
        actions |= player_action::WEAPON_SLOT_2;
    }
    if keys.pressed(bindings.weapon_3) {
        actions |= player_action::WEAPON_SLOT_3;
    }
    if keys.pressed(bindings.weapon_4) {
        actions |= player_action::WEAPON_SLOT_4;
    }

    let physics_pos = local_client_id
        .as_ref()
        .and_then(|lid| {
            predicted_players
                .iter()
                .find(|(marker, _)| marker.client_id == lid.0)
                .map(|(_, transform)| transform.translation)
        })
        .unwrap_or(Vec3::ZERO);

    let input = PlayerInput {
        move_dir: [world_move_x, world_move_z],
        look: [look.yaw.to_degrees(), 0.0],
        actions,
        client_tick: *tick,
        position: [physics_pos.x, physics_pos.y, physics_pos.z],
    };

    debug!(
        "[movement] input sent: wish_dir=({:.3},{:.3}) sprint={} crouch={} jump={}",
        world_move_x,
        world_move_z,
        actions & player_action::SPRINT != 0,
        actions & player_action::CROUCH != 0,
        actions & player_action::JUMP != 0,
    );

    for mut sender in senders.iter_mut() {
        let _ = sender.send::<InputChannel>(input.clone());
    }
}

pub(super) fn publish_input_state_to_lua(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    local_bus: Res<LocalEventBus>,
) {
    let bindings = &cfg.0.input.keys;
    let input = raw_move_input(bindings, &keys);

    let payload = serde_json::to_vec(&serde_json::json!({
        "move": {
            "x": input.x,
            "y": input.y,
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
// Ground contact helpers
// ---------------------------------------------------------------------------

pub(super) fn has_ground_contact(shape_hits: Option<&ShapeHits>) -> bool {
    has_ground_contact_with_state(shape_hits, false)
}

pub(super) fn has_ground_contact_with_state(
    shape_hits: Option<&ShapeHits>,
    was_grounded: bool,
) -> bool {
    has_ground_contact_with_thresholds(
        shape_hits,
        was_grounded,
        GROUND_CONTACT_DIST_ENTER,
        GROUND_CONTACT_DIST_EXIT,
    )
}

pub(super) fn has_ground_contact_with_thresholds(
    shape_hits: Option<&ShapeHits>,
    was_grounded: bool,
    enter_dist: f32,
    exit_dist: f32,
) -> bool {
    let dist_threshold = if was_grounded { exit_dist } else { enter_dist };
    shape_hits
        .map(|hits| {
            hits.iter().any(|hit| {
                ((-hit.normal2).dot(Vec3::Y) > GROUND_NORMAL_Y_MIN)
                    && (hit.distance <= dist_threshold)
            })
        })
        .unwrap_or(false)
}

pub(super) fn get_ground_normal(shape_hits: Option<&ShapeHits>) -> Vec3 {
    let Some(hits) = shape_hits else {
        return Vec3::Y;
    };
    hits.iter()
        .map(|hit| -hit.normal2)
        .filter(|normal| normal.y > GROUND_NORMAL_Y_MIN)
        .max_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
        .map(|normal| normal.normalize_or(Vec3::Y))
        .unwrap_or(Vec3::Y)
}

// ---------------------------------------------------------------------------
// Input helpers
// ---------------------------------------------------------------------------

fn raw_move_input(bindings: &Keybindings, keys: &ButtonInput<KeyCode>) -> Vec2 {
    let mut input_x = 0.0_f32;
    let mut input_y = 0.0_f32;
    if keys.pressed(bindings.move_forward) {
        input_y += 1.0;
    }
    if keys.pressed(bindings.move_backward) {
        input_y -= 1.0;
    }
    if keys.pressed(bindings.move_right) {
        input_x -= 1.0;
    }
    if keys.pressed(bindings.move_left) {
        input_x += 1.0;
    }
    Vec2::new(input_x, input_y)
}

fn normalized_move_input(bindings: &Keybindings, keys: &ButtonInput<KeyCode>) -> Vec2 {
    raw_move_input(bindings, keys).normalize_or_zero()
}

fn rotate_input_by_yaw(input: Vec2, yaw: f32) -> (f32, f32) {
    let sin_yaw = yaw.sin();
    let cos_yaw = yaw.cos();
    let world_x = input.x * cos_yaw + input.y * sin_yaw;
    let world_z = input.y * cos_yaw - input.x * sin_yaw;
    (world_x, world_z)
}
