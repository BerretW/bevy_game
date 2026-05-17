use avian3d::prelude::*;
use bevy::prelude::*;
use core_resources::{DummyObjectMarker, StairsCollider};

use super::StaticWorldCollider;

#[derive(Component, Debug)]
pub(super) struct DummyGeneratedCollider;

#[derive(Component, Debug)]
pub(crate) struct DummyStairsIkSurface;

pub(super) fn clear_generated_dummy_colliders(
    entity: Entity,
    commands: &mut Commands,
    children_q: &Query<&Children>,
    generated_q: &Query<(), With<DummyGeneratedCollider>>,
) {
    let Ok(children) = children_q.get(entity) else {
        return;
    };
    for child in children.iter() {
        if generated_q.get(child).is_ok() {
            commands.entity(child).despawn();
        }
    }
}

pub(super) fn attach_stairs_dummy_colliders(
    entity: Entity,
    marker: &DummyObjectMarker,
    commands: &mut Commands,
) {
    let width = marker.size[0].max(0.05);
    let total_height = marker.height.max(0.05);
    let total_depth = marker.size[2].max(0.05);
    let steps = marker.steps.max(1);
    let step_h = total_height / steps as f32;
    let mut slope_angle = -(total_height / total_depth.max(0.001)).atan();
    if marker.collider.stairs_slope_invert {
        slope_angle = -slope_angle;
    }
    let slope_length = (total_height * total_height + total_depth * total_depth).sqrt();

    let mut parent_cmd = commands.entity(entity);
    parent_cmd.remove::<Collider>();
    parent_cmd.remove::<Sensor>();

    if marker.collider.is_static {
        parent_cmd.insert((RigidBody::Static, StaticWorldCollider));
    } else {
        parent_cmd.insert(RigidBody::Dynamic);
        parent_cmd.remove::<StaticWorldCollider>();
    }

    commands.entity(entity).with_children(|parent| {
        let ramp_thickness = step_h.clamp(0.06, 0.24);
        let ramp_clearance_y = (ramp_thickness * 0.5).min(step_h * 0.6);
        let mut ramp = parent.spawn((
            DummyGeneratedCollider,
            Collider::cuboid(width, ramp_thickness, slope_length.max(0.01)),
            Transform {
                translation: Vec3::new(0.0, ramp_clearance_y, 0.0),
                rotation: Quat::from_rotation_x(slope_angle),
                scale: Vec3::ONE,
            },
            GlobalTransform::default(),
        ));

        if marker.collider.friction > 0.0 {
            ramp.insert(Friction::new(marker.collider.friction));
        }
        if marker.collider.restitution > 0.0 {
            ramp.insert(Restitution::new(marker.collider.restitution));
        }

        for step_index in 0..steps {
            let y_top = -total_height * 0.5 + step_h * (step_index as f32 + 1.0);
            let z_center = -total_depth * 0.5
                + (total_depth / steps as f32) * (step_index as f32 + 0.5);

            parent.spawn((
                DummyGeneratedCollider,
                DummyStairsIkSurface,
                Sensor,
                Collider::cuboid(
                    (width * 0.98).max(0.02),
                    0.01,
                    ((total_depth / steps as f32) * 0.96).max(0.02),
                ),
                Transform::from_xyz(0.0, y_top + 0.01, z_center),
                GlobalTransform::default(),
            ));
        }

        if marker.collider.stairs {
            let trigger_thickness = (step_h * 0.2).clamp(0.02, 0.08);
            let trigger_clearance_y = if marker.collider.stairs_clearance_y > 0.0 {
                marker.collider.stairs_clearance_y
            } else {
                (trigger_thickness * 0.5 + step_h * 0.5).max(0.08)
            };

            parent.spawn((
                DummyGeneratedCollider,
                StairsCollider,
                Sensor,
                Collider::cuboid(width, trigger_thickness, slope_length.max(0.01)),
                Transform {
                    translation: Vec3::new(0.0, trigger_clearance_y, 0.0),
                    rotation: Quat::from_rotation_x(slope_angle),
                    scale: Vec3::ONE,
                },
                GlobalTransform::default(),
            ));
        }
    });
}

pub(super) fn raycast_stairs_under_player(
    spatial_query: SpatialQuery,
    mut on_stairs_q: Query<(&GlobalTransform, &mut core_drawable::OnStairs)>,
) {
    for (global_tf, mut stairs) in &mut on_stairs_q {
        let foot_filter = SpatialQueryFilter::from_mask(LayerMask::DEFAULT);
        let left_foot_origin = global_tf.translation() + Vec3::new(-0.10, 0.05, 0.0);
        if let Some(hit) = spatial_query.cast_ray(
            left_foot_origin,
            Dir3::NEG_Y,
            1.5,
            true,
            &foot_filter,
        ) {
            stairs.left_foot_height = left_foot_origin.y - hit.distance;
        } else {
            stairs.left_foot_height = 0.0;
        }

        let right_foot_origin = global_tf.translation() + Vec3::new(0.10, 0.05, 0.0);
        if let Some(hit) = spatial_query.cast_ray(
            right_foot_origin,
            Dir3::NEG_Y,
            1.5,
            true,
            &foot_filter,
        ) {
            stairs.right_foot_height = right_foot_origin.y - hit.distance;
        } else {
            stairs.right_foot_height = 0.0;
        }
    }
}
