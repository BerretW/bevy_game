use std::collections::HashMap;

use bevy::gltf::{Gltf, GltfAssetLabel, GltfMaterialName};
use bevy::light::NotShadowCaster;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::prelude::On;
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::scene::{InstanceId, SceneInstanceReady, SceneSpawner};

use core_resources::ModelName;

use crate::manifest::{CollisionMaterial, CollisionShape, DrawableManifest, EntityDef, MaterialDef, MaterialParams, TextureSource};
use crate::material::{
    DrawableParams,
    LayeredEnvExtension, LayeredEnvMaterial,
    StandardPbrExtension, StandardPbrMaterial,
    VehicleGlassExtension, VehicleGlassMaterial,
};
use crate::registry::{DrawableManifestRegistry, GltfHandleCache, TextureRegistry};

// ---------------------------------------------------------------------------
// Fallback textury
// ---------------------------------------------------------------------------

/// Bílá 1×1 px textura, používaná jako neutrální fallback pro volitelné
/// texture sloty (palette, snow, mb) — garantuje deterministické sampling chování.
#[derive(Resource)]
pub struct DrawableFallbackTextures {
    pub white_1px: Handle<Image>,
}

pub fn setup_fallback_textures(
    mut commands: Commands,
    mut images:   ResMut<Assets<Image>>,
) {
    let white = Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[255, 255, 255, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    commands.insert_resource(DrawableFallbackTextures {
        white_1px: images.add(white),
    });
}

// ---------------------------------------------------------------------------
// Komponenty
// ---------------------------------------------------------------------------

/// Přítomnost na entitě říká drawable systému: zpracuj tuto GLTF scene
/// (vyměň materiály, schovat COL_ uzly).
#[derive(Component)]
pub struct DrawableSpawnIntent {
    pub manifest_handle: Handle<DrawableManifest>,
}

/// Uložené `InstanceId` ze `SceneInstanceReady` triggeru — umožňuje polling
/// i v případě, že manifest se načítá déle než scene samotná.
#[derive(Component, Clone, Copy)]
#[doc(hidden)]
pub struct SceneReadyId(InstanceId);

/// Marker: drawable hooking byl dokončen. Systém entitu přeskočí.
#[derive(Component)]
pub struct DrawableHooked;

/// Runtime kolizní definice odvozená z `.drawable` manifestu.
///
/// Fyzikální backend (Avian/Rapier) může tento komponent převést na skutečný collider.
#[derive(Component, Debug, Clone)]
pub struct DrawableCollision {
    pub shape: CollisionShape,
    pub half_extents: Option<Vec3>,
    pub radius: Option<f32>,
    pub height: Option<f32>,
    pub mass: f32,
    pub is_static: bool,
    pub climbable: bool,
    pub ladder: bool,
    pub material: CollisionMaterial,
    pub friction: f32,
    pub restitution: f32,
    pub tags: Vec<String>,
    pub lock_translation: Option<[bool; 3]>,
    pub lock_rotation: Option<[bool; 3]>,
}

// ---------------------------------------------------------------------------
// Observer: zachytí SceneInstanceReady a uloží InstanceId
// ---------------------------------------------------------------------------

pub fn observe_scene_ready(
    on: On<SceneInstanceReady>,
    mut commands: Commands,
) {
    commands
        .entity(on.entity)
        .insert(SceneReadyId(on.event().instance_id));
}

// ---------------------------------------------------------------------------
// Systém 1: attach_drawable_intent
// ---------------------------------------------------------------------------

/// Detekuje nově spawnované entity s `ModelName`. Pokud je model v registru
/// drawable manifestů, připojí `DrawableSpawnIntent`.
pub fn attach_drawable_intent(
    mut commands: Commands,
    query: Query<(Entity, &ModelName), (Added<ModelName>, Without<DrawableSpawnIntent>)>,
    drawable_reg: Res<DrawableManifestRegistry>,
) {
    for (entity, model_name) in &query {
        if let Some(handle) = drawable_reg.0.get(&model_name.0) {
            commands.entity(entity).insert(DrawableSpawnIntent {
                manifest_handle: handle.clone(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: sestaví name→Handle<Image> mapu ze jmen GLTF images
// ---------------------------------------------------------------------------

/// Prochází `Gltf::source` (raw gltf crate data) a mapuje jméno každé image
/// na handle načtený přes `GltfAssetLabel::Texture(texture_index)`.
/// Vyžaduje načtení modelu s `GltfLoaderSettings::include_source = true`.
///
/// Dvě normalizace:
/// - image_index → texture_index: Bevy labeluje sub-assety jako Texture(tex_idx),
///   nikoli Image(img_idx), takže musíme najít texturu která odkazuje na daný image.
/// - name stem: Blender exportuje image name s příponou ("foo.png"), ale manifest
///   ukládá jen stem ("foo") přes _image_basename(). Strippujeme příponu pro match.
fn build_embedded_image_map(
    gltf: Option<&Gltf>,
    model_path: &str,
    asset_server: &AssetServer,
) -> HashMap<String, Handle<Image>> {
    let mut map = HashMap::new();
    let Some(gltf) = gltf else { return map };
    let Some(source) = gltf.source.as_ref() else { return map };

    // image_index → first texture_index that references it
    let mut image_to_tex: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    for texture in source.textures() {
        image_to_tex
            .entry(texture.source().index())
            .or_insert(texture.index());
    }

    for image in source.images() {
        let Some(name) = image.name() else { continue };
        let Some(&tex_idx) = image_to_tex.get(&image.index()) else { continue };

        // Strip extension so "foo.png" matches manifest entry "foo"
        let stem = std::path::Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name);

        let label = GltfAssetLabel::Texture(tex_idx);
        let path = label.from_asset(model_path.to_string());
        let handle: Handle<Image> = asset_server.load(path);
        map.insert(stem.to_string(), handle);
    }
    map
}

// ---------------------------------------------------------------------------
// Interní store — sdružuje 3 asset kolekce pro předání do process_mesh_node
// ---------------------------------------------------------------------------

struct MaterialStores<'a> {
    std_pbr: &'a mut Assets<StandardPbrMaterial>,
    layered: &'a mut Assets<LayeredEnvMaterial>,
    glass:   &'a mut Assets<VehicleGlassMaterial>,
}

// ---------------------------------------------------------------------------
// Systém 2: hook_drawable_scenes
// ---------------------------------------------------------------------------

/// Čeká, dokud není scene spawned A manifest načten, pak projde uzly a aplikuje
/// drawable definice (materiály, schování COL_ uzlů, vertex color sanitizace).
#[allow(clippy::too_many_arguments)]
pub fn hook_drawable_scenes(
    mut commands: Commands,
    scene_spawner: Res<SceneSpawner>,
    manifests: Res<Assets<DrawableManifest>>,
    gltf_cache: Res<GltfHandleCache>,
    gltfs: Res<Assets<Gltf>>,
    fallback: Res<DrawableFallbackTextures>,
    pending: Query<
        (Entity, &SceneReadyId, &DrawableSpawnIntent, &ModelName),
        Without<DrawableHooked>,
    >,
    names: Query<&Name>,
    mat_names: Query<&GltfMaterialName>,
    node_query: Query<(Option<&Mesh3d>, Option<&Children>), Or<(With<Mesh3d>, With<Children>)>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut std_materials: ResMut<Assets<StandardPbrMaterial>>,
    mut env_materials: ResMut<Assets<LayeredEnvMaterial>>,
    mut glass_materials: ResMut<Assets<VehicleGlassMaterial>>,
    mut texture_reg: ResMut<TextureRegistry>,
    asset_server: Res<AssetServer>,
) {
    for (root_entity, scene_ready, intent, model_name) in &pending {
        // Čekáme na manifest
        let Some(manifest) = manifests.get(&intent.manifest_handle) else { continue };
        // Čekáme na úplné spawnutí scene
        if !scene_spawner.instance_is_ready(scene_ready.0) { continue }

        // Sestaví name→Handle<Image> mapu pro embedded textury tohoto modelu.
        let embedded_images = {
            let entry = gltf_cache.0.get(&model_name.0);
            let gltf = entry.as_ref().and_then(|(h, _)| gltfs.get(h));
            let path = entry.map(|(_, p)| p.as_str()).unwrap_or("");
            build_embedded_image_map(gltf, path, &asset_server)
        };

        let mut stores = MaterialStores {
            std_pbr: &mut std_materials,
            layered: &mut env_materials,
            glass:   &mut glass_materials,
        };

        for entity in scene_spawner.iter_instance_entities(scene_ready.0) {
            // Look up entity definition by name — optional, unnamed primitives still get materials.
            let entity_def = names.get(entity).ok()
                .and_then(|n| manifest.entities.get(n.as_str()));

            // COLLISION: hide + store physics metadata, then skip mesh processing.
            if let Some(EntityDef::COLLISION {
                shape, half_extents, radius, height, mass, is_static,
                climbable, ladder, material, friction, restitution, tags,
                lock_translation, lock_rotation,
            }) = entity_def {
                commands.entity(entity).insert((
                    Visibility::Hidden,
                    DrawableCollision {
                        shape: shape.clone(),
                        half_extents: half_extents.map(|v| Vec3::new(v[0], v[1], v[2])),
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
                    },
                ));
                continue;
            }

            // MESH: apply material to every entity that owns a Mesh3d.
            // Handles both single-primitive (entity IS the named node) and
            // multi-primitive (unnamed primitive children of the named node).
            if node_query.get(entity).map(|(m, _)| m.is_some()).unwrap_or(false) {
                let cast_shadows = match entity_def {
                    Some(EntityDef::MESH { cast_shadows }) => *cast_shadows,
                    _ => true,
                };
                let log_name = names.get(entity)
                    .map(|n| n.as_str().to_owned())
                    .unwrap_or_default();
                process_mesh_node(
                    entity,
                    &log_name,
                    cast_shadows,
                    manifest,
                    &embedded_images,
                    &fallback,
                    &mut commands,
                    &mat_names,
                    &node_query,
                    &mut meshes,
                    &mut stores,
                    &mut texture_reg,
                    &asset_server,
                );
            }
        }

        commands.entity(root_entity).insert(DrawableHooked);
    }
}

// ---------------------------------------------------------------------------
// Pomocné funkce
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn process_mesh_node(
    entity: Entity,
    node_name: &str,
    cast_shadows: bool,
    manifest: &DrawableManifest,
    embedded_images: &HashMap<String, Handle<Image>>,
    fallback: &DrawableFallbackTextures,
    commands: &mut Commands,
    mat_names: &Query<&GltfMaterialName>,
    node_query: &Query<(Option<&Mesh3d>, Option<&Children>), Or<(With<Mesh3d>, With<Children>)>>,
    meshes: &mut Assets<Mesh>,
    stores: &mut MaterialStores<'_>,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) {
    // Sanitizace vertex dat — zajistí, že shadery vždy dostanou platná data
    if let Ok((Some(mesh_h), _)) = node_query.get(entity) {
        if let Some(mesh) = meshes.get_mut(mesh_h.id()) {
            let n = mesh.count_vertices();
            if !mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR) {
                // Neutrální: žádné efekty (R=0,G=0,B=0), default paleta (A=0)
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_COLOR,
                    vec![[0.0f32, 0.0, 0.0, 0.0]; n],
                );
            }
            if !mesh.contains_attribute(Mesh::ATTRIBUTE_UV_1) {
                // Neutrální UV1 (bevy_masks2): AO=1.0 (žádný efekt), emissive=0.0
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_UV_1,
                    vec![[1.0f32, 0.0]; n],
                );
            }
        }
    }

    // Jméno GLTF materiálu → lookup v manifestu
    let gltf_mat_name = mat_names.get(entity).map(|m| m.0.as_str()).unwrap_or("");
    let mat_def = match manifest.materials.get(gltf_mat_name) {
        Some(m) => m,
        None => {
            if !gltf_mat_name.is_empty() {
                warn!(
                    "[drawable] '{}': GLTF mat '{}' NOT found. Available: {:?}",
                    node_name, gltf_mat_name,
                    manifest.materials.keys().collect::<Vec<_>>()
                );
            }
            // Fallback: use first available material
            match manifest.materials.values().next() {
                Some(m) => {
                    warn!("[drawable] '{}': Using first material as fallback", node_name);
                    m
                }
                None => {
                    error!("[drawable] '{}': No materials available", node_name);
                    return;
                }
            }
        }
    };

    let applied = match mat_def.template.as_str() {
        "standard_pbr" => {
            let mat = build_standard_pbr(mat_def, embedded_images, fallback, texture_reg, asset_server);
            let handle = stores.std_pbr.add(mat);
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle));
            true
        }
        "layered_env" => {
            let mat = build_layered_env(mat_def, embedded_images, fallback, texture_reg, asset_server);
            let handle = stores.layered.add(mat);
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle));
            true
        }
        "vehicle_glass" => {
            let mat = build_vehicle_glass(mat_def, embedded_images, fallback, texture_reg, asset_server);
            let handle = stores.glass.add(mat);
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle));
            true
        }
        other => {
            warn!("[drawable] '{}': neznámý template '{}', přeskočeno", node_name, other);
            false
        }
    };

    if applied && !cast_shadows {
        commands.entity(entity).insert(NotShadowCaster);
    }
}

