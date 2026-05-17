use std::collections::HashSet;

use avian3d::prelude::*;
use bevy::asset::AssetPath;
use bevy::light::FogVolume;
use bevy::prelude::*;
use core_drawable::{apply_runtime_light, clear_runtime_light, DisableDrawableCollisions, OnStairs};
use core_resources::{
    AnimationState, AttachedAnimSets, CrosshairHit, DummyObjectMarker, DummyPrimitiveKind,
    EntityHandle, GameBridges, IkEnabledComponent, LocalObjectMarker, ModelName, ModelRegistry,
};
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::{Confirmed, Predicted};

use super::{resolve_default_ped_profile, resolve_ped_profile_for_model, LocalClientId};
use crate::drawable::{AdmSceneRoot, PedPhysicsDef, PedPhysicsRegistry};
use crate::gameplay::animation::AutoPlayerAnimMemory;
use crate::native_assets::{AdmHandleCache, PedAdsAnimIndex};

const POSITION_SMOOTHING_RATE: f32 = 14.0;
const ROTATION_SMOOTHING_RATE: f32 = 18.0;
const GROUND_CASTER_RADIUS: f32 = 0.14;

#[derive(Component)]
pub(super) struct PlayerVisualAttached;

#[derive(Component)]
pub(super) struct LocalObjectVisualAttached;

#[derive(Component)]
pub(super) struct DummyObjectVisualAttached;

#[derive(Component)]
pub(super) struct DummyFogVolumeAttached;

pub(super) fn attach_player_model_to_new_players(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    ped_anim_index: Res<PedAdsAnimIndex>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    predicted_players: Query<&PlayerMarker, With<Predicted>>,
    new_players: Query<
        (Entity, &PlayerMarker, Option<&Predicted>),
        (With<NetTransform>, Without<PlayerVisualAttached>),
    >,
) {
    let predicted_ids: HashSet<u64> = predicted_players.iter().map(|marker| marker.client_id).collect();

    for (entity, marker, predicted) in new_players.iter() {
        if predicted.is_none() && predicted_ids.contains(&marker.client_id) {
            continue;
        }

        let ped = resolve_default_ped_profile(&ped_reg, &ped_assets);
        let model_name = ped.map(|p| p.identity.model.as_str()).unwrap_or("player");
        let cap_radius = ped.map(|p| p.capsule.radius).unwrap_or(0.35_f32);
        let cap_height = ped.map(|p| p.capsule.height).unwrap_or(1.80_f32);
        let cap_body = (cap_height - cap_radius * 2.0).max(0.001);
        let attached_anim_sets = ped_anim_index.0.get(model_name).cloned().unwrap_or_default();
        let model_path = format!("models/{}.adm", model_name);
        let model_handle = asset_server.load::<crate::drawable::AdmScene>(model_path);

        commands
            .entity(entity)
            .insert((
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Visible,
                InheritedVisibility::default(),
                ViewVisibility::default(),
                PlayerVisualAttached,
                AutoPlayerAnimMemory::default(),
                OnStairs::default(),
                IkEnabledComponent::default(),
                RigidBody::Dynamic,
                GravityScale(0.0),
                LockedAxes::new()
                    .lock_rotation_x()
                    .lock_rotation_y()
                    .lock_rotation_z(),
                ShapeCaster::new(
                    Collider::sphere(GROUND_CASTER_RADIUS),
                    Vec3::new(0.0, GROUND_CASTER_RADIUS + 0.04, 0.0),
                    Quat::IDENTITY,
                    Dir3::NEG_Y,
                )
                .with_max_distance(0.30)
                .with_max_hits(4)
                .with_query_filter(SpatialQueryFilter::from_mask(LayerMask::DEFAULT)),
            ))
            .with_children(|parent| {
                parent.spawn((
                    Collider::capsule(cap_radius, cap_body),
                    CollisionLayers::new(LayerMask(0b10), LayerMask::ALL),
                    Friction::new(0.1).with_combine_rule(CoefficientCombine::Min),
                    Restitution::ZERO,
                    Transform::from_xyz(0.0, cap_height / 2.0, 0.0),
                    GlobalTransform::default(),
                ));
                let mut child = parent.spawn((
                    AdmSceneRoot(model_handle.clone()),
                    DisableDrawableCollisions,
                    ModelName(model_name.to_string()),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                    GlobalTransform::default(),
                    Visibility::default(),
                    InheritedVisibility::default(),
                    ViewVisibility::default(),
                ));

                if let Some(ped) = resolve_ped_profile_for_model(model_name, &ped_reg, &ped_assets) {
                    let idle_clip = ped.animations.idle.clone();
                    if !idle_clip.is_empty() {
                        child.insert(AnimationState {
                            current: Some(idle_clip),
                            speed: 1.0,
                            looping: true,
                            paused: false,
                            blend_time: 0.0,
                            flags: 1,
                        });
                    }
                }

                if !attached_anim_sets.is_empty() {
                    child.insert(AttachedAnimSets {
                        sets: attached_anim_sets.clone(),
                    });
                }
            });

        info!(
            "[gameplay/client] ADM model attached to player {:?} (client_id={})",
            entity, marker.client_id
        );
    }
}

