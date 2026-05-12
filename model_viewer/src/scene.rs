use bevy::prelude::*;
use bevy::window::WindowResolution;

use crate::camera;
use crate::state::{AnimDebugPanel, AnimPanel, ColliderPanel, DebugStatus, InfoOverlay, LodPanel, RigPanel, UiDiagAnimButton, UiLoadAdmButton, UiLoadAnimButton};

pub(crate) fn viewer_asset_root() -> String {
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let workspace = std::path::PathBuf::from(&dir)
            .parent()
            .map(|p| p.to_owned())
            .unwrap_or_else(|| std::path::PathBuf::from(&dir));
        let client_assets = workspace.join("host_client").join("assets");
        if client_assets.exists() {
            return client_assets.to_string_lossy().into_owned();
        }
        let local = std::path::PathBuf::from(&dir).join("assets");
        if local.exists() {
            return local.to_string_lossy().into_owned();
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("assets")))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "assets".to_string())
}

pub(crate) fn setup_scene(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(Vec3::new(0.0, 2.0, 5.0)).looking_at(Vec3::ZERO, Vec3::Y),
        camera::OrbitCamera,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.5, 0.0)),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 3_500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.5, 0.5 + std::f32::consts::PI, 0.0)),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 1_500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.4, 0.5 + std::f32::consts::PI * 0.5, 0.0)),
    ));

    commands.spawn(AmbientLight {
        color: Color::WHITE,
        brightness: 600.0,
        ..default()
    });

    commands.spawn((
        Text::new(
            "Pravé drag: orbit  |  Střední drag: pan  |  Kolečko: zoom  \
             |  R: reset  |  G: mřížka  |  H: info  |  T: textury  |  E: export\n\
               W: weather preset  |  V: vertex color debug  |  P: vertex paint  |  C: collidery  |  X: kostra  |  Z: IK edit  |  L: LOD úroveň",
        ),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.65)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));

    commands.spawn((
        Text::new("Načítám model…"),
        TextFont { font_size: 14.0, ..default() },
        TextColor(Color::srgba(0.9, 0.9, 0.9, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
        InfoOverlay,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(0.95, 0.95, 0.95, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(52.0),
            left: Val::Px(8.0),
            max_width: Val::Px(620.0),
            ..default()
        },
        Visibility::Hidden,
        DebugStatus,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(0.95, 0.95, 0.95, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(8.0),
            max_width: Val::Px(560.0),
            ..default()
        },
        Visibility::Hidden,
        AnimPanel,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(0.95, 0.95, 0.95, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(8.0),
            max_width: Val::Px(560.0),
            ..default()
        },
        Visibility::Hidden,
        AnimDebugPanel,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(0.95, 0.95, 0.95, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(200.0),
            right: Val::Px(8.0),
            max_width: Val::Px(420.0),
            ..default()
        },
        Visibility::Hidden,
        ColliderPanel,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(0.95, 0.95, 0.95, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(52.0),
            right: Val::Px(8.0),
            max_width: Val::Px(400.0),
            ..default()
        },
        Visibility::Hidden,
        LodPanel,
    ));

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            left: Val::Px(6.0),
            right: Val::Px(6.0),
            height: Val::Px(34.0),
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.06, 0.07, 0.10, 0.88)),
    ))
    .with_children(|parent| {
        let button_style = Node {
            min_width: Val::Px(128.0),
            height: Val::Px(24.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        };

        parent
            .spawn((
                Button,
                button_style.clone(),
                BackgroundColor(Color::srgba(0.15, 0.18, 0.25, 0.95)),
                UiLoadAdmButton,
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("Open ADM/GLB"),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgba(0.96, 0.96, 0.98, 1.0)),
                ));
            });

        parent
            .spawn((
                Button,
                button_style.clone(),
                BackgroundColor(Color::srgba(0.15, 0.18, 0.25, 0.95)),
                UiLoadAnimButton,
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("Open ADS_ANIM"),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgba(0.96, 0.96, 0.98, 1.0)),
                ));
            });

        parent
            .spawn((
                Button,
                button_style,
                BackgroundColor(Color::srgba(0.18, 0.15, 0.28, 0.95)),
                UiDiagAnimButton,
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("Diag ADS_ANIM"),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgba(0.98, 0.96, 1.0, 1.0)),
                ));
            });
    });
}
