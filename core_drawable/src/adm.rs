use std::collections::HashMap;
use std::io::{Cursor, Read};

use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages};
use bevy::image::{
    CompressedImageFormats,
    ImageAddressMode,
    ImageSampler,
    ImageSamplerDescriptor,
    ImageType,
};
use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};

use core_resources::ModelName;

use crate::manifest::DrawableManifest;
use crate::material::{LayeredEnvMaterial, StandardPbrMaterial, VehicleGlassMaterial};
use crate::registry::{DrawableManifestRegistry, TextureRegistry};
use crate::hook::{build_standard_pbr, build_layered_env, build_vehicle_glass};

// ---------------------------------------------------------------------------
// Konstanty
// ---------------------------------------------------------------------------

const MAGIC: [u8; 4] = *b"ADM\0";
const VERSION: u32 = 1;

const ATTR_POS:    u32 = 1 << 0;
const ATTR_NRM:    u32 = 1 << 1;
const ATTR_TAN:    u32 = 1 << 2;
const ATTR_UV0:    u32 = 1 << 3;
const ATTR_UV1:    u32 = 1 << 4;
const ATTR_MASKS0: u32 = 1 << 5;
const ATTR_MASKS1: u32 = 1 << 6;

fn repeat_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        ..default()
    })
}

// ---------------------------------------------------------------------------
// Asset typy
// ---------------------------------------------------------------------------

#[derive(Asset, TypePath, Debug)]
pub struct AdmScene {
    pub meshes:      Vec<Handle<Mesh>>,
    /// AABB per mesh (center, half_extents) — computed from vertex positions during load.
    /// `None` if the mesh had no position data.
    pub mesh_aabbs:  Vec<Option<(Vec3, Vec3)>>,
    pub nodes:       Vec<AdmNode>,
    pub embedded:    HashMap<String, Handle<Image>>,
    pub source_path: String,
}

#[derive(Debug, Clone)]
pub struct AdmNode {
    pub name:          String,
    pub node_type:     AdmNodeType,
    pub mesh_index:    Option<usize>,
    pub parent_index:  Option<usize>,
    pub material_name: String,
    pub transform:     Mat4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmNodeType {
    Mesh,
    Collision,
    Empty,
}

#[derive(Component)]
pub struct AdmSceneRoot(pub Handle<AdmScene>);

#[derive(Component)]
pub struct AdmSceneSpawned;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AdmError {
    #[error("IO error")]
    Io,
    #[error("Bad magic — not an ADM file")]
    BadMagic,
    #[error("Unsupported ADM version: {0}")]
    BadVersion(u32),
    #[error("Invalid UTF-8 in string")]
    BadUtf8,
    #[error("Image decode error: {0}")]
    ImageDecode(String),
    #[error("Unknown node type: {0}")]
    BadNodeType(u8),
}

// ---------------------------------------------------------------------------
// AssetLoader
// ---------------------------------------------------------------------------

#[derive(Default, TypePath)]
pub struct AdmLoader;

impl AssetLoader for AdmLoader {
    type Asset  = AdmScene;
    type Settings = ();
    type Error  = AdmError;

    async fn load(
        &self,
        reader: &mut dyn bevy::asset::io::Reader,
        _settings: &Self::Settings,
        ctx: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(|_| AdmError::Io)?;
        let source_path = ctx.path().to_string();
        parse_adm(&bytes, ctx, source_path)
    }

