"""Preview node-graph builder for the standard_pbr ADS template."""
from ..constants import ATTR_NAME
from .common import (
    _material_image, _add_tex_node, _add_value_node, _mix_float,
    build_masks2_ao, build_ao_rgb, build_roughness_clamp,
)


def build_graph(mat, nodes, links):
    props  = mat.bevy_toolkit
    has_ma = props.ma_img is not None

    # --- bevy_masks (COLOR_0): R=layer, G=dirt, B=wet, A=palette -----------
    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-1800, 400)

    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1600, 400)
    links.new(attr.outputs[0], sep.inputs[0])

    # --- UV + tiling --------------------------------------------------------
    uv       = nodes.new("ShaderNodeTexCoord")
    uv.location = (-1800, 50)
    map_node = nodes.new("ShaderNodeMapping")
    map_node.location = (-1600, 50)
    links.new(uv.outputs[2], map_node.inputs[0])

    tiling_val = _add_value_node(nodes, "Tiling", props.tiling, -1800, -120)
    combine    = nodes.new("ShaderNodeCombineXYZ")
    combine.location = (-1600, -120)
    links.new(tiling_val.outputs[0], combine.inputs[0])
    links.new(tiling_val.outputs[0], combine.inputs[1])
    links.new(tiling_val.outputs[0], combine.inputs[2])
    links.new(combine.outputs[0],    map_node.inputs[3])

    # --- Texture nodes ------------------------------------------------------
    alb     = _add_tex_node(nodes, "Albedo",  _material_image(props, "albedo"),  -1320,  420, "sRGB")
    mrao    = _add_tex_node(nodes, "MRAO",    _material_image(props, "mrao"),    -1320,  160, "Non-Color")
    normal  = _add_tex_node(nodes, "Normal",  _material_image(props, "normal"),  -1320,  -80, "Non-Color")
    palette = _add_tex_node(nodes, "Palette", _material_image(props, "palette"), -1320, -340, "sRGB")
    snow    = _add_tex_node(nodes, "Snow",    _material_image(props, "snow"),    -1320, -560, "sRGB")
    ma_tex  = _add_tex_node(nodes, "MA",      _material_image(props, "ma"),      -1320, -780, "Non-Color")
    mb_tex  = _add_tex_node(nodes, "MB",      _material_image(props, "mb"),      -1320,-1000, "Non-Color")

    links.new(map_node.outputs[0], alb.inputs[0])
    links.new(map_node.outputs[0], mrao.inputs[0])
    links.new(map_node.outputs[0], normal.inputs[0])
    links.new(map_node.outputs[0], snow.inputs[0])
    links.new(map_node.outputs[0], ma_tex.inputs[0])
    links.new(uv.outputs[2],       mb_tex.inputs[0])  # MB: raw UV, no tiling

    # --- Tint + palette -----------------------------------------------------
    tint            = nodes.new("ShaderNodeMixRGB")
    tint.label      = "Tint"
    tint.blend_type = "MULTIPLY"
    tint.inputs[0].default_value = 1.0
    tint.inputs[2].default_value = props.tint
    tint.location   = (-980, 440)
    links.new(alb.outputs[0], tint.inputs[1])

    palette_uv = nodes.new("ShaderNodeCombineXYZ")
    palette_uv.location = (-1500, -340)
    links.new(attr.outputs[3], palette_uv.inputs[0])  # A channel = palette index
    palette_uv.inputs[1].default_value = 0.5
    links.new(palette_uv.outputs[0], palette.inputs[0])

    palette_mix            = nodes.new("ShaderNodeMixRGB")
    palette_mix.blend_type = "MIX"
    palette_mix.location   = (-780, 300)
    palette_mix.inputs[1].default_value = (1.0, 1.0, 1.0, 1.0)
    links.new(attr.outputs[3],    palette_mix.inputs[0])
    links.new(palette.outputs[0], palette_mix.inputs[2])

    with_palette            = nodes.new("ShaderNodeMixRGB")
    with_palette.blend_type = "MULTIPLY"
    with_palette.inputs[0].default_value = 1.0
    with_palette.location   = (-560, 420)
    links.new(tint.outputs[0],        with_palette.inputs[1])
    links.new(palette_mix.outputs[0], with_palette.inputs[2])

    # --- Dirt ---------------------------------------------------------------
    dirt_val           = _add_value_node(nodes, "Dirt Level", props.dirt_level, -980, 220)
    dirt_mul           = nodes.new("ShaderNodeMath")
    dirt_mul.operation = "MULTIPLY"
    dirt_mul.location  = (-780, 180)
    links.new(sep.outputs[1],      dirt_mul.inputs[0])
    links.new(dirt_val.outputs[0], dirt_mul.inputs[1])

    dirt_mix = nodes.new("ShaderNodeMixRGB")
    dirt_mix.location = (-340, 360)
    dirt_mix.inputs[2].default_value = (0.2, 0.15, 0.12, 1.0)
    links.new(dirt_mul.outputs[0],     dirt_mix.inputs[0])
    links.new(with_palette.outputs[0], dirt_mix.inputs[1])

    # --- Wet ----------------------------------------------------------------
    wet_val           = _add_value_node(nodes, "Wetness", props.wetness, -980, 80)
    wet_mul           = nodes.new("ShaderNodeMath")
    wet_mul.operation = "MULTIPLY"
    wet_mul.location  = (-780, 60)
    links.new(sep.outputs[2],     wet_mul.inputs[0])
    links.new(wet_val.outputs[0], wet_mul.inputs[1])

    wet_dark            = nodes.new("ShaderNodeMixRGB")
    wet_dark.blend_type = "MULTIPLY"
    wet_dark.inputs[0].default_value = 1.0
    wet_dark.inputs[2].default_value = (0.7, 0.7, 0.7, 1.0)
    wet_dark.location   = (-120, 340)
    links.new(dirt_mix.outputs[0], wet_dark.inputs[1])

    wet_color = nodes.new("ShaderNodeMixRGB")
    wet_color.location = (120, 340)
    links.new(wet_mul.outputs[0],  wet_color.inputs[0])
    links.new(dirt_mix.outputs[0], wet_color.inputs[1])
    links.new(wet_dark.outputs[0], wet_color.inputs[2])

    # --- Snow (geometry normal Y) -------------------------------------------
    geom       = nodes.new("ShaderNodeNewGeometry")
    geom.location = (-980, -460)
    normal_sep = nodes.new("ShaderNodeSeparateXYZ")
    normal_sep.location = (-780, -460)
    links.new(geom.outputs[1], normal_sep.inputs[0])

    snow_val           = _add_value_node(nodes, "Snow Level", props.snow_level, -780, -620)
    snow_mul           = nodes.new("ShaderNodeMath")
    snow_mul.operation = "MULTIPLY"
    snow_mul.location  = (-560, -540)
    links.new(normal_sep.outputs[1], snow_mul.inputs[0])
    links.new(snow_val.outputs[0],   snow_mul.inputs[1])

    snow_mix = nodes.new("ShaderNodeMixRGB")
    snow_mix.location = (340, 300)
    links.new(snow_mul.outputs[0],  snow_mix.inputs[0])
    links.new(wet_color.outputs[0], snow_mix.inputs[1])
    links.new(snow.outputs[0],      snow_mix.inputs[2])

    # --- Roughness / Metalness ----------------------------------------------
    # MA present → G=roughness, B=metalness (AO channel skipped to avoid blackening).
    # MA absent  → MRAO with fallback defaults (rough=0.5, metal=0.0).
    if has_ma:
        ma_sep = nodes.new("ShaderNodeSeparateColor")
        ma_sep.location = (-1100, -780)
        links.new(ma_tex.outputs[0], ma_sep.inputs[0])
        roughness_socket = ma_sep.outputs[1]  # G
        metalness_socket = ma_sep.outputs[2]  # B
    else:
        mrao_sep = nodes.new("ShaderNodeSeparateColor")
        mrao_sep.location = (-980, 120)
        links.new(mrao.outputs[0], mrao_sep.inputs[0])
        rough_default  = _add_value_node(nodes, "Rough Default", 0.5,                             -980,  40)
        rough_has_mrao = _add_value_node(nodes, "Has MRAO",      1.0 if props.mrao_img else 0.0, -980, -40)
        roughness_socket = _mix_float(nodes, links,
                                      rough_has_mrao.outputs[0],
                                      rough_default.outputs[0],
                                      mrao_sep.outputs[1],
                                      -760, 40, "RoughFallback")
        metal_default  = _add_value_node(nodes, "Metal Default", 0.0, -980, -100)
        metalness_socket = _mix_float(nodes, links,
                                      rough_has_mrao.outputs[0],
                                      metal_default.outputs[0],
                                      mrao_sep.outputs[0],
                                      -760, -100, "MetalFallback")

    rough_dirt = _mix_float(nodes, links, dirt_mul.outputs[0], roughness_socket, 0.9,  -340, 100, "RoughDirt")
    rough_wet  = _mix_float(nodes, links, wet_mul.outputs[0],  rough_dirt,        0.02, -120, 100, "RoughWet")
    rough_snow = _mix_float(nodes, links, snow_mul.outputs[0], rough_wet,         0.8,   120, 100, "RoughSnow")
    metal_snow = _mix_float(nodes, links, snow_mul.outputs[0], metalness_socket,  0.0,   120, -20, "MetalSnow")

    normal_map = nodes.new("ShaderNodeNormalMap")
    normal_map.location = (-120, -150)
    links.new(normal.outputs[0], normal_map.inputs[1])

    # --- bevy_masks2: R=AO (1=none), G=emissive ----------------------------
    _, sep2, ao_source = build_masks2_ao(nodes, links, 200, -200)

    ao_rgb = build_ao_rgb(nodes, links, ao_source, 690, -200)

    ao_mul            = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location   = (590, 260)
    links.new(snow_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0],   ao_mul.inputs[2])

    emissive_rgb = nodes.new("ShaderNodeCombineColor")
    emissive_rgb.location = (590, -340)
    links.new(sep2.outputs[1], emissive_rgb.inputs[0])
    links.new(sep2.outputs[1], emissive_rgb.inputs[1])
    links.new(sep2.outputs[1], emissive_rgb.inputs[2])

    emissive_mul            = nodes.new("ShaderNodeMixRGB")
    emissive_mul.blend_type = "MULTIPLY"
    emissive_mul.inputs[0].default_value = 1.0
    emissive_mul.location   = (590, 120)
    links.new(alb.outputs[0],          emissive_mul.inputs[1])
    links.new(emissive_rgb.outputs[0], emissive_mul.inputs[2])

    # --- BSDF ---------------------------------------------------------------
    # Clamp roughness ≥0.2: prevents near-black appearance in EEVEE without HDRI.
    rough_clamped = build_roughness_clamp(nodes, links, rough_snow, 0.2, 720, 90)

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (820, 260)
    links.new(ao_mul.outputs[0],   bsdf.inputs["Base Color"])
    links.new(metal_snow,          bsdf.inputs["Metallic"])
    links.new(rough_clamped,       bsdf.inputs["Roughness"])
    links.new(normal_map.outputs[0], bsdf.inputs["Normal"])

    # Always use albedo.Alpha — connecting MB would cause Blender GLTF exporter
    # to merge textures into a packed PNG artefact. MB opacity is handled by
    # Bevy's StandardPbrExtension directly.
    links.new(alb.outputs[1], bsdf.inputs["Alpha"])

    try:
        links.new(emissive_mul.outputs[0], bsdf.inputs["Emission Color"])
        bsdf.inputs["Emission Strength"].default_value = 1.0
    except KeyError:
        pass

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (1120, 260)
    links.new(bsdf.outputs[0], out.inputs[0])
