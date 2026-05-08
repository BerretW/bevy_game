import bpy
from .constants import (
    ATTR_NAME, ATTR_NAME2,
    TEXTURE_SLOT_FIELDS, TEXTURE_KEYWORDS,
    SLOT_NODE_LABEL, SLOT_COLORSPACE,
)
from .utils import image_basename, format_float, toml_escape, first_non_empty


# ---------------------------------------------------------------------------
# Update callbacks (used as bpy.props update= arguments in properties.py)
# ---------------------------------------------------------------------------

def on_template_changed(props, context):
    mat = getattr(props, "id_data", None)
    if isinstance(mat, bpy.types.Material):
        create_bevy_node_tree(mat)


def _on_texture_update(self, context):
    mat = getattr(self, "id_data", None)
    if not isinstance(mat, bpy.types.Material):
        return
    for slot_name in TEXTURE_SLOT_FIELDS:
        _sync_texture_node(mat, slot_name, getattr(self, f"{slot_name}_img"))


def _on_param_update(self, context):
    mat = getattr(self, "id_data", None)
    if isinstance(mat, bpy.types.Material):
        _sync_params_to_nodes(mat)


def _on_ma_mb_update(self, context):
    """Rebuild full node tree — MA/MB/opacity_mode affect graph topology."""
    mat = getattr(self, "id_data", None)
    if isinstance(mat, bpy.types.Material):
        create_bevy_node_tree(mat)


# ---------------------------------------------------------------------------
# Template / texture helpers
# ---------------------------------------------------------------------------

def get_template_texture_specs(template: str):
    if template == "layered_env":
        return (
            ("l0_albedo", "L0 Albedo", "embedded"),
            ("l0_mrao",   "L0 MRAO",   "shared"),
            ("l0_normal", "L0 Normal", "shared"),
            ("l1_albedo", "L1 Albedo", "embedded"),
            ("l1_mrao",   "L1 MRAO",   "shared"),
            ("l1_normal", "L1 Normal", "shared"),
            ("snow",      "Snow",      "shared"),
            ("ma",        "MA",        "shared"),
            ("mb",        "MB",        "shared"),
        )
    if template == "vehicle_glass":
        return (
            ("glass_albedo", "Glass Albedo", "embedded"),
            ("shatter_map",  "Shatter Map",  "embedded"),
            ("mb",           "MB",           "shared"),
        )
    return (
        ("albedo",  "Albedo",  "embedded"),
        ("mrao",    "MRAO",    "shared"),
        ("normal",  "Normal",  "shared"),
        ("palette", "Palette", "shared"),
        ("snow",    "Snow",    "shared"),
        ("ma",      "MA",      "shared"),   # optional — R=AO  G=Roughness  B=Metalness
        ("mb",      "MB",      "shared"),   # optional — alpha=opacity mask
    )


def material_texture_value(props, slot_name):
    img      = getattr(props, f"{slot_name}_img")
    name     = getattr(props, f"{slot_name}_name")
    embedded = getattr(props, f"{slot_name}_embedded")
    source   = "embedded" if embedded else "shared"
    return img, name, source


def _assign_material_texture(props, slot_name, image):
    setattr(props, f"{slot_name}_img", image)
    if not getattr(props, f"{slot_name}_name").strip() and image is not None:
        setattr(props, f"{slot_name}_name", image_basename(image))


def _texture_entry(slot_name: str, image, explicit_name: str, source: str):
    name = first_non_empty(explicit_name.strip(), image_basename(image))
    if not name:
        return None
    return f'{slot_name} = {{ name = "{toml_escape(name)}", source = "{source}" }}'


# ---------------------------------------------------------------------------
# Node sync helpers
# ---------------------------------------------------------------------------

