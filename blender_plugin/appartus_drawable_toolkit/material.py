import bpy
from .constants import (
    TEXTURE_SLOT_FIELDS, TEXTURE_KEYWORDS,
    SLOT_NODE_LABEL, SLOT_COLORSPACE,
)
from .utils import image_basename, format_float, toml_escape, first_non_empty
from .shaders import (
    build_graph_standard_pbr,
    build_graph_layered_env,
    build_graph_vehicle_glass,
)


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
        build_graph_layered_env(mat, nodes, links)
        _apply_opacity_settings(mat)
    elif template == "vehicle_glass":
        build_graph_vehicle_glass(mat, nodes, links)
        if props.opacity_mode == "OPAQUE":
            mat.blend_method  = "BLEND"
            mat.shadow_method = "HASHED"
        else:
            _apply_opacity_settings(mat)
    else:
        build_graph_standard_pbr(mat, nodes, links)
        _apply_opacity_settings(mat)
