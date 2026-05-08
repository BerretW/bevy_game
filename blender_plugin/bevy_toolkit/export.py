import bpy
from .constants import ADS_VERSION, OPTIONAL_TEXTURE_SLOTS
from .utils import toml_escape, format_float, bool_to_toml, parse_tags, first_non_empty, image_basename
from .mesh import is_collision_object
from .material import get_template_texture_specs, material_texture_value, build_material_inline


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

    return lines
