//! Lobby — obrazovka před připojením na server.
//!
//! Zobrazí se ihned po startu aplikace. Umožňuje:
//! * ručně zadat IP adresu serveru,
//! * připojit se na server z oblíbených,
//! * přidat/odebrat oblíbené servery (persistováno do `client.toml`),
//! * zahájit LAN sken (zatím zástupný — server discovery není implementováno).
//!
//! Po kliknutí na "Connect" (nebo Enter ve vstupním poli) plugin:
//! 1. Ověří, že adresa je platný `SocketAddr`.
//! 2. Aktualizuje `ClientNetConfig.server`.
//! 3. Pošle `ConnectClient` zprávu — `ClientNetPlugin` spawne klientskou entitu.
//! 4. Přepne stav na `AppState::InGame`.

use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use core_net::{ClientNetConfig, ConnectClient};

use crate::{
    AppState,
    config::{ClientConfig, ClientConfigResource, ServerEntry},
};

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyState>()
            .add_systems(OnEnter(AppState::Lobby), (setup_lobby_ui, release_cursor))
            .add_systems(OnExit(AppState::Lobby), cleanup_lobby_ui)
            .add_systems(
                Update,
                (
                    handle_ip_field_click,
                    handle_keyboard_input,
                    handle_connect_btn,
                    handle_fav_connect_btn,
                    handle_add_fav_btn,
                    handle_remove_fav_btn,
                    handle_scan_lan_btn,
                    update_ip_display,
                    update_status_display,
                    update_button_colors,
                )
                    .run_if(in_state(AppState::Lobby)),
            );
    }
}

// ---------------------------------------------------------------------------
// State resource
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub struct LobbyState {
    ip_input:      String,
    input_focused: bool,
    status_msg:    String,
}

// ---------------------------------------------------------------------------
// Marker components
// ---------------------------------------------------------------------------

#[derive(Component)] struct LobbyRoot;
#[derive(Component)] struct IpFieldBtn;
#[derive(Component)] struct IpInputText;
#[derive(Component)] struct ConnectBtn;
#[derive(Component)] struct AddFavBtn;
#[derive(Component)] struct ScanLanBtn;
#[derive(Component)] struct FavConnectBtn(String);
#[derive(Component)] struct FavRemoveBtn(String);
#[derive(Component)] struct StatusText;

/// Barva tlačítka (normální / connect / danger).
/// Komponentou neseme info, aby `update_button_colors` mohl vrátit
/// správnou základní barvu po opuštění hover stavu.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Normal,
    Connect,
    Danger,
}

fn kind_color(k: ButtonKind) -> Color {
    match k {
        ButtonKind::Normal  => BTN_NORMAL,
        ButtonKind::Connect => BTN_CONNECT,
        ButtonKind::Danger  => BTN_DANGER,
    }
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

const SCREEN_BG:       Color = Color::srgb(0.05, 0.05, 0.08);
const PANEL_BG:        Color = Color::srgb(0.10, 0.10, 0.15);
const INPUT_BG:        Color = Color::srgb(0.14, 0.14, 0.22);
const BTN_NORMAL:      Color = Color::srgb(0.22, 0.22, 0.32);
const BTN_HOVER:       Color = Color::srgb(0.30, 0.30, 0.44);
const BTN_CONNECT:     Color = Color::srgb(0.13, 0.42, 0.78);
const BTN_CONNECT_HOV: Color = Color::srgb(0.18, 0.54, 0.92);
const BTN_DANGER:      Color = Color::srgb(0.55, 0.14, 0.14);
const BTN_DANGER_HOV:  Color = Color::srgb(0.70, 0.20, 0.20);
const BTN_PRESSED:     Color = Color::srgb(0.10, 0.65, 1.00);
const TEXT_PRIMARY:    Color = Color::srgb(0.92, 0.92, 0.95);
const TEXT_DIM:        Color = Color::srgb(0.52, 0.52, 0.60);
const TEXT_ACCENT:     Color = Color::srgb(0.38, 0.78, 1.00);
const TEXT_ERROR:      Color = Color::srgb(1.00, 0.40, 0.40);
const DIVIDER_COLOR:   Color = Color::srgba(1.0, 1.0, 1.0, 0.07);

// ---------------------------------------------------------------------------
// OnEnter / OnExit
// ---------------------------------------------------------------------------

fn release_cursor(mut cursor_q: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut cursor) = cursor_q.single_mut() {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible   = true;
    }
}

fn cleanup_lobby_ui(mut commands: Commands, roots: Query<Entity, With<LobbyRoot>>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
}

// ---------------------------------------------------------------------------
// UI builder
// ---------------------------------------------------------------------------

