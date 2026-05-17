use avian3d::prelude::*;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use core_drawable::DrawableCollision;
use core_resources::{
    ColliderObjectMarker, DummyColliderShape, DummyObjectMarker, DummyPrimitiveKind, StairsCollider,
};

use super::colliders::{
    ColliderSpec, collider_from_dummy_def, collider_spec_from_drawable,
    dummy_collider_defaults, has_rigidbody_ancestor, locked_axes_from_drawable,
    topmost_ancestor,
};
use super::stairs::{
    DummyGeneratedCollider, attach_stairs_dummy_colliders, clear_generated_dummy_colliders,
};
use super::StaticWorldCollider;

pub(super) fn attach_or_update_drawable_colliders(
    mut commands: Commands,
    q: Query<
        (Entity, &DrawableCollision, Option<&Mesh3d>, Has<SkinnedMesh>),
        Or<(Added<DrawableCollision>, Changed<DrawableCollision>)>,
    >,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    rb_q: Query<Has<RigidBody>>,
) {
    for (entity, drawable_collision, mesh, is_skinned_collider) in &q {
        let Some(spec) = collider_spec_from_drawable(drawable_collision, mesh.is_some()) else {
            let mut entity_commands = commands.entity(entity);
            entity_commands.remove::<Collider>();
            entity_commands.remove::<ColliderConstructor>();
            entity_commands.remove::<RigidBody>();
            entity_commands.remove::<StaticWorldCollider>();
            entity_commands.remove::<LockedAxes>();
            continue;
        };

        match spec {
            ColliderSpec::Direct(collider) => {
                commands.entity(entity).insert(collider);
                commands.entity(entity).remove::<ColliderConstructor>();
            }
            ColliderSpec::Construct(constructor) => {
                commands.entity(entity).insert(constructor);
                commands.entity(entity).remove::<Collider>();
            }
        }

        let has_rb_ancestor = has_rigidbody_ancestor(entity, &child_of_q, &rb_q);
        let owns_rigidbody = !has_rb_ancestor && !is_skinned_collider;

        if !has_rb_ancestor && is_skinned_collider {
            let owner = topmost_ancestor(entity, &child_of_q);
            let mut owner_commands = commands.entity(owner);
            if drawable_collision.is_static {
                owner_commands.insert((RigidBody::Static, StaticWorldCollider));
            } else {
                owner_commands.insert(RigidBody::Dynamic);
                owner_commands.remove::<StaticWorldCollider>();
            }
            if let Some(locked_axes) = locked_axes_from_drawable(drawable_collision) {
                owner_commands.insert(locked_axes);
            }
        }

        let mut entity_commands = commands.entity(entity);
        if owns_rigidbody {
            if drawable_collision.is_static {
                entity_commands.insert((RigidBody::Static, StaticWorldCollider));
            } else {
                entity_commands.insert(RigidBody::Dynamic);
                entity_commands.remove::<StaticWorldCollider>();
            }

            if let Some(locked_axes) = locked_axes_from_drawable(drawable_collision) {
                entity_commands.insert(locked_axes);
            } else {
                entity_commands.remove::<LockedAxes>();
            }
        } else {
            entity_commands.remove::<RigidBody>();
            entity_commands.remove::<StaticWorldCollider>();
            entity_commands.remove::<LockedAxes>();
        }

        if drawable_collision.friction > 0.0 {
            entity_commands.insert(Friction::new(drawable_collision.friction));
        }
        if drawable_collision.restitution > 0.0 {
            entity_commands.insert(Restitution::new(drawable_collision.restitution));
        }
    }
}

