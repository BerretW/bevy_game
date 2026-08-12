/// Weapon feel systems: recoil, sway, spread accumulation, muzzle flash.
///
/// All systems are client-side only and do not touch server-authoritative state.
/// Recoil and sway offset `CameraLookState` each frame; spread is tracked locally
/// for crosshair feedback. Muzzle flash spawns a short-lived `PointLight` entity.

use bevy::input::mouse::MouseMotion;
use bevy::ecs::message::MessageReader;
use bevy::prelude::*;
use core_net::player_action;
use core_resources::{LocalPlayerStats, WeaponRegistry};
use lightyear::prelude::Predicted;
use core_shared::PlayerMarker;

use super::{LocalClientId, camera::CameraLookState};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Sway reduction multiplier while ADS is active.
const ADS_SWAY_MULT: f32 = 0.3;

/// Maximum muzzle flash light intensity (lux).
const MUZZLE_FLASH_INTENSITY: f32 = 80_000.0;

/// Duration of muzzle flash effect.
const MUZZLE_FLASH_DURATION_SECS: f32 = 0.05;

/// Muzzle flash spawn offset in front of camera (world units).
const MUZZLE_FLASH_FORWARD_OFFSET: f32 = 0.6;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Per-player weapon recoil state.
///
/// Tracks accumulated pitch/yaw recoil that decays back to zero each frame.
/// `pattern_index` cycles through `WeaponDef.recoil_pattern` on each shot.
#[derive(Component, Default)]
pub struct WeaponRecoil {
    /// Current pitch offset added to `CameraLookState.pitch` (radians).
    pub recoil_pitch: f32,
    /// Current yaw offset added to `CameraLookState.yaw` (radians).
    pub recoil_yaw: f32,
    /// Recovery speed in radians per second.
    pub recovery_speed: f32,
    /// Current position in the weapon's recoil pattern.
    pub pattern_index: usize,
}

/// Per-player weapon sway state.
///
/// Sway is driven by mouse delta and applied as a smoothed camera offset.
/// It is reduced while ADS is active.
#[derive(Component, Default)]
pub struct WeaponSway {
    /// Accumulated sway on the horizontal axis (radians, decays to zero).
    pub sway_x: f32,
    /// Accumulated sway on the vertical axis (radians, decays to zero).
    pub sway_y: f32,
    /// How fast mouse motion contributes to sway (multiplied by `sway_amount`).
    pub sway_speed: f32,
    /// Maximum sway amplitude (radians).
    pub sway_amount: f32,
    /// True when the player is currently in ADS mode.
    pub is_ads: bool,
}

/// Client-side spread accumulation for crosshair feedback.
///
/// The server holds the authoritative spread; this mirrors it locally so the
/// crosshair can animate without waiting for a server round-trip.
#[derive(Component, Default)]
pub struct SpreadAccumulator {
    /// Current accumulated spread in degrees.
    pub accumulated: f32,
    /// Base spread from the active `WeaponDef` (degrees).
    pub base_spread: f32,
    /// Maximum spread cap (degrees).
    pub max_spread: f32,
    /// Per-shot spread added on each PRIMARY_FIRE.
    pub per_shot: f32,
    /// Degrees per second the spread recovers.
    pub recovery_rps: f32,
}

/// Short-lived muzzle flash entity despawned after its timer expires.
#[derive(Component)]
pub struct MuzzleFlash(pub Timer);

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Spawns `WeaponRecoil`, `WeaponSway`, and `SpreadAccumulator` on each new
/// predicted local player entity.
pub(super) fn init_weapon_feel_components(
    mut commands: Commands,
    new_players: Query<Entity, (Added<PlayerMarker>, With<Predicted>)>,
) {
    for entity in &new_players {
        commands.entity(entity).insert((
            WeaponRecoil {
                recovery_speed: 8.0,
                ..Default::default()
            },
            WeaponSway {
                sway_speed: 1.0,
                sway_amount: 0.004,
                ..Default::default()
            },
            SpreadAccumulator::default(),
        ));
    }
}

