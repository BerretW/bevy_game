use bevy::light::{FogVolume, GlobalAmbientLight, VolumetricFog, VolumetricLight};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use core_resources::{EnvironmentLightConfig, LocalEventBus};

use super::camera::MainGameplayCamera;
use crate::map_loader::StreamingVisibilityBoundary;

#[derive(Component)]
pub(super) struct EnvironmentLightMarker;

#[derive(Resource, Default)]
pub(super) struct VolumetricFogDebugState {
    pub(super) last_emit_secs: f32,
}

pub(super) fn apply_environment_light(
    mut commands: Commands,
    config: Res<EnvironmentLightConfig>,
    streaming_boundary: Option<Res<StreamingVisibilityBoundary>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut existing: Query<
        (Entity, &mut DirectionalLight, &mut Transform, Option<&VolumetricLight>),
        With<EnvironmentLightMarker>,
    >,
    camera_q: Query<Entity, With<MainGameplayCamera>>,
) {
    ambient.color = Color::srgba(
        config.ambient_color[0].clamp(0.0, 1.0),
        config.ambient_color[1].clamp(0.0, 1.0),
        config.ambient_color[2].clamp(0.0, 1.0),
        config.ambient_color[3].clamp(0.0, 1.0),
    );
    ambient.brightness = if config.ambient_enabled {
        config.ambient_brightness.max(0.0)
    } else {
        0.0
    };

    let mut fog_start = config.fog_start.max(0.0);
    let mut fog_end = config.fog_end.max(fog_start);
    if config.fog_follow_streaming_boundary {
        if let Some(boundary) = streaming_boundary.as_ref() {
            if boundary.active {
                let edge = boundary.radius.max(0.0);
                fog_start = (edge - config.fog_boundary_inner_distance.max(0.0)).max(0.0);
                fog_end = edge + config.fog_boundary_outer_distance.max(0.0);
                if fog_end < fog_start {
                    fog_end = fog_start;
                }
            }
        }
    }

    if let Ok(camera_entity) = camera_q.single() {
        if config.fog_enabled {
            commands.entity(camera_entity).insert(DistanceFog {
                color: Color::srgba(
                    config.fog_color[0].clamp(0.0, 1.0),
                    config.fog_color[1].clamp(0.0, 1.0),
                    config.fog_color[2].clamp(0.0, 1.0),
                    config.fog_color[3].clamp(0.0, 1.0),
                ),
                directional_light_color: Color::srgba(
                    config.fog_directional_light_color[0].clamp(0.0, 1.0),
                    config.fog_directional_light_color[1].clamp(0.0, 1.0),
                    config.fog_directional_light_color[2].clamp(0.0, 1.0),
                    config.fog_directional_light_color[3].clamp(0.0, 1.0),
                ),
                directional_light_exponent: config.fog_directional_light_exponent.max(0.0),
                falloff: FogFalloff::Linear {
                    start: fog_start,
                    end: fog_end,
                },
            });
        } else {
            commands.entity(camera_entity).remove::<DistanceFog>();
        }

        if config.volumetric_fog_enabled {
            commands.entity(camera_entity).insert(VolumetricFog {
                ambient_color: Color::srgba(
                    config.volumetric_fog_ambient_color[0].clamp(0.0, 1.0),
                    config.volumetric_fog_ambient_color[1].clamp(0.0, 1.0),
                    config.volumetric_fog_ambient_color[2].clamp(0.0, 1.0),
                    config.volumetric_fog_ambient_color[3].clamp(0.0, 1.0),
                ),
                ambient_intensity: config.volumetric_fog_ambient_intensity.max(0.0),
                jitter: config.volumetric_fog_jitter.max(0.0),
                step_count: config.volumetric_fog_step_count.max(1),
            });
        } else {
            commands.entity(camera_entity).remove::<VolumetricFog>();
        }
    }

    if !config.enabled {
        for (entity, _, _, _) in &mut existing {
            commands.entity(entity).despawn();
        }
        return;
    }

    let daylight = ((config.hour_of_day - 6.0) / 12.0) * std::f32::consts::PI;
    let elevation = daylight.sin().clamp(-1.0, 1.0)
        * config.max_elevation_deg.clamp(0.0, 89.0).to_radians();
    let azimuth = config.azimuth_deg.to_radians();

    let transform = Transform::from_rotation(Quat::from_euler(
        EulerRot::YXZ,
        azimuth,
        -elevation,
        0.0,
    ));
    let color = Color::srgba(
        config.color[0].clamp(0.0, 1.0),
        config.color[1].clamp(0.0, 1.0),
        config.color[2].clamp(0.0, 1.0),
        config.color[3].clamp(0.0, 1.0),
    );

    if let Ok((entity, mut light, mut light_transform, volumetric)) = existing.single_mut() {
        light.color = color;
        light.illuminance = config.illuminance.max(0.0);
        light.shadows_enabled = config.shadows_enabled;
        *light_transform = transform;
        let enable_volumetric_light = config.volumetric_fog_enabled && config.shadows_enabled;
        if enable_volumetric_light && volumetric.is_none() {
            commands.entity(entity).insert(VolumetricLight);
        } else if !enable_volumetric_light && volumetric.is_some() {
            commands.entity(entity).remove::<VolumetricLight>();
        }
        return;
    }

    let mut entity = commands.spawn((
        DirectionalLight {
            color,
            illuminance: config.illuminance.max(0.0),
            shadows_enabled: config.shadows_enabled,
            ..default()
        },
        transform,
        EnvironmentLightMarker,
    ));
    if config.volumetric_fog_enabled && config.shadows_enabled {
        entity.insert(VolumetricLight);
    }
}

pub(super) fn publish_volumetric_fog_state_to_lua(
    time: Res<Time>,
    local_bus: Res<LocalEventBus>,
    mut debug_state: ResMut<VolumetricFogDebugState>,
    config: Res<EnvironmentLightConfig>,
    camera_q: Query<Option<&VolumetricFog>, With<MainGameplayCamera>>,
    env_light_q: Query<Option<&VolumetricLight>, With<EnvironmentLightMarker>>,
    fog_volumes: Query<(), With<FogVolume>>,
) {
    let now = time.elapsed_secs();
    if now - debug_state.last_emit_secs < 0.5 {
        return;
    }
    debug_state.last_emit_secs = now;

    let camera_has_volumetric_fog = camera_q.single().ok().flatten().is_some();
    let environment_light_count = env_light_q.iter().count();
    let environment_light_with_volumetric = env_light_q.iter().filter(|light| light.is_some()).count();
    let fog_volume_count = fog_volumes.iter().count();

    let payload = serde_json::to_vec(&serde_json::json!({
        "camera_has_volumetric_fog": camera_has_volumetric_fog,
        "environment_light_count": environment_light_count,
        "environment_light_with_volumetric": environment_light_with_volumetric,
        "fog_volume_count": fog_volume_count,
        "config_volumetric_enabled": config.volumetric_fog_enabled,
        "config_shadows_enabled": config.shadows_enabled,
        "config_hour": config.hour_of_day,
        "config_ambient_intensity": config.volumetric_fog_ambient_intensity,
        "config_step_count": config.volumetric_fog_step_count,
    }))
    .unwrap_or_default();

    local_bus.push("debug:volumetric_fog_state".to_string(), payload);
}