// ---------------------------------------------------------------------------
// Sdílený helper: MaterialParams → DrawableParams
// ---------------------------------------------------------------------------

/// `mb_alpha_threshold`: > 0 aktivuje MB alpha clip v shaderu; 0.0 = disabled.
/// Přenáší se v `tiling.w` (dříve nevyužito).
pub(crate) fn build_params(p: &MaterialParams, mb_alpha_threshold: f32) -> DrawableParams {
    DrawableParams {
        tint: p.tint.map(Vec4::from).unwrap_or(Vec4::ONE),
        weather: Vec4::new(
            p.snow_level.unwrap_or(0.0),
            p.dirt_level.unwrap_or(0.0),
            p.wetness   .unwrap_or(0.0),
            p.porosity  .unwrap_or(0.0),
        ),
        tiling: Vec4::new(
            p.tiling   .unwrap_or(1.0),
            p.l0_tiling.unwrap_or(1.0),
            p.l1_tiling.unwrap_or(1.0),
            mb_alpha_threshold,
        ),
        flags: Vec4::ZERO,
    }
}

pub(crate) fn resolve_alpha_mode(opacity_mode: &str, threshold: f32, has_mb: bool) -> AlphaMode {
    match opacity_mode {
        "BLEND" => AlphaMode::Blend,
        "CLIP" | "HASHED" => AlphaMode::Mask(threshold),
        _ if has_mb => AlphaMode::Mask(threshold), // OPAQUE + MB → depth-prepass-friendly alpha test
        _ => AlphaMode::Opaque,
    }
}

