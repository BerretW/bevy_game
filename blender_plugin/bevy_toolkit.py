bl_info = {
    "name": "Bevy ADS Toolkit",
    "author": "Advanced Game Dev",
    "version": (4, 0),
    "blender": (3, 6, 0),
    "location": "View3D > Sidebar > Bevy",
    "description": "ADS pipeline: masks, collision metadata, GLB+Drawable export",
    "category": "Import-Export",
}

import bpy
import os
from mathutils import Vector

ATTR_NAME = "bevy_masks"
ATTR_NAME2 = "bevy_masks2"
UV_MASKS2_NAME = "_ads_masks2_uv"
ADS_VERSION = "1.1"

TEXTURE_SLOT_FIELDS = (
    "albedo",
    "mrao",
    "normal",
    "palette",
    "snow",
    "l0_albedo",
    "l0_mrao",
    "l0_normal",
    "l1_albedo",
    "l1_mrao",
    "l1_normal",
    "glass_albedo",
    "shatter_map",
)

TEXTURE_KEYWORDS = {
    "albedo": ("albedo", "basecolor", "base_color", "diffuse", "color"),
    "mrao": ("mrao", "orm", "rma", "occlusion", "roughness", "metallic"),
    "normal": ("normal", "norm", "nrm"),
    "palette": ("palette", "lut"),
    "snow": ("snow",),
    "l0_albedo": ("l0albedo", "layer0albedo", "basealbedo"),
    "l0_mrao": ("l0mrao", "layer0mrao", "baseorm"),
    "l0_normal": ("l0normal", "layer0normal"),
    "l1_albedo": ("l1albedo", "layer1albedo", "overlayalbedo"),
    "l1_mrao": ("l1mrao", "layer1mrao", "overlayorm"),
    "l1_normal": ("l1normal", "layer1normal"),
    "glass_albedo": ("glassalbedo", "glasscolor", "windowalbedo"),
    "shatter_map": ("shatter", "crack", "breakmap", "shattermap"),
}


def toml_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def format_float(value: float) -> str:
    s = f"{float(value):.6g}"
    # TOML distinguishes integers from floats — ensure we always emit a float literal.
    if "." not in s and "e" not in s and "E" not in s:
        s += ".0"
    return s


def bool_to_toml(value: bool) -> str:
    return "true" if value else "false"


def parse_tags(raw: str):
    if not raw:
        return []
    parts = [part.strip() for part in raw.replace(";", ",").split(",")]
    return [part for part in parts if part]


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


def is_collision_object(obj: bpy.types.Object) -> bool:
    if obj.type != "MESH":
        return False
    props = obj.bevy_toolkit_obj
    return bool(props.is_col or obj.name.startswith("COL_"))


def ensure_mask_attribute(mesh: bpy.types.Mesh):
    color_attributes = mesh.color_attributes
    if ATTR_NAME not in color_attributes:
        color_attributes.new(name=ATTR_NAME, type="BYTE_COLOR", domain="CORNER")
    color_attributes.active_color = color_attributes[ATTR_NAME]


def ensure_masks2_attribute(mesh: bpy.types.Mesh):
    """Create bevy_masks2 with AO=1.0 (no occlusion), emissive=0.0 defaults."""
    color_attributes = mesh.color_attributes
    created = ATTR_NAME2 not in color_attributes
    if created:
        color_attributes.new(name=ATTR_NAME2, type="BYTE_COLOR", domain="CORNER")
    layer = color_attributes[ATTR_NAME2]
    if created:
        for item in layer.data:
            item.color = (1.0, 0.0, 0.0, 1.0)  # R=1 (AO full), G=0 (no emissive)
    color_attributes.active_color = layer


def encode_masks2_to_uv(mesh: bpy.types.Mesh):
    """Bake bevy_masks2 R,G channels into _ads_masks2_uv UV map (TEXCOORD_1).
    Returns the UV layer name, or None if masks2 attribute is absent."""
    color_attributes = mesh.color_attributes
    if ATTR_NAME2 not in color_attributes:
        return None
    layer = color_attributes[ATTR_NAME2]
    uv_layer = mesh.uv_layers.get(UV_MASKS2_NAME)
    if uv_layer is None:
        uv_layer = mesh.uv_layers.new(name=UV_MASKS2_NAME)
    if uv_layer is None:
        return None
    src = layer.data
    dst = uv_layer.data
    count = min(len(src), len(dst))
    for i in range(count):
        c = src[i].color
        dst[i].uv = (c[0], c[1])  # R→U (AO), G→V (emissive)
    return UV_MASKS2_NAME


def remove_temp_uv(mesh: bpy.types.Mesh, name: str):
    uv_layer = mesh.uv_layers.get(name)
    if uv_layer:
        mesh.uv_layers.remove(uv_layer)


def fill_alpha_channel(mesh: bpy.types.Mesh, alpha_value: float):
    color_attributes = mesh.color_attributes
    if ATTR_NAME not in color_attributes:
        color_attributes.new(name=ATTR_NAME, type="BYTE_COLOR", domain="CORNER")
    layer = color_attributes[ATTR_NAME]
    clamped = max(0.0, min(1.0, float(alpha_value)))
    for color_item in layer.data:
        color = list(color_item.color)
        color[3] = clamped
        color_item.color = color
    color_attributes.active_color = layer


def _material_used_by_targets(target_meshes):
    materials = []
    seen_names = set()
    for obj in target_meshes:
        for slot in obj.material_slots:
            mat = slot.material
            if mat and mat.name not in seen_names:
                seen_names.add(mat.name)
                materials.append(mat)
    return materials


def _material_texture_value(props, slot_name):
    img = getattr(props, f"{slot_name}_img")
    name = getattr(props, f"{slot_name}_name")
    embedded = getattr(props, f"{slot_name}_embedded")
    source = "embedded" if embedded else "shared"
    return img, name, source


def get_template_texture_specs(template: str):
    if template == "layered_env":
        return (
            ("l0_albedo", "L0 Albedo", "embedded"),
            ("l0_mrao", "L0 MRAO", "shared"),
            ("l0_normal", "L0 Normal", "shared"),
            ("l1_albedo", "L1 Albedo", "embedded"),
            ("l1_mrao", "L1 MRAO", "shared"),
            ("l1_normal", "L1 Normal", "shared"),
            ("snow", "Snow", "shared"),
        )
    if template == "vehicle_glass":
        return (
            ("glass_albedo", "Glass Albedo", "embedded"),
            ("shatter_map", "Shatter Map", "embedded"),
        )
    return (
        ("albedo", "Albedo", "embedded"),
        ("mrao", "MRAO", "shared"),
        ("normal", "Normal", "shared"),
        ("palette", "Palette", "shared"),
        ("snow", "Snow", "shared"),
    )


def _assign_material_texture(props, slot_name, image):
    setattr(props, f"{slot_name}_img", image)
    if not getattr(props, f"{slot_name}_name").strip() and image is not None:
        setattr(props, f"{slot_name}_name", _image_basename(image))


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
        # Reroute / mapping chains
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
    node = _find_principled_bsdf(mat)
    if node is None:
        return 0

    mapped = 0

    mapping = (
        ("albedo", node.inputs.get("Base Color")),
        ("normal", node.inputs.get("Normal")),
        ("mrao", node.inputs.get("Roughness")),
    )

    for slot_name, socket in mapping:
        image = _image_from_socket(socket)
        if image is None:
            continue

        current_img = getattr(props, f"{slot_name}_img")
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

    props = mat.bevy_toolkit
    for node in mat.node_tree.nodes:
        if node.type != "TEX_IMAGE" or node.image is None:
            continue

        slot_name = _slot_from_node_name(node.name) or _slot_from_node_name(node.label)
        if not slot_name:
            continue

        current_img = getattr(props, f"{slot_name}_img")
        current_name = getattr(props, f"{slot_name}_name").strip()
        if only_if_empty and (current_img is not None or current_name):
            continue

        _assign_material_texture(props, slot_name, node.image)
        mapped += 1

    return mapped


def set_material_texture_source(mat: bpy.types.Material, source: str):
    props = mat.bevy_toolkit
    embedded = (source == "embedded")
    for slot_name in TEXTURE_SLOT_FIELDS:
        setattr(props, f"{slot_name}_embedded", embedded)


def clear_embedded_images(mat: bpy.types.Material):
    props = mat.bevy_toolkit
    cleared = 0
    for slot_name in TEXTURE_SLOT_FIELDS:
        if getattr(props, f"{slot_name}_embedded") and getattr(props, f"{slot_name}_img") is not None:
            setattr(props, f"{slot_name}_img", None)
            cleared += 1
    return cleared


def duplicate_collision_proxy(src_obj: bpy.types.Object, collection: bpy.types.Collection):
    col_name = f"COL_{src_obj.name}"
    existing = bpy.data.objects.get(col_name)
    if existing and existing.type == "MESH":
        existing.bevy_toolkit_obj.is_col = True
        return existing, False

    new_obj = src_obj.copy()
    new_obj.data = src_obj.data.copy()
    new_obj.name = col_name
    new_obj.bevy_toolkit_obj.is_col = True
    new_obj.bevy_toolkit_obj.col_shape = "CONVEX"
    new_obj.display_type = "WIRE"
    new_obj.hide_render = True
    new_obj.parent = src_obj.parent
    new_obj.matrix_world = src_obj.matrix_world.copy()
    collection.objects.link(new_obj)
    return new_obj, True