    fn extensions(&self) -> &[&str] {
        &["adm"]
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn parse_adm(bytes: &[u8], load_context: &mut LoadContext<'_>, source_path: String) -> Result<AdmScene, AdmError> {
    let mut cur = Cursor::new(bytes);

    // Header
    let magic = read_u8_vec(&mut cur, 4)?;
    if magic.as_slice() != MAGIC {
        return Err(AdmError::BadMagic);
    }

    let version      = read_u32(&mut cur)?;
    if version != VERSION {
        return Err(AdmError::BadVersion(version));
    }

    let mesh_count    = read_u32(&mut cur)? as usize;
    let node_count    = read_u32(&mut cur)? as usize;
    let has_textures  = read_u32(&mut cur)?;

    // Mesh section
    let mut mesh_handles = Vec::with_capacity(mesh_count);
    let mut mesh_aabbs: Vec<Option<(Vec3, Vec3)>> = Vec::with_capacity(mesh_count);
    for _ in 0..mesh_count {
        let (name, mesh, aabb) = parse_mesh(&mut cur)?;
        let label = format!("Mesh/{}", name);
        let handle = load_context.add_labeled_asset(label, mesh);
        mesh_handles.push(handle);
        mesh_aabbs.push(aabb);
    }

    // Node section
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        nodes.push(parse_node(&mut cur)?);
    }

    // Texture section
    let mut embedded = HashMap::new();
    if has_textures == 1 {
        let tex_count = read_u32(&mut cur)? as usize;
        for _ in 0..tex_count {
            let img_name = read_string(&mut cur)?;
            let format_byte = read_u8(&mut cur)?;
            let is_srgb     = read_u8(&mut cur)?;
            let data_len    = read_u32(&mut cur)? as usize;
            let data        = read_u8_vec(&mut cur, data_len)?;

            let image_type = match format_byte {
                1 => ImageType::Extension("jpg"),
                2 => ImageType::Extension("dds"),
                _ => ImageType::Extension("png"),
            };

            // Bevy DDS loader ignoruje DXGI format v headeru a řídí se is_srgb parametrem.
            // Proto je_srgb byte z ADM platí pro všechny formáty včetně DDS.
            let is_srgb_bool = is_srgb != 0;

            let image = Image::from_buffer(
                &data,
                image_type,
                CompressedImageFormats::all(),
                is_srgb_bool,
                repeat_sampler(),
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            ).map_err(|e| AdmError::ImageDecode(e.to_string()))?;

            let label = format!("Texture/{}", img_name);
            let handle = load_context.add_labeled_asset(label, image);
            embedded.insert(img_name, handle);
        }
    }

    Ok(AdmScene {
        meshes: mesh_handles,
        mesh_aabbs,
        nodes,
        embedded,
        source_path,
    })
}

fn parse_mesh(cur: &mut Cursor<&[u8]>) -> Result<(String, Mesh, Option<(Vec3, Vec3)>), AdmError> {
    let name         = read_string(cur)?;
    let vertex_count = read_u32(cur)? as usize;
    let index_count  = read_u32(cur)? as usize;
    let attr_flags   = read_u32(cur)?;

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD);

    let mut mesh_aabb: Option<(Vec3, Vec3)> = None;

    if attr_flags & ATTR_POS != 0 {
        let raw = read_f32_vec(cur, vertex_count * 3)?;
        let positions: Vec<[f32; 3]> = raw.chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();

        // Compute AABB from vertex positions.
        if !positions.is_empty() {
            let mut min = Vec3::splat(f32::MAX);
            let mut max = Vec3::splat(f32::MIN);
            for p in &positions {
                min = min.min(Vec3::from(*p));
                max = max.max(Vec3::from(*p));
            }
            mesh_aabb = Some(((min + max) * 0.5, (max - min) * 0.5));
        }

        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    }

    if attr_flags & ATTR_NRM != 0 {
        let raw = read_f32_vec(cur, vertex_count * 3)?;
        let normals: Vec<[f32; 3]> = raw.chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    }

    if attr_flags & ATTR_TAN != 0 {
        let raw = read_f32_vec(cur, vertex_count * 4)?;
        let tangents: Vec<[f32; 4]> = raw.chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }

    if attr_flags & ATTR_UV0 != 0 {
        let raw = read_f32_vec(cur, vertex_count * 2)?;
        let uvs: Vec<[f32; 2]> = raw.chunks_exact(2)
            .map(|c| [c[0], c[1]])
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }

    if attr_flags & ATTR_UV1 != 0 {
        let raw = read_f32_vec(cur, vertex_count * 2)?;
        let uvs: Vec<[f32; 2]> = raw.chunks_exact(2)
            .map(|c| [c[0], c[1]])
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs);
    }

