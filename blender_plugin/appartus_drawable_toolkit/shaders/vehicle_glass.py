"""Preview node-graph builder for the vehicle_glass ADS template."""
from ..constants import ATTR_NAME
from .common import (
    _material_image, _add_tex_node, _add_value_node, _mix_float,
    build_masks2_ao, build_ao_rgb, build_roughness_clamp,
)


def build_graph(mat, nodes, links):
    props = mat.bevy_toolkit

    # --- bevy_masks (COLOR_0): G=dirt ----------------------------------------
    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-1500, 220)
    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1300, 220)
    links.new(attr.outputs[0], sep.inputs[0])

    uv = nodes.new("ShaderNodeTexCoord")
    uv.location = (-1500, 20)

    # --- Texture nodes -------------------------------------------------------
    glass   = _add_tex_node(nodes, "Glass Albedo", _material_image(props, "glass_albedo"), -1180,  260, "sRGB")
    shatter = _add_tex_node(nodes, "Shatter Map",  _material_image(props, "shatter_map"),  -1180,   40, "Non-Color")
    mb_tex  = _add_tex_node(nodes, "MB",           _material_image(props, "mb"),           -1180, -200, "Non-Color")
    links.new(uv.outputs[2], glass.inputs[0])
    links.new(uv.outputs[2], shatter.inputs[0])
    links.new(uv.outputs[2], mb_tex.inputs[0])

    # --- Shatter mask --------------------------------------------------------
    shatter_r = nodes.new("ShaderNodeSeparateColor")
    shatter_r.location = (-980, 40)
    links.new(shatter.outputs[0], shatter_r.inputs[0])

    shatter_cut = nodes.new("ShaderNodeValToRGB")
    shatter_cut.location = (-780, 20)
    shatter_cut.color_ramp.elements[0].position = 0.1
    shatter_cut.color_ramp.elements[0].color    = (0.0, 0.0, 0.0, 1.0)
    shatter_cut.color_ramp.elements[1].position = 0.4
    shatter_cut.color_ramp.elements[1].color    = (1.0, 1.0, 1.0, 1.0)
    links.new(shatter_r.outputs[0], shatter_cut.inputs[0])

    crack_mix = nodes.new("ShaderNodeMixRGB")
    crack_mix.location = (-540, 240)
    crack_mix.inputs[1].default_value = (0.9, 0.9, 0.9, 1.0)
    links.new(shatter_cut.outputs[0], crack_mix.inputs[0])
    links.new(glass.outputs[0],       crack_mix.inputs[2])

    # --- Dirt ----------------------------------------------------------------
    dirt_val           = _add_value_node(nodes, "Dirt Level", props.dirt_level, -980, -140)
    dirt_mul           = nodes.new("ShaderNodeMath")
    dirt_mul.operation = "MULTIPLY"
    dirt_mul.location  = (-780, -140)
    links.new(sep.outputs[1],      dirt_mul.inputs[0])
    links.new(dirt_val.outputs[0], dirt_mul.inputs[1])

    dirt_mix = nodes.new("ShaderNodeMixRGB")
    dirt_mix.location = (-320, 220)
    dirt_mix.inputs[2].default_value = (0.2, 0.18, 0.15, 1.0)
    links.new(dirt_mul.outputs[0],  dirt_mix.inputs[0])
    links.new(crack_mix.outputs[0], dirt_mix.inputs[1])

    # --- Alpha (glass transparency with crack + dirt + rain-clean) -----------
    wet_val   = _add_value_node(nodes, "Wetness", props.wetness, -780, -260)
    alpha_mul = nodes.new("ShaderNodeMath")
    alpha_mul.operation = "MULTIPLY"
    alpha_mul.location  = (-320, 20)
    links.new(glass.outputs[1],       alpha_mul.inputs[0])
    links.new(shatter_cut.outputs[0], alpha_mul.inputs[1])

    alpha_dirt = _mix_float(nodes, links, dirt_mul.outputs[0], alpha_mul.outputs[0], 0.95, -100, 20, "GlassAlphaDirt")

    alpha_wet           = nodes.new("ShaderNodeMath")
    alpha_wet.operation = "SUBTRACT"
    alpha_wet.location  = (120, 20)
    links.new(alpha_dirt, alpha_wet.inputs[0])

    rain_clean           = nodes.new("ShaderNodeMath")
    rain_clean.operation = "MULTIPLY"
    rain_clean.inputs[1].default_value = 0.4
    rain_clean.location  = (-100, -120)
    links.new(wet_val.outputs[0],    rain_clean.inputs[0])
    links.new(rain_clean.outputs[0], alpha_wet.inputs[1])

    # --- Roughness -----------------------------------------------------------
    rough_crack = _mix_float(nodes, links, shatter_cut.outputs[0], 0.7,         0.05,  -320, -200, "GlassRoughCrack")
    rough_dirt  = _mix_float(nodes, links, dirt_mul.outputs[0],    rough_crack, 0.8,   -100, -200, "GlassRoughDirt")
    rough_wet   = _mix_float(nodes, links, wet_val.outputs[0],     rough_dirt,  0.01,   120, -200, "GlassRoughWet")

    # --- bevy_masks2: R=AO (1=none) ------------------------------------------
    _, _sep2, ao_source = build_masks2_ao(nodes, links, 220, -280)

    ao_rgb = build_ao_rgb(nodes, links, ao_source, 660, -280)

    ao_mul            = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location   = (560, 200)
    links.new(dirt_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0],   ao_mul.inputs[2])

    # --- BSDF ----------------------------------------------------------------
    # Glass can be quite smooth; lower roughness floor than other templates.
    rough_clamped = build_roughness_clamp(nodes, links, rough_wet, 0.05, 660, -30)

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (760, 200)
    for _name in ("Transmission Weight", "Transmission"):
        if _name in bsdf.inputs:
            bsdf.inputs[_name].default_value = 1.0
            break
    for _name in ("Specular IOR Level", "Specular"):
        if _name in bsdf.inputs:
            bsdf.inputs[_name].default_value = 0.8
            break
    links.new(ao_mul.outputs[0],    bsdf.inputs["Base Color"])
    links.new(rough_clamped,        bsdf.inputs["Roughness"])
    links.new(alpha_wet.outputs[0], bsdf.inputs["Alpha"])

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (1060, 200)
    links.new(bsdf.outputs[0], out.inputs[0])
