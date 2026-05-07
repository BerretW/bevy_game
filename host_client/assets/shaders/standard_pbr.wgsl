#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::pbr_functions as pbr_functions
#import bevy_pbr::pbr_types as pbr_types

struct StandardMaterialParams {
    tint: vec4<f32>,
    wetness: f32,
    snow_level: f32,
    dirt_level: f32,
    tiling: f32,
};

@group(2) @binding(0) var<uniform> params: StandardMaterialParams;
@group(2) @binding(1) var albedo_tex: texture_2d<f32>;
@group(2) @binding(2) var shared_sampler: sampler;
@group(2) @binding(3) var mrao_tex: texture_2d<f32>; // R=Metal, G=Roughness, B=AO
@group(2) @binding(4) var normal_tex: texture_2d<f32>;
@group(2) @binding(5) var palette_tex: texture_2d<f32>; // 1D Lut textura pro obarvování
@group(2) @binding(6) var snow_tex: texture_2d<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * params.tiling;
    
    // 1. ZÁKLADNÍ TEXTURY
    let albedo = textureSample(albedo_tex, shared_sampler, uv);
    let mrao = textureSample(mrao_tex, shared_sampler, uv);
    var final_albedo = albedo.rgb * params.tint.rgb;
    var final_roughness = mrao.g;
    var final_metallic = mrao.r;
    var final_normal_map = textureSample(normal_tex, shared_sampler, uv).rgb;

    // 2. PALETTE TINTING (Alpha kanál Vertex Color)
    // in.color.a říká, na jaké souřadnici v paletě máme číst barvu
    if (in.color.a > 0.01) {
        let tint_color = textureSample(palette_tex, shared_sampler, vec2<f32>(in.color.a, 0.5)).rgb;
        final_albedo *= tint_color;
    }

    // 3. ŠPÍNA (Zelený kanál)
    let dirt_mask = in.color.g * params.dirt_level;
    let dirt_color = vec3<f32>(0.2, 0.15, 0.12);
    final_albedo = mix(final_albedo, dirt_color, dirt_mask);
    final_roughness = mix(final_roughness, 0.9, dirt_mask);

    // 4. VLHKOST A KALUŽE (Modrý kanál)
    let is_puddle = in.color.b * params.wetness;
    let wet_darken = mix(1.0, 0.5, params.wetness); // Základní ztmavení od deště
    
    final_albedo *= mix(wet_darken, 0.7, is_puddle); 
    final_roughness = mix(final_roughness, 0.02, is_puddle); // Kaluže jsou zrcadlové
    final_normal_map = mix(final_normal_map, vec3<f32>(0.5, 0.5, 1.0), is_puddle); // Kaluže vyhlazují povrch

    // 5. SNÍH (Podle orientace nahoru)
    let up_factor = max(0.0, in.world_normal.y); 
    let snow_mask = smoothstep(0.4, 0.8, up_factor * params.snow_level * 2.0);
    
    let snow_color = textureSample(snow_tex, shared_sampler, in.world_position.xz * 0.2).rgb;
    final_albedo = mix(final_albedo, snow_color, snow_mask);
    final_roughness = mix(final_roughness, 0.8, snow_mask);
    final_metallic = mix(final_metallic, 0.0, snow_mask);

    // 6. BEVY PBR OUTPUT
    var pbr_input = pbr_functions::pbr_input_new();
    pbr_input.material.base_color = vec4<f32>(final_albedo, albedo.a);
    pbr_input.material.perceptual_roughness = final_roughness;
    pbr_input.material.metallic = final_metallic;
    pbr_input.material.occlusion = mrao.b;
    
    pbr_input.frag_coord = in.position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = pbr_functions::prepare_world_normal(
        final_normal_map, false, in.world_normal
    );
    pbr_input.is_orthographic = false;

    return pbr_functions::pbr(pbr_input);
}