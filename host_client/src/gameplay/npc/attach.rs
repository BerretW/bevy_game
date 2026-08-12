use avian3d::prelude::*;
use bevy::prelude::*;
use core_resources::{
    AnimationState, AttachedAnimSets, ModelName, NpcPedMarker,
};
use core_shared::NetTransform;

use super::{NpcCapsuleAttached, NpcMotionTracker, NpcVisualAttached};
use crate::drawable::{AdmSceneRoot, PedPhysicsDef, PedPhysicsRegistry};
use crate::gameplay::resolve_ped_profile_for_model;
use crate::native_assets::PedAdsAnimIndex;

pub(crate) fn attach_capsule_to_new_npcs(
    mut commands: Commands,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    new_npcs: Query<
        (Entity, &ModelName, Option<&core_resources::PedProfileOverride>),
        (With<NpcPedMarker>, Without<NpcCapsuleAttached>),
    >,
) {
    for (entity, model_name, ped_override) in &new_npcs {
        let ped = if let Some(override_comp) = ped_override {
            ped_reg
                .0
                .get(&override_comp.0)
                .and_then(|handle| ped_assets.get(handle))
                .or_else(|| resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets))
        } else {
            resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
        };
        if ped.is_none() {
            warn!("[npc] model NOT FOUND entity={:?}", entity);
        }
        let cap_radius = ped.map(|p| p.capsule.radius).unwrap_or(0.35_f32);
        let cap_height = ped.map(|p| p.capsule.height).unwrap_or(1.80_f32);
        let cap_body = (cap_height - cap_radius * 2.0).max(0.001);
        info!("[npc] capsule attached entity={:?} height={:.2} radius={:.2}", entity, cap_height, cap_radius);

        commands.entity(entity).insert((
            NpcCapsuleAttached,
            RigidBody::Kinematic,
            LockedAxes::new()
                .lock_rotation_x()
                .lock_rotation_y()
                .lock_rotation_z(),
        ));

        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Collider::capsule(cap_radius, cap_body),
                CollisionLayers::new(LayerMask(0b10), LayerMask::ALL),
                Friction::new(0.1).with_combine_rule(CoefficientCombine::Min),
                Restitution::ZERO,
                Transform::from_xyz(0.0, cap_height / 2.0, 0.0),
                GlobalTransform::default(),
            ));
        });
    }
}

pub(crate) fn attach_model_to_new_npcs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    ped_anim_index: Res<PedAdsAnimIndex>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    new_npcs: Query<
        (Entity, &ModelName, Option<&core_resources::PedProfileOverride>, Option<&NetTransform>),
        (With<NpcPedMarker>, Without<NpcVisualAttached>),
    >,
) {
    for (entity, model_name, ped_override, net_tf) in &new_npcs {
        let model_path = format!("models/{}.adm", model_name.0);
        let model_handle = asset_server.load::<crate::drawable::AdmScene>(model_path);

        let attached_anim_sets = ped_anim_index
            .0
            .get(&model_name.0)
            .cloned()
            .or_else(|| ped_override.and_then(|ov| ped_anim_index.0.get(&ov.0).cloned()))
            .or_else(|| {
                resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
                    .and_then(|ped| ped_anim_index.0.get(&ped.identity.model).cloned())
            })
            .unwrap_or_default();

        let spawn_transform = net_tf
            .map(|net_transform| Transform {
                translation: net_transform.translation,
                rotation: net_transform.rotation,
                scale: Vec3::ONE,
            })
            .unwrap_or_default();

        info!("[npc] model attached entity={:?} model={:?}", entity, model_name.0);
        commands.entity(entity).insert((
            NpcVisualAttached,
            NpcMotionTracker {
                prev_pos: spawn_transform.translation,
                current_state: 0,
            },
            spawn_transform,
            GlobalTransform::default(),
            Visibility::Visible,
            InheritedVisibility::default(),
            ViewVisibility::default(),
        ));

        commands.entity(entity).with_children(|parent| {
            let mut child = parent.spawn((
                AdmSceneRoot(model_handle.clone()),
                crate::drawable::DisableDrawableCollisions,
                ModelName(model_name.0.clone()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
                InheritedVisibility::default(),
                ViewVisibility::default(),
            ));

            let ped = if let Some(override_comp) = ped_override {
                ped_reg
                    .0
                    .get(&override_comp.0)
                    .and_then(|handle| ped_assets.get(handle))
                    .or_else(|| {
                        resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
                    })
            } else {
                resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
            };

            let initial_clip = ped
                .and_then(|ped| {
                    let idle = ped.animations.idle.clone();
                    if idle.is_empty() {
                        None
                    } else {
                        Some(idle)
                    }
                })
                .unwrap_or_else(|| "clip:0".to_string());

            child.insert(AnimationState {
                current: Some(initial_clip),
                speed: 1.0,
                looping: true,
                paused: false,
                blend_time: 0.0,
                flags: 1,
            });

            if !attached_anim_sets.is_empty() {
                child.insert(AttachedAnimSets {
                    sets: attached_anim_sets,
                });
            }
        });
    }
}