def _sync_texture_node(mat, slot_name, img):
    """Update the ShaderNodeTexImage labelled slot_name in the node tree."""
    if not mat.use_nodes or not mat.node_tree:
        return
    label      = SLOT_NODE_LABEL.get(slot_name)
    colorspace = SLOT_COLORSPACE.get(slot_name, "sRGB")
    if not label:
        return
    for node in mat.node_tree.nodes:
        if node.type == "TEX_IMAGE" and node.label == label:
            node.image = img
            if img and node.image and node.image.colorspace_settings:
                node.image.colorspace_settings.name = colorspace
            return


def _sync_params_to_nodes(mat):
    """Push current prop values to ShaderNodeValue / tint nodes in the tree."""
    if not mat or not mat.use_nodes or not mat.node_tree:
        return
    props = mat.bevy_toolkit
    nodes = mat.node_tree.nodes
    float_syncs = {
        "Wetness":    props.wetness,
        "Snow Level": props.snow_level,
        "Dirt Level": props.dirt_level,
        "Tiling":     props.tiling,
        "L0 Tiling":  props.l0_tiling,
        "L1 Tiling":  props.l1_tiling,
    }
    for node in nodes:
        if node.type == "VALUE" and node.label in float_syncs:
            node.outputs[0].default_value = float(float_syncs[node.label])
    for node in nodes:
        if node.type == "MIX_RGB" and node.label == "Tint":
            node.inputs[2].default_value = list(props.tint)
            break


# ---------------------------------------------------------------------------
# Material sync from existing Blender node tree
# ---------------------------------------------------------------------------

def _slot_from_node_name(node_name: str):
    lowered = node_name.lower().replace(" ", "").replace("-", "").replace("_", "")
    for slot_name, words in TEXTURE_KEYWORDS.items():
        for word in words:
            if word in lowered:
                return slot_name
    return None


def _image_from_socket(socket):
    if socket is None or not socket.is_linked:
        return None
    for link in socket.links:
        node = link.from_node
        if node is None:
            continue
        if node.type == "TEX_IMAGE" and getattr(node, "image", None) is not None:
            return node.image
        if node.type == "NORMAL_MAP":
            candidate = _image_from_socket(node.inputs.get("Color"))
            if candidate is not None:
                return candidate
        for input_socket in node.inputs:
            candidate = _image_from_socket(input_socket)
            if candidate is not None:
                return candidate
    return None


def _find_principled_bsdf(mat: bpy.types.Material):
    if not mat or not mat.use_nodes or not mat.node_tree:
        return None
    for node in mat.node_tree.nodes:
        if node.type == "BSDF_PRINCIPLED":
            return node
    return None


def sync_material_from_principled(mat: bpy.types.Material, only_if_empty=False):
    props = mat.bevy_toolkit
    node  = _find_principled_bsdf(mat)
    if node is None:
        return 0

    mapped  = 0
    mapping = (
        ("albedo", node.inputs.get("Base Color")),
        ("normal", node.inputs.get("Normal")),
        ("mrao",   node.inputs.get("Roughness")),
    )
    for slot_name, socket in mapping:
        image = _image_from_socket(socket)
        if image is None:
            continue
        current_img  = getattr(props, f"{slot_name}_img")
        current_name = getattr(props, f"{slot_name}_name").strip()
        if only_if_empty and (current_img is not None or current_name):
            continue
        _assign_material_texture(props, slot_name, image)
        mapped += 1
    return mapped


def sync_material_from_nodes(mat: bpy.types.Material, only_if_empty=False):
    if not mat or not mat.use_nodes or not mat.node_tree:
        return 0
    mapped = sync_material_from_principled(mat, only_if_empty=only_if_empty)
    props  = mat.bevy_toolkit
    for node in mat.node_tree.nodes:
        if node.type != "TEX_IMAGE" or node.image is None:
            continue
        slot_name = _slot_from_node_name(node.name) or _slot_from_node_name(node.label)
        if not slot_name:
            continue
        current_img  = getattr(props, f"{slot_name}_img")
        current_name = getattr(props, f"{slot_name}_name").strip()
        if only_if_empty and (current_img is not None or current_name):
            continue
        _assign_material_texture(props, slot_name, node.image)
        mapped += 1
    return mapped


