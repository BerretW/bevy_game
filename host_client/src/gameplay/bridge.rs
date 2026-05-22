use std::collections::HashSet;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use core_resources::{ConnectionInfo, GameBridges, InputSnapshot};

use super::LocalClientId;
use crate::AppState;

fn keycode_name(key: &KeyCode) -> String {
    match key {
        KeyCode::KeyA => "a",
        KeyCode::KeyB => "b",
        KeyCode::KeyC => "c",
        KeyCode::KeyD => "d",
        KeyCode::KeyE => "e",
        KeyCode::KeyF => "f",
        KeyCode::KeyG => "g",
        KeyCode::KeyH => "h",
        KeyCode::KeyI => "i",
        KeyCode::KeyJ => "j",
        KeyCode::KeyK => "k",
        KeyCode::KeyL => "l",
        KeyCode::KeyM => "m",
        KeyCode::KeyN => "n",
        KeyCode::KeyO => "o",
        KeyCode::KeyP => "p",
        KeyCode::KeyQ => "q",
        KeyCode::KeyR => "r",
        KeyCode::KeyS => "s",
        KeyCode::KeyT => "t",
        KeyCode::KeyU => "u",
        KeyCode::KeyV => "v",
        KeyCode::KeyW => "w",
        KeyCode::KeyX => "x",
        KeyCode::KeyY => "y",
        KeyCode::KeyZ => "z",
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::Numpad0 => "num0",
        KeyCode::Numpad1 => "num1",
        KeyCode::Numpad2 => "num2",
        KeyCode::Numpad3 => "num3",
        KeyCode::Numpad4 => "num4",
        KeyCode::Numpad5 => "num5",
        KeyCode::Numpad6 => "num6",
        KeyCode::Numpad7 => "num7",
        KeyCode::Numpad8 => "num8",
        KeyCode::Numpad9 => "num9",
        KeyCode::Space => "space",
        KeyCode::Escape => "escape",
        KeyCode::Enter => "enter",
        KeyCode::NumpadEnter => "enter",
        KeyCode::Tab => "tab",
        KeyCode::Backspace => "backspace",
        KeyCode::Delete => "delete",
        KeyCode::Insert => "insert",
        KeyCode::Home => "home",
        KeyCode::End => "end",
        KeyCode::PageUp => "pageup",
        KeyCode::PageDown => "pagedown",
        KeyCode::ArrowUp => "up",
        KeyCode::ArrowDown => "down",
        KeyCode::ArrowLeft => "left",
        KeyCode::ArrowRight => "right",
        KeyCode::ShiftLeft => "lshift",
        KeyCode::ShiftRight => "rshift",
        KeyCode::ControlLeft => "lctrl",
        KeyCode::ControlRight => "rctrl",
        KeyCode::AltLeft => "lalt",
        KeyCode::AltRight => "ralt",
        KeyCode::SuperLeft | KeyCode::SuperRight => "super",
        KeyCode::CapsLock => "capslock",
        KeyCode::F1 => "f1",
        KeyCode::F2 => "f2",
        KeyCode::F3 => "f3",
        KeyCode::F4 => "f4",
        KeyCode::F5 => "f5",
        KeyCode::F6 => "f6",
        KeyCode::F7 => "f7",
        KeyCode::F8 => "f8",
        KeyCode::F9 => "f9",
        KeyCode::F10 => "f10",
        KeyCode::F11 => "f11",
        KeyCode::F12 => "f12",
        _ => return format!("{:?}", key).to_lowercase(),
    }
    .to_string()
}

fn mousebutton_name(button: &MouseButton) -> String {
    match button {
        MouseButton::Left => "left".to_string(),
        MouseButton::Right => "right".to_string(),
        MouseButton::Middle => "middle".to_string(),
        MouseButton::Other(value) => format!("mouse{value}"),
        _ => format!("{:?}", button).to_lowercase(),
    }
}

pub(super) fn update_input_bridge(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    bridges: Res<GameBridges>,
) {
    let (cursor_x, cursor_y) = window_q
        .single()
        .ok()
        .and_then(|window: &Window| {
            window
                .cursor_position()
                .map(|pos| (pos.x / window.width(), pos.y / window.height()))
        })
        .unwrap_or((0.0, 0.0));

    let snap = InputSnapshot {
        pressed: keys.get_pressed().map(keycode_name).collect::<HashSet<_>>(),
        just_pressed: keys
            .get_just_pressed()
            .map(keycode_name)
            .collect::<HashSet<_>>(),
        just_released: keys
            .get_just_released()
            .map(keycode_name)
            .collect::<HashSet<_>>(),
        mouse_pressed: mouse
            .get_pressed()
            .map(mousebutton_name)
            .collect::<HashSet<_>>(),
        mouse_just_pressed: mouse
            .get_just_pressed()
            .map(mousebutton_name)
            .collect::<HashSet<_>>(),
        mouse_just_released: mouse
            .get_just_released()
            .map(mousebutton_name)
            .collect::<HashSet<_>>(),
        cursor_x,
        cursor_y,
    };
    debug!(
        "[bridge] input_bridge: cursor=({:.3},{:.3}) pressed={} just_pressed={} mouse_pressed={}",
        snap.cursor_x,
        snap.cursor_y,
        snap.pressed.len(),
        snap.just_pressed.len(),
        snap.mouse_pressed.len(),
    );
    bridges.input.update(snap);
}

pub(super) fn update_connection_bridge(
    cfg: Res<core_net::ClientNetConfig>,
    local_client: Option<Res<LocalClientId>>,
    bridges: Res<GameBridges>,
) {
    let info = ConnectionInfo {
        connected: true,
        server_addr: cfg.server.to_string(),
        ping_ms: 0,
        client_id: local_client.as_deref().map_or(0, |client| client.0),
    };
    debug!(
        "[bridge] connection_bridge: connected={} peer_id={:?} latency={}ms",
        info.connected, info.client_id, info.ping_ms,
    );
    bridges.connection.set(info);
}

pub(super) fn reset_connection_bridge(bridges: Res<GameBridges>) {
    info!("[bridge] connection_bridge RESET");
    bridges.connection.set_disconnected();
}

pub(super) fn reset_engine_state(bridges: Res<GameBridges>) {
    bridges.engine.reset();
}

pub(super) fn handle_engine_cmds(
    bridges: Res<GameBridges>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let mut count = 0;
    if bridges.engine.take_disconnect() {
        info!("[bridge] engine_cmd: disconnect");
        count += 1;
        next_state.set(AppState::Lobby);
    }
    if bridges.engine.take_quit() {
        info!("[bridge] engine_cmd: quit");
        count += 1;
        std::process::exit(0);
    }
    debug!("[bridge] engine_cmds processed this frame: {}", count);
}
