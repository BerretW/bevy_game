// layered_env.wgsl — LayeredEnvExtension fragment shader
//
// Dvouvrstvý environment materiál. Vrstva 0 (base) pochází ze StandardMaterial,
// vrstva 1 je definována v extension bindingách a míchá se pomocí vertex color R.
//
// Vertex color konvence (ATTRIBUTE_COLOR):
//   R = blend faktor vrstev (0=jen vrstva 0, 1=jen vrstva 1)
//   G = krev/špína maska
//   B = vlhkost/kaluž maska
//   A = nevyužito (layered_env nepoužívá paletu)

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct DrawableParams {
    tint:    vec4<f32>,  // RGBA multiplikátor
    weather: vec4<f32>,  // x=snow_level, y=dirt_level, z=wetness, w=porosity
    tiling:  vec4<f32>,  // x=tiling, y=l0_tiling, z=l1_tiling, w=mb_alpha_threshold (0=disabled)
}

@group(2) @binding(100) var layer1_albedo_texture: texture_2d<f32>;
@group(2) @binding(101) var layer1_albedo_sampler: sampler;
@group(2) @binding(102) var layer1_normal_texture: texture_2d<f32>;
@group(2) @binding(103) var layer1_normal_sampler: sampler;
@group(2) @binding(104) var<uniform> params: DrawableParams;
@group(2) @binding(105) var mb_texture:            texture_2d<f32>;
@group(2) @binding(106) var mb_sampler:            sampler;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let masks = in.color;       // R=layer_blend, G=dirt, B=wet
    var pbr_in = in;
    pbr_in.color = vec4<f32>(1.0);

    var pbr_input = pbr_input_from_standard_material(pbr_in, is_front);

    // 1. Tint ze StandardMaterial params (nastavuje se v Rustu)
    pbr_input.material.base_color *= params.tint;

    // 2. Míchání vrstev: vertex R = blend faktor
    let blend = clamp(masks.r, 0.0, 1.0);
    if blend > 0.001 {
        let uv1 = in.uv * params.tiling.z;     // l1_tiling
        let l1_col = textureSample(layer1_albedo_texture, layer1_albedo_sampler, uv1);
        pbr_input.material.base_color = mix(pbr_input.material.base_color, l1_col, blend);
        // Normála druhé vrstvy se míchá lineárně v tangent-space před normalizací.
        // pbr_input.N je world-space — pro přesné míchání normál by bylo potřeba
        // tangent frame, zde proto blendujeme jen albedo. Vrstva 0 normálová mapa
        // (z StandardMaterial.normal_map_texture) zůstává.
    }

    // 3. Špína / krev: vertex G × dirt_level
    let dirt_factor = clamp(masks.g * params.weather.y, 0.0, 1.0);
    pbr_input.material.base_color = mix(
        pbr_input.material.base_color,
        pbr_input.material.base_color * vec4<f32>(0.38, 0.28, 0.18, 1.0),
        dirt_factor
    );

    // 4. Vlhkost: vertex B × wetness
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

    // 5. Sníh: snow_level (globální, nereaguje na vertex B tady — vrstva prostředí)
    let snow_factor = clamp(params.weather.x, 0.0, 1.0);
    if snow_factor > 0.001 {
        let snow_col = vec4<f32>(0.92, 0.95, 1.0, 1.0);    // bílá se studeným nádechem
        pbr_input.material.base_color           = mix(pbr_input.material.base_color, snow_col, snow_factor);
        pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.95, snow_factor);
        pbr_input.material.metallic             = mix(pbr_input.material.metallic, 0.0, snow_factor);
    }

    // 6. UV1 — druhá vrstva masek (bevy_masks2): AO + emissive
#ifdef VERTEX_UVS_B
    let ao       = clamp(in.uv_b.x, 0.0, 1.0);
    let emissive = clamp(in.uv_b.y, 0.0, 1.0);
    pbr_input.occlusion         *= vec3(ao);
    pbr_input.material.emissive *= emissive;
#endif

    // MB alpha clip: tiling.w = threshold (0.0 = disabled, >0 = discard pixels below)
    let mb_threshold = params.tiling.w;
    if mb_threshold > 0.0 {
        if textureSample(mb_texture, mb_sampler, in.uv).a < mb_threshold {
            discard;
        }
    }

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
