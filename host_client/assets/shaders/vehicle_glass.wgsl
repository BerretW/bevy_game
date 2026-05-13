// vehicle_glass.wgsl — VehicleGlassExtension fragment shader
//
// Průhledné sklo pro vozidla. StandardMaterial nastavuje alpha_mode=Blend,
// nízkou roughness a double_sided. Tato extension aplikuje tint ze params
// a efekt mokrého skla (wetness sníží roughness → ostřejší odrazy).
//
// Vertex colors: nevyužito (sklo nepoužívá masky).

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct DrawableParams {
    tint:    vec4<f32>,  // RGBA — alpha řídí průhlednost skla
    weather: vec4<f32>,  // x=snow_level, y=dirt_level, z=wetness, w=porosity
    tiling:  vec4<f32>,
    flags:   vec4<f32>,
    profile: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var shatter_map_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var shatter_map_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> params: DrawableParams;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_in = in;
    pbr_in.color = vec4<f32>(1.0);

    var pbr_input = pbr_input_from_standard_material(pbr_in, is_front);

    // 1. Aplikuj tint skla
    pbr_input.material.base_color *= params.tint;

    // 2. Shatter map (R channel + smooth ramp 0.1..0.4)
    let shatter_raw = textureSample(shatter_map_texture, shatter_map_sampler, in.uv).r;
    let shatter_cut = smoothstep(0.1, 0.4, shatter_raw);

    // crack_mix: mix(white-ish, glass, shatter_cut)
    let crack_mix = mix(vec4<f32>(0.9, 0.9, 0.9, 1.0), pbr_input.material.base_color, shatter_cut);

    // 3. Dirt
    let dirt_mul = clamp(in.color.g * params.weather.y, 0.0, 1.0);
    let dirt_mix = mix(crack_mix, vec4<f32>(0.2, 0.18, 0.15, 1.0), dirt_mul);
    pbr_input.material.base_color = dirt_mix;

    // 4. Alpha: alpha_mul -> dirt -> rain clean
    let alpha_mul  = pbr_input.material.base_color.a * shatter_cut;
    let alpha_dirt = mix(alpha_mul, 0.95, dirt_mul);
    let alpha_wet  = alpha_dirt - clamp(params.weather.z, 0.0, 1.0) * 0.4;
    pbr_input.material.base_color.a = clamp(alpha_wet, 0.0, 1.0);

    // 5. Roughness chain (crack -> dirt -> wet), pak clamp min 0.05
    let rough_crack = mix(0.7, 0.05, shatter_cut);
    let rough_dirt  = mix(rough_crack, 0.8, dirt_mul);
    let rough_wet   = mix(rough_dirt, 0.01, clamp(params.weather.z, 0.0, 1.0));
    pbr_input.material.perceptual_roughness = max(rough_wet, 0.05);

    // Blender preview udržuje sklo nemetalické.
    pbr_input.material.metallic = 0.0;

    // 6. UV1 — AO
#ifdef VERTEX_UVS_B
    let ao_raw = clamp(in.uv_b.x, 0.0, 1.0);
    let emissive = clamp(in.uv_b.y, 0.0, 1.0);
    let ao = select(1.0, ao_raw, (ao_raw > 0.0001) || (emissive > 0.0001));
    pbr_input.diffuse_occlusion *= ao;
    pbr_input.material.base_color *= vec4<f32>(ao, ao, ao, 1.0);
#endif

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