def set_material_texture_source(mat: bpy.types.Material, source: str):
    props    = mat.bevy_toolkit
    embedded = source == "embedded"
    for slot_name in TEXTURE_SLOT_FIELDS:
        setattr(props, f"{slot_name}_embedded", embedded)


def clear_embedded_images(mat: bpy.types.Material):
    props   = mat.bevy_toolkit
    cleared = 0
    for slot_name in TEXTURE_SLOT_FIELDS:
        if (
            getattr(props, f"{slot_name}_embedded")
            and getattr(props, f"{slot_name}_img") is not None
        ):
            setattr(props, f"{slot_name}_img", None)
            cleared += 1
    return cleared


# ---------------------------------------------------------------------------
# TOML serialisation
# ---------------------------------------------------------------------------

def build_material_inline(mat: bpy.types.Material) -> str:
    props          = mat.bevy_toolkit
    texture_items  = []
    for slot_name, _label, _default_source in get_template_texture_specs(props.template):
        image, explicit_name, source = material_texture_value(props, slot_name)
        entry = _texture_entry(slot_name, image, explicit_name, source)
        if entry:
            texture_items.append(entry)

    textures_table = "{ " + ", ".join(texture_items) + " }" if texture_items else "{}"
    tint   = props.tint
    params = (
        "{ "
        f"tint = [{format_float(tint[0])}, {format_float(tint[1])}, "
        f"{format_float(tint[2])}, {format_float(tint[3])}], "
        f"tiling = {format_float(props.tiling)}, "
        f"l0_tiling = {format_float(props.l0_tiling)}, "
        f"l1_tiling = {format_float(props.l1_tiling)}, "
        f"porosity = {format_float(props.porosity)}, "
        f"wetness = {format_float(props.wetness)}, "
        f"snow_level = {format_float(props.snow_level)}, "
        f"dirt_level = {format_float(props.dirt_level)}, "
        f'opacity_mode = "{props.opacity_mode}", '
        f"alpha_threshold = {format_float(props.alpha_threshold)}"
        " }"
    )
    return (
        "{ "
        f'template = "{props.template}", '
        f"textures = {textures_table}, "
        f"params = {params}"
        " }"
    )


def _apply_opacity_settings(mat: bpy.types.Material):
    """Sync material blend_method / alpha_threshold from props."""
    props = mat.bevy_toolkit
    mode  = props.opacity_mode
    if mode == "OPAQUE":
        mat.blend_method   = "OPAQUE"
        mat.shadow_method  = "OPAQUE"
    elif mode == "BLEND":
        mat.blend_method   = "BLEND"
        mat.shadow_method  = "HASHED"
        mat.alpha_threshold = props.alpha_threshold
    elif mode == "HASHED":
        mat.blend_method   = "HASHED"
        mat.shadow_method  = "HASHED"
        mat.alpha_threshold = props.alpha_threshold
    elif mode == "CLIP":
        mat.blend_method   = "CLIP"
        mat.shadow_method  = "CLIP"
        mat.alpha_threshold = props.alpha_threshold


# ---------------------------------------------------------------------------
# Preview node-graph builders
# ---------------------------------------------------------------------------

def _material_image(props, slot_name):
    return getattr(props, f"{slot_name}_img")


def _add_tex_node(nodes, label, image, x, y, colorspace="sRGB"):
    node = nodes.new("ShaderNodeTexImage")
    node.label    = label
    node.location = (x, y)
    if image is not None:
        node.image = image
        if node.image.colorspace_settings:
            node.image.colorspace_settings.name = colorspace
    return node


def _add_value_node(nodes, name, value, x, y):
    node = nodes.new("ShaderNodeValue")
    node.label                    = name
    node.outputs[0].default_value = float(value)
    node.location                 = (x, y)
    return node


