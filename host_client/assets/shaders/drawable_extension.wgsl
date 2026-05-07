// drawable_extension.wgsl
// Fragment shader pro DrawableExtension (ExtendedMaterial<StandardMaterial, DrawableExtension>).
//
// Vertex color konvence:
//   R = míchání vrstev (0=base, 1=druhá vrstva)
//   G = krev/špína maska
//   B = vlhkost/kaluž maska
//   A = paleta UV (1D LUT tintování)
//
// Vertex colors slouží jako DATA, ne jako barevný multiplikátor.
// Proto před voláním pbr_input_from_standard_material nahradíme in.color bílou,
// aby Bevy PBR pipeline albedo neztmavila maskami.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct DrawableParams {
    tint:    vec4<f32>,  // RGBA multiplikátor
    weather: vec4<f32>,  // x=snow_level, y=dirt_level, z=wetness, w=porosity
    tiling:  vec4<f32>,  // x=tiling, y=l0_tiling, z=l1_tiling, w=nevyužito
}

@group(2) @binding(100) var palette_texture: texture_2d<f32>;
@group(2) @binding(101) var palette_sampler: sampler;
@group(2) @binding(102) var snow_texture:    texture_2d<f32>;
@group(2) @binding(103) var snow_sampler:    sampler;
@group(2) @binding(104) var<uniform> params: DrawableParams;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    // Zachováme originální masky, ale PBR pipeline pustíme s bílými vertex colors,
    // aby je Bevy nenásobila do albeda.
    let masks = in.color;       // R=layer, G=dirt, B=wet, A=palette
    var pbr_in = in;
    pbr_in.color = vec4<f32>(1.0);

    var pbr_input = pbr_input_from_standard_material(pbr_in, is_front);

    // 1. Paleta (1D LUT): vzorkujeme na pozici vertex alpha
    let palette_uv  = vec2<f32>(masks.a, 0.5);
    let palette_col = textureSample(palette_texture, palette_sampler, palette_uv);
    pbr_input.material.base_color *= palette_col * params.tint;

    // 2. Sníh: vertex B kanál × globální snow_level
    let snow_factor = clamp(masks.b * params.weather.x, 0.0, 1.0);
    if snow_factor > 0.001 {
        let snow_uv  = in.uv * params.tiling.x;
        let snow_col = textureSample(snow_texture, snow_sampler, snow_uv);
        pbr_input.material.base_color           = mix(pbr_input.material.base_color, snow_col, snow_factor);
        pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.95, snow_factor);
        pbr_input.material.metallic             = mix(pbr_input.material.metallic, 0.0, snow_factor);
    }

    // 3. Špína / krev: vertex G × dirt_level — ztmaví povrch do špinavě hnědé
    let dirt_factor = clamp(masks.g * params.weather.y, 0.0, 1.0);
    pbr_input.material.base_color = mix(
        pbr_input.material.base_color,
        pbr_input.material.base_color * vec4<f32>(0.38, 0.28, 0.18, 1.0),
        dirt_factor
    );

    // 4. Vlhkost: vertex B × wetness — ztmaví + sníží roughness (lesklý mokrý povrch)
    let wet_factor = clamp(masks.b * params.weather.z, 0.0, 1.0);
    pbr_input.material.base_color           = mix(
        pbr_input.material.base_color,
        pbr_input.material.base_color * 0.65,
        wet_factor
    );
    pbr_input.material.perceptual_roughness = mix(
        pbr_input.material.perceptual_roughness,
        max(0.04, pbr_input.material.perceptual_roughness - params.weather.w * 0.45),
        wet_factor
    );

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
