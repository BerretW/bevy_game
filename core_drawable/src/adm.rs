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
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};

use core_resources::{AdsSocketMap, AnimationState, EntityHandle, LocalEventBus, ModelName};

use crate::manifest::DrawableManifest;
use crate::material::{LayeredEnvMaterial, StandardPbrMaterial, VehicleGlassMaterial};
use crate::registry::{DrawableManifestRegistry, TextureRegistry};
use crate::hook::{
    build_standard_pbr,
    build_layered_env,
    build_vehicle_glass,
    classify_ads_node_name,
    AdsNodeKind,
    AdsSocket,
    DisableDrawableCollisions,
};

// ---------------------------------------------------------------------------
// Konstanty
// ---------------------------------------------------------------------------

const MAGIC: [u8; 4] = *b"ADM\0";
const VERSION_V1: u32 = 1;
const VERSION_V2: u32 = 2;
const VERSION_V3: u32 = 3;
const VERSION_V4: u32 = 4;

const ATTR_POS:    u32 = 1 << 0;
const ATTR_NRM:    u32 = 1 << 1;
const ATTR_TAN:    u32 = 1 << 2;
const ATTR_UV0:    u32 = 1 << 3;
const ATTR_UV1:    u32 = 1 << 4;
const ATTR_MASKS0: u32 = 1 << 5;
const ATTR_MASKS1: u32 = 1 << 6;
const ATTR_JOINT_INDICES: u32 = 1 << 7;
const ATTR_JOINT_WEIGHTS: u32 = 1 << 8;

fn repeat_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        ..default()
    })
}

fn mesh_uses_skinning(mesh: &Mesh) -> bool {
    mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_some()
        && mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some()
}

fn compute_node_global_bind_transforms(nodes: &[AdmNode]) -> Vec<Mat4> {
    let mut globals = Vec::with_capacity(nodes.len());
    for node in nodes {
        let global = if let Some(parent_idx) = node.parent_index {
            globals[parent_idx] * node.transform
        } else {
            node.transform
        };
        globals.push(global);
    }
    globals
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
    pub animations:  Vec<AdmAnimationClip>,
    pub blend_spaces: Vec<AdmBlendSpace>,
    pub embedded:    HashMap<String, Handle<Image>>,
    pub source_path: String,
}

#[derive(Debug, Clone)]
pub struct AdmAnimationClip {
    pub name: String,
    pub duration: f32,
    pub tracks: Vec<AdmAnimationTrack>,
    pub notifies: Vec<AdmAnimationNotify>,
}

#[derive(Debug, Clone)]
pub struct AdmAnimationNotify {
    pub time: f32,
    pub name: String,
}

