use avian3d::prelude::*;
use bevy::prelude::*;
use core_drawable::{CollisionShape, DrawableCollision};
use core_resources::{DummyColliderDef, DummyColliderShape, DummyObjectMarker, DummyPrimitiveKind};

pub(super) enum ColliderSpec {
    Direct(Collider),
    Construct(ColliderConstructor),
}

pub(super) fn has_rigidbody_ancestor(
    entity: Entity,
    child_of_q: &Query<&bevy::ecs::hierarchy::ChildOf>,
    rb_q: &Query<Has<RigidBody>>,
) -> bool {
    let mut current = entity;
    while let Ok(child_of) = child_of_q.get(current) {
        let parent = child_of.parent();
        if rb_q.get(parent).unwrap_or(false) {
            return true;
        }
        current = parent;
    }
    false
}

pub(super) fn topmost_ancestor(
    entity: Entity,
    child_of_q: &Query<&bevy::ecs::hierarchy::ChildOf>,
) -> Entity {
    let mut current = entity;
    while let Ok(child_of) = child_of_q.get(current) {
        current = child_of.parent();
    }
    current
}

pub(super) fn dummy_collider_defaults(marker: &DummyObjectMarker) -> ([f32; 3], f32, f32) {
    match marker.kind {
        DummyPrimitiveKind::Cuboid => (
            [
                marker.size[0].max(0.01),
                marker.size[1].max(0.01),
                marker.size[2].max(0.01),
            ],
            marker.radius.max(0.01),
            marker.height.max(0.01),
        ),
        DummyPrimitiveKind::Cube => {
            let size = marker.size[0].max(0.01);
            ([size, size, size], size * 0.5, size)
        }
        DummyPrimitiveKind::Sphere => {
            let radius = marker.radius.max(0.01);
            ([radius * 2.0, radius * 2.0, radius * 2.0], radius, radius * 2.0)
        }
        DummyPrimitiveKind::Stairs => (
            [
                marker.size[0].max(0.01),
                marker.height.max(0.01),
                marker.size[2].max(0.01),
            ],
            marker.size[0].max(0.01) * 0.5,
            marker.height.max(0.01),
        ),
        DummyPrimitiveKind::Arch => {
            let radius = marker.radius.max(0.01);
            let height = (radius * 2.0).max(0.01);
            (
                [marker.size[0].max(0.01), height, marker.size[2].max(0.01)],
                marker.size[2].max(0.01) * 0.5,
                height,
            )
        }
        DummyPrimitiveKind::PointLight
        | DummyPrimitiveKind::SpotLight
        | DummyPrimitiveKind::DirectionalLight
        | DummyPrimitiveKind::FogVolume => ([0.1, 0.1, 0.1], 0.1, 0.1),
    }
}

pub(super) fn collider_from_dummy_def(
    def: DummyColliderDef,
    default_size: [f32; 3],
    default_radius: f32,
    default_height: f32,
) -> Option<Collider> {
    let sx = def.size[0].max(0.001);
    let sy = def.size[1].max(0.001);
    let sz = def.size[2].max(0.001);
    let radius = def.radius.max(0.001);
    let height = def.height.max(0.001);

    let shape = match def.shape {
        DummyColliderShape::Auto => DummyColliderShape::Box,
        other => other,
    };

    match shape {
        DummyColliderShape::None => None,
        DummyColliderShape::Auto => None,
        DummyColliderShape::Box => Some(Collider::cuboid(
            if def.size == [1.0, 1.0, 1.0] {
                default_size[0]
            } else {
                sx
            },
            if def.size == [1.0, 1.0, 1.0] {
                default_size[1]
            } else {
                sy
            },
            if def.size == [1.0, 1.0, 1.0] {
                default_size[2]
            } else {
                sz
            },
        )),
        DummyColliderShape::Sphere => Some(Collider::sphere(
            if (def.radius - 0.5).abs() < f32::EPSILON {
                default_radius
            } else {
                radius
            },
        )),
        DummyColliderShape::Capsule => {
            let radius = if (def.radius - 0.5).abs() < f32::EPSILON {
                default_radius
            } else {
                radius
            };
            let full_height = if (def.height - 1.0).abs() < f32::EPSILON {
                default_height
            } else {
                height
            };
            let body_length = (full_height - radius * 2.0).max(0.001);
            Some(Collider::capsule(radius, body_length))
        }
        DummyColliderShape::Cylinder => {
            let radius = if (def.radius - 0.5).abs() < f32::EPSILON {
                default_radius
            } else {
                radius
            };
            let height = if (def.height - 1.0).abs() < f32::EPSILON {
                default_height
            } else {
                height
            };
            Some(Collider::cylinder(radius, height))
        }
    }
}

pub(super) fn locked_axes_from_drawable(dc: &DrawableCollision) -> Option<LockedAxes> {
    let mut axes = LockedAxes::new();
    let mut any = false;

    if let Some([x, y, z]) = dc.lock_translation {
        if x {
            axes = axes.lock_translation_x();
            any = true;
        }
        if y {
            axes = axes.lock_translation_y();
            any = true;
        }
        if z {
            axes = axes.lock_translation_z();
            any = true;
        }
    }
    if let Some([x, y, z]) = dc.lock_rotation {
        if x {
            axes = axes.lock_rotation_x();
            any = true;
        }
        if y {
            axes = axes.lock_rotation_y();
            any = true;
        }
        if z {
            axes = axes.lock_rotation_z();
            any = true;
        }
    }

    if any {
        Some(axes)
    } else {
        None
    }
}

pub(super) fn collider_spec_from_drawable(
    dc: &DrawableCollision,
    has_mesh: bool,
) -> Option<ColliderSpec> {
    match dc.shape {
        CollisionShape::Box => {
            let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
            Some(ColliderSpec::Direct(Collider::cuboid(
                half.x * 2.0,
                half.y * 2.0,
                half.z * 2.0,
            )))
        }
        CollisionShape::Sphere => Some(ColliderSpec::Direct(Collider::sphere(
            dc.radius.unwrap_or(0.5).max(0.0001),
        ))),
        CollisionShape::Capsule => {
            let radius = dc.radius.unwrap_or(0.5).max(0.0001);
            let full_height = dc.height.unwrap_or(radius * 2.0).max(radius * 2.0);
            let body_length = (full_height - radius * 2.0).max(0.0001);
            Some(ColliderSpec::Direct(Collider::capsule(radius, body_length)))
        }
        CollisionShape::Cylinder => {
            let radius = dc.radius.unwrap_or(0.5).max(0.0001);
            let height = dc.height.unwrap_or(1.0).max(0.0001);
            Some(ColliderSpec::Direct(Collider::cylinder(radius, height)))
        }
        CollisionShape::Convex => {
            if has_mesh {
                Some(ColliderSpec::Construct(ColliderConstructor::ConvexHullFromMesh))
            } else {
                let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
                Some(ColliderSpec::Direct(Collider::cuboid(
                    half.x * 2.0,
                    half.y * 2.0,
                    half.z * 2.0,
                )))
            }
        }
        CollisionShape::Mesh => {
            if has_mesh {
                if dc.is_static {
                    Some(ColliderSpec::Construct(ColliderConstructor::TrimeshFromMesh))
                } else {
                    Some(ColliderSpec::Construct(ColliderConstructor::ConvexHullFromMesh))
                }
            } else {
                let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
                Some(ColliderSpec::Direct(Collider::cuboid(
                    half.x * 2.0,
                    half.y * 2.0,
                    half.z * 2.0,
                )))
            }
        }
        CollisionShape::Navmesh => None,
    }
}