    if attr_flags & ATTR_MASKS0 != 0 {
        let raw = read_u8_vec(cur, vertex_count * 4)?;
        // Normalize u8 → f32 [0..1] and store as ATTRIBUTE_COLOR (RGBA)
        let colors: Vec<[f32; 4]> = raw.chunks_exact(4)
            .map(|c| [
                c[0] as f32 / 255.0,
                c[1] as f32 / 255.0,
                c[2] as f32 / 255.0,
                c[3] as f32 / 255.0,
            ])
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    } else {
        // Default: no vertex color effects
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vec![[0.0f32, 0.0, 0.0, 0.0]; vertex_count],
        );
    }

    if attr_flags & ATTR_MASKS1 != 0 {
        let raw = read_u8_vec(cur, vertex_count * 4)?;
        // Normalize u8 → f32, store as UV1 (only RG channels used: AO=R, emissive=G)
        let uvs: Vec<[f32; 2]> = raw.chunks_exact(4)
            .map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0])
            .collect();
        // Only insert if UV1 wasn't already set from ATTR_UV1
        if attr_flags & ATTR_UV1 == 0 {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs);
        }
    } else if attr_flags & ATTR_UV1 == 0 {
        // No UV1 and no masks1 — insert neutral default
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_1,
            vec![[1.0f32, 0.0]; vertex_count],
        );
    }

    // Indices
    let index_data = read_u32_vec(cur, index_count)?;
    mesh.insert_indices(Indices::U32(index_data));

    Ok((name, mesh, mesh_aabb))
}

fn parse_node(cur: &mut Cursor<&[u8]>) -> Result<AdmNode, AdmError> {
    let name          = read_string(cur)?;
    let node_type_raw = read_u8(cur)?;
    let mesh_index_i  = read_i32(cur)?;
    let parent_index_i = read_i32(cur)?;
    let material_name = read_string(cur)?;
    let mat_floats    = read_f32_vec(cur, 16)?;

    let node_type = match node_type_raw {
        0 => AdmNodeType::Mesh,
        1 => AdmNodeType::Collision,
        2 => AdmNodeType::Empty,
        other => return Err(AdmError::BadNodeType(other)),
    };

    let mesh_index   = if mesh_index_i >= 0 { Some(mesh_index_i as usize) } else { None };
    let parent_index = if parent_index_i >= 0 { Some(parent_index_i as usize) } else { None };

    // Column-major 4×4 → Mat4
    let transform = Mat4::from_cols_array(&[
        mat_floats[0],  mat_floats[1],  mat_floats[2],  mat_floats[3],
        mat_floats[4],  mat_floats[5],  mat_floats[6],  mat_floats[7],
        mat_floats[8],  mat_floats[9],  mat_floats[10], mat_floats[11],
        mat_floats[12], mat_floats[13], mat_floats[14], mat_floats[15],
    ]);

    Ok(AdmNode {
        name,
        node_type,
        mesh_index,
        parent_index,
        material_name,
        transform,
    })
}

// ---------------------------------------------------------------------------
// Read helpers
// ---------------------------------------------------------------------------

fn read_u8(cur: &mut Cursor<&[u8]>) -> Result<u8, AdmError> {
    let mut buf = [0u8; 1];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    Ok(buf[0])
}