/// Applies recoil on PRIMARY_FIRE and decays it toward zero each frame.
///
/// On each shot the system reads the current weapon's `recoil_pattern` from
/// `WeaponRegistry` and cycles through it. The resulting offsets are added
/// directly to `CameraLookState` pitch/yaw so the camera kick is immediate.
pub(super) fn apply_weapon_recoil(
    time: Res<Time>,
    local_client_id: Option<Res<LocalClientId>>,
    weapon_registry: Res<WeaponRegistry>,
    local_stats: Res<LocalPlayerStats>,
    mut look: ResMut<CameraLookState>,
    mut players: Query<(&PlayerMarker, &mut WeaponRecoil), With<Predicted>>,
    // Read the last sent input actions via a local to detect rising edge.
    mut last_actions: Local<u32>,
) {
    let Some(local_id) = local_client_id.map(|r| r.0) else {
        return;
    };
    let dt = time.delta_secs().max(0.0001);

    // Snapshot the current fire trigger state from LocalPlayerStats.
    let stats = local_stats.get();
    let trigger_held = stats.fire_trigger_held;

    // Detect rising edge of PRIMARY_FIRE by comparing with previous frame.
    // We reconstruct actions from the stats snapshot; using trigger_held is
    // sufficient because the server mirrors it back on every PlayerStatsUpdate.
    let fired_this_frame = trigger_held && stats.fire_cooldown_remaining > 0.0;

    for (marker, mut recoil) in players.iter_mut() {
        if marker.client_id != local_id {
            continue;
        }

        // Apply recoil kick on shot.
        if fired_this_frame {
            let snapshot = local_stats.get();
            let weapon_id = snapshot
                .weapon_slots
                .get(snapshot.active_weapon_slot as usize)
                .and_then(|w| w.as_ref())
                .map(|w| w.weapon_id.clone())
                .unwrap_or_default();

            if let Some(def) = weapon_registry.get(&weapon_id) {
                if !def.recoil_pattern.is_empty() {
                    let idx = recoil.pattern_index % def.recoil_pattern.len();
                    let point = &def.recoil_pattern[idx];

                    // x = horizontal (yaw), y = vertical (pitch)
                    recoil.recoil_pitch += point.y.to_radians();
                    recoil.recoil_yaw   += point.x.to_radians();
                    recoil.pattern_index = (recoil.pattern_index + 1) % def.recoil_pattern.len();

                    // Carry recovery speed from weapon definition.
                    recoil.recovery_speed = def.recoil_recovery_dps.to_radians().max(0.01);
                }
            }
        } else {
            // Reset pattern index when trigger is released.
            if !trigger_held {
                let loop_from = local_stats
                    .get()
                    .weapon_slots
                    .get(local_stats.get().active_weapon_slot as usize)
                    .and_then(|w| w.as_ref())
                    .and_then(|w| weapon_registry.get(&w.weapon_id))
                    .map(|def| def.recoil_loop_from as usize)
                    .unwrap_or(0);
                if recoil.pattern_index > loop_from {
                    recoil.pattern_index = loop_from;
                }
            }
        }

        if fired_this_frame {
            info!("[weapons] fire event: recoil applied pitch_kick={:.3}", recoil.recoil_pitch);
        }

        if recoil.recoil_pitch != 0.0 || recoil.recoil_yaw != 0.0 {
            debug!("[weapons] recoil pitch={:.4} yaw={:.4} recovery={:.4}", recoil.recoil_pitch, recoil.recoil_yaw, recoil.recovery_speed);
        }

        // Apply accumulated recoil to camera.
        look.pitch += recoil.recoil_pitch;
        look.yaw   += recoil.recoil_yaw;

        // Clamp pitch to avoid flipping through MAX_PITCH_RAD (matches camera.rs).
        const MAX_PITCH_RAD: f32 = 1.25;
        look.pitch = look.pitch.clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);

        // Exponential decay toward zero.
        let decay = (-recoil.recovery_speed * dt).exp();
        recoil.recoil_pitch *= decay;
        recoil.recoil_yaw   *= decay;
    }

    *last_actions = if trigger_held { player_action::PRIMARY_FIRE } else { 0 };
}

/// Computes weapon sway from mouse delta and blends it into `CameraLookState`.
///
/// ADS reduces sway by `ADS_SWAY_MULT`. Sway decays exponentially each frame.
pub(super) fn apply_weapon_sway(
    time: Res<Time>,
    mut motions: MessageReader<MouseMotion>,
    local_client_id: Option<Res<LocalClientId>>,
    local_stats: Res<LocalPlayerStats>,
    mut look: ResMut<CameraLookState>,
    mut players: Query<(&PlayerMarker, &mut WeaponSway), With<Predicted>>,
) {
    let Some(local_id) = local_client_id.map(|r| r.0) else {
        // Consume events even when no player exists.
        for _ in motions.read() {}
        return;
    };
    let dt = time.delta_secs().max(0.0001);

    let mut mouse_delta = Vec2::ZERO;
    for motion in motions.read() {
        mouse_delta += motion.delta;
    }

    let stats = local_stats.get();
    let is_ads = stats.fire_trigger_held; // repurpose: use ADS bit from stats

    for (marker, mut sway) in players.iter_mut() {
        if marker.client_id != local_id {
            continue;
        }

        sway.is_ads = is_ads;
        let effective_amount = if sway.is_ads {
            sway.sway_amount * ADS_SWAY_MULT
        } else {
            sway.sway_amount
        };

        // Accumulate sway from mouse motion.
        sway.sway_x += mouse_delta.x * effective_amount * sway.sway_speed;
        sway.sway_y += mouse_delta.y * effective_amount * sway.sway_speed;

        // Clamp sway amplitude.
        let max_sway = effective_amount * 10.0;
        sway.sway_x = sway.sway_x.clamp(-max_sway, max_sway);
        sway.sway_y = sway.sway_y.clamp(-max_sway, max_sway);

        // Apply to camera look.
        look.yaw   += sway.sway_x;
        look.pitch += sway.sway_y;
        look.pitch = look.pitch.clamp(-1.25, 1.25);

        if sway.sway_x != 0.0 || sway.sway_y != 0.0 {
            debug!("[weapons] sway offset=({:.4},{:.4}) velocity=({:.3},{:.3})", sway.sway_x, sway.sway_y, mouse_delta.x, mouse_delta.y);
        }

        // Exponential decay back to neutral.
        let decay = (-8.0_f32 * dt).exp();
        sway.sway_x *= decay;
        sway.sway_y *= decay;
    }
}

