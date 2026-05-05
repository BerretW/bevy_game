//! Phase 3 — klientská gameplay vrstva.
//!
//! * **Render replikovaných hráčů**: každá entita s `PlayerMarker` dostane
//!   `Sprite` (barva podle `client_id`) a Bevy `Transform`, které se každý
//!   frame synchronizuje s replikovaným `NetTransform`.
//! * **Input collection**: každý `FixedUpdate` se sebere stav klávesnice
//!   podle `[input.keys]` z `client.toml` a pošle se na server přes
//!   lightyear `MessageSender<PlayerInput>` na `InputChannel`.
//!
//! Phase 3 step 8 sem přidá client-side prediction (lokální `NetVelocity`
//! integraci) a step 9–10 Action/Intent + lag compensation.

use bevy::prelude::*;
use core_net::{player_action, InputChannel, PlayerInput};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;

use crate::config::ClientConfigResource;

/// Pixely na 1 jednotku world-space. Default 50 px = 1 m → hráč 50×50 px.
const WORLD_TO_PIXELS: f32 = 50.0;

pub struct ClientGameplayPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_camera);
        app.add_systems(
            Update,
            (attach_sprite_to_new_players, sync_net_transform_to_render),
        );
        app.add_systems(FixedUpdate, collect_and_send_input);
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
    info!("[gameplay/client] 2D camera ready");
}

/// Když dorazí replikovaná entita s `PlayerMarker` a `NetTransform`, přibalíme
/// Sprite + Transform pro vizualizaci. `Without<Sprite>` filtr zaručuje
/// idempotenci — atributy přidáme jen jednou.
fn attach_sprite_to_new_players(
    mut commands: Commands,
    new_players: Query<
        (Entity, &PlayerMarker),
        (With<NetTransform>, Without<Sprite>),
    >,
) {
    for (entity, marker) in new_players.iter() {
        // Barva odvozená z client_id, ať jdou hráči vizuálně rozlišit.
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
            "[gameplay/client] sprite attached to player entity {:?} (client_id={})",
            entity, marker.client_id
        );
    }
}

/// Mapuje 3D world-space `NetTransform` na 2D Bevy `Transform`. Pro Phase 3
/// jednoduché top-down (XZ ground plane → screen XY).
fn sync_net_transform_to_render(
    mut q: Query<(&NetTransform, &mut Transform), Changed<NetTransform>>,
) {
    for (net, mut local) in q.iter_mut() {
        local.translation.x = net.translation.x * WORLD_TO_PIXELS;
        local.translation.y = net.translation.z * WORLD_TO_PIXELS;
    }
}

/// Sebere klávesy z `[input.keys]` configu a pošle `PlayerInput` na server.
/// Volá se v `FixedUpdate`, aby tick rate odpovídal serverové simulaci.
fn collect_and_send_input(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut senders: Query<&mut MessageSender<PlayerInput>>,
    mut tick: Local<u32>,
) {
    *tick = tick.wrapping_add(1);

    let bindings = &cfg.0.input.keys;
    let mouse_b = &cfg.0.input.mouse;

    let mut move_x = 0.0_f32;
    let mut move_y = 0.0_f32;
    if keys.pressed(bindings.move_forward) {
        move_y += 1.0;
    }
    if keys.pressed(bindings.move_backward) {
        move_y -= 1.0;
    }
    if keys.pressed(bindings.move_right) {
        move_x += 1.0;
    }
    if keys.pressed(bindings.move_left) {
        move_x -= 1.0;
    }

    // Normalizace, aby diagonální pohyb nebyl rychlejší.
    let mag2 = move_x * move_x + move_y * move_y;
    if mag2 > 1.0 {
        let inv = mag2.sqrt().recip();
        move_x *= inv;
        move_y *= inv;
    }

    // Action bitfield z keybindings + mouse.
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

    let input = PlayerInput {
        move_dir: [move_x, move_y],
        look: [0.0, 0.0],
        actions,
        client_tick: *tick,
    };

    // Posíláme na všech sender komponentách (klientská strana má jediný
    // sender, ale Query nás chrání před edge-case).
    for mut sender in senders.iter_mut() {
        let _ = sender.send::<InputChannel>(input.clone());
    }
}