def _float_socket(nodes, value, x, y, label="Value"):
    if hasattr(value, "is_linked"):
        return value
    node = _add_value_node(nodes, label, value, x, y)
    return node.outputs[0]


def _mix_float(nodes, links, fac, a, b, x, y, label="MixFloat"):
    fac_socket = _float_socket(nodes, fac, x - 320, y + 40,  f"{label} Fac")
    a_socket   = _float_socket(nodes, a,   x - 320, y - 40,  f"{label} A")
    b_socket   = _float_socket(nodes, b,   x - 320, y - 120, f"{label} B")

    one = _add_value_node(nodes, f"{label} One", 1.0, x - 180, y + 90)

    one_minus           = nodes.new("ShaderNodeMath")
    one_minus.operation = "SUBTRACT"
    one_minus.location  = (x - 20, y + 80)
    links.new(one.outputs[0], one_minus.inputs[0])
    links.new(fac_socket,     one_minus.inputs[1])

    mul_a           = nodes.new("ShaderNodeMath")
    mul_a.operation = "MULTIPLY"
    mul_a.location  = (x + 160, y)
    links.new(a_socket,            mul_a.inputs[0])
    links.new(one_minus.outputs[0], mul_a.inputs[1])

    mul_b           = nodes.new("ShaderNodeMath")
    mul_b.operation = "MULTIPLY"
    mul_b.location  = (x + 160, y - 100)
    links.new(b_socket,   mul_b.inputs[0])
    links.new(fac_socket, mul_b.inputs[1])

    add           = nodes.new("ShaderNodeMath")
    add.operation = "ADD"
    add.location  = (x + 340, y - 40)
    links.new(mul_a.outputs[0], add.inputs[0])
    links.new(mul_b.outputs[0], add.inputs[1])
    return add.outputs[0]


