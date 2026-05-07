use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// Shader parametry sdílené mezi všemi drawable šablonami.
///
/// Layout musí přesně odpovídat `DrawableParams` struct ve všech WGSL shaderech.
///   `tint`    — RGBA multiplikátor barvy (default: bílá = žádná změna)
///   `weather` — x=snow_level, y=dirt_level, z=wetness, w=porosity (0..1)
///   `tiling`  — x=tiling, y=l0_tiling, z=l1_tiling, w=nevyužito
#[derive(ShaderType, Debug, Clone)]
pub struct DrawableParams {
    pub tint:    Vec4,
    pub weather: Vec4,
    pub tiling:  Vec4,
}

impl Default for DrawableParams {
    fn default() -> Self {
        Self {
            tint:    Vec4::ONE,
            weather: Vec4::ZERO,
            tiling:  Vec4::new(1.0, 1.0, 1.0, 0.0),
        }
    }
}

// ─── Template: standard_pbr ───────────────────────────────────────────────

/// Rozšíření pro `standard_pbr` šablonu.
///
/// Vertex color konvence (ATTRIBUTE_COLOR):
///   R = míchání vrstev (0=base, 1=druhá vrstva)
///   G = krev/špína maska
///   B = vlhkost/kaluž maska
///   A = paleta UV (1D LUT tintování)
///
/// Bindings (group 2):
///   100/101 — palette_texture/sampler
///   102/103 — snow_texture/sampler
///   104     — DrawableParams uniform
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct StandardPbrExtension {
    #[texture(100)]
    #[sampler(101)]
    pub palette: Option<Handle<Image>>,

    #[texture(102)]
    #[sampler(103)]
    pub snow: Option<Handle<Image>>,

    #[uniform(104)]
    pub params: DrawableParams,
}

pub type StandardPbrMaterial = ExtendedMaterial<StandardMaterial, StandardPbrExtension>;

impl MaterialExtension for StandardPbrExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/standard_pbr.wgsl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/standard_pbr.wgsl".into()
    }
}

// ─── Template: layered_env ────────────────────────────────────────────────

/// Rozšíření pro `layered_env` šablonu — dvouvrstvý environment materiál.
///
/// Vertex color R řídí míchání vrstev (0=base StandardMaterial albedo, 1=layer1).
/// Vertex color G = špína, B = vlhkost (stejná sémantika jako standard_pbr).
///
/// Bindings (group 2):
///   100/101 — layer1_albedo_texture/sampler (druhá vrstva albeda)
///   102/103 — layer1_normal_texture/sampler (normálová mapa druhé vrstvy)
///   104     — DrawableParams uniform
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct LayeredEnvExtension {
    #[texture(100)]
    #[sampler(101)]
    pub layer1_albedo: Option<Handle<Image>>,

    #[texture(102)]
    #[sampler(103)]
    pub layer1_normal: Option<Handle<Image>>,

    #[uniform(104)]
    pub params: DrawableParams,
}

pub type LayeredEnvMaterial = ExtendedMaterial<StandardMaterial, LayeredEnvExtension>;

impl MaterialExtension for LayeredEnvExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/layered_env.wgsl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/layered_env.wgsl".into()
    }
}

// ─── Template: vehicle_glass ──────────────────────────────────────────────

/// Rozšíření pro `vehicle_glass` šablonu — průhledné sklo vozidel.
///
/// StandardMaterial se nastavuje na alpha_mode=Blend, nízkou roughness a double_sided.
/// params.tint.a řídí průhlednost (0=neviditelné, 1=neprůhledné).
///
/// Bindings (group 2):
///   100 — DrawableParams uniform
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct VehicleGlassExtension {
    #[uniform(100)]
    pub params: DrawableParams,
}

pub type VehicleGlassMaterial = ExtendedMaterial<StandardMaterial, VehicleGlassExtension>;

impl MaterialExtension for VehicleGlassExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/vehicle_glass.wgsl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/vehicle_glass.wgsl".into()
    }
}

// ─── Backward-compat aliases ──────────────────────────────────────────────

#[allow(dead_code)]
pub type DrawableExtension = StandardPbrExtension;
#[allow(dead_code)]
pub type DrawableMaterial  = StandardPbrMaterial;
