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
        }
    }
}
