#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::pbr_functions as pbr_functions
#import bevy_pbr::pbr_types as pbr_types

struct LayeredMaterialParams {
    l0_tiling: f32,
    l1_tiling: f32,
    porosity: f32,
    wetness: f32,
    snow_level: f32,
    dirt_level: f32,
    pad1: f32,
    pad2: f32,
};

@group(2) @binding(0) var<uniform> params: LayeredMaterialParams;

// Sdílený sampler pro všechno
@group(2) @binding(1) var shared_sampler: sampler;

// Layer 0 (Base)
@group(2) @binding(2) var l0_albedo_tex: texture_2d<f32>;
@group(2) @binding(3) var l0_mrao_tex: texture_2d<f32>;
@group(2) @binding(4) var l0_normal_tex: texture_2d<f32>;

// Layer 1 (Overlay)
@group(2) @binding(5) var l1_albedo_tex: texture_2d<f32>;
@group(2) @binding(6) var l1_mrao_tex: texture_2d<f32>;
@group(2) @binding(7) var l1_normal_tex: texture_2d<f32>;

// Ekologie
@group(2) @binding(8) var snow_tex: texture_2d<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv0 = in.uv * params.l0_tiling;
    let uv1 = in.uv * params.l1_tiling;

    // 1. BLENDING VRSTEV (Červený kanál)
    let blend_mask = in.color.r;

    let alb0 = textureSample(l0_albedo_tex, shared_sampler, uv0).rgb;
    let alb1 = textureSample(l1_albedo_tex, shared_sampler, uv1).rgb;
    var final_albedo = mix(alb0, alb1, blend_mask);

    let mrao0 = textureSample(l0_mrao_tex, shared_sampler, uv0);
    let mrao1 = textureSample(l1_mrao_tex, shared_sampler, uv1);
    var final_roughness = mix(mrao0.g, mrao1.g, blend_mask);
    var final_metallic = mix(mrao0.r, mrao1.r, blend_mask);
    var final_ao = mix(mrao0.b, mrao1.b, blend_mask);

    let norm0 = textureSample(l0_normal_tex, shared_sampler, uv0).rgb;
    let norm1 = textureSample(l1_normal_tex, shared_sampler, uv1).rgb;
    var final_normal_map = mix(norm0, norm1, blend_mask);

    // 2. ŠPÍNA (Zelený kanál)
    let dirt_mask = in.color.g * params.dirt_level;
    final_albedo = mix(final_albedo, vec3<f32>(0.15, 0.12, 0.1), dirt_mask);
    final_roughness = mix(final_roughness, 0.9, dirt_mask);

    // 3. VLHKOST A KALUŽE (Modrý kanál)
    let is_puddle = in.color.b * params.wetness;
    let wet_darken = mix(1.0, mix(1.0, 0.4, params.porosity), params.wetness);
    
    final_albedo *= mix(wet_darken, 0.8, is_puddle);
    final_roughness = mix(final_roughness, 0.02, is_puddle);
    final_normal_map = mix(final_normal_map, vec3<f32>(0.5, 0.5, 1.0), is_puddle);

    // 4. SNÍH (Směr nahoru)
    let up_factor = max(0.0, in.world_normal.y); 
    let snow_mask = smoothstep(0.4, 0.8, up_factor * params.snow_level * 2.0);
    
    let snow_color = textureSample(snow_tex, shared_sampler, in.world_position.xz * 0.2).rgb;
    final_albedo = mix(final_albedo, snow_color, snow_mask);
    final_roughness = mix(final_roughness, 0.8, snow_mask);
    final_metallic = mix(final_metallic, 0.0, snow_mask);

    // 5. BEVY PBR OUTPUT
    var pbr_input = pbr_functions::pbr_input_new();
    pbr_input.material.base_color = vec4<f32>(final_albedo, 1.0);
    pbr_input.material.perceptual_roughness = final_roughness;
    pbr_input.material.metallic = final_metallic;
    pbr_input.material.occlusion = final_ao;
    pbr_input.frag_coord = in.position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = pbr_functions::prepare_world_normal(
        final_normal_map, false, in.world_normal
    );
    pbr_input.is_orthographic = false;

    return pbr_functions::pbr(pbr_input);
}