def _build_graph_standard_pbr(mat, nodes, links):
    props  = mat.bevy_toolkit
    has_ma = props.ma_img is not None
    has_mb = props.mb_img is not None
    use_alpha = has_mb or props.opacity_mode != "OPAQUE"

    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-1800, 400)

    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1600, 400)
    links.new(attr.outputs[0], sep.inputs[0])

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

    alb     = _add_tex_node(nodes, "Albedo",  _material_image(props, "albedo"),  -1320,  420, "sRGB")
    mrao    = _add_tex_node(nodes, "MRAO",    _material_image(props, "mrao"),    -1320,  160, "Non-Color")
    normal  = _add_tex_node(nodes, "Normal",  _material_image(props, "normal"),  -1320,  -80, "Non-Color")
    palette = _add_tex_node(nodes, "Palette", _material_image(props, "palette"), -1320, -340, "sRGB")
    snow    = _add_tex_node(nodes, "Snow",    _material_image(props, "snow"),    -1320, -560, "sRGB")

    # _ma — R=AO  G=Roughness  B=Metalness  (RDR2 convention, optional)
    ma_tex = _add_tex_node(nodes, "MA", _material_image(props, "ma"), -1320, -780, "Non-Color")
    # _mb — alpha=opacity mask  (RDR2 convention, optional)
    mb_tex = _add_tex_node(nodes, "MB", _material_image(props, "mb"), -1320, -1000, "Non-Color")

    links.new(map_node.outputs[0], alb.inputs[0])
    links.new(map_node.outputs[0], mrao.inputs[0])
    links.new(map_node.outputs[0], normal.inputs[0])
    links.new(map_node.outputs[0], snow.inputs[0])
    links.new(map_node.outputs[0], ma_tex.inputs[0])
    links.new(uv.outputs[2],       mb_tex.inputs[0])  # MB uses raw UV, no tiling

    tint            = nodes.new("ShaderNodeMixRGB")
    tint.label      = "Tint"
    tint.blend_type = "MULTIPLY"
    tint.inputs[0].default_value = 1.0
    tint.inputs[2].default_value = props.tint
    tint.location   = (-980, 440)
    links.new(alb.outputs[0], tint.inputs[1])

    palette_uv = nodes.new("ShaderNodeCombineXYZ")
    palette_uv.location = (-1500, -340)
    links.new(attr.outputs[3], palette_uv.inputs[0])  # Alpha from ShaderNodeAttribute
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

    # --- Roughness / Metalness source ----------------------------------------
    # _ma present → use _ma.G and _ma.B (overrides MRAO)
    # _ma absent  → use MRAO.G and MRAO.R; defaults 0.5 / 0.0 when MRAO empty
    if has_ma:
        ma_sep = nodes.new("ShaderNodeSeparateColor")
        ma_sep.location = (-1100, -780)
        links.new(ma_tex.outputs[0], ma_sep.inputs[0])
        roughness_socket = ma_sep.outputs[1]  # G
        metalness_socket = ma_sep.outputs[2]  # B
        ao_override      = ma_sep.outputs[0]  # R — overrides bevy_masks2 AO
    else:
        mrao_sep = nodes.new("ShaderNodeSeparateColor")
        mrao_sep.location = (-980, 120)
        links.new(mrao.outputs[0], mrao_sep.inputs[0])
        # Fallback defaults when MRAO has no image (outputs black = 0)
        rough_default    = _add_value_node(nodes, "Rough Default", 0.5, -980, 40)
        rough_has_mrao   = _add_value_node(nodes, "Has MRAO", 1.0 if props.mrao_img else 0.0, -980, -40)
        roughness_socket = _mix_float(nodes, links,
                                      rough_has_mrao.outputs[0],
                                      rough_default.outputs[0],
                                      mrao_sep.outputs[1],
                                      -760, 40, "RoughFallback")
        metal_default    = _add_value_node(nodes, "Metal Default", 0.0, -980, -100)
        metalness_socket = _mix_float(nodes, links,
                                      rough_has_mrao.outputs[0],
                                      metal_default.outputs[0],
                                      mrao_sep.outputs[0],
                                      -760, -100, "MetalFallback")
        ao_override      = None

    rough_dirt = _mix_float(nodes, links, dirt_mul.outputs[0], roughness_socket, 0.9,  -340, 100, "RoughDirt")
    rough_wet  = _mix_float(nodes, links, wet_mul.outputs[0],  rough_dirt,        0.02, -120, 100, "RoughWet")
    rough_snow = _mix_float(nodes, links, snow_mul.outputs[0], rough_wet,         0.8,   120, 100, "RoughSnow")
    metal_snow = _mix_float(nodes, links, snow_mul.outputs[0], metalness_socket,  0.0,   120, -20, "MetalSnow")

    normal_map = nodes.new("ShaderNodeNormalMap")
    normal_map.location = (-120, -150)
    links.new(normal.outputs[0], normal_map.inputs[1])

    # --- bevy_masks2: R=AO (1=none), G=emissive ----------------------------
    attr2 = nodes.new("ShaderNodeAttribute")
    attr2.attribute_name = ATTR_NAME2
    attr2.location = (200, -200)
    sep2 = nodes.new("ShaderNodeSeparateColor")
    sep2.location = (400, -200)
    links.new(attr2.outputs[0], sep2.inputs[0])

    ao_source = ao_override if ao_override is not None else sep2.outputs[0]

    ao_rgb = nodes.new("ShaderNodeCombineColor")
    ao_rgb.location = (590, -200)
    links.new(ao_source, ao_rgb.inputs[0])
    links.new(ao_source, ao_rgb.inputs[1])
    links.new(ao_source, ao_rgb.inputs[2])

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
    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (820, 260)
    links.new(ao_mul.outputs[0],       bsdf.inputs[0])
    links.new(metal_snow,              bsdf.inputs[1])
    links.new(rough_snow,              bsdf.inputs[2])
    links.new(normal_map.outputs[0],   bsdf.inputs[5])

    # Alpha / opacity
    if use_alpha:
        alpha_socket = mb_tex.outputs[1] if has_mb else alb.outputs[1]
        links.new(alpha_socket, bsdf.inputs[4])
    else:
        links.new(alb.outputs[1], bsdf.inputs[4])

    try:
        links.new(emissive_mul.outputs[0], bsdf.inputs["Emission Color"])
        bsdf.inputs["Emission Strength"].default_value = 1.0
    except KeyError:
        pass

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (1120, 260)
    links.new(bsdf.outputs[0], out.inputs[0])


