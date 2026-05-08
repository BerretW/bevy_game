use std::collections::HashMap;

use bevy::prelude::*;
use serde::Deserialize;

/// Párovací soubor k `.glb` assetu — definuje materiály, shadery a entity (mesh/colize).
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
        mass: f32,
        #[serde(default)]
        is_static: bool,
        #[serde(default)]
        friction: f32,
        #[serde(default)]
        restitution: f32,
        #[serde(default)]
        tags: Vec<String>,
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
}
