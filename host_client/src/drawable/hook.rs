use std::collections::HashMap;

use bevy::gltf::{Gltf, GltfAssetLabel, GltfMaterialName};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::prelude::On;
use bevy::scene::{InstanceId, SceneInstanceReady, SceneSpawner};

use core_resources::ModelName;

use super::manifest::{DrawableManifest, EntityDef, MaterialDef, MaterialParams, TextureSource};
use super::material::{
    DrawableParams,
    LayeredEnvExtension, LayeredEnvMaterial,
    StandardPbrExtension, StandardPbrMaterial,
    VehicleGlassExtension, VehicleGlassMaterial,
};
use super::registry::{DrawableManifestRegistry, GltfHandleCache, TextureRegistry};

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
pub(crate) struct SceneReadyId(InstanceId);

/// Marker: drawable hooking byl dokončen. Systém entitu přeskočí.
#[derive(Component)]
pub struct DrawableHooked;

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
/// na handle načtený přes `GltfAssetLabel::Texture(index)`.
/// Vyžaduje načtení modelu s `GltfLoaderSettings::include_source = true`.
fn build_embedded_image_map(
    gltf: Option<&Gltf>,
    model_path: &str,
    asset_server: &AssetServer,
) -> HashMap<String, Handle<Image>> {
    let mut map = HashMap::new();
    let Some(gltf) = gltf else { return map };
    let Some(source) = gltf.source.as_ref() else { return map };

    for image in source.images() {
        let Some(name) = image.name() else { continue };
        let label = GltfAssetLabel::Texture(image.index());
        let path = label.from_asset(model_path.to_string());
        let handle: Handle<Image> = asset_server.load(path);
        map.insert(name.to_string(), handle);
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
    pending: Query<
        (Entity, &SceneReadyId, &DrawableSpawnIntent, &ModelName),
        Without<DrawableHooked>,
    >,
    names: Query<&Name>,
    mat_names: Query<&GltfMaterialName>,
    mesh_handles: Query<&Mesh3d>,
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
            let Ok(name) = names.get(entity) else { continue };
            let Some(entity_def) = manifest.entities.get(name.as_str()) else { continue };

            match entity_def {
                EntityDef::MESH { cast_shadows } => {
                    process_mesh_node(
                        entity,
                        name.as_str(),
                        *cast_shadows,
                        manifest,
                        &embedded_images,
                        &mut commands,
                        &mat_names,
                        &mesh_handles,
                        &mut meshes,
                        &mut stores,
                        &mut texture_reg,
                        &asset_server,
                    );
                }
                EntityDef::COLLISION { .. } => {
                    // Phase 5: physics. Prozatím jen schováme vizuál.
                    commands.entity(entity).insert(Visibility::Hidden);
                }
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
    commands: &mut Commands,
    mat_names: &Query<&GltfMaterialName>,
    mesh_handles: &Query<&Mesh3d>,
    meshes: &mut Assets<Mesh>,
    stores: &mut MaterialStores<'_>,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) {
    // Sanitizace vertex colors — zajistí, že shader vždy dostane platná data
    if let Ok(mesh_h) = mesh_handles.get(entity) {
        if let Some(mesh) = meshes.get_mut(mesh_h.id()) {
            if !mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR) {
                let n = mesh.count_vertices();
                // Neutrální: žádné efekty (R=0,G=0,B=0), default paleta (A=0)
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_COLOR,
                    vec![[0.0f32, 0.0, 0.0, 0.0]; n],
                );
            }
        }
    }

    // Jméno GLTF materiálu → lookup v manifestu
    let gltf_mat_name = mat_names.get(entity).map(|m| m.0.as_str()).unwrap_or("");
    let Some(mat_def) = manifest.materials.get(gltf_mat_name) else {
        debug!(
            "[drawable] '{}': GLTF mat '{}' není v manifestu, swap přeskočen",
            node_name, gltf_mat_name
        );
        return;
    };

    let applied = match mat_def.template.as_str() {
        "standard_pbr" => {
            let mat = build_standard_pbr(mat_def, embedded_images, texture_reg, asset_server);
            let handle = stores.std_pbr.add(mat);
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle));
            true
        }
        "layered_env" => {
            let mat = build_layered_env(mat_def, embedded_images, texture_reg, asset_server);
            let handle = stores.layered.add(mat);
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle));
            true
        }
        "vehicle_glass" => {
            let mat = build_vehicle_glass(mat_def, embedded_images, texture_reg, asset_server);
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

fn build_params(p: &MaterialParams) -> DrawableParams {
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
            0.0,
        ),
    }
}

// ---------------------------------------------------------------------------
// Buildery per template
// ---------------------------------------------------------------------------

fn build_standard_pbr(
    def: &MaterialDef,
    embedded_images: &HashMap<String, Handle<Image>>,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) -> StandardPbrMaterial {
    let p = &def.params;

    let albedo  = def.textures.get("albedo") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let mrao    = def.textures.get("mrao")   .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let normal  = def.textures.get("normal") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let palette = def.textures.get("palette").map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let snow    = def.textures.get("snow")   .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));

    let mut base = StandardMaterial {
        perceptual_roughness: 0.5,
        metallic: 0.0,
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

    StandardPbrMaterial {
        base,
        extension: StandardPbrExtension { palette, snow, params: build_params(p) },
    }
}

fn build_layered_env(
    def: &MaterialDef,
    embedded_images: &HashMap<String, Handle<Image>>,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) -> LayeredEnvMaterial {
    let p = &def.params;

    let albedo        = def.textures.get("albedo")        .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let mrao          = def.textures.get("mrao")          .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let normal        = def.textures.get("normal")        .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let layer1_albedo = def.textures.get("layer1_albedo") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let layer1_normal = def.textures.get("layer1_normal") .map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));

    let mut base = StandardMaterial {
        perceptual_roughness: 0.5,
        metallic: 0.0,
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

    LayeredEnvMaterial {
        base,
        extension: LayeredEnvExtension { layer1_albedo, layer1_normal, params: build_params(p) },
    }
}

fn build_vehicle_glass(
    def: &MaterialDef,
    embedded_images: &HashMap<String, Handle<Image>>,
    texture_reg: &mut TextureRegistry,
    asset_server: &AssetServer,
) -> VehicleGlassMaterial {
    let p = &def.params;

    let albedo = def.textures.get("albedo").map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));
    let normal = def.textures.get("normal").map(|t| resolve_tex(t, embedded_images, texture_reg, asset_server));

    // Průhlednost ze tint alpha, fallback 0.3 (tmavé záhadné sklo)
    let glass_alpha = p.tint.map(|t| t[3]).unwrap_or(0.3);
    let glass_tint  = p.tint.map(|t| Color::srgba(t[0], t[1], t[2], glass_alpha))
                            .unwrap_or(Color::srgba(0.9, 0.95, 1.0, glass_alpha));

    let mut base = StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.05,
        metallic: 0.0,
        double_sided: true,
        cull_mode: None,
        base_color: glass_tint,
        ..default()
    };
    if let Some(h) = albedo { base.base_color_texture = Some(h); }
    if let Some(h) = normal { base.normal_map_texture = Some(h); }

    VehicleGlassMaterial {
        base,
        extension: VehicleGlassExtension { params: build_params(p) },
    }
}

fn resolve_tex(
    info: &super::manifest::TextureInfo,
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