pub(super) fn sync_net_transform_to_render(
    time: Res<Time>,
    local_client_id: Option<Res<LocalClientId>>,
    mut predicted_q: Query<
        (
            &mut Transform,
            &PlayerMarker,
            &NetTransform,
            Option<&Confirmed<NetTransform>>,
        ),
        With<Predicted>,
    >,
) {
    let local_id = local_client_id.map(|resource| resource.0);
    for (mut local, marker, predicted_net, confirmed_net) in predicted_q.iter_mut() {
        if Some(marker.client_id) == local_id {
            let src = confirmed_net.map(|confirmed| &confirmed.0).unwrap_or(predicted_net);
            let dt = time.delta_secs();
            let rot_alpha = (1.0 - (-ROTATION_SMOOTHING_RATE * dt).exp()).clamp(0.0, 1.0);
            local.rotation = local.rotation.slerp(src.rotation, rot_alpha);
            continue;
        }
        let src = confirmed_net.map(|confirmed| &confirmed.0).unwrap_or(predicted_net);
        let target_pos = Vec3::new(src.translation.x, src.translation.y, src.translation.z);
        let dt = time.delta_secs();
        let pos_alpha = (1.0 - (-POSITION_SMOOTHING_RATE * dt).exp()).clamp(0.0, 1.0);
        let rot_alpha = (1.0 - (-ROTATION_SMOOTHING_RATE * dt).exp()).clamp(0.0, 1.0);

        local.translation = local.translation.lerp(target_pos, pos_alpha);
        local.rotation = local.rotation.slerp(src.rotation, rot_alpha);
    }
}

