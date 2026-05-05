//! Dev console — toggle with F8.
//!
//! * Captures all `tracing` log events (INFO/WARN/ERROR/DEBUG) via a custom layer
//!   passed to `LogPlugin::custom_layer`.
//! * Draws a semi-transparent overlay in the top half of the screen (Bevy UI).
//! * Accepts typed commands; Phase 3.8 will route them through the Lua event bus.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use bevy::ecs::message::MessageReader;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

// ── Global ring buffer shared by the tracing layer and the Bevy resource ──────

static LOG_RING: OnceLock<Arc<Mutex<VecDeque<String>>>> = OnceLock::new();

fn global_ring() -> Arc<Mutex<VecDeque<String>>> {
    LOG_RING
        .get_or_init(|| Arc::new(Mutex::new(VecDeque::with_capacity(512))))
        .clone()
}

pub(crate) fn ring_push(line: impl Into<String>) {
    let ring = global_ring();
    let mut g = ring.lock().unwrap();
    if g.len() >= 512 {
        g.pop_front();
    }
    g.push_back(line.into());
}

// ── tracing Layer — captures log output from the entire app ──────────────────

struct ConsoleLayer;

struct MsgCollect(String);

impl tracing::field::Visit for MsgCollect {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ConsoleLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut v = MsgCollect(String::new());
        event.record(&mut v);
        if v.0.is_empty() {
            return;
        }
        let pfx = match *event.metadata().level() {
            tracing::Level::ERROR => "ERR",
            tracing::Level::WARN  => "WRN",
            tracing::Level::INFO  => "INF",
            tracing::Level::DEBUG => "DBG",
            tracing::Level::TRACE => "TRC",
        };
        ring_push(format!("[{pfx}] {}", v.0));
    }
}

/// Passed to `LogPlugin::custom_layer` — must be a plain `fn` pointer.
pub fn tracing_layer(_app: &mut App) -> Option<bevy::log::BoxedLayer> {
    let _ = global_ring(); // ensure OnceLock is initialised before tracing starts
    Some(Box::new(ConsoleLayer))
}

// ── Bevy resource wrapping the same ring ──────────────────────────────────────

#[derive(Resource, Clone)]
pub struct ConsoleLog(pub Arc<Mutex<VecDeque<String>>>);

// ── Console open/close state ──────────────────────────────────────────────────

#[derive(Resource, Default)]
pub struct ConsoleState {
    pub open: bool,
    pub input: String,
}

// ── UI marker components ──────────────────────────────────────────────────────

#[derive(Component)]
struct ConsoleRoot;

#[derive(Component)]
struct LogText;

#[derive(Component)]
struct InputText;

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ConsoleLog(global_ring()))
            .init_resource::<ConsoleState>()
            .add_systems(Startup, spawn_ui)
            .add_systems(Update, (toggle, type_input, redraw).chain());
    }
}

// ── UI ────────────────────────────────────────────────────────────────────────

fn spawn_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                // top half of the screen
                height: Val::Percent(50.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.04, 0.04, 0.90)),
            Visibility::Hidden,
            ZIndex(999),
            ConsoleRoot,
        ))
        .with_children(|p| {
            // Scrollable log area.
            p.spawn(Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                overflow: Overflow::clip(),
                ..default()
            })
            .with_children(|p| {
                p.spawn((
                    Text::new(""),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgb(0.82, 0.82, 0.82)),
                    LogText,
                ));
            });

            // Thin divider.
            p.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(1.0),
                    margin: UiRect::vertical(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.35, 0.35, 0.35)),
            ));

            // Input row  "> " + cursor-blinking text.
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|p| {
                p.spawn((
                    Text::new("> "),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgb(0.3, 0.9, 0.3)),
                ));
                p.spawn((
                    Text::new(""),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::WHITE),
                    InputText,
                ));
            });
        });
}

// ── Systems ───────────────────────────────────────────────────────────────────

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<ConsoleState>,
    mut q: Query<&mut Visibility, With<ConsoleRoot>>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    state.open = !state.open;
    if let Ok(mut vis) = q.single_mut() {
        *vis = if state.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn type_input(
    mut evs: MessageReader<KeyboardInput>,
    mut state: ResMut<ConsoleState>,
    log: Res<ConsoleLog>,
) {
    if !state.open {
        evs.clear();
        return;
    }
    for ev in evs.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }
        match &ev.logical_key {
            Key::Character(ch) => {
                for c in ch.chars() {
                    if !c.is_control() {
                        state.input.push(c);
                    }
                }
            }
            Key::Backspace => {
                state.input.pop();
            }
            Key::Enter => {
                let cmd = std::mem::take(&mut state.input);
                let cmd = cmd.trim().to_string();
                if !cmd.is_empty() {
                    ring_push(format!("> {cmd}"));
                    // TODO Phase 3.8: dispatch via Lua event bus.
                    let mut g = log.0.lock().unwrap();
                    if g.len() >= 512 {
                        g.pop_front();
                    }
                    g.push_back(format!("[console] unknown command: {cmd}"));
                }
            }
            _ => {}
        }
    }
}

fn redraw(
    state: Res<ConsoleState>,
    log: Res<ConsoleLog>,
    mut log_q: Query<&mut Text, (With<LogText>, Without<InputText>)>,
    mut inp_q: Query<&mut Text, (With<InputText>, Without<LogText>)>,
    time: Res<Time>,
    mut blink: Local<f32>,
) {
    if !state.open {
        return;
    }
    *blink += time.delta_secs();
    let cursor = if (*blink % 0.8) < 0.4 { "_" } else { " " };

    if let Ok(mut text) = log_q.single_mut() {
        let ring = log.0.lock().unwrap();
        // Show the most recent 30 lines, oldest first.
        let lines: String = ring
            .iter()
            .rev()
            .take(30)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        *text = Text::new(lines);
    }

    if let Ok(mut text) = inp_q.single_mut() {
        *text = Text::new(format!("{}{cursor}", state.input));
    }
}