fn setup_lobby_ui(
    mut commands: Commands,
    cfg: Res<ClientConfigResource>,
    mut lobby: ResMut<LobbyState>,
) {
    if lobby.ip_input.is_empty() {
        lobby.ip_input = cfg.0.network.default_server.clone();
    }
    lobby.status_msg.clear();

    // ── Full-screen root ──────────────────────────────────────────────────
    let root = commands.spawn((
        Node {
            width:             Val::Percent(100.0),
            height:            Val::Percent(100.0),
            justify_content:   JustifyContent::Center,
            align_items:       AlignItems::Center,
            ..default()
        },
        BackgroundColor(SCREEN_BG),
        LobbyRoot,
    )).id();

    // ── Center panel ──────────────────────────────────────────────────────
    let panel = commands.spawn((
        Node {
            width:           Val::Px(580.0),
            flex_direction:  FlexDirection::Column,
            align_items:     AlignItems::Stretch,
            padding:         UiRect::all(Val::Px(32.0)),
            row_gap:         Val::Px(10.0),
            ..default()
        },
        BackgroundColor(PANEL_BG),
    )).id();
    commands.entity(root).add_child(panel);

    spawn_text(&mut commands, panel, "bevy_game",
        38.0, TEXT_ACCENT, UiRect::bottom(Val::Px(4.0)));
    spawn_text(&mut commands, panel,
        &format!("Player: {}", cfg.0.player.name),
        13.0, TEXT_DIM, UiRect::default());

    spawn_divider(&mut commands, panel);

    spawn_text(&mut commands, panel, "Connect to Server",
        12.0, TEXT_DIM, UiRect::default());

    // ── IP row ────────────────────────────────────────────────────────────
    let ip_row = commands.spawn(Node {
        flex_direction:  FlexDirection::Row,
        align_items:     AlignItems::Center,
        column_gap:      Val::Px(8.0),
        ..default()
    }).id();
    commands.entity(panel).add_child(ip_row);

    // Input field (clickable box, no ButtonKind so hover won't override INPUT_BG)
    let ip_field = commands.spawn((
        Button,
        Node {
            flex_grow:     1.0,
            height:        Val::Px(36.0),
            padding:       UiRect::axes(Val::Px(10.0), Val::Px(0.0)),
            align_items:   AlignItems::Center,
            ..default()
        },
        BackgroundColor(INPUT_BG),
        IpFieldBtn,
    )).id();
    commands.entity(ip_row).add_child(ip_field);

    let ip_text = commands.spawn((
        Text::new(cfg.0.network.default_server.clone()),
        TextFont { font_size: 15.0, ..default() },
        TextColor(TEXT_PRIMARY),
        IpInputText,
    )).id();
    commands.entity(ip_field).add_child(ip_text);

    spawn_btn(&mut commands, ip_row, "Connect",
        ButtonKind::Connect, 92.0, ConnectBtn);

    // Error / status line
    let status = commands.spawn((
        Text::new(""),
        TextFont { font_size: 12.0, ..default() },
        TextColor(TEXT_DIM),
        StatusText,
    )).id();
    commands.entity(panel).add_child(status);

    spawn_divider(&mut commands, panel);

    // ── Favorites header ──────────────────────────────────────────────────
    let fav_hdr = commands.spawn(Node {
        flex_direction:   FlexDirection::Row,
        justify_content:  JustifyContent::SpaceBetween,
        align_items:      AlignItems::Center,
        ..default()
    }).id();
    commands.entity(panel).add_child(fav_hdr);

    spawn_text(&mut commands, fav_hdr, "Favorites",
        12.0, TEXT_DIM, UiRect::default());
    spawn_btn(&mut commands, fav_hdr, "+ Add Current IP",
        ButtonKind::Normal, 130.0, AddFavBtn);

    // ── Favorites list ────────────────────────────────────────────────────
    if cfg.0.server_history.favorites.is_empty() {
        spawn_text(&mut commands, panel,
            "No favorites saved yet. Enter an IP above and click \"+\".",
            13.0, TEXT_DIM,
            UiRect::left(Val::Px(2.0)));
    } else {
        for entry in &cfg.0.server_history.favorites {
            let label = entry.label.clone().unwrap_or_else(|| entry.address.clone());

            let row = commands.spawn(Node {
                flex_direction:  FlexDirection::Row,
                align_items:     AlignItems::Center,
                column_gap:      Val::Px(8.0),
                ..default()
            }).id();
            commands.entity(panel).add_child(row);

            let addr_lbl = commands.spawn((
                Text::new(label),
                TextFont { font_size: 14.0, ..default() },
                TextColor(TEXT_PRIMARY),
                Node { flex_grow: 1.0, ..default() },
            )).id();
            commands.entity(row).add_child(addr_lbl);

            let addr = entry.address.clone();
            spawn_btn(&mut commands, row, "Connect",
                ButtonKind::Connect, 82.0, FavConnectBtn(addr.clone()));
            spawn_btn(&mut commands, row, "Remove",
                ButtonKind::Danger, 72.0, FavRemoveBtn(addr));
        }
    }

    // ── Bottom row: LAN scan ──────────────────────────────────────────────
    let bot = commands.spawn(Node {
        flex_direction:   FlexDirection::Row,
        justify_content:  JustifyContent::End,
        margin:           UiRect::top(Val::Px(8.0)),
        ..default()
    }).id();
    commands.entity(panel).add_child(bot);

    spawn_btn(&mut commands, bot, "Scan LAN",
        ButtonKind::Normal, 90.0, ScanLanBtn);
}

