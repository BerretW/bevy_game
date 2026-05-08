// standard_pbr.wgsl — StandardPbrExtension fragment shader
//
// Vertex color konvence (COLOR_0 = bevy_masks):
//   R = potlačení normal mapy (0=plná normála, 1=úplně flat/geometrická)
//   G = krev/špína maska (0=čisto, 1=max špína)
//   B = vlhkost/kaluž maska (0=sucho, 1=mokro)
//   A = paleta UV (1D LUT pro tintování)
//
// UV1 konvence (TEXCOORD_1 = bevy_masks2, podmíněno VERTEX_UVS_B):
//   x = AO multiplikátor (1.0=žádný vliv, 0.0=maximální ztmavení)
//   y = emissive intenzita (0.0=žádná, 1.0=plné)
//
// Vertex colors (COLOR_0) slouží jako DATA. Před pbr_input_from_standard_material
// nahradíme in.color bílou, aby Bevy PBR pipeline albedo neztmavila maskami.

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

@group(2) @binding(100) var palette_texture: texture_2d<f32>;
@group(2) @binding(101) var palette_sampler: sampler;
@group(2) @binding(102) var snow_texture:    texture_2d<f32>;
@group(2) @binding(103) var snow_sampler:    sampler;
@group(2) @binding(104) var<uniform> params: DrawableParams;
@group(2) @binding(105) var mb_texture:      texture_2d<f32>;
@group(2) @binding(106) var mb_sampler:      sampler;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let masks = in.color;   // R=normal_suppress, G=dirt, B=wet, A=palette
    var pbr_in = in;
    pbr_in.color = vec4<f32>(1.0);

    var pbr_input = pbr_input_from_standard_material(pbr_in, is_front);

    // 1. Normal map intenzita: vertex R = míra potlačení (0=plná, 1=flat geometrická)
    let normal_suppress = clamp(masks.r, 0.0, 1.0);
    let geo_normal = normalize(pbr_input.world_normal);
    pbr_input.N = normalize(mix(pbr_input.N, geo_normal, normal_suppress));

    // 2. Paleta (1D LUT): vzorkujeme na pozici vertex alpha
    let palette_uv  = vec2<f32>(masks.a, 0.5);
    let palette_col = textureSample(palette_texture, palette_sampler, palette_uv);
    pbr_input.material.base_color *= palette_col * params.tint;

    // 3. Sníh: snow_level (globální) × snow_tiling
    let snow_factor = clamp(params.weather.x, 0.0, 1.0);
    if snow_factor > 0.001 {
        let snow_uv  = in.uv * params.tiling.x;
        let snow_col = textureSample(snow_texture, snow_sampler, snow_uv);
        pbr_input.material.base_color           = mix(pbr_input.material.base_color, snow_col, snow_factor);
        pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.95, snow_factor);
        pbr_input.material.metallic             = mix(pbr_input.material.metallic, 0.0, snow_factor);
    }

    // 4. Špína / krev: vertex G × dirt_level — ztmaví povrch do špinavě hnědé
    let dirt_factor = clamp(masks.g * params.weather.y, 0.0, 1.0);
    pbr_input.material.base_color = mix(
        pbr_input.material.base_color,
        pbr_input.material.base_color * vec4<f32>(0.38, 0.28, 0.18, 1.0),
        dirt_factor
    );
    pbr_input.material.perceptual_roughness = mix(
        pbr_input.material.perceptual_roughness, 0.85, dirt_factor
    );

    // 5. Vlhkost: vertex B × wetness — ztmaví + sníží roughness (lesklý mokrý povrch)
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

    // 6. UV1 — druhá vrstva masek (bevy_masks2): AO + emissive
    //    Dostupná jen pokud mesh má TEXCOORD_1 (Bevy nastaví VERTEX_UVS_B).
#ifdef VERTEX_UVS_B
    let ao        = clamp(in.uv_b.x, 0.0, 1.0);  // 1.0=žádný AO, 0.0=maximální ztmavení
    let emissive  = clamp(in.uv_b.y, 0.0, 1.0);  // 0.0=žádný glow
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
