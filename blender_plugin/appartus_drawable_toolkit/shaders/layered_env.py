"""Preview node-graph builder for the layered_env ADS template."""
from ..constants import ATTR_NAME
from .common import (
    _material_image, _add_tex_node, _add_value_node, _mix_float,
    build_masks2_ao, build_ao_rgb, build_roughness_clamp,
)


def build_graph(mat, nodes, links):
    props  = mat.bevy_toolkit
    has_ma = props.ma_img is not None

    # --- bevy_masks (COLOR_0): R=layer-blend, G=dirt, B=wet -----------------
    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-2000, 420)
    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1800, 420)
    links.new(attr.outputs[0], sep.inputs[0])

    # --- UV + per-layer tiling -----------------------------------------------
    uv = nodes.new("ShaderNodeTexCoord")
    uv.location = (-2000, 60)

    map0 = nodes.new("ShaderNodeMapping")
    map0.location = (-1780, 80)
    map1 = nodes.new("ShaderNodeMapping")
    map1.location = (-1780, -120)
    links.new(uv.outputs[2], map0.inputs[0])
    links.new(uv.outputs[2], map1.inputs[0])

    l0_val = _add_value_node(nodes, "L0 Tiling", props.l0_tiling, -2000,  -40)
    l1_val = _add_value_node(nodes, "L1 Tiling", props.l1_tiling, -2000, -240)
    c0 = nodes.new("ShaderNodeCombineXYZ")
    c0.location = (-1900, -40)
    c1 = nodes.new("ShaderNodeCombineXYZ")
    c1.location = (-1900, -240)
    for idx in (0, 1, 2):
        links.new(l0_val.outputs[0], c0.inputs[idx])
        links.new(l1_val.outputs[0], c1.inputs[idx])
    links.new(c0.outputs[0], map0.inputs[3])
    links.new(c1.outputs[0], map1.inputs[3])

    # --- Texture nodes -------------------------------------------------------
    l0_alb  = _add_tex_node(nodes, "L0 Albedo", _material_image(props, "l0_albedo"), -1520,  420, "sRGB")
    l0_mrao = _add_tex_node(nodes, "L0 MRAO",   _material_image(props, "l0_mrao"),   -1520,  180, "Non-Color")
    l0_nrm  = _add_tex_node(nodes, "L0 Normal", _material_image(props, "l0_normal"), -1520,  -40, "Non-Color")
    l1_alb  = _add_tex_node(nodes, "L1 Albedo", _material_image(props, "l1_albedo"), -1520, -300, "sRGB")
    l1_mrao = _add_tex_node(nodes, "L1 MRAO",   _material_image(props, "l1_mrao"),   -1520, -520, "Non-Color")
    l1_nrm  = _add_tex_node(nodes, "L1 Normal", _material_image(props, "l1_normal"), -1520, -760, "Non-Color")
    snow    = _add_tex_node(nodes, "Snow",       _material_image(props, "snow"),      -1520, -980, "sRGB")
    ma_tex  = _add_tex_node(nodes, "MA",         _material_image(props, "ma"),        -1520,-1180, "Non-Color")
    mb_tex  = _add_tex_node(nodes, "MB",         _material_image(props, "mb"),        -1520,-1380, "Non-Color")

    for tex in (l0_alb, l0_mrao, l0_nrm):
        links.new(map0.outputs[0], tex.inputs[0])
    for tex in (l1_alb, l1_mrao, l1_nrm):
        links.new(map1.outputs[0], tex.inputs[0])
    links.new(map0.outputs[0], snow.inputs[0])
    links.new(map0.outputs[0], ma_tex.inputs[0])
    links.new(uv.outputs[2],   mb_tex.inputs[0])  # MB: raw UV, no tiling

    blend     = sep.outputs[0]
    dirt_mask = sep.outputs[1]
    wet_mask  = sep.outputs[2]

    # --- Layer blend ---------------------------------------------------------
    alb_mix = nodes.new("ShaderNodeMixRGB")
    alb_mix.location = (-1180, 260)
    links.new(blend,             alb_mix.inputs[0])
    links.new(l0_alb.outputs[0], alb_mix.inputs[1])
    links.new(l1_alb.outputs[0], alb_mix.inputs[2])

    l0_m = nodes.new("ShaderNodeSeparateColor")
    l0_m.location = (-1180, 60)
    links.new(l0_mrao.outputs[0], l0_m.inputs[0])
    l1_m = nodes.new("ShaderNodeSeparateColor")
    l1_m.location = (-1180, -140)
    links.new(l1_mrao.outputs[0], l1_m.inputs[0])

    rough_mix = _mix_float(nodes, links, blend, l0_m.outputs[1], l1_m.outputs[1], -940,   20, "LayerRough")
    metal_mix = _mix_float(nodes, links, blend, l0_m.outputs[0], l1_m.outputs[0], -940, -100, "LayerMetal")

    nrm_mix = nodes.new("ShaderNodeMixRGB")
    nrm_mix.location = (-940, -260)
    links.new(blend,             nrm_mix.inputs[0])
    links.new(l0_nrm.outputs[0], nrm_mix.inputs[1])
    links.new(l1_nrm.outputs[0], nrm_mix.inputs[2])

    # --- Dirt ----------------------------------------------------------------
    dirt_val           = _add_value_node(nodes, "Dirt Level", props.dirt_level, -1180, -320)
    dirt_mul           = nodes.new("ShaderNodeMath")
    dirt_mul.operation = "MULTIPLY"
    dirt_mul.location  = (-980, -340)
    links.new(dirt_mask,           dirt_mul.inputs[0])
    links.new(dirt_val.outputs[0], dirt_mul.inputs[1])

    dirt_mix = nodes.new("ShaderNodeMixRGB")
    dirt_mix.location = (-720, 260)
    dirt_mix.inputs[2].default_value = (0.15, 0.12, 0.1, 1.0)
    links.new(dirt_mul.outputs[0], dirt_mix.inputs[0])
    links.new(alb_mix.outputs[0],  dirt_mix.inputs[1])

    # --- Wet -----------------------------------------------------------------
    _add_value_node(nodes, "Porosity", props.porosity, -1180, -440)
    wet_val           = _add_value_node(nodes, "Wetness", props.wetness, -1180, -520)
    wet_mul           = nodes.new("ShaderNodeMath")
    wet_mul.operation = "MULTIPLY"
    wet_mul.location  = (-980, -500)
    links.new(wet_mask,           wet_mul.inputs[0])
    links.new(wet_val.outputs[0], wet_mul.inputs[1])

    wet_dark            = nodes.new("ShaderNodeMixRGB")
    wet_dark.blend_type = "MULTIPLY"
    wet_dark.inputs[0].default_value = 1.0
    wet_dark.inputs[2].default_value = (0.8, 0.8, 0.8, 1.0)
    wet_dark.location   = (-500, 240)
    links.new(dirt_mix.outputs[0], wet_dark.inputs[1])

    wet_color = nodes.new("ShaderNodeMixRGB")
    wet_color.location = (-280, 240)
    links.new(wet_mul.outputs[0],  wet_color.inputs[0])
    links.new(dirt_mix.outputs[0], wet_color.inputs[1])
    links.new(wet_dark.outputs[0], wet_color.inputs[2])

    # --- Snow (geometry normal Y) --------------------------------------------
    geom = nodes.new("ShaderNodeNewGeometry")
    geom.location = (-980, -700)
    nsep = nodes.new("ShaderNodeSeparateXYZ")
    nsep.location = (-780, -700)
    links.new(geom.outputs[1], nsep.inputs[0])

    snow_val           = _add_value_node(nodes, "Snow Level", props.snow_level, -980, -820)
    snow_mask_mul      = nodes.new("ShaderNodeMath")
    snow_mask_mul.operation = "MULTIPLY"
    snow_mask_mul.location  = (-580, -760)
    links.new(nsep.outputs[1],     snow_mask_mul.inputs[0])
    links.new(snow_val.outputs[0], snow_mask_mul.inputs[1])

    snow_mix = nodes.new("ShaderNodeMixRGB")
    snow_mix.location = (-40, 240)
    links.new(snow_mask_mul.outputs[0], snow_mix.inputs[0])
    links.new(wet_color.outputs[0],     snow_mix.inputs[1])
    links.new(snow.outputs[0],          snow_mix.inputs[2])

    # --- Roughness / Metalness -----------------------------------------------
    # MA present → G=roughness, B=metalness. MA absent → layer-blended MRAO.
    if has_ma:
        ma_sep = nodes.new("ShaderNodeSeparateColor")
        ma_sep.location = (-1300, -1180)
        links.new(ma_tex.outputs[0], ma_sep.inputs[0])
        roughness_base = ma_sep.outputs[1]  # G
        metalness_base = ma_sep.outputs[2]  # B
    else:
        roughness_base = rough_mix
        metalness_base = metal_mix

    rough_wet  = _mix_float(nodes, links, wet_mul.outputs[0],       roughness_base, 0.02, -500, 80,  "LayerRoughWet")
    rough_snow = _mix_float(nodes, links, snow_mask_mul.outputs[0], rough_wet,      0.8,  -280, 80,  "LayerRoughSnow")
    metal_snow = _mix_float(nodes, links, snow_mask_mul.outputs[0], metalness_base, 0.0,  -280, -20, "LayerMetalSnow")

    normal_map = nodes.new("ShaderNodeNormalMap")
    normal_map.location = (-500, -180)
    links.new(nrm_mix.outputs[0], normal_map.inputs[1])

    # --- bevy_masks2: R=AO (1=none), G=emissive ----------------------------
    _, _sep2, ao_source = build_masks2_ao(nodes, links, -80, -360)

    ao_rgb = build_ao_rgb(nodes, links, ao_source, 420, -360)

    ao_mul            = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location   = (320, 200)
    links.new(snow_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0],   ao_mul.inputs[2])

    # --- BSDF ----------------------------------------------------------------
    rough_clamped = build_roughness_clamp(nodes, links, rough_snow, 0.2, 460, -30)

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (560, 200)
    links.new(ao_mul.outputs[0],      bsdf.inputs["Base Color"])
    links.new(metal_snow,             bsdf.inputs["Metallic"])
    links.new(rough_clamped,          bsdf.inputs["Roughness"])
    links.new(normal_map.outputs[0],  bsdf.inputs["Normal"])

    # Always use layer-0 albedo alpha — connecting MB would create a packed PNG artefact.
    links.new(l0_alb.outputs[1], bsdf.inputs["Alpha"])

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (860, 200)
    links.new(bsdf.outputs[0], out.inputs[0])