// ---------------------------------------------------------------------------
// Buildery per template
// ---------------------------------------------------------------------------

pub(crate) fn build_standard_pbr(
    def: &MaterialDef,
    embedded_images: &HashMap<String, Handle<Image>>,
    fallback: &DrawableFallbackTextures,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) -> StandardPbrMaterial {
    let p = &def.params;

    let albedo  = def.textures.get("albedo") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let mrao    = def.textures.get("mrao")   .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let normal  = def.textures.get("normal") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let palette = def.textures.get("palette").map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let snow    = def.textures.get("snow")   .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let ma      = def.textures.get("ma")     .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let mb      = def.textures.get("mb")     .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));

    let has_mb = mb.is_some();
    let mb_threshold = if has_mb { p.alpha_threshold.unwrap_or(0.5) } else { 0.0 };
    let opacity_mode = p.opacity_mode.as_deref().unwrap_or("OPAQUE");
    let alpha_mode   = resolve_alpha_mode(opacity_mode, mb_threshold, has_mb);

    let tiling = p.tiling.unwrap_or(1.0);
    let mut base = StandardMaterial {
        perceptual_roughness: 0.5,
        metallic: 0.0,
        alpha_mode,

        uv_transform: Affine2::from_scale(Vec2::splat(tiling)),
        ..default()
    };
    if let Some(h) = albedo { base.base_color_texture          = Some(h); }
    if let Some(h) = normal { base.normal_map_texture          = Some(h); }
    if let Some(h) = mrao {
        base.metallic_roughness_texture = Some(h.clone());
        base.occlusion_texture          = Some(h);
    }
    if let Some(t) = p.tint {
        base.base_color = Color::srgba(t[0], t[1], t[2], t[3]);
    }

    let white = || fallback.white_1px.clone();
    let mut params = build_params(p, mb_threshold);
    params.flags.x = if ma.is_some() { 1.0 } else { 0.0 };
    params.flags.y = if snow.is_some() { 1.0 } else { 0.0 };

    StandardPbrMaterial {
        base,
        extension: StandardPbrExtension {
            palette: palette.unwrap_or_else(white),
            snow:    snow.unwrap_or_else(white),
            ma:      ma.unwrap_or_else(white),
            mb:      mb.unwrap_or_else(white),
            params,
        },
    }
}