def _build_graph_layered_env(mat, nodes, links):
    props     = mat.bevy_toolkit
    has_ma    = props.ma_img is not None
    has_mb    = props.mb_img is not None
    use_alpha = has_mb or props.opacity_mode != "OPAQUE"

    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-2000, 420)
    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1800, 420)
    links.new(attr.outputs[0], sep.inputs[0])

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
    links.new(uv.outputs[2],   mb_tex.inputs[0])

    blend     = sep.outputs[0]
    dirt_mask = sep.outputs[1]
    wet_mask  = sep.outputs[2]

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

    # --- Roughness / Metalness: _ma overrides layer mix when present --------
    if has_ma:
        ma_sep = nodes.new("ShaderNodeSeparateColor")
        ma_sep.location = (-1300, -1180)
        links.new(ma_tex.outputs[0], ma_sep.inputs[0])
        roughness_base = ma_sep.outputs[1]   # G
        metalness_base = ma_sep.outputs[2]   # B
        ao_override    = ma_sep.outputs[0]   # R
    else:
        roughness_base = rough_mix
        metalness_base = metal_mix
        ao_override    = None

    rough_wet  = _mix_float(nodes, links, wet_mul.outputs[0],       roughness_base, 0.02, -500, 80,  "LayerRoughWet")
    rough_snow = _mix_float(nodes, links, snow_mask_mul.outputs[0], rough_wet,      0.8,  -280, 80,  "LayerRoughSnow")
    metal_snow = _mix_float(nodes, links, snow_mask_mul.outputs[0], metalness_base, 0.0,  -280, -20, "LayerMetalSnow")

    normal_map = nodes.new("ShaderNodeNormalMap")
    normal_map.location = (-500, -180)
    links.new(nrm_mix.outputs[0], normal_map.inputs[1])

    # bevy_masks2 — TEXCOORD_1: R=AO (1=none)
    attr2 = nodes.new("ShaderNodeAttribute")
    attr2.attribute_name = ATTR_NAME2
    attr2.location = (-80, -360)
    sep2 = nodes.new("ShaderNodeSeparateColor")
    sep2.location = (120, -360)
    links.new(attr2.outputs[0], sep2.inputs[0])

    ao_source = ao_override if ao_override is not None else sep2.outputs[0]

    ao_rgb = nodes.new("ShaderNodeCombineColor")
    ao_rgb.location = (320, -360)
    links.new(ao_source, ao_rgb.inputs[0])
    links.new(ao_source, ao_rgb.inputs[1])
    links.new(ao_source, ao_rgb.inputs[2])

    ao_mul            = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location   = (320, 200)
    links.new(snow_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0],   ao_mul.inputs[2])

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (560, 200)
    links.new(ao_mul.outputs[0],     bsdf.inputs[0])
    links.new(metal_snow,            bsdf.inputs[1])
    links.new(rough_snow,            bsdf.inputs[2])
    links.new(normal_map.outputs[0], bsdf.inputs[5])

    if use_alpha:
        links.new(mb_tex.outputs[1] if has_mb else l0_alb.outputs[1], bsdf.inputs[4])

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (860, 200)
    links.new(bsdf.outputs[0], out.inputs[0])