class BevyObjectProps(bpy.types.PropertyGroup):
    is_col: bpy.props.BoolProperty(name="Is Collision", default=False)
    cast_shadows: bpy.props.BoolProperty(name="Cast Shadows", default=True)
    col_shape: bpy.props.EnumProperty(
        name="Shape",
        items=[
            ("BOX", "BOX", ""),
            ("SPHERE", "SPHERE", ""),
            ("CAPSULE", "CAPSULE", ""),
            ("CYLINDER", "CYLINDER", ""),
            ("CONVEX", "CONVEX", ""),
            ("MESH", "MESH", ""),
        ],
        default="CONVEX",
    )
    mass: bpy.props.FloatProperty(name="Mass", default=1.0, min=0.0)
    is_static: bpy.props.BoolProperty(name="Static", default=False)
    friction: bpy.props.FloatProperty(name="Friction", default=0.6, min=0.0)
    restitution: bpy.props.FloatProperty(name="Restitution", default=0.2, min=0.0)
    tags_csv: bpy.props.StringProperty(name="Tags", default="")


class BevyMaterialProps(bpy.types.PropertyGroup):
    template: bpy.props.EnumProperty(
        name="Template",
        items=[
            ("standard_pbr", "standard_pbr", "Standard PBR template"),
            ("layered_env", "layered_env", "Layered environment template"),
            ("vehicle_glass", "vehicle_glass", "Vehicle glass template"),
        ],
        default="standard_pbr",
        update=on_template_changed,
    )

    albedo_img: bpy.props.PointerProperty(type=bpy.types.Image, name="Albedo Image", update=_on_texture_update)
    albedo_name: bpy.props.StringProperty(name="Albedo Name", default="")
    albedo_embedded: bpy.props.BoolProperty(name="Embedded", default=True)

    mrao_img: bpy.props.PointerProperty(type=bpy.types.Image, name="MRAO Image", update=_on_texture_update)
    mrao_name: bpy.props.StringProperty(name="MRAO Name", default="")
    mrao_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    normal_img: bpy.props.PointerProperty(type=bpy.types.Image, name="Normal Image", update=_on_texture_update)
    normal_name: bpy.props.StringProperty(name="Normal Name", default="")
    normal_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    palette_img: bpy.props.PointerProperty(type=bpy.types.Image, name="Palette Image", update=_on_texture_update)
    palette_name: bpy.props.StringProperty(name="Palette Name", default="")
    palette_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    snow_img: bpy.props.PointerProperty(type=bpy.types.Image, name="Snow Image", update=_on_texture_update)
    snow_name: bpy.props.StringProperty(name="Snow Name", default="")
    snow_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    l0_albedo_img: bpy.props.PointerProperty(type=bpy.types.Image, name="L0 Albedo Image", update=_on_texture_update)
    l0_albedo_name: bpy.props.StringProperty(name="L0 Albedo Name", default="")
    l0_albedo_embedded: bpy.props.BoolProperty(name="Embedded", default=True)

    l0_mrao_img: bpy.props.PointerProperty(type=bpy.types.Image, name="L0 MRAO Image", update=_on_texture_update)
    l0_mrao_name: bpy.props.StringProperty(name="L0 MRAO Name", default="")
    l0_mrao_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    l0_normal_img: bpy.props.PointerProperty(type=bpy.types.Image, name="L0 Normal Image", update=_on_texture_update)
    l0_normal_name: bpy.props.StringProperty(name="L0 Normal Name", default="")
    l0_normal_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    l1_albedo_img: bpy.props.PointerProperty(type=bpy.types.Image, name="L1 Albedo Image", update=_on_texture_update)
    l1_albedo_name: bpy.props.StringProperty(name="L1 Albedo Name", default="")
    l1_albedo_embedded: bpy.props.BoolProperty(name="Embedded", default=True)

    l1_mrao_img: bpy.props.PointerProperty(type=bpy.types.Image, name="L1 MRAO Image", update=_on_texture_update)
    l1_mrao_name: bpy.props.StringProperty(name="L1 MRAO Name", default="")
    l1_mrao_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    l1_normal_img: bpy.props.PointerProperty(type=bpy.types.Image, name="L1 Normal Image", update=_on_texture_update)
    l1_normal_name: bpy.props.StringProperty(name="L1 Normal Name", default="")
    l1_normal_embedded: bpy.props.BoolProperty(name="Embedded", default=False)

    glass_albedo_img: bpy.props.PointerProperty(type=bpy.types.Image, name="Glass Albedo Image", update=_on_texture_update)
    glass_albedo_name: bpy.props.StringProperty(name="Glass Albedo Name", default="")
    glass_albedo_embedded: bpy.props.BoolProperty(name="Embedded", default=True)

    shatter_map_img: bpy.props.PointerProperty(type=bpy.types.Image, name="Shatter Map Image", update=_on_texture_update)
    shatter_map_name: bpy.props.StringProperty(name="Shatter Map Name", default="")
    shatter_map_embedded: bpy.props.BoolProperty(name="Embedded", default=True)

    tint: bpy.props.FloatVectorProperty(name="Tint", size=4, default=(1.0, 1.0, 1.0, 1.0), min=0.0, max=1.0, update=_on_param_update)
    tiling: bpy.props.FloatProperty(name="Tiling", default=1.0, update=_on_param_update)
    l0_tiling: bpy.props.FloatProperty(name="L0 Tiling", default=1.0, update=_on_param_update)
    l1_tiling: bpy.props.FloatProperty(name="L1 Tiling", default=1.0, update=_on_param_update)
    porosity: bpy.props.FloatProperty(name="Porosity", default=0.0, min=0.0, max=1.0, update=_on_param_update)
    wetness: bpy.props.FloatProperty(name="Wetness", default=0.0, min=0.0, max=1.0, update=_on_param_update)
    snow_level: bpy.props.FloatProperty(name="Snow Level", default=0.0, min=0.0, max=1.0, update=_on_param_update)
    dirt_level: bpy.props.FloatProperty(name="Dirt Level", default=0.0, min=0.0, max=1.0, update=_on_param_update)


class BevyExportProps(bpy.types.PropertyGroup):
    export_scope: bpy.props.EnumProperty(
        name="Scope",
        items=[
            ("ALL", "All Meshes", "Export all mesh objects"),
            ("SELECTED", "Selected", "Export only selected mesh objects"),
            ("ACTIVE_COLLECTION", "Active Collection", "Export meshes from active collection"),
        ],
        default="ALL",
    )
    apply_modifiers: bpy.props.BoolProperty(name="Apply Modifiers", default=True)
    auto_embed_collision: bpy.props.BoolProperty(name="Auto-Embed Collision", default=True)
    center_to_selection: bpy.props.BoolProperty(name="Center To Selection", default=True)
    drawable_dict_name: bpy.props.StringProperty(name="Dictionary Name", default="DrawableDictionary")
    alpha_fill_value: bpy.props.FloatProperty(name="Tint Alpha", default=1.0, min=0.0, max=1.0)


def gather_target_meshes(context, settings):
    scope = settings.export_scope
    if scope == "SELECTED":
        return [obj for obj in context.selected_objects if obj.type == "MESH"]
    if scope == "ACTIVE_COLLECTION":
        active_layer = context.view_layer.active_layer_collection
        if not active_layer:
            return []
        collection = active_layer.collection
        return [obj for obj in collection.all_objects if obj.type == "MESH"]
    return [obj for obj in bpy.data.objects if obj.type == "MESH"]


def validate_export_consistency(target_meshes):
    warnings = []
    checked_materials = set()

    for obj in target_meshes:
        if not obj.material_slots:
            warnings.append(f"Mesh '{obj.name}' has no material slots")
        for slot_idx, slot in enumerate(obj.material_slots, start=1):
            if not slot.material:
                warnings.append(f"Mesh '{obj.name}' has empty material slot #{slot_idx}")

        if is_collision_object(obj):
            props = obj.bevy_toolkit_obj
            if not props.is_static and props.mass <= 0.0:
                warnings.append(f"Collision '{obj.name}' is dynamic but mass <= 0")

    for obj in target_meshes:
        for slot in obj.material_slots:
            mat = slot.material
            if not mat or mat.name in checked_materials:
                continue
            checked_materials.add(mat.name)
            props = mat.bevy_toolkit
            specs = get_template_texture_specs(props.template)

            has_any_texture = False
            for slot_name, label, _default_source in specs:
                image, explicit_name, source = _material_texture_value(props, slot_name)
                resolved_name = _first_non_empty(explicit_name.strip(), _image_basename(image))
                if resolved_name:
                    has_any_texture = True
                if source == "shared" and not resolved_name:
                    warnings.append(
                        f"Material '{mat.name}' slot '{slot_name}' uses shared source but has no texture name/image"
                    )
                if source == "embedded" and image is None and explicit_name.strip():
                    warnings.append(
                        f"Material '{mat.name}' slot '{slot_name}' is embedded with only name; assign image to embed in GLB"
                    )

            if not has_any_texture:
                warnings.append(f"Material '{mat.name}' has no mapped texture slots")

    return warnings


def _first_non_empty(*values):
    for value in values:
        if value:
            return value
    return ""


