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
    tiling:  vec4<f32>,  // x=tiling, y=l0_tiling, z=l1_tiling, w=mb_alpha_threshold
    flags:   vec4<f32>,  // x=has_ma, y=has_snow_tex, z=snow_height_cutoff_y, w=wet_height_cutoff_y
    profile: vec4<f32>,  // x=shader profile id
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var layer1_albedo_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var layer1_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var layer1_normal_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var layer1_normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var layer1_mrao_texture:   texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var layer1_mrao_sampler:   sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var snow_texture:          texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var snow_sampler:          sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var ma_texture:            texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var ma_sampler:            sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<uniform> params: DrawableParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var mb_texture:            texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var mb_sampler:            sampler;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let masks = in.color;       // R=layer_blend, G=dirt, B=wet
    var pbr_in = in;
    pbr_in.color = vec4<f32>(1.0);

    var pbr_input = pbr_input_from_standard_material(pbr_in, is_front);

    // 1. Míchání vrstev: vertex R = blend faktor
    let blend = clamp(masks.r, 0.0, 1.0);
    let uv1 = in.uv * params.tiling.z;
    let l1_col  = textureSample(layer1_albedo_texture, layer1_albedo_sampler, uv1);
    let l1_mrao = textureSample(layer1_mrao_texture, layer1_mrao_sampler, uv1);
    let l1_nrm  = textureSample(layer1_normal_texture, layer1_normal_sampler, uv1).xyz * 2.0 - 1.0;

    var layer_color = mix(pbr_input.material.base_color, l1_col, blend);

    // Approximace Blenderu: normály mícháme lineárně v tangent prostoru.
    let l0_nrm = vec3<f32>(0.0, 0.0, 1.0);
    let nrm_mix = normalize(mix(l0_nrm, l1_nrm, blend));
    pbr_input.N = normalize(mix(pbr_input.N, pbr_input.N + nrm_mix * 0.15, blend));

    var roughness_base = mix(pbr_input.material.perceptual_roughness, l1_mrao.g, blend);
    var metallic_base  = mix(pbr_input.material.metallic, l1_mrao.r, blend);
    if params.flags.x > 0.5 {
        let ma = textureSample(ma_texture, ma_sampler, in.uv * params.tiling.y);
        roughness_base = ma.g;
        metallic_base  = ma.b;
    }

    // 2. Dirt / wet / snow dle Blender graphu
    let dirt_mul = clamp(masks.g * params.weather.y, 0.0, 1.0);

    // wet_height (flags.w > 0): vlhkost mizí nad touto world Y hranicí (gradient 0.3m).
    var wet_intensity = params.weather.z;
    if params.flags.w > 0.0 {
        wet_intensity *= 1.0 - smoothstep(params.flags.w - 0.15, params.flags.w + 0.15, in.world_position.y);
    }
    let wet_mul = clamp(masks.b * wet_intensity, 0.0, 1.0);

    let dirt_col = vec4<f32>(0.15, 0.12, 0.10, 1.0);
    let dirt_mix = mix(layer_color, dirt_col, dirt_mul);

    let wet_dark = dirt_mix * vec4<f32>(0.8, 0.8, 0.8, 1.0);
    let wet_color = mix(dirt_mix, wet_dark, wet_mul);

    let geom_up = max(normalize(pbr_input.world_normal).y, 0.0);
    let detail_up = max(normalize(pbr_input.N).y, 0.0);
    let snow_geom = pow(geom_up, 1.6);
    let snow_detail = pow(detail_up, 2.2);
    var snow_mul = clamp(params.weather.x * (0.7 * snow_geom + 0.3 * snow_detail), 0.0, 1.0);

    // snow_height (flags.z > 0): sníh mizí pod touto world Y hranicí (gradient 0.3m).
    if params.flags.z > 0.0 {
        snow_mul *= smoothstep(params.flags.z - 0.15, params.flags.z + 0.15, in.world_position.y);
    }
    var snow_col = textureSample(snow_texture, snow_sampler, in.uv * params.tiling.y);
    if params.flags.y < 0.5 {
        let luma = dot(layer_color.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let desat = mix(layer_color.rgb, vec3<f32>(luma), 0.75);
        let relief = clamp((detail_up - geom_up) * 1.6 + 0.5, 0.0, 1.0);
        let height_mask = smoothstep(0.35, 0.75, luma);
        let patchiness = mix(0.65, 1.0, height_mask);
        snow_mul *= patchiness;

        let snow_tint = vec3<f32>(0.90, 0.93, 0.98);
        let tint = mix(desat, snow_tint, 0.45 + relief * 0.2);
        let detailed_snow = mix(tint, desat, 0.25);
        snow_col = vec4<f32>(detailed_snow, 1.0);

        let snow_normal_blend = clamp(snow_mul * (0.35 + relief * 0.35), 0.0, 0.7);
        let up_n = normalize(pbr_input.world_normal);
        pbr_input.N = normalize(mix(pbr_input.N, up_n, snow_normal_blend));
    }
    snow_mul = min(snow_mul, 0.82);
    let snow_mix = mix(wet_color, snow_col, snow_mul);
    pbr_input.material.base_color = snow_mix;

    let rough_wet  = mix(roughness_base, 0.02, wet_mul);
    let rough_snow = mix(rough_wet, 0.8, snow_mul);
    let metal_snow = mix(metallic_base, 0.0, snow_mul);
    pbr_input.material.perceptual_roughness = max(rough_snow, 0.2);
    pbr_input.material.metallic = metal_snow;

    // 6. UV1 — druhá vrstva masek (bevy_masks2): AO + emissive
#ifdef VERTEX_UVS_B
    let ao_raw   = clamp(in.uv_b.x, 0.0, 1.0);
    let emissive = clamp(in.uv_b.y, 0.0, 1.0);
    let ao = select(1.0, ao_raw, (ao_raw > 0.0001) || (emissive > 0.0001));
    pbr_input.diffuse_occlusion  *= ao;
    pbr_input.material.base_color *= vec4<f32>(ao, ao, ao, 1.0);
#endif

    // Blender layered_env preview používá alpha z layer-0 albeda, ne MB clip.
    let _mb_alpha = textureSample(mb_texture, mb_sampler, in.uv).a;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
