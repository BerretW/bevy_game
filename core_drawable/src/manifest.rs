use std::collections::HashMap;

use bevy::prelude::*;
use serde::Deserialize;

/// Párovací soubor k `.glb` assetu — definuje materiály, shadery a entity (mesh/kolize).
/// Načítá se přes Bevy AssetServer jako `DrawableManifestLoader` (přípona `.drawable`).
#[derive(Debug, Clone, Deserialize, Asset, TypePath)]
pub struct DrawableManifest {
    pub asset_name: String,
    pub version: String,
    #[serde(default)]
    pub materials: HashMap<String, MaterialDef>,
    #[serde(default)]
    pub entities: HashMap<String, EntityDef>,
    /// Volitelný odkaz na `.ped.toml` soubor (bez přípony).
    /// Pokud je nastaven, drawable registry automaticky nahraje ped physics profil.
    /// Příklad: `ped_physics = "player"` → načte `models/player.ped.toml`.
    #[serde(default)]
    pub ped_physics: Option<String>,
    /// Volitelná konfigurace LOD přepínání.
    /// Prázdná = single LOD (žádné přepínání, engine použije výchozí vzdálenosti pokud existují _LODn uzly).
    #[serde(default)]
    pub lod: LodConfig,
}

/// Konfigurace LOD pro drawable asset.
///
/// Definuje vzdálenosti (v metrech) pro přechody mezi LOD úrovněmi.
/// GLTF uzly pojmenované `*_LOD1`, `*_LOD2`, `*_LOD3` jsou automaticky
/// přiřazeny ke svým LOD úrovním. Uzly bez suffixu (nebo `*_LOD0`) = LOD0.
///
/// Příklad v `.drawable`:
/// ```toml
/// [lod]
/// distances = [15.0, 40.0, 80.0]
/// cull_beyond_last = false
/// ```
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LodConfig {
    /// Vzdálenosti v metrech kde dochází k LOD přechodům.
    /// `distances[0]` = přechod LOD0→LOD1, `distances[1]` = LOD1→LOD2, atd.
    /// Prázdné = použij `DefaultLodDistances` resource (pokud existují _LODn uzly).
    #[serde(default)]
    pub distances: Vec<f32>,
    /// Pokud `true`, model se skryje za poslední LOD vzdáleností.
    #[serde(default)]
    pub cull_beyond_last: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaterialDef {
    /// Název WGSL šablony: `"standard_pbr"` | `"layered_env"` | `"vehicle_glass"`
    pub template: String,
    #[serde(default)]
    pub textures: HashMap<String, TextureInfo>,
    #[serde(default)]
    pub params: MaterialParams,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextureInfo {
    pub name: String,
    pub source: TextureSource,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextureSource {
    /// Sdílená textura ze `stream/textures/` — načte se jen jednou (TextureRegistry).
    Shared,
    /// Vložená textura uvnitř GLB souboru — načte ji Bevy GLTF loader automaticky.
    Embedded,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MaterialParams {
    pub tint: Option<[f32; 4]>,
    pub tiling: Option<f32>,
    pub l0_tiling: Option<f32>,
    pub l1_tiling: Option<f32>,
    pub porosity: Option<f32>,
    pub wetness: Option<f32>,
    pub snow_level: Option<f32>,
    pub dirt_level: Option<f32>,
    /// `"OPAQUE"` | `"CLIP"` | `"BLEND"` | `"HASHED"` — řídí Bevy AlphaMode.
    pub opacity_mode: Option<String>,
    /// Práh pro MB alpha clip (0.0–1.0). Použit jen pokud je přítomna MB textura.
    pub alpha_threshold: Option<f32>,
}

/// Definice GLTF uzlu z sekce `[entities]`.
///
/// Klíč = jméno uzlu v GLB (= `Name` komponenta na Bevy entitě po spawnění).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum EntityDef {
    MESH {
        #[serde(default)]
        cast_shadows: bool,
    },
    /// Phase 5: generování collideru. Prozatím uzel schováme.
    COLLISION {
        shape: CollisionShape,
        #[serde(default)]
        half_extents: Option<[f32; 3]>,
        #[serde(default)]
        radius: Option<f32>,
        #[serde(default)]
        height: Option<f32>,
        #[serde(default)]
        mass: f32,
        #[serde(default)]
        is_static: bool,
        #[serde(default)]
        climbable: bool,
        #[serde(default)]
        ladder: bool,
        #[serde(default)]
        material: CollisionMaterial,
        #[serde(default)]
        friction: f32,
        #[serde(default)]
        restitution: f32,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        lock_translation: Option<[bool; 3]>,
        #[serde(default)]
        lock_rotation: Option<[bool; 3]>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CollisionShape {
    Box,
    Sphere,
    Capsule,
    Cylinder,
    Convex,
    Mesh,
    Navmesh,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CollisionMaterial {
    #[default]
    Concrete,
    Stone,
    Brick,
    Wood,
    Metal,
    Glass,
    Dirt,
    Grass,
    Sand,
    Gravel,
    Mud,
    Snow,
    Ice,
    Water,
    Rubber,
    Plastic,
    Ceramic,
    Carpet,
    Asphalt,
    LadderMetal,
    Stairs,
}

impl CollisionMaterial {
    pub fn footstep_profile(&self) -> &'static str {
        match self {
            CollisionMaterial::Concrete => "footstep_concrete",
            CollisionMaterial::Stone => "footstep_stone",
            CollisionMaterial::Brick => "footstep_brick",
            CollisionMaterial::Wood => "footstep_wood",
            CollisionMaterial::Metal => "footstep_metal",
            CollisionMaterial::Glass => "footstep_glass",
            CollisionMaterial::Dirt => "footstep_dirt",
            CollisionMaterial::Grass => "footstep_grass",
            CollisionMaterial::Sand => "footstep_sand",
            CollisionMaterial::Gravel => "footstep_gravel",
            CollisionMaterial::Mud => "footstep_mud",
            CollisionMaterial::Snow => "footstep_snow",
            CollisionMaterial::Ice => "footstep_ice",
            CollisionMaterial::Water => "footstep_water",
            CollisionMaterial::Rubber => "footstep_rubber",
            CollisionMaterial::Plastic => "footstep_plastic",
            CollisionMaterial::Ceramic => "footstep_ceramic",
            CollisionMaterial::Carpet => "footstep_carpet",
            CollisionMaterial::Asphalt => "footstep_asphalt",
            CollisionMaterial::LadderMetal => "footstep_ladder_metal",
            CollisionMaterial::Stairs => "footstep_stone",
        }
    }

    pub fn impact_profile(&self) -> &'static str {
        match self {
            CollisionMaterial::Concrete => "impact_concrete_dust",
            CollisionMaterial::Stone => "impact_stone_chip",
            CollisionMaterial::Brick => "impact_brick_chip",
            CollisionMaterial::Wood => "impact_wood_splinter",
            CollisionMaterial::Metal => "impact_metal_spark",
            CollisionMaterial::Glass => "impact_glass_shard",
            CollisionMaterial::Dirt => "impact_dirt_puff",
            CollisionMaterial::Grass => "impact_grass_puff",
            CollisionMaterial::Sand => "impact_sand_puff",
            CollisionMaterial::Gravel => "impact_gravel_spray",
            CollisionMaterial::Mud => "impact_mud_splash",
            CollisionMaterial::Snow => "impact_snow_puff",
            CollisionMaterial::Ice => "impact_ice_shard",
            CollisionMaterial::Water => "impact_water_splash",
            CollisionMaterial::Rubber => "impact_rubber_thud",
            CollisionMaterial::Plastic => "impact_plastic_frag",
            CollisionMaterial::Ceramic => "impact_ceramic_shard",
            CollisionMaterial::Carpet => "impact_carpet_thud",
            CollisionMaterial::Asphalt => "impact_asphalt_chip",
            CollisionMaterial::LadderMetal => "impact_ladder_metal_spark",
            CollisionMaterial::Stairs => "impact_stone_chip",
        }
    }
}
