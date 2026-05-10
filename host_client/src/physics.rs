use avian3d::debug_render::PhysicsDebugPlugin;
use avian3d::prelude::*;
use bevy::prelude::*;
use core_drawable::{CollisionShape, DrawableCollision};

pub struct ClientPhysicsPlugin;

impl Plugin for ClientPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default());
        app.add_plugins(PhysicsDebugPlugin::default());
        // Vizualizace colliderů je při startu vypnutá — F3 ji přepíná.
        app.add_systems(Startup, disable_physics_debug_on_start);
        app.add_systems(Update, (attach_or_update_drawable_colliders, toggle_physics_debug));
    }
}

fn disable_physics_debug_on_start(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PhysicsGizmos>();
    config.enabled = false;
}

fn toggle_physics_debug(
    keys: Res<ButtonInput<KeyCode>>,
    mut store: ResMut<GizmoConfigStore>,
) {
    if keys.just_pressed(KeyCode::F3) {
        let (config, _) = store.config_mut::<PhysicsGizmos>();
        config.enabled = !config.enabled;
        if config.enabled {
            info!("[physics] Collider debug: ON (F3 pro vypnutí)");
        } else {
            info!("[physics] Collider debug: OFF");
        }
    }
}

#[derive(Component, Debug)]
pub struct StaticWorldCollider;

fn attach_or_update_drawable_colliders(
    mut commands: Commands,
    q: Query<(Entity, &DrawableCollision, Option<&Mesh3d>), Or<(Added<DrawableCollision>, Changed<DrawableCollision>)>>,
) {
    for (entity, dc, mesh) in &q {
        let mut ecmd = commands.entity(entity);
        match collider_spec_from_drawable(dc, mesh.is_some()) {
            ColliderSpec::Direct(collider) => {
                ecmd.insert(collider);
                ecmd.remove::<ColliderConstructor>();
            }
            ColliderSpec::Construct(constructor) => {
                ecmd.insert(constructor);
                ecmd.remove::<Collider>();
            }
        }

        if dc.is_static {
            ecmd.insert((RigidBody::Static, StaticWorldCollider));
        } else {
            ecmd.insert(RigidBody::Dynamic);
            ecmd.remove::<StaticWorldCollider>();
        }

        if dc.friction > 0.0 {
            ecmd.insert(Friction::new(dc.friction));
        }
        if dc.restitution > 0.0 {
            ecmd.insert(Restitution::new(dc.restitution));
        }
    }
}

enum ColliderSpec {
    Direct(Collider),
    Construct(ColliderConstructor),
}

fn collider_spec_from_drawable(dc: &DrawableCollision, has_mesh: bool) -> ColliderSpec {
    match dc.shape {
        CollisionShape::Box => {
            let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
            ColliderSpec::Direct(Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0))
        }
        CollisionShape::Sphere => {
            ColliderSpec::Direct(Collider::sphere(dc.radius.unwrap_or(0.5).max(0.0001)))
        }
        CollisionShape::Capsule => {
            let radius = dc.radius.unwrap_or(0.5).max(0.0001);
            let full_h = dc.height.unwrap_or(radius * 2.0).max(radius * 2.0);
            let body_length = (full_h - radius * 2.0).max(0.0001);
            ColliderSpec::Direct(Collider::capsule(radius, body_length))
        }
        CollisionShape::Cylinder => {
            let radius = dc.radius.unwrap_or(0.5).max(0.0001);
            let height = dc.height.unwrap_or(1.0).max(0.0001);
            ColliderSpec::Direct(Collider::cylinder(radius, height))
        }
        CollisionShape::Convex => {
            if has_mesh {
                ColliderSpec::Construct(ColliderConstructor::ConvexHullFromMesh)
            } else {
                let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
                ColliderSpec::Direct(Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0))
            }
        }
        CollisionShape::Mesh => {
            if has_mesh {
                if dc.is_static {
                    // Trimesh = pouze pro statická tělesa (Avian/Rapier omezení)
                    ColliderSpec::Construct(ColliderConstructor::TrimeshFromMesh)
                } else {
                    // Dynamická tělesa — konvexní obal (trimesh pro dynamic Avian odmítne)
                    ColliderSpec::Construct(ColliderConstructor::ConvexHullFromMesh)
                }
            } else {
                let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
                ColliderSpec::Direct(Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0))
            }
        }
    }
}