def _image_basename(image: bpy.types.Image):
    if image is None:
        return ""
    return os.path.splitext(os.path.basename(image.name))[0]


def _texture_entry(slot_name: str, image, explicit_name: str, source: str):
    name = _first_non_empty(explicit_name.strip(), _image_basename(image))
    if not name:
        return None
    return f'{slot_name} = {{ name = "{toml_escape(name)}", source = "{source}" }}'


_SLOT_NODE_LABEL = {
    "albedo": "Albedo",
    "mrao": "MRAO",
    "normal": "Normal",
    "palette": "Palette",
    "snow": "Snow",
    "l0_albedo": "L0 Albedo",
    "l0_mrao": "L0 MRAO",
    "l0_normal": "L0 Normal",
    "l1_albedo": "L1 Albedo",
    "l1_mrao": "L1 MRAO",
    "l1_normal": "L1 Normal",
    "glass_albedo": "Glass Albedo",
    "shatter_map": "Shatter Map",
}

_SLOT_COLORSPACE = {
    "albedo": "sRGB",
    "mrao": "Non-Color",
    "normal": "Non-Color",
    "palette": "sRGB",
    "snow": "sRGB",
    "l0_albedo": "sRGB",
    "l0_mrao": "Non-Color",
    "l0_normal": "Non-Color",
    "l1_albedo": "sRGB",
    "l1_mrao": "Non-Color",
    "l1_normal": "Non-Color",
    "glass_albedo": "sRGB",
    "shatter_map": "Non-Color",
}


def _sync_texture_node(mat, slot_name, img):
    """Update the ShaderNodeTexImage with label matching slot_name in the material's node tree."""
    if not mat.use_nodes or not mat.node_tree:
        return
    label = _SLOT_NODE_LABEL.get(slot_name)
    if not label:
        return
    colorspace = _SLOT_COLORSPACE.get(slot_name, "sRGB")
    for node in mat.node_tree.nodes:
        if node.type == "TEX_IMAGE" and node.label == label:
            node.image = img
            if img and node.image and node.image.colorspace_settings:
                node.image.colorspace_settings.name = colorspace
            return


def _sync_params_to_nodes(mat):
    """Push current param values to ShaderNodeValue / tint nodes in the material's node tree."""
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


def _add_tex_node(nodes, label, image, x, y, colorspace="sRGB"):
    node = nodes.new("ShaderNodeTexImage")
    node.label = label
    node.location = (x, y)
    if image is not None:
        node.image = image
        if node.image.colorspace_settings:
            node.image.colorspace_settings.name = colorspace
    return node


def _add_value_node(nodes, name, value, x, y):
    node = nodes.new("ShaderNodeValue")
    node.label = name
    node.outputs[0].default_value = float(value)
    node.location = (x, y)
    return node


def _float_socket(nodes, value, x, y, label="Value"):
    if hasattr(value, "is_linked"):
        return value
    node = _add_value_node(nodes, label, value, x, y)
    return node.outputs[0]


def _mix_float(nodes, links, fac, a, b, x, y, label="MixFloat"):
    fac_socket = _float_socket(nodes, fac, x - 320, y + 40, f"{label} Fac")
    a_socket = _float_socket(nodes, a, x - 320, y - 40, f"{label} A")
    b_socket = _float_socket(nodes, b, x - 320, y - 120, f"{label} B")

    one = _add_value_node(nodes, f"{label} One", 1.0, x - 180, y + 90)
    one_minus = nodes.new("ShaderNodeMath")
    one_minus.operation = "SUBTRACT"
    one_minus.location = (x - 20, y + 80)
    links.new(one.outputs[0], one_minus.inputs[0])
    links.new(fac_socket, one_minus.inputs[1])

    mul_a = nodes.new("ShaderNodeMath")
    mul_a.operation = "MULTIPLY"
    mul_a.location = (x + 160, y)
    links.new(a_socket, mul_a.inputs[0])
    links.new(one_minus.outputs[0], mul_a.inputs[1])

    mul_b = nodes.new("ShaderNodeMath")
    mul_b.operation = "MULTIPLY"
    mul_b.location = (x + 160, y - 100)
    links.new(b_socket, mul_b.inputs[0])
    links.new(fac_socket, mul_b.inputs[1])

    add = nodes.new("ShaderNodeMath")
    add.operation = "ADD"
    add.location = (x + 340, y - 40)
    links.new(mul_a.outputs[0], add.inputs[0])
    links.new(mul_b.outputs[0], add.inputs[1])
    return add.outputs[0]


def _material_image(props, slot_name):
    return getattr(props, f"{slot_name}_img")