pub(crate) fn build_layered_env(
    def: &MaterialDef,
    embedded_images: &HashMap<String, Handle<Image>>,
    fallback: &DrawableFallbackTextures,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) -> LayeredEnvMaterial {
    let p = &def.params;

    let albedo        = def.textures.get("albedo")        .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let mrao          = def.textures.get("mrao")          .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let normal        = def.textures.get("normal")        .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let layer1_albedo = def.textures.get("layer1_albedo") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let layer1_mrao   = def.textures.get("layer1_mrao")   .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let layer1_normal = def.textures.get("layer1_normal") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let snow          = def.textures.get("snow")          .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let ma            = def.textures.get("ma")            .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let mb            = def.textures.get("mb")            .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));

    let has_mb = mb.is_some();
    let mb_threshold = if has_mb { p.alpha_threshold.unwrap_or(0.5) } else { 0.0 };
    let opacity_mode = p.opacity_mode.as_deref().unwrap_or("OPAQUE");
    let alpha_mode   = resolve_alpha_mode(opacity_mode, mb_threshold, has_mb);

    let l0_tiling = p.l0_tiling.or(p.tiling).unwrap_or(1.0);
    let mut base = StandardMaterial {
        perceptual_roughness: 0.5,
        metallic: 0.0,
        alpha_mode,

        uv_transform: Affine2::from_scale(Vec2::splat(l0_tiling)),
        ..default()
    };
    if let Some(h) = albedo { base.base_color_texture          = Some(h); }
    if let Some(h) = normal { base.normal_map_texture          = Some(h); }
    if let Some(h) = mrao {
        base.metallic_roughness_texture = Some(h.clone());
        base.occlusion_texture          = Some(h);
    }
    if let Some(t) = p.tint {
        base.base_color = Color::srgba(t[0], t[1], t[2], t[3]);
    }

    let white = || fallback.white_1px.clone();
    let mut params = build_params(p, mb_threshold);
    params.flags.x = if ma.is_some() { 1.0 } else { 0.0 };
    params.flags.y = if snow.is_some() { 1.0 } else { 0.0 };

    LayeredEnvMaterial {
        base,
        extension: LayeredEnvExtension {
            layer1_albedo: layer1_albedo.unwrap_or_else(white),
            layer1_normal: layer1_normal.unwrap_or_else(white),
            layer1_mrao:   layer1_mrao.unwrap_or_else(white),
            snow:          snow.unwrap_or_else(white),
            ma:            ma.unwrap_or_else(white),
            mb:            mb.unwrap_or_else(white),
            params,
        },
    }
}

