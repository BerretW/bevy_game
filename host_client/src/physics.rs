use avian3d::debug_render::PhysicsDebugPlugin;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::mesh::{Indices, VertexAttributeValues};
use core_drawable::{CollisionShape, DrawableCollision};

pub struct ClientPhysicsPlugin;

impl Plugin for ClientPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default());
        app.add_plugins(PhysicsDebugPlugin::default());
        app.init_resource::<NavMeshSurfaceCache>();
        // Vizualizace colliderů je při startu vypnutá — F3 ji přepíná.
        app.add_systems(Startup, disable_physics_debug_on_start);
        app.add_systems(Update, (attach_or_update_drawable_colliders, rebuild_navmesh_surface_cache, toggle_physics_debug));
    }
}

#[derive(Debug, Clone)]
pub struct NavMeshTriangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
}

#[derive(Resource, Default, Debug, Clone)]
pub struct NavMeshSurfaceCache {
    pub triangles: Vec<NavMeshTriangle>,
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
        let Some(spec) = collider_spec_from_drawable(dc, mesh.is_some()) else {
            ecmd.remove::<Collider>();
            ecmd.remove::<ColliderConstructor>();
            ecmd.remove::<RigidBody>();
            ecmd.remove::<StaticWorldCollider>();
            ecmd.remove::<LockedAxes>();
            continue;
        };

        match spec {
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

        if let Some(locked_axes) = locked_axes_from_drawable(dc) {
            ecmd.insert(locked_axes);
        } else {
            ecmd.remove::<LockedAxes>();
        }

        if dc.friction > 0.0 {
            ecmd.insert(Friction::new(dc.friction));
        }
        if dc.restitution > 0.0 {
            ecmd.insert(Restitution::new(dc.restitution));
        }
    }
}

fn locked_axes_from_drawable(dc: &DrawableCollision) -> Option<LockedAxes> {
    let mut axes = LockedAxes::new();
    let mut any = false;

    if let Some([x, y, z]) = dc.lock_translation {
        if x { axes = axes.lock_translation_x(); any = true; }
        if y { axes = axes.lock_translation_y(); any = true; }
        if z { axes = axes.lock_translation_z(); any = true; }
    }
    if let Some([x, y, z]) = dc.lock_rotation {
        if x { axes = axes.lock_rotation_x(); any = true; }
        if y { axes = axes.lock_rotation_y(); any = true; }
        if z { axes = axes.lock_rotation_z(); any = true; }
    }

    if any { Some(axes) } else { None }
}

enum ColliderSpec {
    Direct(Collider),
    Construct(ColliderConstructor),
}

fn collider_spec_from_drawable(dc: &DrawableCollision, has_mesh: bool) -> Option<ColliderSpec> {
    match dc.shape {
        CollisionShape::Box => {
            let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
            Some(ColliderSpec::Direct(Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0)))
        }
        CollisionShape::Sphere => {
            Some(ColliderSpec::Direct(Collider::sphere(dc.radius.unwrap_or(0.5).max(0.0001))))
        }
        CollisionShape::Capsule => {
            let radius = dc.radius.unwrap_or(0.5).max(0.0001);
            let full_h = dc.height.unwrap_or(radius * 2.0).max(radius * 2.0);
            let body_length = (full_h - radius * 2.0).max(0.0001);
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
                Some(ColliderSpec::Direct(Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0)))
            }
        }
        CollisionShape::Mesh => {
            if has_mesh {
                if dc.is_static {
                    // Trimesh = pouze pro statická tělesa (Avian/Rapier omezení)
                    Some(ColliderSpec::Construct(ColliderConstructor::TrimeshFromMesh))
                } else {
                    // Dynamická tělesa — konvexní obal (trimesh pro dynamic Avian odmítne)
                    Some(ColliderSpec::Construct(ColliderConstructor::ConvexHullFromMesh))
                }
            } else {
                let half = dc.half_extents.unwrap_or(Vec3::splat(0.5));
                Some(ColliderSpec::Direct(Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0)))
            }
        }
        CollisionShape::Navmesh => None,
    }
}

fn rebuild_navmesh_surface_cache(
    mut cache: ResMut<NavMeshSurfaceCache>,
    query: Query<(&DrawableCollision, &GlobalTransform, &Mesh3d)>,
    meshes: Res<Assets<Mesh>>,
) {
    let mut triangles = Vec::new();

    for (dc, transform, mesh3d) in &query {
        if !matches!(dc.shape, CollisionShape::Navmesh) {
            continue;
        }

        let Some(mesh) = meshes.get(mesh3d.id()) else { continue };
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            continue;
        };

        match mesh.indices() {
            Some(Indices::U32(indices)) => {
                for tri in indices.chunks_exact(3) {
                    let [ia, ib, ic] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                    let a = transform.transform_point(Vec3::from(positions[ia]));
                    let b = transform.transform_point(Vec3::from(positions[ib]));
                    let c = transform.transform_point(Vec3::from(positions[ic]));
                    triangles.push(NavMeshTriangle { a, b, c });
                }
            }
            Some(Indices::U16(indices)) => {
                for tri in indices.chunks_exact(3) {
                    let [ia, ib, ic] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                    let a = transform.transform_point(Vec3::from(positions[ia]));
                    let b = transform.transform_point(Vec3::from(positions[ib]));
                    let c = transform.transform_point(Vec3::from(positions[ic]));
                    triangles.push(NavMeshTriangle { a, b, c });
                }
            }
            None => {}
        }
    }

    cache.triangles = triangles;
}
