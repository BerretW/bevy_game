use avian3d::debug_render::PhysicsDebugPlugin;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::mesh::skinning::SkinnedMesh;
use core_drawable::{CollisionShape, DrawableCollision};
use core_resources::{ColliderObjectMarker, CollisionEnabled, DummyColliderDef, DummyColliderShape, DummyObjectMarker, DummyPrimitiveKind, StairsCollider};

pub struct ClientPhysicsPlugin;

impl Plugin for ClientPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default());
        app.add_plugins(PhysicsDebugPlugin::default());
        app.init_resource::<NavMeshSurfaceCache>();
        // Vizualizace colliderů je při startu vypnutá — F3 ji přepíná.
        app.add_systems(Startup, disable_physics_debug_on_start);
        app.add_systems(Update, (
            attach_or_update_drawable_colliders,
            attach_or_update_dummy_colliders,
            attach_or_update_collider_objects,
            rebuild_navmesh_surface_cache,
            toggle_physics_debug,
            apply_collision_enabled,
            raycast_stairs_under_player,
        ));
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

#[derive(Component, Debug)]
struct DummyGeneratedCollider;

#[derive(Component, Debug)]
pub(crate) struct DummyStairsIkSurface;

/// Projde řetěz ChildOf nahoru a vrátí `true` pokud nějaký předek má `RigidBody`.
/// Zabraňuje tomu, aby child COL_* entity (compound colliders) dostávaly vlastní RigidBody.
fn has_rigidbody_ancestor(
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

fn topmost_ancestor(
    entity: Entity,
    child_of_q: &Query<&bevy::ecs::hierarchy::ChildOf>,
) -> Entity {
    let mut current = entity;
    while let Ok(child_of) = child_of_q.get(current) {
        current = child_of.parent();
    }
    current
}

fn attach_or_update_drawable_colliders(
    mut commands: Commands,
    q: Query<
        (Entity, &DrawableCollision, Option<&Mesh3d>, Has<SkinnedMesh>),
        Or<(Added<DrawableCollision>, Changed<DrawableCollision>)>,
    >,
    child_of_q: Query<&bevy::ecs::hierarchy::ChildOf>,
    rb_q: Query<Has<RigidBody>>,
) {
    for (entity, dc, mesh, is_skinned_collider) in &q {
        let Some(spec) = collider_spec_from_drawable(dc, mesh.is_some()) else {
            let mut ecmd = commands.entity(entity);
            ecmd.remove::<Collider>();
            ecmd.remove::<ColliderConstructor>();
            ecmd.remove::<RigidBody>();
            ecmd.remove::<StaticWorldCollider>();
            ecmd.remove::<LockedAxes>();
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

        // Pokud má entita předka s RigidBody, je součástí compound collideru.
        // Přidáme jen Collider (bez vlastního RigidBody) — Avian ho automaticky
        // přiřadí k nejbližšímu RigidBody předkovi.
        let has_rb_ancestor = has_rigidbody_ancestor(entity, &child_of_q, &rb_q);
        let owns_rigidbody = !has_rb_ancestor && !is_skinned_collider;

        if !has_rb_ancestor && is_skinned_collider {
            // Skinned collider má být součástí jednoho těla na rootu modelu,
            // ne samostatné RB na child uzlu (to roztrhne mesh/collider vazbu).
            let owner = topmost_ancestor(entity, &child_of_q);
            let mut owner_cmd = commands.entity(owner);
            if dc.is_static {
                owner_cmd.insert((RigidBody::Static, StaticWorldCollider));
            } else {
                owner_cmd.insert(RigidBody::Dynamic);
                owner_cmd.remove::<StaticWorldCollider>();
            }
            if let Some(locked_axes) = locked_axes_from_drawable(dc) {
                owner_cmd.insert(locked_axes);
            }
        }

        let mut ecmd = commands.entity(entity);

        if owns_rigidbody {
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
        } else {
            ecmd.remove::<RigidBody>();
            ecmd.remove::<StaticWorldCollider>();
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

fn attach_or_update_dummy_colliders(
    mut commands: Commands,
    q: Query<(Entity, &DummyObjectMarker), Or<(Added<DummyObjectMarker>, Changed<DummyObjectMarker>)>>,
    children_q: Query<&Children>,
    generated_q: Query<(), With<DummyGeneratedCollider>>,
) {
    for (entity, marker) in &q {
        clear_generated_dummy_colliders(entity, &mut commands, &children_q, &generated_q);

        if !marker.collider.enabled || matches!(marker.collider.shape, DummyColliderShape::None) {
            let mut ecmd = commands.entity(entity);
            ecmd.remove::<Collider>();
            ecmd.remove::<RigidBody>();
            ecmd.remove::<StaticWorldCollider>();
            ecmd.remove::<Sensor>();
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
                if (marker.collider.radius - 0.5).abs() < f32::EPSILON { default_radius } else { radius }
            )),
            DummyColliderShape::Capsule => {
                let r = if (marker.collider.radius - 0.5).abs() < f32::EPSILON { default_radius } else { radius };
                let full_h = if (marker.collider.height - 1.0).abs() < f32::EPSILON { default_height } else { height };
                let body_length = (full_h - r * 2.0).max(0.001);
                Some(Collider::capsule(r, body_length))
            }
            DummyColliderShape::Cylinder => {
                let r = if (marker.collider.radius - 0.5).abs() < f32::EPSILON { default_radius } else { radius };
                let h = if (marker.collider.height - 1.0).abs() < f32::EPSILON { default_height } else { height };
                Some(Collider::cylinder(r, h))
            }
        };

        let Some(collider) = collider else { continue };
        let mut ecmd = commands.entity(entity);
        ecmd.insert(collider);
        if marker.collider.is_static {
            ecmd.insert((RigidBody::Static, StaticWorldCollider));
        } else {
            ecmd.insert(RigidBody::Dynamic);
            ecmd.remove::<StaticWorldCollider>();
        }
        if marker.collider.is_trigger {
            ecmd.insert(Sensor);
        } else {
            ecmd.remove::<Sensor>();
        }
        if marker.collider.friction > 0.0 {
            ecmd.insert(Friction::new(marker.collider.friction));
        }
        if marker.collider.restitution > 0.0 {
            ecmd.insert(Restitution::new(marker.collider.restitution));
        }
    }
}

fn attach_or_update_collider_objects(
    mut commands: Commands,
    q: Query<(Entity, &ColliderObjectMarker), Or<(Added<ColliderObjectMarker>, Changed<ColliderObjectMarker>)>>,
) {
    for (entity, marker) in &q {
        let def = marker.collider;
        if !def.enabled || matches!(def.shape, DummyColliderShape::None) {
            let mut ecmd = commands.entity(entity);
            ecmd.remove::<Collider>();
            ecmd.remove::<RigidBody>();
            ecmd.remove::<StaticWorldCollider>();
            ecmd.remove::<Sensor>();
            ecmd.remove::<StairsCollider>();
            continue;
        }

        let Some(collider) = collider_from_dummy_def(def, [1.0, 1.0, 1.0], 0.5, 1.0) else {
            continue;
        };

        let mut ecmd = commands.entity(entity);
        ecmd.insert(collider);

        if def.is_static {
            ecmd.insert((RigidBody::Static, StaticWorldCollider));
        } else {
            ecmd.insert(RigidBody::Dynamic);
            ecmd.remove::<StaticWorldCollider>();
        }

        if def.is_trigger {
            ecmd.insert(Sensor);
        } else {
            ecmd.remove::<Sensor>();
        }

        if def.stairs {
            ecmd.insert(StairsCollider);
        } else {
            ecmd.remove::<StairsCollider>();
        }

        if def.friction > 0.0 {
            ecmd.insert(Friction::new(def.friction));
        }
        if def.restitution > 0.0 {
            ecmd.insert(Restitution::new(def.restitution));
        }
    }
}

fn clear_generated_dummy_colliders(
    entity: Entity,
    commands: &mut Commands,
    children_q: &Query<&Children>,
    generated_q: &Query<(), With<DummyGeneratedCollider>>,
) {
    let Ok(children) = children_q.get(entity) else { return };
    for child in children.iter() {
        if generated_q.get(child).is_ok() {
            commands.entity(child).despawn();
        }
    }
}

fn attach_stairs_dummy_colliders(
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

    commands.entity(entity).with_children(|p| {
        // Fyzický helper: plynulá rampa místo zubatých stupňů.
        // Umožní kapsli hráče plynule vystoupat bez zasekávání o hrany stupňů.
        let ramp_thickness = step_h.clamp(0.06, 0.24);
        let ramp_clearance_y = (ramp_thickness * 0.5).min(step_h * 0.6);
        let mut ramp = p.spawn((
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

        // IK helper: top plochy jednotlivých stupňů jako tenké sensory.
        // Neovlivňují pohyb hráče, ale poskytují přesný "stupňový" profil
        // pro budoucí foot-placement raycasty.
        for i in 0..steps {
            let y_top = -total_height * 0.5 + step_h * (i as f32 + 1.0);
            let z_center = -total_depth * 0.5 + (total_depth / steps as f32) * (i as f32 + 0.5);

            p.spawn((
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
            // Trigger chceme mít nad hranou schodů po celé délce, ne uvnitř mesh objemu.
            let trigger_clearance_y = if marker.collider.stairs_clearance_y > 0.0 {
                marker.collider.stairs_clearance_y
            } else {
                (trigger_thickness * 0.5 + step_h * 0.5).max(0.08)
            };

            p.spawn((
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

fn dummy_collider_defaults(marker: &DummyObjectMarker) -> ([f32; 3], f32, f32) {
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
            let s = marker.size[0].max(0.01);
            ([s, s, s], s * 0.5, s)
        }
        DummyPrimitiveKind::Sphere => {
            let r = marker.radius.max(0.01);
            ([r * 2.0, r * 2.0, r * 2.0], r, r * 2.0)
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
            let r = marker.radius.max(0.01);
            let h = (r * 2.0).max(0.01);
            ([marker.size[0].max(0.01), h, marker.size[2].max(0.01)], marker.size[2].max(0.01) * 0.5, h)
        }
    }
}

fn collider_from_dummy_def(
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
            if def.size == [1.0, 1.0, 1.0] { default_size[0] } else { sx },
            if def.size == [1.0, 1.0, 1.0] { default_size[1] } else { sy },
            if def.size == [1.0, 1.0, 1.0] { default_size[2] } else { sz },
        )),
        DummyColliderShape::Sphere => Some(Collider::sphere(
            if (def.radius - 0.5).abs() < f32::EPSILON { default_radius } else { radius }
        )),
        DummyColliderShape::Capsule => {
            let r = if (def.radius - 0.5).abs() < f32::EPSILON { default_radius } else { radius };
            let full_h = if (def.height - 1.0).abs() < f32::EPSILON { default_height } else { height };
            let body_length = (full_h - r * 2.0).max(0.001);
            Some(Collider::capsule(r, body_length))
        }
        DummyColliderShape::Cylinder => {
            let r = if (def.radius - 0.5).abs() < f32::EPSILON { default_radius } else { radius };
            let h = if (def.height - 1.0).abs() < f32::EPSILON { default_height } else { height };
            Some(Collider::cylinder(r, h))
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

/// Reaguje na `Changed<CollisionEnabled>` na root Lua entitách.
/// Prochází celý strom potomků a přidává nebo odebírá Avian `ColliderDisabled`
/// marker komponent, čímž zapíná/vypíná fyzikální kolize bez destrukce collideru.
fn apply_collision_enabled(
    changed: Query<(Entity, &CollisionEnabled), Changed<CollisionEnabled>>,
    children_q: Query<&Children>,
    has_collider: Query<Has<Collider>>,
    mut commands: Commands,
) {
    for (root, ce) in &changed {
        toggle_colliders_recursive(root, ce.0, &children_q, &has_collider, &mut commands);
    }
}

fn toggle_colliders_recursive(
    entity: Entity,
    enabled: bool,
    children_q: &Query<&Children>,
    has_collider: &Query<Has<Collider>>,
    commands: &mut Commands,
) {
    if has_collider.get(entity).unwrap_or(false) {
        let mut ecmd = commands.entity(entity);
        if enabled {
            ecmd.remove::<ColliderDisabled>();
        } else {
            ecmd.insert(ColliderDisabled);
        }
    }
    if let Ok(children) = children_q.get(entity) {
        for &child in children {
            toggle_colliders_recursive(child, enabled, children_q, has_collider, commands);
        }
    }
}

/// Raycastuje pod nohama hráče a naplňuje `OnStairs` komponentu s výškami terénů.
/// Vyžaduje Avian3d `SpatialQuery` pro raycast detekci.
fn raycast_stairs_under_player(
    spatial_query: SpatialQuery,
    mut on_stairs_q: Query<(&GlobalTransform, &mut core_drawable::OnStairs)>,
) {
    for (global_tf, mut stairs) in &mut on_stairs_q {
        // Raycast pod levou nohou (-X offset)
        let left_foot_origin = global_tf.translation() + Vec3::new(-0.10, 0.05, 0.0);
        if let Some(hit) = spatial_query.cast_ray(
            left_foot_origin,
            Dir3::NEG_Y,
            1.5,  // max distance [m]
            true,
            &SpatialQueryFilter::default(),
        ) {
            // Ukládáme world Y pozici povrchu, NE vzdálenost paprsku.
            // apply_ik_to_skeleton ji používá jako cílovou Y pro nohu.
            stairs.left_foot_height = left_foot_origin.y - hit.distance;
        } else {
            stairs.left_foot_height = 0.0;
        }

        // Raycast pod pravou nohou (+X offset)
        let right_foot_origin = global_tf.translation() + Vec3::new(0.10, 0.05, 0.0);
        if let Some(hit) = spatial_query.cast_ray(
            right_foot_origin,
            Dir3::NEG_Y,
            1.5,
            true,
            &SpatialQueryFilter::default(),
        ) {
            stairs.right_foot_height = right_foot_origin.y - hit.distance;
        } else {
            stairs.right_foot_height = 0.0;
        }
    }
}