def _build_graph_standard_pbr(mat, nodes, links):
    props = mat.bevy_toolkit

    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-1800, 400)

    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1600, 400)
    links.new(attr.outputs[0], sep.inputs[0])

    uv = nodes.new("ShaderNodeTexCoord")
    uv.location = (-1800, 50)
    map_node = nodes.new("ShaderNodeMapping")
    map_node.location = (-1600, 50)
    links.new(uv.outputs[2], map_node.inputs[0])

    tiling_val = _add_value_node(nodes, "Tiling", props.tiling, -1800, -120)
    combine = nodes.new("ShaderNodeCombineXYZ")
    combine.location = (-1600, -120)
    links.new(tiling_val.outputs[0], combine.inputs[0])
    links.new(tiling_val.outputs[0], combine.inputs[1])
    links.new(tiling_val.outputs[0], combine.inputs[2])
    links.new(combine.outputs[0], map_node.inputs[3])

    alb = _add_tex_node(nodes, "Albedo", _material_image(props, "albedo"), -1320, 420, "sRGB")
    mrao = _add_tex_node(nodes, "MRAO", _material_image(props, "mrao"), -1320, 160, "Non-Color")
    normal = _add_tex_node(nodes, "Normal", _material_image(props, "normal"), -1320, -80, "Non-Color")
    palette = _add_tex_node(nodes, "Palette", _material_image(props, "palette"), -1320, -340, "sRGB")
    snow = _add_tex_node(nodes, "Snow", _material_image(props, "snow"), -1320, -560, "sRGB")
    links.new(map_node.outputs[0], alb.inputs[0])
    links.new(map_node.outputs[0], mrao.inputs[0])
    links.new(map_node.outputs[0], normal.inputs[0])
    links.new(map_node.outputs[0], snow.inputs[0])

    tint = nodes.new("ShaderNodeMixRGB")
    tint.label = "Tint"
    tint.blend_type = "MULTIPLY"
    tint.inputs[0].default_value = 1.0
    tint.inputs[2].default_value = props.tint
    tint.location = (-980, 440)
    links.new(alb.outputs[0], tint.inputs[1])

    palette_uv = nodes.new("ShaderNodeCombineXYZ")
    palette_uv.location = (-1500, -340)
    links.new(attr.outputs[3], palette_uv.inputs[0])   # Alpha channel from ShaderNodeAttribute
    palette_uv.inputs[1].default_value = 0.5
    links.new(palette_uv.outputs[0], palette.inputs[0])

    palette_mix = nodes.new("ShaderNodeMixRGB")
    palette_mix.blend_type = "MIX"
    palette_mix.location = (-780, 300)
    palette_mix.inputs[1].default_value = (1.0, 1.0, 1.0, 1.0)
    links.new(attr.outputs[3], palette_mix.inputs[0])
    links.new(palette.outputs[0], palette_mix.inputs[2])

    with_palette = nodes.new("ShaderNodeMixRGB")
    with_palette.blend_type = "MULTIPLY"
    with_palette.inputs[0].default_value = 1.0
    with_palette.location = (-560, 420)
    links.new(tint.outputs[0], with_palette.inputs[1])
    links.new(palette_mix.outputs[0], with_palette.inputs[2])

    dirt_val = _add_value_node(nodes, "Dirt Level", props.dirt_level, -980, 220)
    dirt_mul = nodes.new("ShaderNodeMath")
    dirt_mul.operation = "MULTIPLY"
    dirt_mul.location = (-780, 180)
    links.new(sep.outputs[1], dirt_mul.inputs[0])
    links.new(dirt_val.outputs[0], dirt_mul.inputs[1])

    dirt_mix = nodes.new("ShaderNodeMixRGB")
    dirt_mix.location = (-340, 360)
    dirt_mix.inputs[2].default_value = (0.2, 0.15, 0.12, 1.0)
    links.new(dirt_mul.outputs[0], dirt_mix.inputs[0])
    links.new(with_palette.outputs[0], dirt_mix.inputs[1])

    wet_val = _add_value_node(nodes, "Wetness", props.wetness, -980, 80)
    wet_mul = nodes.new("ShaderNodeMath")
    wet_mul.operation = "MULTIPLY"
    wet_mul.location = (-780, 60)
    links.new(sep.outputs[2], wet_mul.inputs[0])
    links.new(wet_val.outputs[0], wet_mul.inputs[1])

    wet_dark = nodes.new("ShaderNodeMixRGB")
    wet_dark.blend_type = "MULTIPLY"
    wet_dark.inputs[0].default_value = 1.0
    wet_dark.inputs[2].default_value = (0.7, 0.7, 0.7, 1.0)
    wet_dark.location = (-120, 340)
    links.new(dirt_mix.outputs[0], wet_dark.inputs[1])

    wet_color = nodes.new("ShaderNodeMixRGB")
    wet_color.location = (120, 340)
    links.new(wet_mul.outputs[0], wet_color.inputs[0])
    links.new(dirt_mix.outputs[0], wet_color.inputs[1])
    links.new(wet_dark.outputs[0], wet_color.inputs[2])

    geom = nodes.new("ShaderNodeNewGeometry")
    geom.location = (-980, -460)
    normal_sep = nodes.new("ShaderNodeSeparateXYZ")
    normal_sep.location = (-780, -460)
    links.new(geom.outputs[1], normal_sep.inputs[0])

    snow_mul = nodes.new("ShaderNodeMath")
    snow_mul.operation = "MULTIPLY"
    snow_mul.location = (-560, -540)
    snow_val = _add_value_node(nodes, "Snow Level", props.snow_level, -780, -620)
    links.new(normal_sep.outputs[1], snow_mul.inputs[0])
    links.new(snow_val.outputs[0], snow_mul.inputs[1])

    snow_mix = nodes.new("ShaderNodeMixRGB")
    snow_mix.location = (340, 300)
    links.new(snow_mul.outputs[0], snow_mix.inputs[0])
    links.new(wet_color.outputs[0], snow_mix.inputs[1])
    links.new(snow.outputs[0], snow_mix.inputs[2])

    mrao_sep = nodes.new("ShaderNodeSeparateColor")
    mrao_sep.location = (-980, 120)
    links.new(mrao.outputs[0], mrao_sep.inputs[0])

    rough_dirt = _mix_float(nodes, links, dirt_mul.outputs[0], mrao_sep.outputs[1], 0.9, -340, 100, "RoughDirt")
    rough_wet = _mix_float(nodes, links, wet_mul.outputs[0], rough_dirt, 0.02, -120, 100, "RoughWet")
    rough_snow = _mix_float(nodes, links, snow_mul.outputs[0], rough_wet, 0.8, 120, 100, "RoughSnow")
    metal_snow = _mix_float(nodes, links, snow_mul.outputs[0], mrao_sep.outputs[0], 0.0, 120, -20, "MetalSnow")

    normal_map = nodes.new("ShaderNodeNormalMap")
    normal_map.location = (-120, -150)
    links.new(normal.outputs[0], normal_map.inputs[1])

    # bevy_masks2 — TEXCOORD_1: R=AO (1=none), G=emissive
    attr2 = nodes.new("ShaderNodeAttribute")
    attr2.attribute_name = ATTR_NAME2
    attr2.location = (200, -200)
    sep2 = nodes.new("ShaderNodeSeparateColor")
    sep2.location = (400, -200)
    links.new(attr2.outputs[0], sep2.inputs[0])

    ao_rgb = nodes.new("ShaderNodeCombineColor")
    ao_rgb.location = (590, -200)
    links.new(sep2.outputs[0], ao_rgb.inputs[0])
    links.new(sep2.outputs[0], ao_rgb.inputs[1])
    links.new(sep2.outputs[0], ao_rgb.inputs[2])

    ao_mul = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location = (590, 260)
    links.new(snow_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0], ao_mul.inputs[2])

    emissive_rgb = nodes.new("ShaderNodeCombineColor")
    emissive_rgb.location = (590, -340)
    links.new(sep2.outputs[1], emissive_rgb.inputs[0])
    links.new(sep2.outputs[1], emissive_rgb.inputs[1])
    links.new(sep2.outputs[1], emissive_rgb.inputs[2])

    emissive_mul = nodes.new("ShaderNodeMixRGB")
    emissive_mul.blend_type = "MULTIPLY"
    emissive_mul.inputs[0].default_value = 1.0
    emissive_mul.location = (590, 120)
    links.new(alb.outputs[0], emissive_mul.inputs[1])
    links.new(emissive_rgb.outputs[0], emissive_mul.inputs[2])

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (820, 260)
    links.new(ao_mul.outputs[0], bsdf.inputs[0])
    links.new(metal_snow, bsdf.inputs[1])
    links.new(rough_snow, bsdf.inputs[2])
    links.new(normal_map.outputs[0], bsdf.inputs[5])
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
    props = mat.bevy_toolkit

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

    l0_val = _add_value_node(nodes, "L0 Tiling", props.l0_tiling, -2000, -40)
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

    l0_alb = _add_tex_node(nodes, "L0 Albedo", _material_image(props, "l0_albedo"), -1520, 420, "sRGB")
    l0_mrao = _add_tex_node(nodes, "L0 MRAO", _material_image(props, "l0_mrao"), -1520, 180, "Non-Color")
    l0_nrm = _add_tex_node(nodes, "L0 Normal", _material_image(props, "l0_normal"), -1520, -40, "Non-Color")
    l1_alb = _add_tex_node(nodes, "L1 Albedo", _material_image(props, "l1_albedo"), -1520, -300, "sRGB")
    l1_mrao = _add_tex_node(nodes, "L1 MRAO", _material_image(props, "l1_mrao"), -1520, -520, "Non-Color")
    l1_nrm = _add_tex_node(nodes, "L1 Normal", _material_image(props, "l1_normal"), -1520, -760, "Non-Color")
    snow = _add_tex_node(nodes, "Snow", _material_image(props, "snow"), -1520, -980, "sRGB")

    for tex in (l0_alb, l0_mrao, l0_nrm):
        links.new(map0.outputs[0], tex.inputs[0])
    for tex in (l1_alb, l1_mrao, l1_nrm):
        links.new(map1.outputs[0], tex.inputs[0])
    links.new(map0.outputs[0], snow.inputs[0])

    blend = sep.outputs[0]
    dirt_mask = sep.outputs[1]
    wet_mask = sep.outputs[2]

    alb_mix = nodes.new("ShaderNodeMixRGB")
    alb_mix.location = (-1180, 260)
    links.new(blend, alb_mix.inputs[0])
    links.new(l0_alb.outputs[0], alb_mix.inputs[1])
    links.new(l1_alb.outputs[0], alb_mix.inputs[2])

    l0_m = nodes.new("ShaderNodeSeparateColor")
    l0_m.location = (-1180, 60)
    links.new(l0_mrao.outputs[0], l0_m.inputs[0])
    l1_m = nodes.new("ShaderNodeSeparateColor")
    l1_m.location = (-1180, -140)
    links.new(l1_mrao.outputs[0], l1_m.inputs[0])

    rough_mix = _mix_float(nodes, links, blend, l0_m.outputs[1], l1_m.outputs[1], -940, 20, "LayerRough")
    metal_mix = _mix_float(nodes, links, blend, l0_m.outputs[0], l1_m.outputs[0], -940, -100, "LayerMetal")

    nrm_mix = nodes.new("ShaderNodeMixRGB")
    nrm_mix.location = (-940, -260)
    links.new(blend, nrm_mix.inputs[0])
    links.new(l0_nrm.outputs[0], nrm_mix.inputs[1])
    links.new(l1_nrm.outputs[0], nrm_mix.inputs[2])

    dirt_val = _add_value_node(nodes, "Dirt Level", props.dirt_level, -1180, -320)
    dirt_mul = nodes.new("ShaderNodeMath")
    dirt_mul.operation = "MULTIPLY"
    dirt_mul.location = (-980, -340)
    links.new(dirt_mask, dirt_mul.inputs[0])
    links.new(dirt_val.outputs[0], dirt_mul.inputs[1])

    dirt_mix = nodes.new("ShaderNodeMixRGB")
    dirt_mix.location = (-720, 260)
    dirt_mix.inputs[2].default_value = (0.15, 0.12, 0.1, 1.0)
    links.new(dirt_mul.outputs[0], dirt_mix.inputs[0])
    links.new(alb_mix.outputs[0], dirt_mix.inputs[1])

    por_val = _add_value_node(nodes, "Porosity", props.porosity, -1180, -440)
    wet_val = _add_value_node(nodes, "Wetness", props.wetness, -1180, -520)
    wet_mul = nodes.new("ShaderNodeMath")
    wet_mul.operation = "MULTIPLY"
    wet_mul.location = (-980, -500)
    links.new(wet_mask, wet_mul.inputs[0])
    links.new(wet_val.outputs[0], wet_mul.inputs[1])

    wet_dark = nodes.new("ShaderNodeMixRGB")
    wet_dark.blend_type = "MULTIPLY"
    wet_dark.inputs[0].default_value = 1.0
    wet_dark.inputs[2].default_value = (0.8, 0.8, 0.8, 1.0)
    wet_dark.location = (-500, 240)
    links.new(dirt_mix.outputs[0], wet_dark.inputs[1])

    wet_color = nodes.new("ShaderNodeMixRGB")
    wet_color.location = (-280, 240)
    links.new(wet_mul.outputs[0], wet_color.inputs[0])
    links.new(dirt_mix.outputs[0], wet_color.inputs[1])
    links.new(wet_dark.outputs[0], wet_color.inputs[2])

    geom = nodes.new("ShaderNodeNewGeometry")
    geom.location = (-980, -700)
    nsep = nodes.new("ShaderNodeSeparateXYZ")
    nsep.location = (-780, -700)
    links.new(geom.outputs[1], nsep.inputs[0])
    snow_val = _add_value_node(nodes, "Snow Level", props.snow_level, -980, -820)
    snow_mask_mul = nodes.new("ShaderNodeMath")
    snow_mask_mul.operation = "MULTIPLY"
    snow_mask_mul.location = (-580, -760)
    links.new(nsep.outputs[1], snow_mask_mul.inputs[0])
    links.new(snow_val.outputs[0], snow_mask_mul.inputs[1])

    snow_mix = nodes.new("ShaderNodeMixRGB")
    snow_mix.location = (-40, 240)
    links.new(snow_mask_mul.outputs[0], snow_mix.inputs[0])
    links.new(wet_color.outputs[0], snow_mix.inputs[1])
    links.new(snow.outputs[0], snow_mix.inputs[2])

    rough_wet = _mix_float(nodes, links, wet_mul.outputs[0], rough_mix, 0.02, -500, 80, "LayerRoughWet")
    rough_snow = _mix_float(nodes, links, snow_mask_mul.outputs[0], rough_wet, 0.8, -280, 80, "LayerRoughSnow")
    metal_snow = _mix_float(nodes, links, snow_mask_mul.outputs[0], metal_mix, 0.0, -280, -20, "LayerMetalSnow")

    normal_map = nodes.new("ShaderNodeNormalMap")
    normal_map.location = (-500, -180)
    links.new(nrm_mix.outputs[0], normal_map.inputs[1])

    # bevy_masks2 — TEXCOORD_1: R=AO (1=none), G=emissive
    attr2 = nodes.new("ShaderNodeAttribute")
    attr2.attribute_name = ATTR_NAME2
    attr2.location = (-80, -360)
    sep2 = nodes.new("ShaderNodeSeparateColor")
    sep2.location = (120, -360)
    links.new(attr2.outputs[0], sep2.inputs[0])

    ao_rgb = nodes.new("ShaderNodeCombineColor")
    ao_rgb.location = (320, -360)
    links.new(sep2.outputs[0], ao_rgb.inputs[0])
    links.new(sep2.outputs[0], ao_rgb.inputs[1])
    links.new(sep2.outputs[0], ao_rgb.inputs[2])

    ao_mul = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location = (320, 200)
    links.new(snow_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0], ao_mul.inputs[2])

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (560, 200)
    links.new(ao_mul.outputs[0], bsdf.inputs[0])
    links.new(metal_snow, bsdf.inputs[1])
    links.new(rough_snow, bsdf.inputs[2])
    links.new(normal_map.outputs[0], bsdf.inputs[5])

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (860, 200)
    links.new(bsdf.outputs[0], out.inputs[0])


