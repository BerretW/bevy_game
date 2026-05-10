import os
import bpy
from .constants import ADS_VERSION, OPTIONAL_TEXTURE_SLOTS, NON_GLTF_SLOTS
from .utils import toml_escape, format_float, bool_to_toml, parse_tags, first_non_empty, image_basename
from .mesh import is_collision_object
from .material import get_template_texture_specs, material_texture_value, build_material_inline


def _collision_shape_inline(obj, shape: str) -> str:
    # Use obj.bound_box (local mesh space, before obj.scale) so the exported
    # half_extents match the entity-local space that Bevy expects.
    # Coordinate mapping: Blender X → Bevy X, Blender Z (up) → Bevy Y, Blender Y → Bevy Z.
    bb = obj.bound_box
    xs = [v[0] for v in bb]
    ys = [v[1] for v in bb]  # Blender Y (forward) → Bevy Z
    zs = [v[2] for v in bb]  # Blender Z (up)     → Bevy Y

    hx = max(0.0001, max(xs) - min(xs)) * 0.5  # Bevy X
    hy = max(0.0001, max(zs) - min(zs)) * 0.5  # Bevy Y (Blender up)
    hz = max(0.0001, max(ys) - min(ys)) * 0.5  # Bevy Z (Blender forward)

    if shape == "BOX":
        return (
            f'half_extents = [{format_float(hx)}, {format_float(hy)}, {format_float(hz)}], '
        )
    if shape == "SPHERE":
        radius = max(hx, hy, hz)
        return f"radius = {format_float(radius)}, "
    if shape in ("CAPSULE", "CYLINDER"):
        radius = max(hx, hz)  # radius in Bevy XZ plane
        height = hy * 2.0     # full height in Bevy Y direction
        return f"radius = {format_float(radius)}, height = {format_float(height)}, "
    if shape in ("CONVEX", "MESH", "NAVMESH"):
        return (
            f'half_extents = [{format_float(hx)}, {format_float(hy)}, {format_float(hz)}], '
        )
    return ""


def gather_target_meshes(context, settings):
    scope = settings.export_scope
    if scope == "SELECTED":
        return [obj for obj in context.selected_objects if obj.type == "MESH"]
    if scope == "ACTIVE_COLLECTION":
        active_layer = context.view_layer.active_layer_collection
        if not active_layer:
            return []
        return [obj for obj in active_layer.collection.all_objects if obj.type == "MESH"]
    return [obj for obj in bpy.data.objects if obj.type == "MESH"]


def validate_export_consistency(target_meshes):
    warnings         = []
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
            if props.col_ladder and not props.col_climbable:
                warnings.append(
                    f"Collision '{obj.name}' has Ladder=true but Climbable=false (ladder will not be climbable)"
                )

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
                image, explicit_name, source = material_texture_value(props, slot_name)
                resolved_name = first_non_empty(explicit_name.strip(), image_basename(image))
                if resolved_name:
                    has_any_texture = True
                if source == "shared" and not resolved_name and slot_name not in OPTIONAL_TEXTURE_SLOTS:
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


def build_drawable_toml(asset_name: str, target_meshes, used_materials) -> list:
    """Return a list of TOML lines for the .drawable file."""
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
            props      = obj.bevy_toolkit_obj
            tags       = parse_tags(props.tags_csv)
            tags_array = "[" + ", ".join(f'"{toml_escape(tag)}"' for tag in tags) + "]"
            lock_translation = (
                f"[{bool_to_toml(props.lock_tx)}, {bool_to_toml(props.lock_ty)}, {bool_to_toml(props.lock_tz)}]"
            )
            lock_rotation = (
                f"[{bool_to_toml(props.lock_rx)}, {bool_to_toml(props.lock_ry)}, {bool_to_toml(props.lock_rz)}]"
            )
            shape = props.col_shape
            shape_inline = _collision_shape_inline(obj, shape)
            lines.append(
                key
                + " = { "
                + 'type = "COLLISION", '
                + f'shape = "{shape}", '
                + shape_inline
                + f"mass = {format_float(props.mass)}, "
                + f"is_static = {bool_to_toml(props.is_static)}, "
                + f"climbable = {bool_to_toml(props.col_climbable)}, "
                + f"ladder = {bool_to_toml(props.col_ladder)}, "
                + f'material = "{props.col_material}", '
                + f"friction = {format_float(props.friction)}, "
                + f"restitution = {format_float(props.restitution)}, "
                + f"tags = {tags_array}, "
                + f"lock_translation = {lock_translation}, "
                + f"lock_rotation = {lock_rotation}"
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

    return lines


def save_companion_textures(report_fn, export_dir: str, used_materials) -> int:
    """Save non-GLTF-embeddable embedded textures to disk next to the .drawable.

    Blender's GLTF exporter omits custom slots (ma, mb, palette, snow, shatter_map)
    when the material blend_method is OPAQUE, so they would be lost on re-import.
    Writing them alongside the drawable lets _find_image recover them from disk.
    """
    saved = 0
    for mat in used_materials:
        props = mat.bevy_toolkit
        for slot_name, _, _ in get_template_texture_specs(props.template):
            if slot_name not in NON_GLTF_SLOTS:
                continue
            img, explicit_name, source = material_texture_value(props, slot_name)
            if img is None or source != "embedded":
                continue
            base_name = first_non_empty(explicit_name.strip(), image_basename(img))
            if not base_name:
                continue
            raw = img.filepath_raw or ""
            ext = os.path.splitext(raw)[1] or os.path.splitext(img.name)[1] or ".png"
            save_path = os.path.join(export_dir, base_name + ext)
            orig_path = img.filepath_raw
            img.filepath_raw = save_path
            try:
                img.save()
                saved += 1
            except Exception as exc:
                report_fn({"WARNING"}, f"Could not save '{slot_name}' texture '{base_name}': {exc}")
            finally:
                img.filepath_raw = orig_path
    return saved
