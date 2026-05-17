use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use core_drawable::{CollisionShape, DrawableCollision};

#[allow(dead_code)]
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

pub(super) fn rebuild_navmesh_surface_cache(
    mut cache: ResMut<NavMeshSurfaceCache>,
    query: Query<(&DrawableCollision, &GlobalTransform, &Mesh3d)>,
    meshes: Res<Assets<Mesh>>,
) {
    let mut triangles = Vec::new();

    for (drawable_collision, transform, mesh3d) in &query {
        if !matches!(drawable_collision.shape, CollisionShape::Navmesh) {
            continue;
        }

        let Some(mesh) = meshes.get(mesh3d.id()) else {
            continue;
        };
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
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