pub(super) fn prefer_predicted_player_visuals(
    predicted_ids_q: Query<&PlayerMarker, With<Predicted>>,
    mut fallback_visuals_q: Query<(&PlayerMarker, &mut Visibility), (With<PlayerVisualAttached>, Without<Predicted>)>,
) {
    let predicted_ids: HashSet<u64> = predicted_ids_q.iter().map(|marker| marker.client_id).collect();

    for (marker, mut visibility) in fallback_visuals_q.iter_mut() {
        *visibility = if predicted_ids.contains(&marker.client_id) {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}

pub(super) fn update_local_player_visibility(
    local_client_id: Option<Res<LocalClientId>>,
    mut players: Query<(&PlayerMarker, &mut Visibility), With<PlayerVisualAttached>>,
) {
    let Some(local_client_id) = local_client_id else {
        return;
    };
    for (marker, mut visibility) in players.iter_mut() {
        if marker.client_id == local_client_id.0 {
            *visibility = Visibility::Visible;
        }
    }
}

pub(super) fn update_crosshair_entity(
    camera_q: Query<&GlobalTransform, With<super::camera::MainGameplayCamera>>,
    spatial_query: SpatialQuery,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    handle_q: Query<&EntityHandle>,
    player_q: Query<Has<PlayerMarker>>,
    bridges: Res<GameBridges>,
) {
    let Ok(camera) = camera_q.single() else {
        bridges.crosshair.set(None);
        return;
    };
    let origin = camera.translation();
    let dir = camera.forward();

    let not_player = |entity: Entity| -> bool {
        let mut current = entity;
        loop {
            if player_q.get(current).unwrap_or(false) {
                return false;
            }
            match child_of_q.get(current) {
                Ok(child_of) => current = child_of.parent(),
                Err(_) => return true,
            }
        }
    };

    let filter = SpatialQueryFilter::default();
    let hit = spatial_query.cast_ray_predicate(origin, dir, 100.0, true, &filter, &not_player);
    let Some(hit) = hit else {
        bridges.crosshair.set(None);
        return;
    };

    let mut current = hit.entity;
    let root_with_handle = loop {
        if let Ok(handle) = handle_q.get(current) {
            break Some(handle.0);
        }
        if let Ok(child_of) = child_of_q.get(current) {
            current = child_of.parent();
        } else {
            break None;
        }
    };

    match root_with_handle {
        Some(handle) => bridges
            .crosshair
            .set(Some(CrosshairHit { handle, distance: hit.distance })),
        None => bridges.crosshair.set(None),
    }
}

pub(super) fn attach_mesh_to_local_objects(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    model_registry: Res<ModelRegistry>,
    adm_cache: Res<AdmHandleCache>,
    new_objs: Query<(Entity, &LocalObjectMarker), (With<LocalObjectMarker>, Without<LocalObjectVisualAttached>)>,
) {
    for (entity, marker) in new_objs.iter() {
        commands.entity(entity).insert((
            GlobalTransform::default(),
            Visibility::Visible,
            InheritedVisibility::default(),
            ViewVisibility::default(),
            LocalObjectVisualAttached,
        ));

        if let Some(path) = model_registry.path(&marker.model) {
            let ext = path.extension().and_then(|extension| extension.to_str()).unwrap_or("");
            if ext == "adm" {
                let handle = adm_cache.0.get(&marker.model).cloned().unwrap_or_else(|| {
                    let bevy_path = path.to_string_lossy().replace('\\', "/");
                    asset_server.load(bevy_path)
                });
                commands.entity(entity).insert((AdmSceneRoot(handle), ModelName(marker.model.clone())));
            } else {
                let scene_path = AssetPath::from_path_buf(path.clone()).with_label("Scene0");
                let scene: Handle<Scene> = asset_server.load_override(scene_path);
                commands.entity(entity).with_children(|parent| {
                    parent.spawn((SceneRoot(scene), Transform::default()));
                });
            }
            continue;
        }

        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::new(0.9, 0.9, 0.9))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.2, 0.85, 0.3),
                ..default()
            })),
        ));
        warn!(
            "[gameplay/client] LocalObject '{}' not found in ModelRegistry; using fallback cube",
            marker.model
        );
    }
}