def _build_graph_vehicle_glass(mat, nodes, links):
    props  = mat.bevy_toolkit
    has_mb = props.mb_img is not None

    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-1500, 220)
    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1300, 220)
    links.new(attr.outputs[0], sep.inputs[0])

    uv = nodes.new("ShaderNodeTexCoord")
    uv.location = (-1500, 20)

    glass   = _add_tex_node(nodes, "Glass Albedo", _material_image(props, "glass_albedo"), -1180,  260, "sRGB")
    shatter = _add_tex_node(nodes, "Shatter Map",  _material_image(props, "shatter_map"),  -1180,   40, "Non-Color")
    mb_tex  = _add_tex_node(nodes, "MB",           _material_image(props, "mb"),           -1180, -200, "Non-Color")
    links.new(uv.outputs[2], glass.inputs[0])
    links.new(uv.outputs[2], shatter.inputs[0])
    links.new(uv.outputs[2], mb_tex.inputs[0])

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

    wet_val    = _add_value_node(nodes, "Wetness", props.wetness, -780, -260)
    alpha_mul  = nodes.new("ShaderNodeMath")
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
    links.new(wet_val.outputs[0], rain_clean.inputs[0])
    links.new(rain_clean.outputs[0], alpha_wet.inputs[1])

    rough_crack = _mix_float(nodes, links, shatter_cut.outputs[0], 0.7,          0.05,  -320, -200, "GlassRoughCrack")
    rough_dirt  = _mix_float(nodes, links, dirt_mul.outputs[0],    rough_crack,  0.8,   -100, -200, "GlassRoughDirt")
    rough_wet   = _mix_float(nodes, links, wet_val.outputs[0],     rough_dirt,   0.01,   120, -200, "GlassRoughWet")

    # bevy_masks2 — TEXCOORD_1: R=AO (1=none)
    attr2 = nodes.new("ShaderNodeAttribute")
    attr2.attribute_name = ATTR_NAME2
    attr2.location = (220, -280)
    sep2 = nodes.new("ShaderNodeSeparateColor")
    sep2.location = (400, -280)
    links.new(attr2.outputs[0], sep2.inputs[0])

    ao_rgb = nodes.new("ShaderNodeCombineColor")
    ao_rgb.location = (560, -280)
    links.new(sep2.outputs[0], ao_rgb.inputs[0])
    links.new(sep2.outputs[0], ao_rgb.inputs[1])
    links.new(sep2.outputs[0], ao_rgb.inputs[2])

    ao_mul            = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location   = (560, 200)
    links.new(dirt_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0],   ao_mul.inputs[2])

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (760, 200)
    bsdf.inputs[17].default_value = 1.0  # Transmission
    bsdf.inputs[14].default_value = 0.8  # Specular IOR Level
    links.new(ao_mul.outputs[0], bsdf.inputs[0])
    links.new(rough_wet,         bsdf.inputs[2])

    # _mb alpha overrides the computed glass alpha when present
    alpha_source = mb_tex.outputs[1] if has_mb else alpha_wet.outputs[0]
    links.new(alpha_source, bsdf.inputs[4])

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (1060, 200)
    links.new(bsdf.outputs[0], out.inputs[0])


def create_bevy_node_tree(mat: bpy.types.Material):
    if not mat:
        return
    if not mat.use_nodes:
        mat.use_nodes = True

    nodes = mat.node_tree.nodes
    links = mat.node_tree.links
    nodes.clear()

    props    = mat.bevy_toolkit
    template = props.template
    if template == "layered_env":
        _build_graph_layered_env(mat, nodes, links)
        _apply_opacity_settings(mat)
    elif template == "vehicle_glass":
        _build_graph_vehicle_glass(mat, nodes, links)
        if props.opacity_mode == "OPAQUE":
            # Glass always needs at least BLEND; OPAQUE is just the default placeholder
            mat.blend_method  = "BLEND"
            mat.shadow_method = "HASHED"
        else:
            _apply_opacity_settings(mat)
    else:
        _build_graph_standard_pbr(mat, nodes, links)
        _apply_opacity_settings(mat)