// ── Spawn helpers ─────────────────────────────────────────────────────────────

fn spawn_text(
    commands: &mut Commands,
    parent:   Entity,
    text:     &str,
    size:     f32,
    color:    Color,
    margin:   UiRect,
) {
    let e = commands.spawn((
        Text::new(text),
        TextFont { font_size: size, ..default() },
        TextColor(color),
        Node { margin, ..default() },
    )).id();
    commands.entity(parent).add_child(e);
}

fn spawn_divider(commands: &mut Commands, parent: Entity) {
    let e = commands.spawn((
        Node {
            height: Val::Px(1.0),
            margin: UiRect::vertical(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(DIVIDER_COLOR),
    )).id();
    commands.entity(parent).add_child(e);
}

fn spawn_btn<C: Component>(
    commands: &mut Commands,
    parent:   Entity,
    label:    &str,
    kind:     ButtonKind,
    width:    f32,
    marker:   C,
) {
    let btn = commands.spawn((
        Button,
        Node {
            width:            Val::Px(width),
            height:           Val::Px(32.0),
            justify_content:  JustifyContent::Center,
            align_items:      AlignItems::Center,
            ..default()
        },
        BackgroundColor(kind_color(kind)),
        kind,
        marker,
    )).id();

    let lbl = commands.spawn((
        Text::new(label),
        TextFont { font_size: 13.0, ..default() },
        TextColor(TEXT_PRIMARY),
    )).id();
    commands.entity(btn).add_child(lbl);
    commands.entity(parent).add_child(btn);
}

// ---------------------------------------------------------------------------
// Systems — input
// ---------------------------------------------------------------------------

fn handle_ip_field_click(
    field_q: Query<&Interaction, (Changed<Interaction>, With<IpFieldBtn>)>,
    other_q: Query<&Interaction, (Changed<Interaction>, With<Button>, Without<IpFieldBtn>)>,
    mut lobby: ResMut<LobbyState>,
) {
    for &ia in &field_q {
        if ia == Interaction::Pressed {
            lobby.input_focused = true;
        }
    }
    for &ia in &other_q {
        if ia == Interaction::Pressed {
            lobby.input_focused = false;
        }
    }
}

fn handle_keyboard_input(
    mut evs: MessageReader<KeyboardInput>,
    mut lobby: ResMut<LobbyState>,
    mut writer: MessageWriter<ConnectClient>,
    mut net_config: ResMut<ClientNetConfig>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if !lobby.input_focused {
        evs.clear();
        return;
    }

    let mut do_connect = false;
    for ev in evs.read() {
        if ev.state != ButtonState::Pressed { continue; }
        match &ev.logical_key {
            Key::Character(ch) => {
                for c in ch.chars() {
                    if !c.is_control() && c != ' ' {
                        lobby.ip_input.push(c);
                    }
                }
            }
            Key::Backspace => { lobby.ip_input.pop(); }
            Key::Enter     => { lobby.input_focused = false; do_connect = true; }
            Key::Escape    => { lobby.input_focused = false; }
            _ => {}
        }
    }

    if do_connect {
        let ip = lobby.ip_input.clone();
        try_connect(&ip, &mut lobby, &mut net_config, &mut writer, &mut next_state);
    }
}

// ---------------------------------------------------------------------------
// Systems — buttons
// ---------------------------------------------------------------------------

fn handle_connect_btn(
    q: Query<&Interaction, (Changed<Interaction>, With<ConnectBtn>)>,
    mut lobby:      ResMut<LobbyState>,
    mut net_config: ResMut<ClientNetConfig>,
    mut writer:     MessageWriter<ConnectClient>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    for &ia in &q {
        if ia == Interaction::Pressed {
            let ip = lobby.ip_input.clone();
            lobby.input_focused = false;
            try_connect(&ip, &mut lobby, &mut net_config, &mut writer, &mut next_state);
        }
    }
}

fn handle_fav_connect_btn(
    q: Query<(&Interaction, &FavConnectBtn), Changed<Interaction>>,
    mut lobby:      ResMut<LobbyState>,
    mut net_config: ResMut<ClientNetConfig>,
    mut writer:     MessageWriter<ConnectClient>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    for (&ia, fav) in &q {
        if ia == Interaction::Pressed {
            lobby.ip_input    = fav.0.clone();
            lobby.input_focused = false;
            let addr = fav.0.clone();
            try_connect(&addr, &mut lobby, &mut net_config, &mut writer, &mut next_state);
        }
    }
}

fn handle_add_fav_btn(
    q: Query<&Interaction, (Changed<Interaction>, With<AddFavBtn>)>,
    lobby: Res<LobbyState>,
    mut cfg: ResMut<ClientConfigResource>,
) {
    for &ia in &q {
        if ia != Interaction::Pressed { continue; }
        let addr = lobby.ip_input.trim().to_string();
        if addr.is_empty() { return; }
        if cfg.0.server_history.favorites.iter().any(|e| e.address == addr) { return; }

        cfg.0.server_history.favorites.push(ServerEntry {
            address:        addr.clone(),
            label:          None,
            last_connected: None,
        });
        if let Ok(path) = ClientConfig::resolve_path() {
            let _ = cfg.0.save(&path);
        }
        info!("[lobby] added {addr} to favorites");
    }
}

fn handle_remove_fav_btn(
    q: Query<(&Interaction, &FavRemoveBtn), Changed<Interaction>>,
    mut cfg: ResMut<ClientConfigResource>,
) {
    for (&ia, rem) in &q {
        if ia != Interaction::Pressed { continue; }
        cfg.0.server_history.favorites.retain(|e| e.address != rem.0);
        if let Ok(path) = ClientConfig::resolve_path() {
            let _ = cfg.0.save(&path);
        }
        info!("[lobby] removed {} from favorites", rem.0);
    }
}

fn handle_scan_lan_btn(
    q: Query<&Interaction, (Changed<Interaction>, With<ScanLanBtn>)>,
    mut lobby: ResMut<LobbyState>,
) {
    for &ia in &q {
        if ia == Interaction::Pressed {
            lobby.status_msg = "LAN scan not yet implemented.".to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// Systems — display refresh
// ---------------------------------------------------------------------------

fn update_ip_display(
    lobby: Res<LobbyState>,
    mut q: Query<&mut Text, With<IpInputText>>,
) {
    if !lobby.is_changed() { return; }
    let display = if lobby.input_focused {
        format!("{}_", lobby.ip_input)
    } else {
        lobby.ip_input.clone()
    };
    for mut t in &mut q {
        *t = Text::new(display.clone());
    }
}

fn update_status_display(
    lobby: Res<LobbyState>,
    mut q: Query<(&mut Text, &mut TextColor), With<StatusText>>,
) {
    if !lobby.is_changed() { return; }
    let is_err = lobby.status_msg.starts_with("Invalid");
    for (mut t, mut c) in &mut q {
        *t = Text::new(lobby.status_msg.clone());
        c.0 = if is_err { TEXT_ERROR } else { TEXT_DIM };
    }
}

fn update_button_colors(
    // Exclude IpFieldBtn — its INPUT_BG should never be overridden by hover logic
    mut q: Query<
        (&Interaction, &mut BackgroundColor, &ButtonKind),
        (Changed<Interaction>, Without<IpFieldBtn>),
    >,
) {
    for (&ia, mut bg, &kind) in &mut q {
        bg.0 = match (ia, kind) {
            (Interaction::Pressed, _)                   => BTN_PRESSED,
            (Interaction::Hovered, ButtonKind::Connect) => BTN_CONNECT_HOV,
            (Interaction::Hovered, ButtonKind::Danger)  => BTN_DANGER_HOV,
            (Interaction::Hovered, _)                   => BTN_HOVER,
            (Interaction::None,    _)                   => kind_color(kind),
        };
    }
}

// ---------------------------------------------------------------------------
// Shared connect logic
// ---------------------------------------------------------------------------

fn try_connect(
    addr:       &str,
    lobby:      &mut LobbyState,
    net_config: &mut ClientNetConfig,
    writer:     &mut MessageWriter<ConnectClient>,
    next_state: &mut NextState<AppState>,
) {
    let trimmed = addr.trim();
    match trimmed.parse::<std::net::SocketAddr>() {
        Ok(socket_addr) => {
            net_config.server = socket_addr;
            writer.write(ConnectClient);
            next_state.set(AppState::InGame);
            lobby.status_msg.clear();
            info!("[lobby] connecting to {socket_addr}");
        }
        Err(e) => {
            lobby.status_msg = format!("Invalid address: {e}");
            warn!("[lobby] bad address '{trimmed}': {e}");
        }
    }
}
