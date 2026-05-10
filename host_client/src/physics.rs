use bevy::prelude::*;
use core_drawable::{CollisionMaterial, CollisionShape, DrawableCollision};

pub struct ClientPhysicsPlugin;

impl Plugin for ClientPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CollisionWorld>();
        app.add_systems(Update, rebuild_collision_world);
    }
}

#[derive(Resource, Default, Debug)]
pub struct CollisionWorld {
    pub colliders: Vec<WorldCollider>,
}

#[derive(Debug, Clone)]
pub struct WorldCollider {
    pub entity: Entity,
    pub is_static: bool,
    pub climbable: bool,
    pub ladder: bool,
    pub material: CollisionMaterial,
    pub friction: f32,
    pub restitution: f32,
    pub center: Vec3,
    pub half_extents: Vec3,
    pub shape: CollisionShape,
}

fn rebuild_collision_world(
    mut world: ResMut<CollisionWorld>,
    q: Query<(Entity, &DrawableCollision, &GlobalTransform)>,
) {
    world.colliders.clear();
    world.colliders.reserve(q.iter().len());

    for (entity, dc, gt) in &q {
        world.colliders.push(WorldCollider {
            entity,
            is_static: dc.is_static,
            climbable: dc.climbable,
            ladder: dc.ladder,
            material: dc.material.clone(),
            friction: dc.friction,
            restitution: dc.restitution,
            center: gt.translation(),
            half_extents: half_extents_from_drawable(dc),
            shape: dc.shape.clone(),
        });
    }
}

fn half_extents_from_drawable(dc: &DrawableCollision) -> Vec3 {
    // Foundation step: use AABB envelopes for broad-phase until a physics backend is plugged in.
    match dc.shape {
        CollisionShape::Box => dc.half_extents.unwrap_or(Vec3::splat(0.5)),
        CollisionShape::Sphere => {
            let r = dc.radius.unwrap_or(0.5).max(0.0001);
            Vec3::splat(r)
        }
        CollisionShape::Capsule => {
            let radius = dc.radius.unwrap_or(0.5).max(0.0001);
            let full_h = dc.height.unwrap_or(radius * 2.0).max(radius * 2.0);
            Vec3::new(radius, full_h * 0.5, radius)
        }
        CollisionShape::Cylinder => {
            let radius = dc.radius.unwrap_or(0.5).max(0.0001);
            let half_h = dc.height.map(|v| v * 0.5).unwrap_or(0.5).max(0.0001);
            Vec3::new(radius, half_h, radius)
        }
        CollisionShape::Convex | CollisionShape::Mesh => dc.half_extents.unwrap_or(Vec3::splat(0.5)),
    }
}