/// Blend Space — míchání více klipů podle 2D/1D vektoru pohybu.
#[derive(Debug, Clone)]
pub struct AdmBlendSpace {
    pub name: String,
    pub kind: AdmBlendSpaceKind,
    pub clips: Vec<AdmBlendSpaceClip>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmBlendSpaceKind {
    OneD,
    TwoD,
}

#[derive(Debug, Clone)]
pub struct AdmBlendSpaceClip {
    pub name: String,
    pub position: Vec2,  // 1D: (x, _), 2D: (x, y)
}

#[derive(Debug, Clone)]
pub struct AdmAnimationTrack {
    pub node_name: String,
    pub flags: u32,
    pub keys: Vec<AdmKeyframe>,
}

#[derive(Debug, Clone)]
pub struct AdmKeyframe {
    pub time: f32,
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
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
    Bone,
    Socket,
    IkTarget,
    Mechanical,
}

#[derive(Component)]
pub struct AdmSceneRoot(pub Handle<AdmScene>);

#[derive(Component)]
pub struct AdmSceneSpawned;

#[derive(Component, Default)]
pub struct AdmNodeEntityMap(pub HashMap<String, Entity>);

#[derive(Component, Default)]
pub struct AdmAnimationPlayback {
    pub active_clip: Option<String>,
    pub time: f32,
    pub previous_clip: Option<String>,
    pub previous_time: f32,
    pub blend_timer: f32,
    pub total_blend_time: f32,
}

#[derive(Message, Debug, Clone)]
pub struct AdmAnimNotifyEvent {
    pub root_entity: Entity,
    pub handle: u64,
    pub clip_name: String,
    pub notify_name: String,
}

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
    if version != VERSION_V1 && version != VERSION_V2 && version != VERSION_V3 && version != VERSION_V4 {
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

    // ADM v2 optional animation section.
    let animations = if version >= VERSION_V2 && (cur.position() as usize) < bytes.len() {
        parse_animations(&mut cur, version)?
    } else {
        Vec::new()
    };

    Ok(AdmScene {
        meshes: mesh_handles,
        mesh_aabbs,
        nodes,
        animations,
        blend_spaces: Vec::new(),  // TODO: implementovat blend space parsing
        embedded,
        source_path,
    })
}

fn parse_animations(cur: &mut Cursor<&[u8]>, version: u32) -> Result<Vec<AdmAnimationClip>, AdmError> {
    let clip_count = read_u32(cur)? as usize;
    let mut clips = Vec::with_capacity(clip_count);

    for _ in 0..clip_count {
        let name = read_string(cur)?;
        let duration = read_f32(cur)?;
        let track_count = read_u32(cur)? as usize;
        let mut tracks = Vec::with_capacity(track_count);

        for _ in 0..track_count {
            let node_name = read_string(cur)?;
            let flags = if version >= VERSION_V3 { read_u32(cur)? } else { 1 };
            let key_count = read_u32(cur)? as usize;
            let mut keys = Vec::with_capacity(key_count);

            for _ in 0..key_count {
                let time = read_f32(cur)?;
                let pos = Vec3::new(read_f32(cur)?, read_f32(cur)?, read_f32(cur)?);
                let rot = Quat::from_xyzw(read_f32(cur)?, read_f32(cur)?, read_f32(cur)?, read_f32(cur)?);
                let scale = Vec3::new(read_f32(cur)?, read_f32(cur)?, read_f32(cur)?);
                keys.push(AdmKeyframe { time, pos, rot, scale });
            }

            tracks.push(AdmAnimationTrack { node_name, flags, keys });
        }

        let mut notifies = Vec::new();
        if version >= VERSION_V4 {
            let notify_count = read_u32(cur)? as usize;
            notifies.reserve(notify_count);
            for _ in 0..notify_count {
                let notify_time = read_f32(cur)?;
                let notify_name = read_string(cur)?;
                notifies.push(AdmAnimationNotify {
                    time: notify_time,
                    name: notify_name,
                });
            }
        }

        clips.push(AdmAnimationClip {
            name,
            duration,
            tracks,
            notifies,
        });
    }

    Ok(clips)
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

    // Skinning: optional 4-joint weights per vertex (ADM v1.2+).
    if attr_flags & ATTR_JOINT_INDICES != 0 {
        let raw = read_u16_vec(cur, vertex_count * 4)?;
        let joints: Vec<[u16; 4]> = raw.chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
    }
    if attr_flags & ATTR_JOINT_WEIGHTS != 0 {
        let raw = read_f32_vec(cur, vertex_count * 4)?;
        let weights: Vec<[f32; 4]> = raw.chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
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
        3 => AdmNodeType::Bone,
        4 => AdmNodeType::Socket,
        5 => AdmNodeType::IkTarget,
        6 => AdmNodeType::Mechanical,
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

fn read_f32(cur: &mut Cursor<&[u8]>) -> Result<f32, AdmError> {
    let mut buf = [0u8; 4];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    Ok(f32::from_le_bytes(buf))
}

fn read_u8_vec(cur: &mut Cursor<&[u8]>, count: usize) -> Result<Vec<u8>, AdmError> {
    let mut buf = vec![0u8; count];
    cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
    Ok(buf)
}

fn read_u16_vec(cur: &mut Cursor<&[u8]>, count: usize) -> Result<Vec<u16>, AdmError> {
    let mut result = Vec::with_capacity(count);
    let mut buf = [0u8; 2];
    for _ in 0..count {
        cur.read_exact(&mut buf).map_err(|_| AdmError::Io)?;
        result.push(u16::from_le_bytes(buf));
    }
    Ok(result)
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
    query: Query<
        (Entity, &AdmSceneRoot, &ModelName, Option<&DisableDrawableCollisions>),
        Without<AdmSceneSpawned>,
    >,
    adm_assets: Res<Assets<AdmScene>>,
    mesh_assets: Res<Assets<Mesh>>,
    drawable_reg: Res<DrawableManifestRegistry>,
    manifests: Res<Assets<DrawableManifest>>,
    fallback: Res<crate::hook::DrawableFallbackTextures>,
    mut std_mats: ResMut<Assets<StandardPbrMaterial>>,
    mut env_mats: ResMut<Assets<LayeredEnvMaterial>>,
    mut glass_mats: ResMut<Assets<VehicleGlassMaterial>>,
    mut tex_reg: ResMut<TextureRegistry>,
    mut std_base_mats: ResMut<Assets<StandardMaterial>>,
    mut inverse_bindposes_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    asset_server: Res<AssetServer>,
) {
    // Ensure default material exists
    let default_mat_handle = default_mat.get_or_insert_with(|| {
        std_base_mats.add(StandardMaterial::default())
    }).clone();

    for (root_entity, adm_root, model_name, disable_collisions) in &query {
        let Some(scene) = adm_assets.get(&adm_root.0) else { continue };

        // Optionally get drawable manifest
        let manifest = drawable_reg.0.get(&model_name.0)
            .and_then(|h| manifests.get(h));

        // Embedded images from the AdmScene itself
        let embedded = &scene.embedded;
        let global_bind_transforms = compute_node_global_bind_transforms(&scene.nodes);

        // Spawn node entities
        let mut node_entities: Vec<Entity> = Vec::with_capacity(scene.nodes.len());
        let mut sockets = AdsSocketMap::default();
        let mut node_map = AdmNodeEntityMap::default();

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

            let node_kind = classify_ads_node_name(&node.name);
            entity_cmd.insert(node_kind);

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

                    if disable_collisions.is_some() {
                        let entity_id = entity_cmd.id();
                        node_entities.push(entity_id);
                        continue;
                    }

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

                    // Kolize MUSÍ být definované v .drawable manifestu — žádná auto-generace.
                    if let Some(col) = col {
                        entity_cmd.insert(col);
                    } else if node.node_type == AdmNodeType::Collision {
                        // COLLISION uzel bez manifest definice = chyba v authoringu
                        warn!(
                            "[drawable] ADM node '{}' je COLLISION ale chybí definice v .drawable manifestu",
                            node.name
                        );
                    }
                }
                AdmNodeType::Empty | AdmNodeType::Bone | AdmNodeType::IkTarget | AdmNodeType::Mechanical => {}
                AdmNodeType::Socket => {
                    entity_cmd.insert(Visibility::Hidden);
                }
            }

            let entity_id = entity_cmd.id();
            node_map.0.insert(node.name.clone(), entity_id);
            if node.name.starts_with("SOC_") || node.node_type == AdmNodeType::Socket || node_kind == AdsNodeKind::Socket {
                sockets.0.insert(node.name.clone(), entity_id);
                commands.entity(entity_id).insert(AdsSocket { name: node.name.clone() });
            }
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

        let bone_node_indices: Vec<usize> = scene.nodes.iter().enumerate()
            .filter_map(|(index, node)| (node.node_type == AdmNodeType::Bone).then_some(index))
            .collect();

        if !bone_node_indices.is_empty() {
            let joint_entities: Vec<Entity> = bone_node_indices.iter()
                .map(|&index| node_entities[index])
                .collect();

            for (node_index, node) in scene.nodes.iter().enumerate() {
                let Some(mesh_index) = node.mesh_index else { continue };
                if node.node_type != AdmNodeType::Mesh && node.node_type != AdmNodeType::Collision {
                    continue;
                }
                let Some(mesh_handle) = scene.meshes.get(mesh_index) else { continue };
                let Some(mesh) = mesh_assets.get(mesh_handle) else { continue };
                if !mesh_uses_skinning(mesh) {
                    continue;
                }

                let mesh_global = global_bind_transforms[node_index];
                let inverse_bindposes = bone_node_indices.iter()
                    .map(|&bone_index| global_bind_transforms[bone_index].inverse() * mesh_global)
                    .collect::<Vec<_>>();
                let inverse_bindposes = inverse_bindposes_assets.add(
                    SkinnedMeshInverseBindposes::from(inverse_bindposes)
                );

                commands.entity(node_entities[node_index]).insert(SkinnedMesh {
                    inverse_bindposes,
                    joints: joint_entities.clone(),
                });
            }
        }

        // Mark as spawned
        let mut root_cmd = commands.entity(root_entity);
        root_cmd.insert(AdmSceneSpawned);
        root_cmd.insert(node_map);
        root_cmd.insert(AdmAnimationPlayback::default());
        if !sockets.0.is_empty() {
            root_cmd.insert(sockets);
        }
    }
}

pub fn apply_adm_animations(
    time: Res<Time>,
    mut roots: Query<(
        Entity,
        &AdmSceneRoot,
        Option<&EntityHandle>,
        &AnimationState,
        &AdmNodeEntityMap,
        &mut AdmAnimationPlayback,
    ), With<AdmSceneSpawned>>,
    adm_assets: Res<Assets<AdmScene>>,
    mut notify_events: MessageWriter<AdmAnimNotifyEvent>,
    mut transforms: Query<&mut Transform>,
) {
    for (root_entity, scene_root, entity_handle, anim_state, node_map, mut playback) in &mut roots {
        let Some(clip_name) = anim_state.current.as_ref() else { continue };
        let Some(scene) = adm_assets.get(&scene_root.0) else { continue };
        if scene.animations.is_empty() { continue; }

        let clip_idx = resolve_clip_index(scene, clip_name);
        let Some(clip_idx) = clip_idx else { continue };
        let clip = &scene.animations[clip_idx];

        let clip_prev_time = if playback.active_clip.as_deref() == Some(&clip.name) {
            playback.time
        } else {
            0.0
        };

        if playback.active_clip.as_deref() != Some(&clip.name) {
            let prev_clip_name = playback.active_clip.clone();
            let prev_time = playback.time;

            // Při přechodu zachovej fázi (0..1) předchozího klipu, aby blend fungoval
            // plynule i při přepnutí uprostřed kroku/běhu.
            let phase_aligned_time = prev_clip_name
                .as_deref()
                .and_then(|prev_name| resolve_clip_index(scene, prev_name))
                .and_then(|prev_idx| {
                    let prev_duration = scene.animations[prev_idx].duration;
                    if prev_duration <= 0.0 || clip.duration <= 0.0 {
                        return None;
                    }

                    let phase = if anim_state.looping {
                        (prev_time / prev_duration).rem_euclid(1.0)
                    } else {
                        (prev_time / prev_duration).clamp(0.0, 1.0)
                    };
                    Some(phase * clip.duration)
                });

            playback.previous_clip = prev_clip_name;
            playback.previous_time = prev_time;
            playback.active_clip = Some(clip.name.clone());
            playback.time = phase_aligned_time.unwrap_or(0.0);
            playback.blend_timer = 0.0;
            playback.total_blend_time = anim_state.blend_time.max(0.0);
            if playback.total_blend_time <= 0.0 {
                playback.previous_clip = None;
            }
        }

        if !anim_state.paused {
            playback.time += time.delta_secs() * anim_state.speed.max(0.0);
            if clip.duration > 0.0 {
                if anim_state.looping {
                    playback.time = playback.time.rem_euclid(clip.duration);
                } else {
                    playback.time = playback.time.min(clip.duration);
                }
            }

            if playback.previous_clip.is_some() && playback.total_blend_time > 0.0 {
                playback.blend_timer += time.delta_secs();
                if let Some(prev_name) = playback.previous_clip.as_deref() {
                    if let Some(prev_idx) = resolve_clip_index(scene, prev_name) {
                        let prev_clip = &scene.animations[prev_idx];
                        playback.previous_time += time.delta_secs() * anim_state.speed.max(0.0);
                        if prev_clip.duration > 0.0 {
                            if anim_state.looping {
                                playback.previous_time = playback.previous_time.rem_euclid(prev_clip.duration);
                            } else {
                                playback.previous_time = playback.previous_time.min(prev_clip.duration);
                            }
                        }
                    }
                }
            }

            if let Some(handle) = entity_handle.map(|h| h.0) {
                emit_animation_notifies(
                    &mut notify_events,
                    root_entity,
                    handle,
                    clip,
                    clip_prev_time,
                    playback.time,
                    anim_state.looping,
                );
            }
        }

        let blend_alpha = if playback.previous_clip.is_some() && playback.total_blend_time > 0.0 {
            (playback.blend_timer / playback.total_blend_time).clamp(0.0, 1.0)
        } else {
            1.0
        };

        let mut current_tracks: HashMap<&str, &AdmAnimationTrack> = HashMap::new();
        for track in &clip.tracks {
            if animation_track_matches_flags(track.flags, anim_state.flags) {
                current_tracks.insert(track.node_name.as_str(), track);
            }
        }

        let previous_tracks = if blend_alpha < 1.0 {
            playback
                .previous_clip
                .as_deref()
                .and_then(|prev_name| resolve_clip_index(scene, prev_name))
                .map(|prev_idx| {
                    let mut map: HashMap<&str, &AdmAnimationTrack> = HashMap::new();
                    for track in &scene.animations[prev_idx].tracks {
                        if animation_track_matches_flags(track.flags, anim_state.flags) {
                            map.insert(track.node_name.as_str(), track);
                        }
                    }
                    map
                })
        } else {
            None
        };

        for (node_name, entity) in &node_map.0 {
            let current_sample = current_tracks
                .get(node_name.as_str())
                .and_then(|track| sample_track(track, playback.time));

            let previous_sample = previous_tracks
                .as_ref()
                .and_then(|tracks| tracks.get(node_name.as_str()))
                .and_then(|track| sample_track(track, playback.previous_time));

            let sampled = match (current_sample, previous_sample) {
                (Some(current), Some(previous)) if blend_alpha < 1.0 => AdmKeyframe {
                    time: playback.time,
                    pos: previous.pos.lerp(current.pos, blend_alpha),
                    rot: previous.rot.slerp(current.rot, blend_alpha),
                    scale: previous.scale.lerp(current.scale, blend_alpha),
                },
                (Some(current), _) => current,
                (None, Some(previous)) if blend_alpha < 1.0 => previous,
                _ => continue,
            };

            if let Ok(mut tf) = transforms.get_mut(*entity) {
                tf.translation = sampled.pos;
                tf.rotation = sampled.rot;
                tf.scale = sampled.scale;
            }
        }

        if blend_alpha >= 1.0 {
            playback.previous_clip = None;
            playback.blend_timer = 0.0;
            playback.total_blend_time = 0.0;
        }
    }
}

pub fn forward_adm_anim_notifies_to_local_bus(
    mut notify_events: MessageReader<AdmAnimNotifyEvent>,
    local_bus: Option<Res<LocalEventBus>>,
) {
    let Some(local_bus) = local_bus else { return };
    for evt in notify_events.read() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "handle": evt.handle,
            "clip_name": evt.clip_name,
            "notify_name": evt.notify_name,
        }))
        .unwrap_or_default();
        local_bus.push("onAnimNotify".to_string(), payload);
    }
}

