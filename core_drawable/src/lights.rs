use bevy::prelude::*;
use core_resources::{RuntimeLightDef, RuntimeLightKind};

pub fn clear_runtime_light(entity: &mut EntityCommands<'_>) {
    entity.remove::<PointLight>();
    entity.remove::<SpotLight>();
    entity.remove::<DirectionalLight>();
}

pub fn apply_runtime_light(entity: &mut EntityCommands<'_>, light: &RuntimeLightDef) {
    clear_runtime_light(entity);

    if !light.enabled {
        return;
    }

    let color = Color::srgba(
        light.color[0].clamp(0.0, 1.0),
        light.color[1].clamp(0.0, 1.0),
        light.color[2].clamp(0.0, 1.0),
        light.color[3].clamp(0.0, 1.0),
    );

    match light.kind {
        RuntimeLightKind::Point => {
            entity.insert(PointLight {
                color,
                intensity: light.intensity.max(0.0),
                range: light.range.max(0.01),
                radius: light.radius.max(0.0),
                shadows_enabled: light.shadows_enabled,
                ..default()
            });
        }
        RuntimeLightKind::Spot => {
            let inner_angle = light.inner_angle_deg.to_radians().clamp(0.0, std::f32::consts::PI - 0.01);
            let outer_angle = light.outer_angle_deg.to_radians().clamp(inner_angle, std::f32::consts::PI - 0.01);
            entity.insert(SpotLight {
                color,
                intensity: light.intensity.max(0.0),
                range: light.range.max(0.01),
                radius: light.radius.max(0.0),
                shadows_enabled: light.shadows_enabled,
                inner_angle,
                outer_angle,
                ..default()
            });
        }
        RuntimeLightKind::Directional => {
            entity.insert(DirectionalLight {
                color,
                illuminance: light.illuminance.max(0.0),
                shadows_enabled: light.shadows_enabled,
                ..default()
            });
        }
    }
}