def _build_graph_vehicle_glass(mat, nodes, links):
    props = mat.bevy_toolkit

    attr = nodes.new("ShaderNodeAttribute")
    attr.attribute_name = ATTR_NAME
    attr.location = (-1500, 220)
    sep = nodes.new("ShaderNodeSeparateColor")
    sep.location = (-1300, 220)
    links.new(attr.outputs[0], sep.inputs[0])

    uv = nodes.new("ShaderNodeTexCoord")
    uv.location = (-1500, 20)

    glass = _add_tex_node(nodes, "Glass Albedo", _material_image(props, "glass_albedo"), -1180, 260, "sRGB")
    shatter = _add_tex_node(nodes, "Shatter Map", _material_image(props, "shatter_map"), -1180, 40, "Non-Color")
    links.new(uv.outputs[2], glass.inputs[0])
    links.new(uv.outputs[2], shatter.inputs[0])

    shatter_r = nodes.new("ShaderNodeSeparateColor")
    shatter_r.location = (-980, 40)
    links.new(shatter.outputs[0], shatter_r.inputs[0])

    shatter_cut = nodes.new("ShaderNodeValToRGB")
    shatter_cut.location = (-780, 20)
    shatter_cut.color_ramp.elements[0].position = 0.1
    shatter_cut.color_ramp.elements[0].color = (0.0, 0.0, 0.0, 1.0)
    shatter_cut.color_ramp.elements[1].position = 0.4
    shatter_cut.color_ramp.elements[1].color = (1.0, 1.0, 1.0, 1.0)
    links.new(shatter_r.outputs[0], shatter_cut.inputs[0])

    crack_mix = nodes.new("ShaderNodeMixRGB")
    crack_mix.location = (-540, 240)
    crack_mix.inputs[1].default_value = (0.9, 0.9, 0.9, 1.0)
    links.new(shatter_cut.outputs[0], crack_mix.inputs[0])
    links.new(glass.outputs[0], crack_mix.inputs[2])

    dirt_val = _add_value_node(nodes, "Dirt Level", props.dirt_level, -980, -140)
    dirt_mul = nodes.new("ShaderNodeMath")
    dirt_mul.operation = "MULTIPLY"
    dirt_mul.location = (-780, -140)
    links.new(sep.outputs[1], dirt_mul.inputs[0])
    links.new(dirt_val.outputs[0], dirt_mul.inputs[1])

    dirt_mix = nodes.new("ShaderNodeMixRGB")
    dirt_mix.location = (-320, 220)
    dirt_mix.inputs[2].default_value = (0.2, 0.18, 0.15, 1.0)
    links.new(dirt_mul.outputs[0], dirt_mix.inputs[0])
    links.new(crack_mix.outputs[0], dirt_mix.inputs[1])

    wet_val = _add_value_node(nodes, "Wetness", props.wetness, -780, -260)
    alpha_mul = nodes.new("ShaderNodeMath")
    alpha_mul.operation = "MULTIPLY"
    alpha_mul.location = (-320, 20)
    links.new(glass.outputs[1], alpha_mul.inputs[0])
    links.new(shatter_cut.outputs[0], alpha_mul.inputs[1])

    alpha_dirt = _mix_float(nodes, links, dirt_mul.outputs[0], alpha_mul.outputs[0], 0.95, -100, 20, "GlassAlphaDirt")

    alpha_wet = nodes.new("ShaderNodeMath")
    alpha_wet.operation = "SUBTRACT"
    alpha_wet.location = (120, 20)
    links.new(alpha_dirt, alpha_wet.inputs[0])
    rain_clean = nodes.new("ShaderNodeMath")
    rain_clean.operation = "MULTIPLY"
    rain_clean.inputs[1].default_value = 0.4
    rain_clean.location = (-100, -120)
    links.new(wet_val.outputs[0], rain_clean.inputs[0])
    links.new(rain_clean.outputs[0], alpha_wet.inputs[1])

    rough_crack = _mix_float(nodes, links, shatter_cut.outputs[0], 0.7, 0.05, -320, -200, "GlassRoughCrack")
    rough_dirt = _mix_float(nodes, links, dirt_mul.outputs[0], rough_crack, 0.8, -100, -200, "GlassRoughDirt")
    rough_wet = _mix_float(nodes, links, wet_val.outputs[0], rough_dirt, 0.01, 120, -200, "GlassRoughWet")

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

    ao_mul = nodes.new("ShaderNodeMixRGB")
    ao_mul.blend_type = "MULTIPLY"
    ao_mul.inputs[0].default_value = 1.0
    ao_mul.location = (560, 200)
    links.new(dirt_mix.outputs[0], ao_mul.inputs[1])
    links.new(ao_rgb.outputs[0], ao_mul.inputs[2])

    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (760, 200)
    bsdf.inputs[17].default_value = 1.0  # Transmission
    bsdf.inputs[14].default_value = 0.8  # Specular IOR Level
    links.new(ao_mul.outputs[0], bsdf.inputs[0])
    links.new(rough_wet, bsdf.inputs[2])
    links.new(alpha_wet.outputs[0], bsdf.inputs[4])

    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (1060, 200)
    links.new(bsdf.outputs[0], out.inputs[0])

    mat.blend_method = "BLEND"
    mat.shadow_method = "HASHED"


def build_material_inline(mat: bpy.types.Material) -> str:
    props = mat.bevy_toolkit
    texture_items = []
    for slot_name, _label, _default_source in get_template_texture_specs(props.template):
        image, explicit_name, source = _material_texture_value(props, slot_name)
        entry = _texture_entry(slot_name, image, explicit_name, source)
        if entry:
            texture_items.append(entry)

    textures_table = "{ " + ", ".join(texture_items) + " }" if texture_items else "{}"
    tint = props.tint
    params = (
        "{ "
        f"tint = [{format_float(tint[0])}, {format_float(tint[1])}, {format_float(tint[2])}, {format_float(tint[3])}], "
        f"tiling = {format_float(props.tiling)}, "
        f"l0_tiling = {format_float(props.l0_tiling)}, "
        f"l1_tiling = {format_float(props.l1_tiling)}, "
        f"porosity = {format_float(props.porosity)}, "
        f"wetness = {format_float(props.wetness)}, "
        f"snow_level = {format_float(props.snow_level)}, "
        f"dirt_level = {format_float(props.dirt_level)}"
        " }"
    )
    return (
        "{ "
        f'template = "{props.template}", '
        f"textures = {textures_table}, "
        f"params = {params}"
        " }"
    )


def create_bevy_node_tree(mat: bpy.types.Material):
    if not mat:
        return
    if not mat.use_nodes:
        mat.use_nodes = True

    nodes = mat.node_tree.nodes
    links = mat.node_tree.links
    nodes.clear()

    template = mat.bevy_toolkit.template
    if template == "layered_env":
        _build_graph_layered_env(mat, nodes, links)
        return
    if template == "vehicle_glass":
        _build_graph_vehicle_glass(mat, nodes, links)
        return
    _build_graph_standard_pbr(mat, nodes, links)


