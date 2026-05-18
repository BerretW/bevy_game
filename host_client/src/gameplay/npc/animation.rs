use bevy::prelude::*;
use core_resources::{AnimationState, ModelName, NpcPedMarker};

use super::NpcMotionTracker;
use crate::drawable::{AdmSceneRoot, PedPhysicsDef, PedPhysicsRegistry};
use crate::gameplay::resolve_ped_profile_for_model;

pub(crate) fn drive_npc_animations(
    time: Res<Time>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    mut npcs: Query<
        (
            &Transform,
            &ModelName,
            Option<&core_resources::PedProfileOverride>,
            &mut NpcMotionTracker,
            &Children,
        ),
        With<NpcPedMarker>,
    >,
    mut adm_roots: Query<&mut AnimationState, With<AdmSceneRoot>>,
) {
    let dt = time.delta_secs().max(0.001);

    for (transform, model_name, ped_override, mut tracker, children) in &mut npcs {
        let speed = transform.translation.distance(tracker.prev_pos) / dt;
        tracker.prev_pos = transform.translation;

        let ped = if let Some(override_comp) = ped_override {
            ped_reg
                .0
                .get(&override_comp.0)
                .and_then(|handle| ped_assets.get(handle))
                .or_else(|| resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets))
        } else {
            resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets)
        };

        let idle_threshold = ped.map(|p| p.movement.idle_threshold).unwrap_or(0.10);
        let run_threshold = ped.map(|p| p.movement.run_speed * 0.6).unwrap_or(3.0);

        let new_state = if speed <= idle_threshold {
            0u8
        } else if speed < run_threshold {
            1u8
        } else {
            2u8
        };

        if new_state == tracker.current_state {
            continue;
        }
        tracker.current_state = new_state;

        let clip = ped
            .map(|ped| match new_state {
                1 => ped.animations.walk.clone(),
                2 => ped.animations.run.clone(),
                _ => ped.animations.idle.clone(),
            })
            .unwrap_or_else(|| "clip:0".to_string());

        for child in children.iter() {
            if let Ok(mut anim) = adm_roots.get_mut(child) {
                anim.current = Some(clip.clone());
                anim.blend_time = 0.15;
            }
        }
    }
}
