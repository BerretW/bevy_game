import bpy
from .constants import ATTR_NAME, ATTR_NAME2, UV_MASKS2_NAME


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
            item.color = (1.0, 0.0, 0.0, 1.0)
    color_attributes.active_color = layer


def encode_masks2_to_uv(mesh: bpy.types.Mesh):
    """Bake bevy_masks2 R,G channels into _ads_masks2_uv UV map (TEXCOORD_1).

    Returns the UV layer name, or None if masks2 attribute is absent.
    """
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


def decode_masks2_from_uv(mesh: bpy.types.Mesh) -> bool:
    """Decode _ads_masks2_uv (U=AO, V=emissive) back into bevy_masks2 vertex color.

    Removes the UV layer afterward. Returns True if the layer was found.
    """
    uv_layer = mesh.uv_layers.get(UV_MASKS2_NAME)
    if uv_layer is None:
        return False
    color_attributes = mesh.color_attributes
    if ATTR_NAME2 not in color_attributes:
        color_attributes.new(name=ATTR_NAME2, type="BYTE_COLOR", domain="CORNER")
    layer = color_attributes[ATTR_NAME2]
    src   = uv_layer.data
    dst   = layer.data
    for i in range(min(len(src), len(dst))):
        uv = src[i].uv
        dst[i].color = (uv[0], uv[1], 0.0, 1.0)  # U→R (AO), V→G (emissive)
    mesh.uv_layers.remove(mesh.uv_layers[UV_MASKS2_NAME])
    return True


def fix_imported_vertex_attributes(mesh: bpy.types.Mesh):
    """Post-import cleanup: ensure bevy_masks/bevy_masks2 names, decode from UV."""
    color_attributes = mesh.color_attributes

    # COLOR_0 → bevy_masks
    if ATTR_NAME not in color_attributes and len(color_attributes) > 0:
        color_attributes[0].name = ATTR_NAME

    # COLOR_1 → bevy_masks2 (Blender imports secondary color attrs as 'Color', 'Color.001', …)
    if ATTR_NAME2 not in color_attributes:
        for attr in color_attributes:
            if attr.name != ATTR_NAME:
                attr.name = ATTR_NAME2
                break

    if ATTR_NAME in color_attributes:
        color_attributes.active_color = color_attributes[ATTR_NAME]

    # If bevy_masks2 was also encoded into _ads_masks2_uv, decode it (authoritative)
    decode_masks2_from_uv(mesh)


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


def duplicate_collision_proxy(
    src_obj: bpy.types.Object, collection: bpy.types.Collection
):
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