pub(crate) fn build_vehicle_glass(
    def: &MaterialDef,
    embedded_images: &HashMap<String, Handle<Image>>,
    fallback: &DrawableFallbackTextures,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) -> VehicleGlassMaterial {
    let p = &def.params;

    let albedo = def.textures.get("albedo").map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let normal = def.textures.get("normal").map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let shatter_map = def.textures.get("shatter_map").map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));

    // Průhlednost ze tint alpha, fallback 0.3 (tmavé záhadné sklo)
    let glass_alpha = p.tint.map(|t| t[3]).unwrap_or(0.3);
    let glass_tint  = p.tint.map(|t| Color::srgba(t[0], t[1], t[2], glass_alpha))
                            .unwrap_or(Color::srgba(0.9, 0.95, 1.0, glass_alpha));

    let glass_tiling = p.tiling.unwrap_or(1.0);
    let mut base = StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.05,
        metallic: 0.0,
        double_sided: true,
        cull_mode: None,

        base_color: glass_tint,
        uv_transform: Affine2::from_scale(Vec2::splat(glass_tiling)),
        ..default()
    };
    if let Some(h) = albedo { base.base_color_texture = Some(h); }
    if let Some(h) = normal { base.normal_map_texture = Some(h); }

    VehicleGlassMaterial {
        base,
        extension: VehicleGlassExtension {
            shatter_map: shatter_map.unwrap_or_else(|| fallback.white_1px.clone()),
            params: build_params(p, 0.0),
        },
    }
}

/// Automaticky skryje entity pojmenované s prefixem `COL_` které nemají `DrawableCollision`.
/// Pokrývá případ GLB modelů bez `.drawable` manifestu — COL_ uzly by jinak zůstaly viditelné.
pub fn auto_hide_col_nodes(
    mut commands: Commands,
    query: Query<(Entity, &Name), (Added<Name>, Without<DrawableCollision>)>,
) {
    for (entity, name) in &query {
        if name.as_str().starts_with("COL_") {
            commands.entity(entity).insert(Visibility::Hidden);
        }
    }
}

pub(crate) fn resolve_tex(
    info: &crate::manifest::TextureInfo,
    embedded: &HashMap<String, Handle<Image>>,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) -> Handle<Image> {
    match info.source {
        TextureSource::Shared => texture_reg.request(&info.name, asset_server),
        TextureSource::Embedded => {
            if let Some(handle) = embedded.get(&info.name) {
                return handle.clone();
            }
            warn!("[drawable] embedded tex '{}' not found in GLB images, fallback to shared", info.name);
            texture_reg.request(&info.name, asset_server)
        }
    }
}