pub(super) fn attach_mesh_to_dummy_objects(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    new_objs: Query<(Entity, &DummyObjectMarker), (With<DummyObjectMarker>, Without<DummyObjectVisualAttached>)>,
) {
    for (entity, marker) in new_objs.iter() {
        let mut entity_cmd = commands.entity(entity);
        entity_cmd.insert((
            GlobalTransform::default(),
            Visibility::Visible,
            InheritedVisibility::default(),
            ViewVisibility::default(),
            DummyObjectVisualAttached,
        ));

        if let Some(light) = marker.light {
            apply_runtime_light(&mut entity_cmd, &light);
        } else {
            clear_runtime_light(&mut entity_cmd);
        }

        if let Some(fog) = marker.fog_volume {
            entity_cmd.insert((
                FogVolume {
                    fog_color: Color::srgba(
                        fog.color[0].clamp(0.0, 1.0),
                        fog.color[1].clamp(0.0, 1.0),
                        fog.color[2].clamp(0.0, 1.0),
                        fog.color[3].clamp(0.0, 1.0),
                    ),
                    density_factor: fog.density_factor.max(0.0),
                    absorption: fog.absorption.max(0.0),
                    scattering: fog.scattering.max(0.0),
                    scattering_asymmetry: fog.scattering_asymmetry.clamp(-0.99, 0.99),
                    light_tint: Color::srgba(
                        fog.light_tint[0].clamp(0.0, 1.0),
                        fog.light_tint[1].clamp(0.0, 1.0),
                        fog.light_tint[2].clamp(0.0, 1.0),
                        fog.light_tint[3].clamp(0.0, 1.0),
                    ),
                    light_intensity: fog.light_intensity.max(0.0),
                    ..default()
                },
                DummyFogVolumeAttached,
            ));
        }

        let base_material = materials.add(StandardMaterial {
            base_color: Color::srgba(
                marker.color[0].clamp(0.0, 1.0),
                marker.color[1].clamp(0.0, 1.0),
                marker.color[2].clamp(0.0, 1.0),
                marker.color[3].clamp(0.0, 1.0),
            ),
            ..default()
        });

        match marker.kind {
            DummyPrimitiveKind::Cuboid => {
                let sx = marker.size[0].max(0.01);
                let sy = marker.size[1].max(0.01);
                let sz = marker.size[2].max(0.01);
                entity_cmd.insert((
                    Mesh3d(meshes.add(Cuboid::new(sx, sy, sz))),
                    MeshMaterial3d(base_material.clone()),
                ));
            }
            DummyPrimitiveKind::Cube => {
                let s = marker.size[0].max(0.01);
                entity_cmd.insert((
                    Mesh3d(meshes.add(Cuboid::new(s, s, s))),
                    MeshMaterial3d(base_material.clone()),
                ));
            }
            DummyPrimitiveKind::Sphere => {
                let r = marker.radius.max(0.01);
                entity_cmd.insert((
                    Mesh3d(meshes.add(Sphere::new(r))),
                    MeshMaterial3d(base_material.clone()),
                ));
            }
            DummyPrimitiveKind::Stairs => {
                let width = marker.size[0].max(0.05);
                let total_height = marker.height.max(0.05);
                let total_depth = marker.size[2].max(0.05);
                let steps = marker.steps.max(1);
                let step_h = total_height / steps as f32;
                let step_d = total_depth / steps as f32;

                entity_cmd.with_children(|parent| {
                    for i in 0..steps {
                        let y = -total_height * 0.5 + step_h * (i as f32 + 0.5);
                        let z = -total_depth * 0.5 + step_d * (i as f32 + 0.5);
                        parent.spawn((
                            Mesh3d(meshes.add(Cuboid::new(width, step_h.max(0.01), step_d.max(0.01)))),
                            MeshMaterial3d(base_material.clone()),
                            Transform::from_xyz(0.0, y, z),
                            GlobalTransform::default(),
                            Visibility::Visible,
                            InheritedVisibility::default(),
                            ViewVisibility::default(),
                        ));
                    }
                });
            }
            DummyPrimitiveKind::Arch => {
                let width = marker.size[0].max(0.05);
                let outer_r = marker.radius.max(0.1);
                let thickness = marker.size[2].max(0.05);
                let segments = marker.segments.max(3);
                let inner_r = (outer_r - thickness).max(0.02);

                entity_cmd.with_children(|parent| {
                    for i in 0..segments {
                        let t0 = (i as f32 / segments as f32) * std::f32::consts::PI;
                        let t1 = ((i + 1) as f32 / segments as f32) * std::f32::consts::PI;
                        let a = (t0 + t1) * 0.5;
                        let center_r = (outer_r + inner_r) * 0.5;
                        let seg_len = (t1 - t0).abs() * center_r;
                        let y = a.sin() * center_r;
                        let z = a.cos() * center_r;
                        let rot = Quat::from_rotation_x(a);

                        parent.spawn((
                            Mesh3d(meshes.add(Cuboid::new(width, thickness, seg_len.max(0.02)))),
                            MeshMaterial3d(base_material.clone()),
                            Transform {
                                translation: Vec3::new(0.0, y, z),
                                rotation: rot,
                                scale: Vec3::ONE,
                            },
                            GlobalTransform::default(),
                            Visibility::Visible,
                            InheritedVisibility::default(),
                            ViewVisibility::default(),
                        ));
                    }
                });
            }
            DummyPrimitiveKind::PointLight
            | DummyPrimitiveKind::SpotLight
            | DummyPrimitiveKind::DirectionalLight
            | DummyPrimitiveKind::FogVolume => {
                entity_cmd.remove::<Mesh3d>();
                entity_cmd.remove::<MeshMaterial3d<StandardMaterial>>();

                if marker.light.is_none() {
                    let default_kind = match marker.kind {
                        DummyPrimitiveKind::SpotLight => core_resources::RuntimeLightKind::Spot,
                        DummyPrimitiveKind::DirectionalLight => {
                            core_resources::RuntimeLightKind::Directional
                        }
                        _ => core_resources::RuntimeLightKind::Point,
                    };
                    if !matches!(marker.kind, DummyPrimitiveKind::FogVolume) {
                        warn!(
                            "[dummy] light entity {:?} missing explicit light config, defaulting to {:?}",
                            entity, default_kind
                        );
                    }
                }
            }
        }
    }
}