class BEVY_OT_InitProject(bpy.types.Operator):
    bl_idname = "bevy.init_project"
    bl_label = "Initialize Masks"
    bl_description = "Create/activate bevy_masks vertex color channel on selected meshes"

    def execute(self, context):
        targets = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets and context.active_object and context.active_object.type == "MESH":
            targets = [context.active_object]

        if not targets:
            self.report({"WARNING"}, "No mesh object selected")
            return {"CANCELLED"}

        for obj in targets:
            ensure_mask_attribute(obj.data)
        self.report({"INFO"}, f"Initialized masks for {len(targets)} mesh object(s)")
        return {"FINISHED"}


class BEVY_OT_InitMasks2(bpy.types.Operator):
    bl_idname = "bevy.init_masks2"
    bl_label = "Initialize bevy_masks2"
    bl_description = "Create/activate bevy_masks2 vertex color channel (AO=1, emissive=0)"

    def execute(self, context):
        targets = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets and context.active_object and context.active_object.type == "MESH":
            targets = [context.active_object]
        if not targets:
            self.report({"WARNING"}, "No mesh object selected")
            return {"CANCELLED"}
        for obj in targets:
            ensure_masks2_attribute(obj.data)
        self.report({"INFO"}, f"Initialized masks2 for {len(targets)} mesh object(s)")
        return {"FINISHED"}


class BEVY_OT_SetPaint(bpy.types.Operator):
    bl_idname = "bevy.set_paint"
    bl_label = "Set Paint Mask"
    mode: bpy.props.StringProperty()

    def execute(self, context):
        obj = context.active_object
        if not obj or obj.type != "MESH":
            self.report({"WARNING"}, "Active object must be a mesh")
            return {"CANCELLED"}

        brush = context.tool_settings.vertex_paint.brush

        # bevy_masks2 modes — switch active color attribute to masks2
        if self.mode in ("INIT2", "AO", "EMISSIVE", "ERASE2"):
            ensure_masks2_attribute(obj.data)
            if context.object.mode != "PAINT_VERTEX":
                bpy.ops.object.mode_set(mode="PAINT_VERTEX")
            if self.mode == "AO":
                brush.color = (1.0, 0.0, 0.0)   # R=1: full AO (no occlusion)
            elif self.mode == "EMISSIVE":
                brush.color = (0.0, 1.0, 0.0)   # G=1: full emissive
            elif self.mode == "ERASE2":
                brush.color = (1.0, 0.0, 0.0)   # R=1: reset to AO=1 (no effect)
            return {"FINISHED"}

        # bevy_masks modes
        ensure_mask_attribute(obj.data)
        if context.object.mode != "PAINT_VERTEX":
            bpy.ops.object.mode_set(mode="PAINT_VERTEX")

        if self.mode == "NORMAL_SUPP":
            brush.color = (1.0, 0.0, 0.0)   # R=1: suppress normal map
        elif self.mode == "L1":
            brush.color = (1.0, 0.0, 0.0)   # R=1: blend to layer 1
        elif self.mode == "DIRT":
            brush.color = (0.0, 1.0, 0.0)   # G=1: full dirt
        elif self.mode == "BLOOD":
            brush.color = (0.0, 1.0, 0.0)
        elif self.mode == "WET":
            brush.color = (0.0, 0.0, 1.0)   # B=1: fully wet
        elif self.mode == "ERASE":
            brush.color = (0.0, 0.0, 0.0)
        else:
            self.report({"WARNING"}, f"Unknown paint mode: {self.mode}")
            return {"CANCELLED"}
        return {"FINISHED"}


class BEVY_OT_FillAlphaMask(bpy.types.Operator):
    bl_idname = "bevy.fill_alpha_mask"
    bl_label = "Fill Tint Alpha"
    bl_description = "Write a constant alpha value to bevy_masks for selected meshes"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        targets = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets and context.active_object and context.active_object.type == "MESH":
            targets = [context.active_object]

        if not targets:
            self.report({"WARNING"}, "No mesh object selected")
            return {"CANCELLED"}

        for obj in targets:
            fill_alpha_channel(obj.data, settings.alpha_fill_value)
        self.report({"INFO"}, f"Filled alpha for {len(targets)} mesh object(s)")
        return {"FINISHED"}


class BEVY_OT_GenerateCol(bpy.types.Operator):
    bl_idname = "bevy.gen_col"
    bl_label = "Create Collision Proxy"

    def execute(self, context):
        src = context.active_object
        if not src or src.type != "MESH":
            self.report({"WARNING"}, "Active object must be a mesh")
            return {"CANCELLED"}

        new_obj = src.copy()
        new_obj.data = src.data.copy()
        new_obj.name = f"COL_{src.name}"
        new_obj.bevy_toolkit_obj.is_col = True
        new_obj.bevy_toolkit_obj.col_shape = "CONVEX"
        new_obj.display_type = "WIRE"
        new_obj.hide_render = True
        context.collection.objects.link(new_obj)
        self.report({"INFO"}, f"Collision proxy created: {new_obj.name}")
        return {"FINISHED"}


class BEVY_OT_SetupNodes(bpy.types.Operator):
    bl_idname = "bevy.setup_nodes"
    bl_label = "Setup Preview Nodes"

    def execute(self, context):
        mat = context.active_material
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        create_bevy_node_tree(mat)
        return {"FINISHED"}


class BEVY_OT_ConvertToDrawableModel(bpy.types.Operator):
    bl_idname = "bevy.convert_to_drawable_model"
    bl_label = "Convert to Drawable Model"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        targets = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets:
            self.report({"WARNING"}, "Select at least one mesh object")
            return {"CANCELLED"}

        active = context.active_object if context.active_object in targets else targets[0]
        model_name = f"{active.name}.model"
        model_obj = bpy.data.objects.get(model_name)
        if not model_obj:
            model_obj = bpy.data.objects.new(model_name, None)
            context.collection.objects.link(model_obj)

        if settings.center_to_selection:
            center = sum((obj.location for obj in targets), Vector((0.0, 0.0, 0.0))) / len(targets)
            model_obj.location = center
        else:
            model_obj.location = active.location.copy()

        for obj in targets:
            world_matrix = obj.matrix_world.copy()
            obj.parent = model_obj
            obj.matrix_world = world_matrix

        self.report({"INFO"}, f"Created drawable model root '{model_obj.name}'")
        return {"FINISHED"}


class BEVY_OT_ConvertToDrawable(bpy.types.Operator):
    bl_idname = "bevy.convert_to_drawable"
    bl_label = "Convert to Drawable"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        targets = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets:
            self.report({"WARNING"}, "Select at least one mesh object")
            return {"CANCELLED"}

        created_col = 0
        mapped_textures = 0
        created_proxy_names = []
        for obj in targets:
            if obj.name.startswith("COL_"):
                obj.bevy_toolkit_obj.is_col = True
            else:
                obj.bevy_toolkit_obj.is_col = False
                ensure_mask_attribute(obj.data)
                ensure_masks2_attribute(obj.data)
                if settings.auto_embed_collision:
                    proxy_obj, created = duplicate_collision_proxy(obj, context.collection)
                    created_col += 1 if created else 0
                    created_proxy_names.append(proxy_obj.name)

            for slot in obj.material_slots:
                if slot.material:
                    mapped_textures += sync_material_from_nodes(slot.material, only_if_empty=True)

        # Ensure generated proxies are marked and included in the same root hierarchy.
        for proxy_name in created_proxy_names:
            proxy_obj = bpy.data.objects.get(proxy_name)
            if proxy_obj:
                proxy_obj.bevy_toolkit_obj.is_col = True

        self.report(
            {"INFO"},
            f"Converted {len(targets)} object(s); created {created_col} collision proxy object(s); mapped {mapped_textures} texture slot(s)",
        )
        return {"FINISHED"}


class BEVY_OT_CreateDrawable(bpy.types.Operator):
    bl_idname = "bevy.create_drawable"
    bl_label = "Create Drawable"

    def execute(self, context):
        result_model = bpy.ops.bevy.convert_to_drawable_model()
        if "FINISHED" not in result_model:
            return {"CANCELLED"}
        result_drawable = bpy.ops.bevy.convert_to_drawable()
        if "FINISHED" not in result_drawable:
            return {"CANCELLED"}
        self.report({"INFO"}, "Drawable workflow complete")
        return {"FINISHED"}


class BEVY_OT_CreateDrawableDictionary(bpy.types.Operator):
    bl_idname = "bevy.create_drawable_dictionary"
    bl_label = "Create Drawable Dictionary"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        dict_name = settings.drawable_dict_name.strip() or "DrawableDictionary"
        collection = bpy.data.collections.get(dict_name)
        created = False
        if collection is None:
            collection = bpy.data.collections.new(dict_name)
            context.scene.collection.children.link(collection)
            created = True

        roots = [obj for obj in context.selected_objects if obj.type == "EMPTY" and obj.name.endswith(".model")]
        for root in roots:
            if root not in collection.objects:
                collection.objects.link(root)

        verb = "Created" if created else "Updated"
        self.report({"INFO"}, f"{verb} drawable dictionary '{dict_name}'")
        return {"FINISHED"}


