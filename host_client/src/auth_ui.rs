//! Nativní login / register UI panel — zobrazí se před stažením resources.
//!
//! Spustí se automaticky jakmile `ClientHandshakeState.status == AwaitingAuth`.
//! Hráč zadá uživatelské jméno a heslo; po úspěchu handshake pokračuje
//! stažením resources a spuštěním Lua sandboxů.

use bevy::ecs::message::MessageReader;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

use core_net::{ClientHandshakeState, HandshakeStatus};
use core_resources::{GameBridges, PendingAuthCredentials};

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct AuthUiPlugin;

impl Plugin for AuthUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AuthUiState>().add_systems(
            Update,
            (
                manage_auth_ui,
                handle_keyboard_input,
                handle_button_clicks,
                sync_field_text,
                poll_auth_result,
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub struct AuthUiState {
    username:   String,
    password:   String,
    focused:    FocusedField,
    status_msg: String,
    status_ok:  bool,
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum FocusedField {
    #[default]
    Username,
    Password,
}

// ---------------------------------------------------------------------------
// Marker components
// ---------------------------------------------------------------------------

#[derive(Component)] struct AuthUiRoot;
#[derive(Component)] struct UsernameField;
#[derive(Component)] struct PasswordField;
#[derive(Component)] struct AuthStatusText;

#[derive(Component, Clone, Copy)]
enum AuthBtn {
    Login,
    Register,
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

const OVERLAY_BG:  Color = Color::srgba(0.0, 0.0, 0.0, 0.72);
const PANEL_BG:    Color = Color::srgb(0.10, 0.10, 0.15);
const FIELD_FOCUS: Color = Color::srgb(0.18, 0.18, 0.28);
const FIELD_IDLE:  Color = Color::srgb(0.08, 0.08, 0.12);
const BTN_LOGIN:   Color = Color::srgb(0.13, 0.42, 0.78);
const BTN_REG:     Color = Color::srgb(0.18, 0.50, 0.24);
const TEXT_MAIN:   Color = Color::srgb(0.92, 0.92, 0.92);
const TEXT_DIM:    Color = Color::srgb(0.52, 0.52, 0.62);
const TEXT_ERR:    Color = Color::srgb(1.00, 0.38, 0.38);
const TEXT_OK:     Color = Color::srgb(0.28, 0.85, 0.44);

// ---------------------------------------------------------------------------
// UI management — spawn / despawn
// ---------------------------------------------------------------------------

fn manage_auth_ui(
    mut commands: Commands,
    handshake:    Res<ClientHandshakeState>,
    roots:        Query<Entity, With<AuthUiRoot>>,
    mut ui_state: ResMut<AuthUiState>,
) {
    let need_ui = handshake.status == HandshakeStatus::AwaitingAuth;
    let has_ui  = !roots.is_empty();

    if need_ui && !has_ui {
        *ui_state = AuthUiState::default();
        spawn_auth_ui(&mut commands);
    } else if !need_ui && has_ui {
        for e in roots.iter() {
            commands.entity(e).despawn();
        }
    }
}

fn spawn_auth_ui(commands: &mut Commands) {
    // Full-screen overlay
    let root = commands.spawn((
        AuthUiRoot,
        Node {
            width:           Val::Percent(100.0),
            height:          Val::Percent(100.0),
            align_items:     AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(OVERLAY_BG),
        GlobalZIndex(100),
    )).id();

    // Center panel
    let panel = commands.spawn((
        Node {
            width:           Val::Px(360.0),
            flex_direction:  FlexDirection::Column,
            align_items:     AlignItems::Stretch,
            padding:         UiRect::all(Val::Px(28.0)),
            row_gap:         Val::Px(10.0),
            ..default()
        },
        BackgroundColor(PANEL_BG),
    )).id();
    commands.entity(root).add_child(panel);

    // Title
    spawn_text(commands, panel, "Server Login", 22.0, TEXT_MAIN,
               UiRect::bottom(Val::Px(8.0)));

    // Username
    spawn_text(commands, panel, "Username", 12.0, TEXT_DIM, UiRect::default());
    let u_field = commands.spawn((
        UsernameField,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(FIELD_FOCUS),
    )).id();
    commands.entity(panel).add_child(u_field);
    let u_text = commands.spawn((
        Text::new("_"),
        TextFont { font_size: 15.0, ..default() },
        TextColor(TEXT_MAIN),
    )).id();
    commands.entity(u_field).add_child(u_text);

    // Password
    spawn_text(commands, panel, "Password", 12.0, TEXT_DIM, UiRect::default());
    let p_field = commands.spawn((
        PasswordField,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(FIELD_IDLE),
    )).id();
    commands.entity(panel).add_child(p_field);
    let p_text = commands.spawn((
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(TEXT_MAIN),
    )).id();
    commands.entity(p_field).add_child(p_text);

    // Hint
    spawn_text(commands, panel,
               "Tab — switch field     Enter — login",
               11.0, TEXT_DIM, UiRect::default());

    // Buttons row
    let btn_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap:     Val::Px(10.0),
        margin:         UiRect::top(Val::Px(4.0)),
        ..default()
    }).id();
    commands.entity(panel).add_child(btn_row);

    spawn_btn(commands, btn_row, AuthBtn::Login,    "Login",    BTN_LOGIN, 100.0);
    spawn_btn(commands, btn_row, AuthBtn::Register, "Register", BTN_REG,  110.0);

    // Status
    let status = commands.spawn((
        AuthStatusText,
        Text::new(""),
        TextFont { font_size: 13.0, ..default() },
        TextColor(TEXT_DIM),
    )).id();
    commands.entity(panel).add_child(status);
}

fn spawn_text(commands: &mut Commands, parent: Entity, text: &str,
              size: f32, color: Color, margin: UiRect)
{
    let e = commands.spawn((
        Text::new(text),
        TextFont { font_size: size, ..default() },
        TextColor(color),
        Node { margin, ..default() },
    )).id();
    commands.entity(parent).add_child(e);
}

fn spawn_btn(commands: &mut Commands, parent: Entity,
             kind: AuthBtn, label: &str, color: Color, width: f32)
{
    let btn = commands.spawn((
        kind,
        Button,
        Node {
            width:           Val::Px(width),
            height:          Val::Px(32.0),
            justify_content: JustifyContent::Center,
            align_items:     AlignItems::Center,
            ..default()
        },
        BackgroundColor(color),
    )).id();
    let lbl = commands.spawn((
        Text::new(label),
        TextFont { font_size: 13.0, ..default() },
        TextColor(TEXT_MAIN),
    )).id();
    commands.entity(btn).add_child(lbl);
    commands.entity(parent).add_child(btn);
}

// ---------------------------------------------------------------------------
// Keyboard input
// ---------------------------------------------------------------------------

fn handle_keyboard_input(
    mut evs:      MessageReader<KeyboardInput>,
    mut ui_state: ResMut<AuthUiState>,
    handshake:    Res<ClientHandshakeState>,
    bridges:      Res<GameBridges>,
) {
    if handshake.status != HandshakeStatus::AwaitingAuth {
        evs.clear();
        return;
    }

    for ev in evs.read() {
        if ev.state != ButtonState::Pressed { continue; }

        match &ev.logical_key {
            Key::Character(ch) => {
                for c in ch.chars() {
                    if !c.is_control() {
                        match ui_state.focused {
                            FocusedField::Username => ui_state.username.push(c),
                            FocusedField::Password => ui_state.password.push(c),
                        }
                    }
                }
            }
            Key::Backspace => {
                let field = match ui_state.focused {
                    FocusedField::Username => &mut ui_state.username,
                    FocusedField::Password => &mut ui_state.password,
                };
                let mut chars = field.chars();
                chars.next_back();
                *field = chars.as_str().to_string();
            }
            Key::Tab => {
                ui_state.focused = match ui_state.focused {
                    FocusedField::Username => FocusedField::Password,
                    FocusedField::Password => FocusedField::Username,
                };
            }
            Key::Enter => {
                submit(&mut ui_state, &bridges, 0);
            }
            _ => {}
        }
    }
}

fn submit(ui: &mut AuthUiState, bridges: &GameBridges, action: u8) {
    let username = ui.username.trim().to_string();
    if username.is_empty() {
        ui.status_msg = "Username cannot be empty.".into();
        ui.status_ok  = false;
        return;
    }
    if ui.password.is_empty() {
        ui.status_msg = "Password cannot be empty.".into();
        ui.status_ok  = false;
        return;
    }
    let verb = if action == 0 { "login" } else { "register" };
    ui.status_msg = format!("Sending {} request…", verb);
    ui.status_ok  = true;
    bridges.auth.push_outgoing(PendingAuthCredentials {
        action,
        username,
        password: ui.password.clone(),
    });
}

// ---------------------------------------------------------------------------
// Button clicks
// ---------------------------------------------------------------------------

fn handle_button_clicks(
    q:            Query<(&Interaction, &AuthBtn), (Changed<Interaction>, With<Button>)>,
    mut ui_state: ResMut<AuthUiState>,
    bridges:      Res<GameBridges>,
    handshake:    Res<ClientHandshakeState>,
) {
    if handshake.status != HandshakeStatus::AwaitingAuth { return; }
    for (ia, btn) in q.iter() {
        if *ia == Interaction::Pressed {
            let action = match btn { AuthBtn::Login => 0u8, AuthBtn::Register => 1u8 };
            submit(&mut ui_state, &bridges, action);
        }
    }
}

// ---------------------------------------------------------------------------
// Text sync
// ---------------------------------------------------------------------------

fn sync_field_text(
    ui_state:  Res<AuthUiState>,
    u_field_q: Query<&Children, With<UsernameField>>,
    p_field_q: Query<&Children, With<PasswordField>>,
    status_q:  Query<Entity, With<AuthStatusText>>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
) {
    if !ui_state.is_changed() { return; }

    // Username
    for children in u_field_q.iter() {
        for child in children.iter() {
            if let Ok((mut text, _)) = texts.get_mut(child) {
                let cursor = if ui_state.focused == FocusedField::Username { "_" } else { "" };
                let display = if ui_state.username.is_empty() && ui_state.focused != FocusedField::Username {
                    format!("(empty){}", cursor)
                } else {
                    format!("{}{}", ui_state.username, cursor)
                };
                *text = Text::new(display);
            }
        }
    }

    // Password (mask with *)
    for children in p_field_q.iter() {
        for child in children.iter() {
            if let Ok((mut text, _)) = texts.get_mut(child) {
                let stars  = "*".repeat(ui_state.password.len());
                let cursor = if ui_state.focused == FocusedField::Password { "_" } else { "" };
                *text = Text::new(format!("{}{}", stars, cursor));
            }
        }
    }

    // Status
    for entity in status_q.iter() {
        if let Ok((mut text, mut color)) = texts.get_mut(entity) {
            *text  = Text::new(ui_state.status_msg.clone());
            color.0 = if ui_state.status_ok { TEXT_OK } else { TEXT_ERR };
        }
    }
}

// ---------------------------------------------------------------------------
// Poll bridge for error feedback
// ---------------------------------------------------------------------------

fn poll_auth_result(
    bridges:      Res<GameBridges>,
    mut ui_state: ResMut<AuthUiState>,
    handshake:    Res<ClientHandshakeState>,
) {
    if handshake.status != HandshakeStatus::AwaitingAuth { return; }
    if let Some(err) = bridges.auth.take_client_error() {
        ui_state.status_msg = err;
        ui_state.status_ok  = false;
    }
    // Success: bridge sets client_authenticated → manage_auth_ui despawns panel.
}
