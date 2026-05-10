use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// Shader parametry sdílené mezi všemi drawable šablonami.
///
/// Layout musí přesně odpovídat `DrawableParams` struct ve všech WGSL shaderech.
///   `tint`    — RGBA multiplikátor barvy (default: bílá = žádná změna)
///   `weather` — x=snow_level, y=dirt_level, z=wetness, w=porosity (0..1)
///   `tiling`  — x=tiling, y=l0_tiling, z=l1_tiling, w=mb_alpha_threshold (0=disabled)
#[derive(ShaderType, Debug, Clone)]
pub struct DrawableParams {
    pub tint:    Vec4,
    pub weather: Vec4,
    pub tiling:  Vec4,
    pub flags:   Vec4,
}

impl Default for DrawableParams {
    fn default() -> Self {
        Self {
            tint:    Vec4::ONE,
            weather: Vec4::ZERO,
            tiling:  Vec4::new(1.0, 1.0, 1.0, 0.0),
            flags:   Vec4::ZERO,
        }
    }
}

// ─── Template: standard_pbr ───────────────────────────────────────────────

/// Rozšíření pro `standard_pbr` šablonu.
///
/// Vertex color konvence (ATTRIBUTE_COLOR):
///   R = rezervováno (nevyužito)
///   G = krev/špína maska
///   B = vlhkost/kaluž maska
///   A = paleta UV (1D LUT tintování)
///
/// Bindings (group 2):
///   100/101 — palette_texture/sampler
///   102/103 — snow_texture/sampler
///   104     — DrawableParams uniform
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct StandardPbrExtension {
    #[texture(100)]
    #[sampler(101)]
    pub palette: Handle<Image>,

    #[texture(102)]
    #[sampler(103)]
    pub snow: Handle<Image>,

    #[texture(104)]
    #[sampler(105)]
    pub ma: Handle<Image>,

    #[uniform(106)]
    pub params: DrawableParams,

    /// MB textura — alpha kanál řídí průhlednostní masku.
    /// `params.tiling.w` (mb_alpha_threshold) > 0 aktivuje alpha clip v shaderu.
    #[texture(107)]
    #[sampler(108)]
    pub mb: Handle<Image>,
}

impl Default for StandardPbrExtension {
    fn default() -> Self {
        Self {
            palette: Handle::default(),
            snow:    Handle::default(),
            ma:      Handle::default(),
            params:  DrawableParams::default(),
            mb:      Handle::default(),
        }
    }
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
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct LayeredEnvExtension {
    #[texture(100)]
    #[sampler(101)]
    pub layer1_albedo: Handle<Image>,

    #[texture(102)]
    #[sampler(103)]
    pub layer1_normal: Handle<Image>,

    #[texture(104)]
    #[sampler(105)]
    pub layer1_mrao: Handle<Image>,

    #[texture(106)]
    #[sampler(107)]
    pub snow: Handle<Image>,

    #[texture(108)]
    #[sampler(109)]
    pub ma: Handle<Image>,

    #[uniform(110)]
    pub params: DrawableParams,

    /// MB textura — alpha kanál řídí průhlednostní masku.
    #[texture(111)]
    #[sampler(112)]
    pub mb: Handle<Image>,
}

impl Default for LayeredEnvExtension {
    fn default() -> Self {
        Self {
            layer1_albedo: Handle::default(),
            layer1_normal: Handle::default(),
            layer1_mrao:   Handle::default(),
            snow:          Handle::default(),
            ma:            Handle::default(),
            params:        DrawableParams::default(),
            mb:            Handle::default(),
        }
    }
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
    #[texture(100)]
    #[sampler(101)]
    pub shatter_map: Handle<Image>,

    #[uniform(102)]
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