fn read_u16(cur: &mut Cursor<&[u8]>) -> Result<u16, AdmError> {
    let mut buf = [0u8; 2];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32(cur: &mut Cursor<&[u8]>) -> Result<u32, AdmError> {
    let mut buf = [0u8; 4];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_i32(cur: &mut Cursor<&[u8]>) -> Result<i32, AdmError> {
    let mut buf = [0u8; 4];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    Ok(i32::from_le_bytes(buf))
}

fn read_string(cur: &mut Cursor<&[u8]>) -> Result<String, AdmError> {
    let len = read_u16(cur)? as usize;
    let mut buf = vec![0u8; len];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    String::from_utf8(buf).map_err(|_| AdmError::BadUtf8)
}

fn read_f32_vec(cur: &mut Cursor<&[u8]>, count: usize) -> Result<Vec<f32>, AdmError> {
    let mut result = Vec::with_capacity(count);
    let mut buf = [0u8; 4];
    for _ in 0..count {
        cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
        result.push(f32::from_le_bytes(buf));
    }
    Ok(result)
}

fn read_u8_vec(cur: &mut Cursor<&[u8]>, count: usize) -> Result<Vec<u8>, AdmError> {
    let mut buf = vec![0u8; count];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    Ok(buf)
}

fn read_u32_vec(cur: &mut Cursor<&[u8]>, count: usize) -> Result<Vec<u32>, AdmError> {
    let mut result = Vec::with_capacity(count);
    let mut buf = [0u8; 4];
    for _ in 0..count {
        cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
        result.push(u32::from_le_bytes(buf));
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Spawn system
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn spawn_adm_scenes(
    mut default_mat: Local<Option<Handle<StandardMaterial>>>,
    mut commands: Commands,
    query: Query<(Entity, &AdmSceneRoot, &ModelName), Without<AdmSceneSpawned>>,
    adm_assets: Res<Assets<AdmScene>>,
    drawable_reg: Res<DrawableManifestRegistry>,
    manifests: Res<Assets<DrawableManifest>>,
    fallback: Res<crate::hook::DrawableFallbackTextures>,
    mut std_mats: ResMut<Assets<StandardPbrMaterial>>,
    mut env_mats: ResMut<Assets<LayeredEnvMaterial>>,
    mut glass_mats: ResMut<Assets<VehicleGlassMaterial>>,
    mut tex_reg: ResMut<TextureRegistry>,
    mut std_base_mats: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    // Ensure default material exists
    let default_mat_handle = default_mat.get_or_insert_with(|| {
        std_base_mats.add(StandardMaterial::default())
    }).clone();

    for (root_entity, adm_root, model_name) in &query {
        let Some(scene) = adm_assets.get(&adm_root.0) else { continue };

        // Optionally get drawable manifest
        let manifest = drawable_reg.0.get(&model_name.0)
            .and_then(|h| manifests.get(h));

        // Embedded images from the AdmScene itself
        let embedded = &scene.embedded;

        // Spawn node entities
        let mut node_entities: Vec<Entity> = Vec::with_capacity(scene.nodes.len());

        for node in &scene.nodes {
            let (scale, rotation, translation) = node.transform.to_scale_rotation_translation();
            let transform = Transform { translation, rotation, scale };

            let mut entity_cmd = commands.spawn((
                Name::new(node.name.clone()),
                transform,
                GlobalTransform::default(),
                Visibility::default(),
                InheritedVisibility::default(),
                ViewVisibility::default(),
            ));

            match node.node_type {
                AdmNodeType::Mesh => {
                    if let Some(mesh_idx) = node.mesh_index {
                        if let Some(mesh_handle) = scene.meshes.get(mesh_idx) {
                            entity_cmd.insert(Mesh3d(mesh_handle.clone()));

                            // Look up material from manifest
                            let mat_applied = if let Some(manifest) = manifest {
                                if let Some(mat_def) = manifest.materials.get(&node.material_name) {
                                    match mat_def.template.as_str() {
                                        "standard_pbr" => {
                                            let mat = build_standard_pbr(mat_def, embedded, &fallback, &mut tex_reg, &asset_server);
                                            let handle = std_mats.add(mat);
                                            entity_cmd.insert(MeshMaterial3d(handle));
                                            true
                                        }
                                        "layered_env" => {
                                            let mat = build_layered_env(mat_def, embedded, &fallback, &mut tex_reg, &asset_server);
                                            let handle = env_mats.add(mat);
                                            entity_cmd.insert(MeshMaterial3d(handle));
                                            true
                                        }
                                        "vehicle_glass" => {
                                            let mat = build_vehicle_glass(mat_def, embedded, &fallback, &mut tex_reg, &asset_server);
                                            let handle = glass_mats.add(mat);
                                            entity_cmd.insert(MeshMaterial3d(handle));
                                            true
                                        }
                                        _ => false,
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if !mat_applied {
                                entity_cmd.insert(MeshMaterial3d(default_mat_handle.clone()));
                            }
                        }
                    }
                }
                AdmNodeType::Collision => {
                    entity_cmd.insert(Visibility::Hidden);

                    // PŘIDÁNO: Pokud má collider z Blenderu exportovanou geometrii, 
                    // připojíme mu Handle<Mesh>. Fyzikální systém si ho pak přečte.
                    if let Some(mesh_idx) = node.mesh_index {
                        if let Some(mesh_handle) = scene.meshes.get(mesh_idx) {
                            entity_cmd.insert(Mesh3d(mesh_handle.clone()));
                        }
                    }

                    // Build DrawableCollision from manifest...
                    let col = if let Some(manifest) = manifest {
                        if let Some(crate::manifest::EntityDef::COLLISION {
                            shape,
                            half_extents: manifest_he,
                            radius,
                            height,
                            mass,
                            is_static,
                            climbable,
                            ladder,
                            material,
                            friction,
                            restitution,
                            tags,
                            lock_translation,
                            lock_rotation,
                        }) = manifest.entities.get(&node.name)
                        {
                            // Prefer manifest half_extents, then fall back to mesh AABB.
                            let he = manifest_he
                                .map(|v| bevy::math::Vec3::new(v[0], v[1], v[2]))
                                .or_else(|| {
                                    node.mesh_index
                                        .and_then(|mi| scene.mesh_aabbs.get(mi))
                                        .and_then(|ab| ab.as_ref())
                                        .map(|(_, he)| *he)
                                });

                            Some(crate::hook::DrawableCollision {
                                shape: shape.clone(),
                                half_extents: he,
                                radius: *radius,
                                height: *height,
                                mass: *mass,
                                is_static: *is_static,
                                climbable: *climbable,
                                ladder: *ladder,
                                material: material.clone(),
                                friction: *friction,
                                restitution: *restitution,
                                tags: tags.clone(),
                                lock_translation: *lock_translation,
                                lock_rotation: *lock_rotation,
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    // Fallback: no manifest — use mesh AABB with sane defaults.
                    let col = col.or_else(|| {
                        let he = node.mesh_index
                            .and_then(|mi| scene.mesh_aabbs.get(mi))
                            .and_then(|ab| ab.as_ref())
                            .map(|(_, he)| *he);
                        Some(crate::hook::DrawableCollision {
                            shape: crate::manifest::CollisionShape::Box,
                            half_extents: he,
                            radius: None,
                            height: None,
                            mass: 0.0,
                            is_static: true,
                            climbable: false,
                            ladder: false,
                            material: crate::manifest::CollisionMaterial::Concrete,
                            friction: 0.5,
                            restitution: 0.2,
                            tags: vec![],
                            lock_translation: None,
                            lock_rotation: None,
                        })
                    });

                    if let Some(col) = col {
                        entity_cmd.insert(col);
                    }
                }
                AdmNodeType::Empty => {}
            }

            let entity_id = entity_cmd.id();
            node_entities.push(entity_id);
        }

        // Set up parent-child relationships
        for (i, node) in scene.nodes.iter().enumerate() {
            if let Some(parent_idx) = node.parent_index {
                if parent_idx < node_entities.len() {
                    let child_entity  = node_entities[i];
                    let parent_entity = node_entities[parent_idx];
                    commands.entity(child_entity).insert(bevy::ecs::hierarchy::ChildOf(parent_entity));
                }
            } else {
                // Root node — parent to the AdmSceneRoot entity
                let child_entity = node_entities[i];
                commands.entity(child_entity).insert(bevy::ecs::hierarchy::ChildOf(root_entity));
            }
        }

        // Mark as spawned
        commands.entity(root_entity).insert(AdmSceneSpawned);
    }
}