pub(super) fn attach_or_update_dummy_colliders(
    mut commands: Commands,
    q: Query<
        (Entity, &DummyObjectMarker),
        Or<(Added<DummyObjectMarker>, Changed<DummyObjectMarker>)>,
    >,
    children_q: Query<&Children>,
    generated_q: Query<(), With<DummyGeneratedCollider>>,
) {
    for (entity, marker) in &q {
        clear_generated_dummy_colliders(entity, &mut commands, &children_q, &generated_q);

        if !marker.collider.enabled || matches!(marker.collider.shape, DummyColliderShape::None) {
            let mut entity_commands = commands.entity(entity);
            entity_commands.remove::<Collider>();
            entity_commands.remove::<RigidBody>();
            entity_commands.remove::<StaticWorldCollider>();
            entity_commands.remove::<Sensor>();
            continue;
        }

        if matches!(marker.kind, DummyPrimitiveKind::Stairs) {
            attach_stairs_dummy_colliders(entity, marker, &mut commands);
            continue;
        }

        let shape = match marker.collider.shape {
            DummyColliderShape::Auto => match marker.kind {
                DummyPrimitiveKind::Sphere => DummyColliderShape::Sphere,
                DummyPrimitiveKind::Arch => DummyColliderShape::Capsule,
                DummyPrimitiveKind::Cuboid
                | DummyPrimitiveKind::Cube
                | DummyPrimitiveKind::Stairs => DummyColliderShape::Box,
                DummyPrimitiveKind::PointLight
                | DummyPrimitiveKind::SpotLight
                | DummyPrimitiveKind::DirectionalLight
                | DummyPrimitiveKind::FogVolume => DummyColliderShape::None,
            },
            other => other,
        };

        let (default_size, default_radius, default_height) = dummy_collider_defaults(marker);
        let sx = marker.collider.size[0].max(0.001);
        let sy = marker.collider.size[1].max(0.001);
        let sz = marker.collider.size[2].max(0.001);
        let radius = marker.collider.radius.max(0.001);
        let height = marker.collider.height.max(0.001);

        let collider = match shape {
            DummyColliderShape::None => None,
            DummyColliderShape::Auto => None,
            DummyColliderShape::Box => Some(Collider::cuboid(
                if marker.collider.size == [1.0, 1.0, 1.0] { default_size[0] } else { sx },
                if marker.collider.size == [1.0, 1.0, 1.0] { default_size[1] } else { sy },
                if marker.collider.size == [1.0, 1.0, 1.0] { default_size[2] } else { sz },
            )),
            DummyColliderShape::Sphere => Some(Collider::sphere(
                if (marker.collider.radius - 0.5).abs() < f32::EPSILON {
                    default_radius
                } else {
                    radius
                },
            )),
            DummyColliderShape::Capsule => {
                let radius = if (marker.collider.radius - 0.5).abs() < f32::EPSILON {
                    default_radius
                } else {
                    radius
                };
                let full_height = if (marker.collider.height - 1.0).abs() < f32::EPSILON {
                    default_height
                } else {
                    height
                };
                let body_length = (full_height - radius * 2.0).max(0.001);
                Some(Collider::capsule(radius, body_length))
            }
            DummyColliderShape::Cylinder => {
                let radius = if (marker.collider.radius - 0.5).abs() < f32::EPSILON {
                    default_radius
                } else {
                    radius
                };
                let height = if (marker.collider.height - 1.0).abs() < f32::EPSILON {
                    default_height
                } else {
                    height
                };
                Some(Collider::cylinder(radius, height))
            }
        };

        let Some(collider) = collider else {
            continue;
        };

        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(collider);
        if marker.collider.is_static {
            entity_commands.insert((RigidBody::Static, StaticWorldCollider));
        } else {
            entity_commands.insert(RigidBody::Dynamic);
            entity_commands.remove::<StaticWorldCollider>();
        }
        if marker.collider.is_trigger {
            entity_commands.insert(Sensor);
        } else {
            entity_commands.remove::<Sensor>();
        }
        if marker.collider.friction > 0.0 {
            entity_commands.insert(Friction::new(marker.collider.friction));
        }
        if marker.collider.restitution > 0.0 {
            entity_commands.insert(Restitution::new(marker.collider.restitution));
        }
    }
}

pub(super) fn attach_or_update_collider_objects(
    mut commands: Commands,
    q: Query<
        (Entity, &ColliderObjectMarker),
        Or<(Added<ColliderObjectMarker>, Changed<ColliderObjectMarker>)>,
    >,
) {
    for (entity, marker) in &q {
        let def = marker.collider;
        if !def.enabled || matches!(def.shape, DummyColliderShape::None) {
            let mut entity_commands = commands.entity(entity);
            entity_commands.remove::<Collider>();
            entity_commands.remove::<RigidBody>();
            entity_commands.remove::<StaticWorldCollider>();
            entity_commands.remove::<Sensor>();
            entity_commands.remove::<StairsCollider>();
            continue;
        }

        let Some(collider) = collider_from_dummy_def(def, [1.0, 1.0, 1.0], 0.5, 1.0) else {
            continue;
        };

        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(collider);

        if def.is_static {
            entity_commands.insert((RigidBody::Static, StaticWorldCollider));
        } else {
            entity_commands.insert(RigidBody::Dynamic);
            entity_commands.remove::<StaticWorldCollider>();
        }

        if def.is_trigger {
            entity_commands.insert(Sensor);
        } else {
            entity_commands.remove::<Sensor>();
        }

        if def.stairs {
            entity_commands.insert(StairsCollider);
        } else {
            entity_commands.remove::<StairsCollider>();
        }

        if def.friction > 0.0 {
            entity_commands.insert(Friction::new(def.friction));
        }
        if def.restitution > 0.0 {
            entity_commands.insert(Restitution::new(def.restitution));
        }
    }
}