class BEVY_OT_CreateShaderMaterial(bpy.types.Operator):
    bl_idname = "bevy.create_shader_material"
    bl_label = "Create Shader Material"

    def execute(self, context):
        active = context.active_object
        if not active or active.type != "MESH":
            self.report({"WARNING"}, "Active object must be a mesh")
            return {"CANCELLED"}

        base_name = "standard"
        idx = 1
        mat_name = base_name
        while mat_name in bpy.data.materials:
            idx += 1
            mat_name = f"{base_name}.{idx:03d}"

        mat = bpy.data.materials.new(mat_name)
        mat.use_nodes = True
        mat.bevy_toolkit.template = "standard_pbr"
        create_bevy_node_tree(mat)

        if active.data.materials:
            active.data.materials[active.active_material_index] = mat
        else:
            active.data.materials.append(mat)

        self.report({"INFO"}, f"Created material '{mat.name}'")
        return {"FINISHED"}


class BEVY_OT_ConvertActiveMaterial(bpy.types.Operator):
    bl_idname = "bevy.convert_active_material"
    bl_label = "Convert Active Material"

    def execute(self, context):
        mat = context.active_material
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        mapped = sync_material_from_nodes(mat, only_if_empty=False)
        self.report({"INFO"}, f"Mapped {mapped} texture slot(s) from material nodes")
        return {"FINISHED"}


class BEVY_OT_ConvertAllMaterials(bpy.types.Operator):
    bl_idname = "bevy.convert_all_materials"
    bl_label = "Convert All Materials"

    def execute(self, context):
        total = 0
        mats = [mat for mat in bpy.data.materials if hasattr(mat, "bevy_toolkit")]
        for mat in mats:
            total += sync_material_from_nodes(mat, only_if_empty=False)
        self.report({"INFO"}, f"Mapped {total} texture slot(s) across {len(mats)} material(s)")
        return {"FINISHED"}


class BEVY_OT_SetAllTexturesEmbedded(bpy.types.Operator):
    bl_idname = "bevy.set_all_textures_embedded"
    bl_label = "Set all Textures Embedded"

    def execute(self, context):
        mat = context.active_material
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        sync_material_from_nodes(mat, only_if_empty=True)
        set_material_texture_source(mat, "embedded")
        self.report({"INFO"}, f"Material '{mat.name}' texture sources set to embedded")
        return {"FINISHED"}


class BEVY_OT_RemoveAllEmbeddedTextures(bpy.types.Operator):
    bl_idname = "bevy.remove_all_embedded_textures"
    bl_label = "Remove all Embedded Textures"

    def execute(self, context):
        mat = context.active_material
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        cleared = clear_embedded_images(mat)
        self.report({"INFO"}, f"Removed {cleared} embedded image assignment(s) from '{mat.name}'")
        return {"FINISHED"}


class BEVY_OT_SetAllMaterialsEmbedded(bpy.types.Operator):
    bl_idname = "bevy.set_all_materials_embedded"
    bl_label = "Set all Materials Embedded"

    def execute(self, context):
        mats = [mat for mat in bpy.data.materials if hasattr(mat, "bevy_toolkit")]
        for mat in mats:
            sync_material_from_nodes(mat, only_if_empty=True)
            set_material_texture_source(mat, "embedded")
        self.report({"INFO"}, f"Set {len(mats)} material(s) to embedded source")
        return {"FINISHED"}


class BEVY_OT_SetAllMaterialsUnembedded(bpy.types.Operator):
    bl_idname = "bevy.set_all_materials_unembedded"
    bl_label = "Set all Materials Unembedded"

    def execute(self, context):
        mats = [mat for mat in bpy.data.materials if hasattr(mat, "bevy_toolkit")]
        for mat in mats:
            set_material_texture_source(mat, "shared")
        self.report({"INFO"}, f"Set {len(mats)} material(s) to shared source")
        return {"FINISHED"}


