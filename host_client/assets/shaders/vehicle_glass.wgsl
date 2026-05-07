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
    tiling:  vec4<f32>,  // nevyužito
}

@group(2) @binding(100) var<uniform> params: DrawableParams;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_in = in;
    pbr_in.color = vec4<f32>(1.0);

    var pbr_input = pbr_input_from_standard_material(pbr_in, is_front);

    // 1. Aplikuj tint (přebarvení skla, alpha z params.tint)
    pbr_input.material.base_color *= params.tint;

    // 2. Špína na skle (dirt_level) — sklo se zakalí
    let dirt = clamp(params.weather.y, 0.0, 1.0);
    pbr_input.material.base_color = mix(
        pbr_input.material.base_color,
        vec4<f32>(0.18, 0.15, 0.12, 0.9),  // zakalená špinavá vrstva
        dirt * 0.6
    );
    pbr_input.material.perceptual_roughness = mix(
        pbr_input.material.perceptual_roughness,
        0.6,
        dirt
    );

    // 3. Vlhkost (wetness) — mokré sklo má velmi nízkou roughness → ostré odrazy
    let wet = clamp(params.weather.z, 0.0, 1.0);
    pbr_input.material.perceptual_roughness = mix(
        pbr_input.material.perceptual_roughness,
        0.02,
        wet
    );
    // Déšť částečně splachuje špínu
    pbr_input.material.base_color.a = mix(
        pbr_input.material.base_color.a,
        params.tint.a,
        wet * 0.4
    );

    // 4. UV1 — bevy_masks2: AO (sklo nemá emissive)
#ifdef VERTEX_UVS_B
    let ao = clamp(in.uv_b.x, 0.0, 1.0);
    pbr_input.occlusion *= vec3(ao);
#endif

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