fn emit_animation_notifies(
    notify_events: &mut MessageWriter<AdmAnimNotifyEvent>,
    root_entity: Entity,
    handle: u64,
    clip: &AdmAnimationClip,
    previous_time: f32,
    current_time: f32,
    looping: bool,
) {
    if clip.notifies.is_empty() {
        return;
    }
    if !looping && current_time <= previous_time {
        return;
    }

    let loop_wrapped = looping && clip.duration > 0.0 && current_time < previous_time;

    for notify in &clip.notifies {
        let should_fire = if loop_wrapped {
            // Loop wrap: frame přešel přes clip.duration → rem_euclid → 0.
            // Notifies s time > previous_time (tj. v části [previous_time, clip.duration])
            // nebo s time <= current_time (tj. v části [0, current_time]).
            // Notifies umístěné na nebo za clip.duration (>= duration) se vždy odpalují při wrapping.
            let at_or_beyond_end = clip.duration > 0.0 && notify.time >= clip.duration;
            at_or_beyond_end || notify.time > previous_time || notify.time <= current_time
        } else {
            notify.time > previous_time && notify.time <= current_time
        };

        if should_fire {
            notify_events.write(AdmAnimNotifyEvent {
                root_entity,
                handle,
                clip_name: clip.name.clone(),
                notify_name: notify.name.clone(),
            });
        }
    }
}