class BEVY_OT_Export(bpy.types.Operator):
    bl_idname = "bevy.export_project"
    bl_label = "Export ADS (GLB + Drawable)"

    def execute(self, context):
        if not bpy.data.filepath:
            self.report({"WARNING"}, "Save .blend file first")
            return {"CANCELLED"}

        base_path = bpy.data.filepath.replace(".blend", "")
        glb_path = base_path + ".glb"
        toml_path = base_path + ".drawable"
        asset_name = os.path.basename(base_path)
        settings = context.scene.bevy_toolkit_export

        target_meshes = gather_target_meshes(context, settings)
        if not target_meshes:
            self.report({"WARNING"}, "No mesh objects match selected export scope")
            return {"CANCELLED"}

        warnings = validate_export_consistency(target_meshes)
        for warning_text in warnings[:8]:
            self.report({"WARNING"}, warning_text)
        if len(warnings) > 8:
            self.report({"WARNING"}, f"... and {len(warnings) - 8} more consistency warning(s)")

        encoded_uv_meshes = []
        for obj in target_meshes:
            if not is_collision_object(obj):
                ensure_mask_attribute(obj.data)
                uv_name = encode_masks2_to_uv(obj.data)
                if uv_name:
                    idx = list(obj.data.uv_layers.keys()).index(uv_name)
                    if idx != 1:
                        self.report({"WARNING"}, f"'{obj.name}': {UV_MASKS2_NAME} is at UV index {idx}, expected 1 (TEXCOORD_1). Add exactly one primary UV map before exporting.")
                    encoded_uv_meshes.append(obj.data)

        original_active = context.view_layer.objects.active
        original_selection = [obj for obj in context.selected_objects]
        export_selected = False
        try:
            if settings.export_scope == "ALL":
                export_selected = False
            else:
                export_selected = True
                bpy.ops.object.select_all(action="DESELECT")
                for obj in target_meshes:
                    obj.select_set(True)
                if target_meshes:
                    context.view_layer.objects.active = target_meshes[0]

            gltf_result = bpy.ops.export_scene.gltf(
                filepath=glb_path,
                export_format="GLB",
                use_selection=export_selected,
                export_apply=settings.apply_modifiers,
                export_yup=True,
                export_tangents=True,
                export_vertex_color='ACTIVE',
                export_materials="EXPORT",
                export_normals=True,
                export_texcoords=True,
                export_image_format="AUTO",
            )
        finally:
            bpy.ops.object.select_all(action="DESELECT")
            for obj in original_selection:
                if obj.name in bpy.data.objects:
                    obj.select_set(True)
            if original_active and original_active.name in bpy.data.objects:
                context.view_layer.objects.active = original_active

        for mesh in encoded_uv_meshes:
            remove_temp_uv(mesh, UV_MASKS2_NAME)

        if "FINISHED" not in gltf_result:
            self.report({"ERROR"}, "GLB export failed")
            return {"CANCELLED"}

        used_materials = []
        seen_material_names = set()
        for obj in target_meshes:
            for slot in obj.material_slots:
                mat = slot.material
                if mat and mat.name not in seen_material_names:
                    used_materials.append(mat)
                    seen_material_names.add(mat.name)

        lines = []
        lines.append(f'asset_name = "{toml_escape(asset_name)}"')
        lines.append(f'version = "{ADS_VERSION}"')
        lines.append("")
        lines.append("[materials]")
        for mat in used_materials:
            inline = build_material_inline(mat)
            lines.append(f'"{toml_escape(mat.name)}" = {inline}')

        lines.append("")
        lines.append("[entities]")
        for obj in target_meshes:
            key = f'"{toml_escape(obj.name)}"'
            if is_collision_object(obj):
                props = obj.bevy_toolkit_obj
                tags = parse_tags(props.tags_csv)
                tags_array = "[" + ", ".join(f'"{toml_escape(tag)}"' for tag in tags) + "]"
                lines.append(
                    key
                    + " = { "
                    + 'type = "COLLISION", '
                    + f'shape = "{props.col_shape}", '
                    + f"mass = {format_float(props.mass)}, "
                    + f"is_static = {bool_to_toml(props.is_static)}, "
                    + f"friction = {format_float(props.friction)}, "
                    + f"restitution = {format_float(props.restitution)}, "
                    + f"tags = {tags_array}"
                    + " }"
                )
            else:
                lines.append(
                    key
                    + " = { "
                    + 'type = "MESH", '
                    + f"cast_shadows = {bool_to_toml(obj.bevy_toolkit_obj.cast_shadows)}"
                    + " }"
                )

        with open(toml_path, "w", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")

        self.report({"INFO"}, f"Exported: {os.path.basename(glb_path)} and {os.path.basename(toml_path)}")
        return {"FINISHED"}


class BEVY_OT_BrowseTexture(bpy.types.Operator):
    """Select a texture file and assign it to this material slot"""
    bl_idname = "bevy.browse_texture"
    bl_label = "Browse Texture"
    bl_options = {'INTERNAL', 'UNDO'}

    filepath: bpy.props.StringProperty(subtype='FILE_PATH')
    filter_glob: bpy.props.StringProperty(
        default="*.png;*.jpg;*.jpeg;*.tga;*.bmp;*.tif;*.tiff;*.dds;*.exr",
        options={'HIDDEN'},
    )
    slot_name: bpy.props.StringProperty(options={'HIDDEN'})
    mat_name: bpy.props.StringProperty(options={'HIDDEN'})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        mat = bpy.data.materials.get(self.mat_name)
        if not mat:
            return {'CANCELLED'}
        props = mat.bevy_toolkit
        img = bpy.data.images.load(self.filepath, check_existing=True)
        setattr(props, f"{self.slot_name}_img", img)
        if not getattr(props, f"{self.slot_name}_name").strip():
            setattr(
                props,
                f"{self.slot_name}_name",
                os.path.splitext(os.path.basename(self.filepath))[0],
            )
        _sync_texture_node(mat, self.slot_name, img)
        return {'FINISHED'}


def _draw_bevy_material_props(layout, mat):
    props = mat.bevy_toolkit
    template = props.template

    header = layout.row(align=True)
    header.prop(props, "template")
    header.operator("bevy.setup_nodes", text="", icon="NODETREE")

    tex_box = layout.box()
    tex_box.label(text="Texture Parameters")

    for slot_name, label, _ in get_template_texture_specs(template):
        col = tex_box.column(align=True)

        # Slot label + image template_ID on the same row
        row = col.row(align=True)
        split = row.split(factor=0.32, align=True)
        split.label(text=label)
        split.template_ID(props, f"{slot_name}_img", open="image.open")

        # Embedded checkbox + Color Space label — only when image is assigned
        img = getattr(props, f"{slot_name}_img")
        if img is not None:
            sub = col.row(align=False)
            sub.separator(factor=3.5)
            sub.prop(props, f"{slot_name}_embedded", text="Embedded")
            cs = sub.row()
            cs.label(text=f"Color Space:   {_SLOT_COLORSPACE.get(slot_name, 'sRGB')}")

        col.separator(factor=0.3)

    params_box = layout.box()
    params_box.label(text="Value Parameters")
    col = params_box.column(align=True)
    col.prop(props, "snow_level", slider=True)
    col.prop(props, "dirt_level", slider=True)
    col.prop(props, "wetness", slider=True)
    col.prop(props, "porosity", slider=True)
    col.separator()
    col.prop(props, "tiling")
    if template == "layered_env":
        col.prop(props, "l0_tiling")
        col.prop(props, "l1_tiling")
    col.separator()
    col.prop(props, "tint")


class BEVY_PT_MaterialPanel(bpy.types.Panel):
    bl_label = "Bevy ADS"
    bl_idname = "MATERIAL_PT_bevy_ads"
    bl_space_type = 'PROPERTIES'
    bl_region_type = 'WINDOW'
    bl_context = 'material'

    @classmethod
    def poll(cls, context):
        return (
            context.active_object is not None
            and context.active_object.active_material is not None
        )

    def draw(self, context):
        _draw_bevy_material_props(self.layout, context.active_object.active_material)


class BEVY_PT_Panel(bpy.types.Panel):
    bl_label = "Bevy ADS Toolkit"
    bl_idname = "BEVY_PT_main"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Bevy"

    def draw(self, context):
        layout = self.layout
        obj = context.active_object
        settings = context.scene.bevy_toolkit_export

        layout.label(text="Drawables", icon="OUTLINER_COLLECTION")

        convert_box = layout.box()
        convert_box.label(text="Convert", icon="FILE_REFRESH")
        row = convert_box.row(align=True)
        row.operator("bevy.convert_to_drawable_model", text="Convert to Drawable Model")
        row = convert_box.row(align=True)
        row.operator("bevy.convert_to_drawable", text="Convert to Drawable")

        options_row = convert_box.row(align=True)
        options_row.prop(settings, "auto_embed_collision")
        options_row.prop(settings, "center_to_selection")

        create_box = layout.box()
        create_box.label(text="Create", icon="ADD")
        create_row = create_box.row(align=True)
        create_row.operator("bevy.create_drawable", text="Create Drawable")
        create_row.operator("bevy.create_drawable_dictionary", text="Create Drawable Dictionary")
        create_box.prop(settings, "drawable_dict_name", text="Dictionary")

        shader_box = layout.box()
        shader_box.label(text="Shader Tools", icon="SHADING_RENDERED")
        shader_box.operator("bevy.create_shader_material", text="Create Shader Material")
        shader_row = shader_box.row(align=True)
        shader_row.operator("bevy.convert_active_material", text="Convert Active Material")
        shader_row.operator("bevy.convert_all_materials", text="Convert All Materials")

        tools_box = layout.box()
        tools_box.label(text="Tools", icon="TOOL_SETTINGS")
        tool_row = tools_box.row(align=True)
        tool_row.operator("bevy.set_all_textures_embedded", text="Set all Textures Embedded")
        tool_row.operator("bevy.remove_all_embedded_textures", text="Remove all Embedded Textures")
        tool_row = tools_box.row(align=True)
        tool_row.operator("bevy.set_all_materials_embedded", text="Set all Materials Embedded")
        tool_row.operator("bevy.set_all_materials_unembedded", text="Set all Materials Unembedded")

        if not obj:
            export_box = layout.box()
            export_box.label(text="Export", icon="EXPORT")
            export_box.prop(settings, "export_scope")
            export_box.prop(settings, "apply_modifiers")
            export_box.operator("bevy.export_project", text="Export ADS", icon="EXPORT")
            return

        if obj.type == "MESH":
            physics_box = layout.box()
            physics_box.label(text="Entity", icon="PHYSICS")
            physics_box.prop(obj.bevy_toolkit_obj, "is_col")
            if is_collision_object(obj):
                physics_box.prop(obj.bevy_toolkit_obj, "col_shape")
                physics_box.prop(obj.bevy_toolkit_obj, "mass")
                physics_box.prop(obj.bevy_toolkit_obj, "is_static")
                physics_box.prop(obj.bevy_toolkit_obj, "friction")
                physics_box.prop(obj.bevy_toolkit_obj, "restitution")
                physics_box.prop(obj.bevy_toolkit_obj, "tags_csv")
            else:
                physics_box.prop(obj.bevy_toolkit_obj, "cast_shadows")
                physics_box.operator("bevy.gen_col", icon="MESH_CUBE", text="Generate COL_ proxy")

            paint_box = layout.box()
            paint_box.label(text="Vertex Masks", icon="VPAINT_HLT")

            paint_box.label(text="bevy_masks  (COLOR_0)")
            paint_box.label(text="R=NormSupp/L1  G=dirt  B=wet  A=palette")
            paint_box.operator("bevy.init_project", text="Initialize bevy_masks")
            row = paint_box.row(align=True)
            row.operator("bevy.set_paint", text="Norm/L1 (R)").mode = "NORMAL_SUPP"
            row.operator("bevy.set_paint", text="Dirt (G)").mode = "DIRT"
            row.operator("bevy.set_paint", text="Wet (B)").mode = "WET"
            row.operator("bevy.set_paint", text="Erase").mode = "ERASE"
            row = paint_box.row(align=True)
            row.prop(settings, "alpha_fill_value")
            row.operator("bevy.fill_alpha_mask", text="Fill A (palette)")

            paint_box.separator(factor=0.5)
            paint_box.label(text="bevy_masks2  (→ TEXCOORD_1)")
            paint_box.label(text="R=AO (1=none)  G=emissive")
            paint_box.operator("bevy.init_masks2", text="Initialize bevy_masks2")
            row = paint_box.row(align=True)
            row.operator("bevy.set_paint", text="AO (R)").mode = "AO"
            row.operator("bevy.set_paint", text="Emissive (G)").mode = "EMISSIVE"
            row.operator("bevy.set_paint", text="Erase").mode = "ERASE2"

        export_box = layout.box()
        export_box.label(text="Export", icon="EXPORT")
        export_box.prop(settings, "export_scope")
        export_box.prop(settings, "apply_modifiers")
        export_box.operator("bevy.export_project", text="Export ADS", icon="EXPORT")


classes = (
    BevyObjectProps,
    BevyMaterialProps,
    BevyExportProps,
    BEVY_OT_InitProject,
    BEVY_OT_InitMasks2,
    BEVY_OT_SetPaint,
    BEVY_OT_FillAlphaMask,
    BEVY_OT_GenerateCol,
    BEVY_OT_SetupNodes,
    BEVY_OT_ConvertToDrawableModel,
    BEVY_OT_ConvertToDrawable,
    BEVY_OT_CreateDrawable,
    BEVY_OT_CreateDrawableDictionary,
    BEVY_OT_CreateShaderMaterial,
    BEVY_OT_ConvertActiveMaterial,
    BEVY_OT_ConvertAllMaterials,
    BEVY_OT_SetAllTexturesEmbedded,
    BEVY_OT_RemoveAllEmbeddedTextures,
    BEVY_OT_SetAllMaterialsEmbedded,
    BEVY_OT_SetAllMaterialsUnembedded,
    BEVY_OT_Export,
    BEVY_OT_BrowseTexture,
    BEVY_PT_MaterialPanel,
    BEVY_PT_Panel,
)


def register():
    for cls in classes:
        bpy.utils.register_class(cls)
    bpy.types.Object.bevy_toolkit_obj = bpy.props.PointerProperty(type=BevyObjectProps)
    bpy.types.Material.bevy_toolkit = bpy.props.PointerProperty(type=BevyMaterialProps)
    bpy.types.Scene.bevy_toolkit_export = bpy.props.PointerProperty(type=BevyExportProps)


def unregister():
    del bpy.types.Scene.bevy_toolkit_export
    del bpy.types.Material.bevy_toolkit
    del bpy.types.Object.bevy_toolkit_obj
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    register()