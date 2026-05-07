#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::pbr_functions as pbr_functions
#import bevy_pbr::pbr_types as pbr_types

struct GlassParams {
    wetness: f32,
    dirt_level: f32,
    pad1: f32,
    pad2: f32,
};

@group(2) @binding(0) var<uniform> params: GlassParams;
@group(2) @binding(1) var shared_sampler: sampler;
@group(2) @binding(2) var glass_albedo_tex: texture_2d<f32>; 
@group(2) @binding(3) var shatter_map_tex: texture_2d<f32>; // Maska rozbití

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let glass = textureSample(glass_albedo_tex, shared_sampler, in.uv);
    let shatter = textureSample(shatter_map_tex, shared_sampler, in.uv).r;

    var final_color = glass.rgb;
    var final_alpha = glass.a;
    var final_roughness = 0.05; // Sklo je velmi hladké

    // 1. SHATTER MAP (Zničení z Rustu)
    if (shatter < 0.1) {
        discard; // Zde je sklo vysypané
    }
    
    // Hrany prasklin jsou matné a bílé (imitace roztříštěného skla)
    let crack_edge = smoothstep(0.1, 0.4, shatter);
    final_color = mix(vec3<f32>(0.9, 0.9, 0.9), final_color, crack_edge);
    final_roughness = mix(0.7, final_roughness, crack_edge);

    // 2. ŠPÍNA NA SKLE (Zelený kanál)
    // Špína dělá okno neprůhledným!
    let dirt_mask = in.color.g * params.dirt_level;
    let dirt_color = vec3<f32>(0.2, 0.18, 0.15);
    final_color = mix(final_color, dirt_color, dirt_mask);
    final_alpha = mix(final_alpha, 0.95, dirt_mask); // Špinavé okno nepropouští světlo
    final_roughness = mix(final_roughness, 0.8, dirt_mask);

    // 3. VYČIŠTĚNÍ DEŠTĚM
    // Pokud prší, špína ze skla trochu stéká
    let rain_cleaning = params.wetness * 0.4;
    final_alpha = max(glass.a, final_alpha - rain_cleaning);
    final_roughness = mix(final_roughness, 0.01, params.wetness);

    // 4. BEVY PBR OUTPUT (Transparent)
    var pbr_input = pbr_functions::pbr_input_new();
    pbr_input.material.base_color = vec4<f32>(final_color, final_alpha);
    pbr_input.material.perceptual_roughness = final_roughness;
    pbr_input.material.metallic = 0.0;
    
    // Sklo hodně zrcadlí okolí pod úhlem (Fresnelův jev), v Bevy k tomu slouží reflectance
    pbr_input.material.reflectance = 0.8; 
    
    pbr_input.frag_coord = in.position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = pbr_functions::prepare_world_normal(
        vec3<f32>(0.5, 0.5, 1.0), false, in.world_normal
    );
    pbr_input.is_orthographic = false;

    return pbr_functions::pbr(pbr_input);
}