fn animation_track_matches_flags(track_flags: u32, animation_flags: u32) -> bool {
    if animation_flags == 1 || track_flags == 1 {
        return true;
    }
    (track_flags & animation_flags) != 0
}

fn resolve_clip_index(scene: &AdmScene, selector: &str) -> Option<usize> {
    if let Some(rest) = selector.strip_prefix("clip:") {
        if let Ok(i) = rest.parse::<usize>() {
            if i < scene.animations.len() {
                return Some(i);
            }
        }
    }
    if let Some(rest) = selector.strip_prefix("anim:") {
        if let Ok(i) = rest.parse::<usize>() {
            if i < scene.animations.len() {
                return Some(i);
            }
        }
    }
    if let Ok(i) = selector.parse::<usize>() {
        if i < scene.animations.len() {
            return Some(i);
        }
    }

    scene.animations.iter().position(|c| c.name == selector)
}

fn sample_track(track: &AdmAnimationTrack, time: f32) -> Option<AdmKeyframe> {
    if track.keys.is_empty() {
        return None;
    }
    if track.keys.len() == 1 {
        return Some(track.keys[0].clone());
    }

    if time <= track.keys[0].time {
        return Some(track.keys[0].clone());
    }

    let last = track.keys.len() - 1;
    if time >= track.keys[last].time {
        return Some(track.keys[last].clone());
    }

    for w in track.keys.windows(2) {
        let a = &w[0];
        let b = &w[1];
        if time >= a.time && time <= b.time {
            let span = (b.time - a.time).max(1e-6);
            let alpha = ((time - a.time) / span).clamp(0.0, 1.0);
            return Some(AdmKeyframe {
                time,
                pos: a.pos.lerp(b.pos, alpha),
                rot: a.rot.slerp(b.rot, alpha),
                scale: a.scale.lerp(b.scale, alpha),
            });
        }
    }

    Some(track.keys[last].clone())
}