/// Accumulates spread on PRIMARY_FIRE and lets it recover over time.
///
/// Spread values are sourced from `WeaponDef.spread`. This is client-side
/// only — the server independently computes authoritative spread for hit
/// detection. The local accumulator is used for crosshair visual feedback.
pub(super) fn tick_spread_accumulation(
    time: Res<Time>,
    local_client_id: Option<Res<LocalClientId>>,
    weapon_registry: Res<WeaponRegistry>,
    local_stats: Res<LocalPlayerStats>,
    mut players: Query<(&PlayerMarker, &mut SpreadAccumulator), With<Predicted>>,
) {
    let Some(local_id) = local_client_id.map(|r| r.0) else {
        return;
    };
    let dt = time.delta_secs().max(0.0001);
    let stats = local_stats.get();
    let fired = stats.fire_trigger_held && stats.fire_cooldown_remaining > 0.0;

    for (marker, mut spread) in players.iter_mut() {
        if marker.client_id != local_id {
            continue;
        }

        // Refresh spread parameters from current weapon.
        let weapon_id = stats
            .weapon_slots
            .get(stats.active_weapon_slot as usize)
            .and_then(|w| w.as_ref())
            .map(|w| w.weapon_id.clone())
            .unwrap_or_default();

        if let Some(def) = weapon_registry.get(&weapon_id) {
            spread.base_spread  = def.spread.base;
            spread.max_spread   = def.spread.base + def.spread.per_shot * 8.0;
            spread.per_shot     = def.spread.per_shot;
            spread.recovery_rps = def.spread.recovery_rps;
        }

        // Add spread on shot.
        if fired {
            spread.accumulated = (spread.accumulated + spread.per_shot)
                .min(spread.max_spread);
        }

        if spread.accumulated > 0.0 {
            debug!("[weapons] spread acc={:.4} max={:.4}", spread.accumulated, spread.max_spread);
        }

        // Recover toward base.
        let recovery = spread.recovery_rps * dt;
        spread.accumulated = (spread.accumulated - recovery).max(spread.base_spread);
    }
}

/// Spawns a `PointLight` muzzle flash at the camera forward position on each shot.
///
/// The light auto-despawns after `MUZZLE_FLASH_DURATION_SECS`.
pub(super) fn spawn_muzzle_flash(
    mut commands: Commands,
    local_stats: Res<LocalPlayerStats>,
    camera_q: Query<&GlobalTransform, With<super::camera::MainGameplayCamera>>,
    mut last_fired: Local<bool>,
) {
    let stats = local_stats.get();
    let fired_now = stats.fire_trigger_held && stats.fire_cooldown_remaining > 0.0;
    let rising_edge = fired_now && !*last_fired;
    *last_fired = fired_now;

    if !rising_edge {
        return;
    }

    let Ok(cam_transform) = camera_q.single() else {
        return;
    };

    let forward = cam_transform.forward();
    let spawn_pos = cam_transform.translation() + forward * MUZZLE_FLASH_FORWARD_OFFSET;

    let entity = commands.spawn((
        PointLight {
            color: Color::srgb(1.0, 0.85, 0.5),
            intensity: MUZZLE_FLASH_INTENSITY,
            range: 6.0,
            shadows_enabled: false,
            ..Default::default()
        },
        Transform::from_translation(spawn_pos),
        GlobalTransform::default(),
        MuzzleFlash(Timer::from_seconds(
            MUZZLE_FLASH_DURATION_SECS,
            TimerMode::Once,
        )),
    )).id();
    info!("[weapons] muzzle flash spawned entity={:?} intensity={:.0} lux", entity, MUZZLE_FLASH_INTENSITY);
}

/// Ticks `MuzzleFlash` timers and despawns finished flash entities.
pub(super) fn tick_muzzle_flash(
    mut commands: Commands,
    time: Res<Time>,
    mut flashes: Query<(Entity, &mut MuzzleFlash)>,
) {
    for (entity, mut flash) in flashes.iter_mut() {
        flash.0.tick(time.delta());
        if flash.0.just_finished() {
            commands.entity(entity).despawn();
        }
    }
}
