//! Phase 3 — klientska gameplay vrstva.
//!
//! Phase 3.7: `RaycastBridge` se aktualizuje kazdy frame z pozice mysi.
//! Lua sandbox cte pres `Raycast.GetGroundPosition()`.

use bevy::prelude::*;
use core_net::{player_action, InputChannel, PlayerInput};
use core_resources::{LocalEventBus, LocalObjectMarker, RaycastBridge};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::Predicted;

use crate::config::ClientConfigResource;

const WORLD_TO_PIXELS: f32 = 50.0;

pub struct ClientGameplayPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_camera);
        app.add_systems(
            Update,
            (
                update_raycast_bridge,
                publish_input_state_to_lua,
                attach_sprite_to_new_players,
                attach_sprite_to_local_objects,
                sync_net_transform_to_render,
            )
                .chain(),
        );
        app.add_systems(FixedUpdate, collect_and_send_input);
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
    info!("[gameplay/client] 2D camera ready");
}

/// Aktualizuje `RaycastBridge` podle aktualni pozice mysi.
/// Tato pozice se pouziva v Lua `Raycast.GetGroundPosition()`.
fn update_raycast_bridge(
    windows: Query<&Window>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    raycast: Res<RaycastBridge>,
) {
    let Ok(window) = windows.single() else { return };
    let Ok((camera, cam_transform)) = camera_q.single() else { return };

    if let Some(cursor) = window.cursor_position() {
        let cursor: Vec2 = cursor;
        if let Ok(world_2d) = camera.viewport_to_world_2d(cam_transform, cursor) {
            // Top-down 2D: screen X->world X, screen Y->world Z
            raycast.set_pos([
                world_2d.x / WORLD_TO_PIXELS,
                0.0,
                world_2d.y / WORLD_TO_PIXELS,
            ]);
        }
    }
}

fn attach_sprite_to_new_players(
    mut commands: Commands,
    new_players: Query<
        (Entity, &PlayerMarker),
        (With<NetTransform>, With<Predicted>, Without<Sprite>),
    >,
) {
    for (entity, marker) in new_players.iter() {
        let hue = (marker.client_id as f32 * 47.0).rem_euclid(360.0);
        let color = Color::hsl(hue, 0.7, 0.6);
        commands.entity(entity).insert((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(WORLD_TO_PIXELS)),
                ..default()
            },
            Transform::default(),
        ));
        info!(
            "[gameplay/client] sprite attached to player {:?} (client_id={})",
            entity, marker.client_id
        );
    }
}

fn sync_net_transform_to_render(mut q: Query<(&NetTransform, &mut Transform, &Predicted)>) {
    for (net, mut local, _) in q.iter_mut() {
        local.translation.x = net.translation.x * WORLD_TO_PIXELS;
        local.translation.y = net.translation.z * WORLD_TO_PIXELS;
    }
}

fn collect_and_send_input(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    mouse: Res<ButtonInput<MouseButton>>,
    raycast: Res<RaycastBridge>,
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
    if keys.pressed(bindings.move_right) { move_x += 1.0; }
    if keys.pressed(bindings.move_left) { move_x -= 1.0; }

    let mag2 = move_x * move_x + move_y * move_y;
    if mag2 > 1.0 {
        let inv = mag2.sqrt().recip();
        move_x *= inv;
        move_y *= inv;
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

    // Phase 3.7: look yaw = uhel mysi od severu (atan2 v rovine XZ)
    let p = raycast.get_pos();
    let yaw = p[0].atan2(p[2]).to_degrees();

    let input = PlayerInput {
        move_dir: [move_x, move_y],
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
/// Bez Sprite by byly neviditelne.
fn attach_sprite_to_local_objects(
    mut commands: Commands,
    new_objs: Query<Entity, (With<LocalObjectMarker>, Without<Sprite>)>,
) {
    for entity in new_objs.iter() {
        commands.entity(entity).insert(Sprite {
            color: Color::srgb(0.2, 0.85, 0.3),
            custom_size: Some(Vec2::splat(WORLD_TO_PIXELS)),
            ..default()
        });
    }
}
