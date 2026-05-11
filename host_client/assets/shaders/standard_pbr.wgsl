// standard_pbr.wgsl — StandardPbrExtension fragment shader
//
// Vertex color konvence (COLOR_0 = bevy_masks):
//   R = rezervováno (nevyužito)
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
    tiling:  vec4<f32>,  // x=tiling, y=debug_viewer_mode, z=l1_tiling, w=mb_alpha_threshold
    flags:   vec4<f32>,  // x=has_ma, y=has_snow_tex, z=snow_height_cutoff_y, w=wet_height_cutoff_y
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var palette_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var palette_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var snow_texture:    texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var snow_sampler:    sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var ma_texture:      texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var ma_sampler:      sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var<uniform> params: DrawableParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var mb_texture:      texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var mb_sampler:      sampler;

// Value noise pro procedurální snow grain (nezávislý na base textuře)
fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash2(i),                 hash2(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let masks = in.color;   // R=reserved, G=dirt, B=wet, A=palette
    // Debug mode: params.tiling.y < 0 → zobrazit vertex color kanály jako albedo (viewer only)
    if params.tiling.y < 0.0 {
        var dbg: FragmentOutput;
        let ch = abs(params.tiling.y);
        // -1=RGB, -2=R (normal_suppress), -3=G (dirt), -4=B (wet), -5=A (palette)
        if ch < 1.5 {
            dbg.color = vec4<f32>(masks.rgb, 1.0);
        } else if ch < 2.5 {
            dbg.color = vec4<f32>(masks.r, masks.r, masks.r, 1.0);
        } else if ch < 3.5 {
            dbg.color = vec4<f32>(masks.g, masks.g, masks.g, 1.0);
        } else if ch < 4.5 {
            dbg.color = vec4<f32>(masks.b, masks.b, masks.b, 1.0);
        } else {
            dbg.color = vec4<f32>(masks.a, masks.a, masks.a, 1.0);
        }
        return dbg;
    }
    var pbr_in = in;
    pbr_in.color = vec4<f32>(1.0);

    var pbr_input = pbr_input_from_standard_material(pbr_in, is_front);

    let base_albedo = pbr_input.material.base_color;

    // 2. Tint + paleta (1D LUT): palette mix = lerp(white, palette, A)
    let palette_uv  = vec2<f32>(masks.a, 0.5);
    let palette_col = textureSample(palette_texture, palette_sampler, palette_uv);
    let palette_mix = mix(vec4<f32>(1.0), palette_col, clamp(masks.a, 0.0, 1.0));
    let with_palette = pbr_input.material.base_color * params.tint * palette_mix;

    // 3. Dirt / wet / snow přesně podle Blender graphu
    let dirt_mul = clamp(masks.g * params.weather.y, 0.0, 1.0);

    // wet_height (flags.w > 0): vlhkost mizí nad touto world Y hranicí (gradient 0.3m).
    var wet_intensity = params.weather.z;
    if params.flags.w > 0.0 {
        wet_intensity *= 1.0 - smoothstep(params.flags.w - 0.15, params.flags.w + 0.15, in.world_position.y);
    }
    let wet_mul = clamp(masks.b * wet_intensity, 0.0, 1.0);

    let dirt_col = vec4<f32>(0.2, 0.15, 0.12, 1.0);
    let dirt_mix = mix(with_palette, dirt_col, dirt_mul);

    let wet_dark = dirt_mix * vec4<f32>(0.7, 0.7, 0.7, 1.0);
    let wet_color = mix(dirt_mix, wet_dark, wet_mul);

    let geom_up   = max(normalize(pbr_input.world_normal).y, 0.0);
    let detail_up = max(normalize(pbr_input.N).y, 0.0);

    // Akumulace: součin geom × detail → sníh jen kde obě normály míří nahoru.
    // pow(0.55) = mírnější křivka → mortar dostane trochu sněhu při silném sněžení.
    let snow_coverage = geom_up * detail_up;
    let snow_raw = clamp(params.weather.x * pow(snow_coverage, 0.55), 0.0, 1.0);
    var snow_mul = smoothstep(0.05, 0.62, snow_raw);

    // snow_height (flags.z > 0): sníh mizí pod touto world Y hranicí (gradient 0.3m).
    if params.flags.z > 0.0 {
        snow_mul *= smoothstep(params.flags.z - 0.15, params.flags.z + 0.15, in.world_position.y);
    }

    // Hranový AO: kde pokrytí prudce klesá (přechod sníh→holý kamen), ztmavíme sníh.
    // Simuluje stín, který hrana sněhové vrstvy vrhá na bok — iluze výšky/objemu.
    let edge_dx  = dpdx(snow_raw);
    let edge_dy  = dpdy(snow_raw);
    let edge_mag = length(vec2<f32>(edge_dx, edge_dy));
    let edge_ao  = 1.0 - clamp(edge_mag * 12.0, 0.0, 0.35);

    let snow_uv  = in.uv * params.tiling.x;
    var snow_col = textureSample(snow_texture, snow_sampler, snow_uv);

    // Procedurální snow grain: value noise nezávislý na base textuře.
    // Dává sněhu vlastní krystalový pattern (neodvozený od kamene pod ním).
    let grain_uv  = snow_uv * 14.0;
    let grain_a   = value_noise(grain_uv);
    let grain_b   = value_noise(grain_uv * 2.3 + vec2<f32>(5.1, 3.7));
    let grain_c   = value_noise(grain_uv * 5.1 + vec2<f32>(1.3, 8.2));
    // Tříoktávový noise: hrubý základ + střední detail + jemné krystaly
    let snow_grain = grain_a * 0.55 + grain_b * 0.30 + grain_c * 0.15;

    if params.flags.y < 0.5 {
        // Bez snow textury: čistá bílá modulovaná grain noisem
        let luma        = dot(with_palette.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let relief      = clamp((detail_up - geom_up) * 1.6 + 0.5, 0.0, 1.0);
        let height_mask = smoothstep(0.28, 0.64, luma);
        snow_mul       *= mix(0.60, 1.0, height_mask);

        let snow_white = vec3<f32>(0.92, 0.95, 1.00);
        // Grain moduluje jas: jemná variace 0.82–1.0 → krystalový vzhled
        let grain_bright = mix(0.82, 1.00, snow_grain);
        // Prohlubeň (low height_mask + relief) = stopa materiálu; výčnělek = čistá bílá
        let hollow = mix(with_palette.rgb * 0.5, snow_white, 0.20);
        let crest  = snow_white * grain_bright;
        snow_col = vec4<f32>(
            mix(hollow, crest, clamp(height_mask * 0.55 + relief * 0.45, 0.0, 1.0)) * edge_ao,
            1.0
        );
    } else {
        // Se snow texturou: přimíchej grain pro krystalový detail
        let grain_bright = mix(0.88, 1.00, snow_grain);
        snow_col = vec4<f32>(snow_col.rgb * grain_bright * edge_ao, snow_col.a);
    }

    snow_mul = min(snow_mul, 0.94);
    pbr_input.material.base_color = mix(wet_color, snow_col, snow_mul);

    // Snow micro-normaly: vlastní grain noise → normals nezávislé na základní textuře.
    // NEBLENDUJE k flat: base normal + snow grain se sčítají.
    if snow_mul > 0.01 {
        let geo_n  = normalize(pbr_input.world_normal);
        let dP_dx  = dpdx(in.world_position.xyz);
        let dP_dy  = dpdy(in.world_position.xyz);
        // Výška = snow grain noise (ne stone albedo!) → skutečný snow grain pattern
        let str    = snow_mul * 12.0;
        let T      = dP_dx + dpdx(snow_grain) * str * geo_n;
        let B      = dP_dy + dpdy(snow_grain) * str * geo_n;
        var mN     = normalize(cross(T, B));
        mN = select(-mN, mN, dot(mN, geo_n) > 0.0);
        // 50% snow grain normal, 50% původní → nerovnosti pod sněhem prosvítí
        pbr_input.N = normalize(mix(pbr_input.N, mN, clamp(snow_mul * 0.50, 0.0, 0.50)));
    }

    // 4. Roughness / metallic: MA přepisuje MRAO stejně jako v Blenderu.
    var roughness_base = pbr_input.material.perceptual_roughness;
    var metallic_base  = pbr_input.material.metallic;
    if params.flags.x > 0.5 {
        let ma = textureSample(ma_texture, ma_sampler, in.uv * params.tiling.x);
        roughness_base = ma.g;
        metallic_base  = ma.b;
    }

    let rough_dirt = mix(roughness_base, 0.9, dirt_mul);
    let rough_wet  = mix(rough_dirt, 0.02, wet_mul);
    let rough_snow = mix(rough_wet, 0.62, snow_mul);  // nižší = krystalové specular highlights
    let metal_snow = mix(metallic_base, 0.0, snow_mul);

    // Blender clamp: min roughness 0.2 (EEVEE parity)
    pbr_input.material.perceptual_roughness = max(rough_snow, 0.2);
    pbr_input.material.metallic = metal_snow;

    // 6. UV1 — druhá vrstva masek (bevy_masks2): AO + emissive
    //    Dostupná jen pokud mesh má TEXCOORD_1 (Bevy nastaví VERTEX_UVS_B).
#ifdef VERTEX_UVS_B
    let ao_raw    = clamp(in.uv_b.x, 0.0, 1.0);  // 1.0=žádný AO, 0.0=maximální ztmavení
    let emissive  = clamp(in.uv_b.y, 0.0, 1.0);  // 0.0=žádný glow
    // Fallback: pokud bevy_masks2 není inicializované, bývá (0,0) a způsobí černý materiál.
    let ao = select(1.0, ao_raw, (ao_raw > 0.0001) || (emissive > 0.0001));
    pbr_input.diffuse_occlusion  *= ao;
    pbr_input.material.emissive *= emissive;
    pbr_input.material.base_color *= vec4<f32>(ao, ao, ao, 1.0);

    // Blender emissive: emissive = albedo * emissive_mask
    pbr_input.material.emissive += vec4<f32>(base_albedo.rgb * emissive, 0.0);
#endif

    // Blender preview nepoužívá MB alpha clip; alpha jde z albedo texture.
    let _mb_alpha = textureSample(mb_texture, mb_sampler, in.uv).a;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
