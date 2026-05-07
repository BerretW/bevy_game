use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// Finální typ materiálu použitý na mesh entity z Drawable systému.
pub type DrawableMaterial = ExtendedMaterial<StandardMaterial, DrawableExtension>;

/// Rozšíření základního `StandardMaterial` o weather efekty a vertex color masky.
///
/// Vertex color konvence (uložené jako `ATTRIBUTE_COLOR`, plněné z Blender pluginu):
///   R = míchání vrstev (0 = základní vrstva, 1 = druhá vrstva)
///   G = krev / špína maska
///   B = vlhkost / kaluž maska
///   A = paleta UV (poloha na 1D LUT pro tintování)
///
/// Neutrální hodnoty pro modely bez vertex colors: [0, 0, 0, 0].
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct DrawableExtension {
    /// 1D paleta barev (LUT) — vzorkuje se pomocí vertex alpha pro tintování.
    #[texture(100)]
    #[sampler(101)]
    pub palette: Option<Handle<Image>>,

    /// Textura sněhu — prolíná se pomocí vertex B kanálu × snow_level.
    #[texture(102)]
    #[sampler(103)]
    pub snow: Option<Handle<Image>>,

    /// Parametry zabalené do tří `vec4` — odpovídají WGSL struktuře `DrawableParams`.
    #[uniform(104)]
    pub params: DrawableParams,
}

/// Shader parametry — layout musí přesně odpovídat `drawable_extension.wgsl`.
///
/// `tint`    — RGBA multiplikátor barvy (default: bílá = žádná změna)
/// `weather` — x=snow_level, y=dirt_level, z=wetness, w=porosity (0..1)
/// `tiling`  — x=tiling, y=l0_tiling, z=l1_tiling, w=nevyužito
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

impl MaterialExtension for DrawableExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/drawable_extension.wgsl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/drawable_extension.wgsl".into()
    }
}
