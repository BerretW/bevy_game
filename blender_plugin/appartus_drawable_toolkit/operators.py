import os
import re
import bpy
from mathutils import Vector

from .constants import UV_MASKS2_NAME
from .mesh import (
    ensure_mask_attribute, ensure_masks2_attribute,
    fill_alpha_channel, fill_vertex_preset, duplicate_collision_proxy,
    is_collision_object, encode_masks2_to_uv, remove_temp_uv,
    fix_imported_vertex_attributes,
)
from .material import (
    create_bevy_node_tree, sync_material_from_nodes,
    set_material_texture_source, clear_embedded_images, _sync_texture_node,
)
from .export import gather_target_meshes, validate_export_consistency, build_drawable_toml, save_companion_textures
from .utils import parse_tags


def _get_active_material(context):
    """Return active material regardless of whether we're in Properties or 3D View."""
    mat = getattr(context, 'active_material', None)
    if mat is None and context.active_object:
        mat = context.active_object.active_material
    return mat


def _resolve_export_asset_name(context, objects):
    candidates = []
    active_object = context.active_object if context else None
    if active_object and active_object.type == 'MESH':
        candidates.append(active_object)
    candidates.extend(obj for obj in objects if obj and obj.type == 'MESH')

    for obj in candidates:
        props = getattr(obj, "bevy_toolkit_obj", None)
        if props is not None:
            export_name = props.export_name.strip()
            if export_name:
                return bpy.path.clean_name(export_name)
        if obj.name:
            return bpy.path.clean_name(obj.name)

    if context and context.scene:
        return bpy.path.clean_name(context.scene.name)
    return "Drawable"


def _resolve_anim_export_asset_name(context, armature=None, objects=None):
    if armature is not None and getattr(armature, 'name', ''):
        return bpy.path.clean_name(armature.name)
    return _resolve_export_asset_name(context, objects or [])


def _gather_anim_export_objects(context, use_selection):
    if use_selection:
        return [o for o in context.selected_objects if o.type in {"MESH", "ARMATURE"}]
    return [o for o in context.scene.objects if o.type in {"MESH", "ARMATURE"}]


def _prefill_export_name(obj):
    if not obj or obj.type != "MESH":
        return
    props = getattr(obj, "bevy_toolkit_obj", None)
    if props is None:
        return
    if not props.export_name.strip():
        props.export_name = bpy.path.clean_name(obj.name)


def _blender_pos_to_map(pos: Vector):
    return (float(pos.x), float(pos.z), float(-pos.y))


def _map_pos_to_blender(pos):
    return Vector((float(pos[0]), float(-pos[2]), float(pos[1])))


def _blender_rot_to_map_deg(rot_deg: Vector):
    return (float(rot_deg.x), float(rot_deg.z), float(-rot_deg.y))


def _map_rot_deg_to_blender(rot_deg):
    return Vector((float(rot_deg[0]), float(-rot_deg[2]), float(rot_deg[1])))


def _format_map_float(value: float) -> str:
    s = f"{float(value):.6g}"
    if "." not in s and "e" not in s and "E" not in s:
        s += ".0"
    return s


def _format_map_vec3(values):
    return "[" + ", ".join(_format_map_float(v) for v in values) + "]"


def _build_map_manifest_lines(instances):
    lines = ['version = "1.0"', ""]
    for item in instances:
        lines.append("[[instances]]")
        lines.append(f'id = "{item["id"]}"')
        lines.append(f'model = "{item["model"]}"')
        lines.append(f'position = {_format_map_vec3(item["position"])}')
        lines.append(f'rotation_deg = {_format_map_vec3(item["rotation_deg"])}')
        lines.append(f'scale = {_format_map_vec3(item["scale"])}')
        if item["navmesh_only"]:
            lines.append("navmesh_only = true")
        if item["tags"]:
            tags = ", ".join(f'"{t}"' for t in item["tags"])
            lines.append(f"tags = [{tags}]")
        lines.append("")
    return lines


_MIXAMO_BONE_MAP = {
    'Hips': 'pelvis',
    'Spine': 'spine_01',
    'Spine1': 'spine_02',
    'Spine2': 'spine_03',
    'Neck': 'neck',
    'Head': 'head',
    'LeftShoulder': 'clavicle_l',
    'LeftArm': 'upperarm_l',
    'LeftForeArm': 'forearm_l',
    'LeftHand': 'hand_l',
    'RightShoulder': 'clavicle_r',
    'RightArm': 'upperarm_r',
    'RightForeArm': 'forearm_r',
    'RightHand': 'hand_r',
    'LeftUpLeg': 'thigh_l',
    'LeftLeg': 'calf_l',
    'LeftFoot': 'foot_l',
    'LeftToeBase': 'toe_l',
    'RightUpLeg': 'thigh_r',
    'RightLeg': 'calf_r',
    'RightFoot': 'foot_r',
    'RightToeBase': 'toe_r',
}


def _find_target_armature(context, objects=None):
    active = getattr(context, 'active_object', None)
    if active and active.type == 'ARMATURE':
        return active

    if objects:
        for obj in objects:
            if obj.type == 'ARMATURE':
                return obj
        for obj in objects:
            parent = getattr(obj, 'parent', None)
            if parent and parent.type == 'ARMATURE':
                return parent
            for modifier in getattr(obj, 'modifiers', []):
                if modifier.type == 'ARMATURE' and getattr(modifier, 'object', None) and modifier.object.type == 'ARMATURE':
                    return modifier.object

    for obj in context.scene.objects:
        if obj.type == 'ARMATURE':
            return obj

    return None


def _normalize_mixamo_bone_name(name):
    base = name.split(':')[-1].strip()
    if base.startswith(('DEF_', 'IK_', 'SOC_', 'MEC_')):
        return base

    mapped = _MIXAMO_BONE_MAP.get(base)
    if mapped:
        return f'DEF_{mapped}'

    finger_match = re.match(r'^(Left|Right)Hand(Thumb|Index|Middle|Ring|Pinky)(\d+)$', base)
    if finger_match:
        side = 'l' if finger_match.group(1) == 'Left' else 'r'
        finger = finger_match.group(2).lower()
        index = int(finger_match.group(3))
        return f'DEF_{finger}_{index:02d}_{side}'

    side_match = re.match(r'^(Left|Right)(.+)$', base)
    if side_match:
        side = 'l' if side_match.group(1) == 'Left' else 'r'
        core = re.sub(r'(?<!^)([A-Z])', r'_\1', side_match.group(2)).lower()
        core = re.sub(r'[^a-z0-9_]+', '_', core)
        return f'DEF_{core}_{side}'

    core = re.sub(r'(?<!^)([A-Z])', r'_\1', base).lower()
    core = re.sub(r'[^a-z0-9_]+', '_', core)
    return f'DEF_{core}'


def _rename_mixamo_armature(armature_obj):
    rename_map = {}
    for bone in armature_obj.data.bones:
        old_name = bone.name
        new_name = _normalize_mixamo_bone_name(old_name)
        if new_name == old_name:
            continue
        rename_map[old_name] = new_name
        stripped = old_name.split(':')[-1]
        rename_map[stripped] = new_name
        bone.name = new_name

    if not rename_map:
        return rename_map

    for obj in bpy.data.objects:
        if obj.type != 'MESH':
            continue
        uses_armature = False
        for modifier in getattr(obj, 'modifiers', []):
            if modifier.type == 'ARMATURE' and getattr(modifier, 'object', None) == armature_obj:
                uses_armature = True
                break
        if not uses_armature and obj.parent != armature_obj:
            continue

        for vertex_group in getattr(obj, 'vertex_groups', []):
            if vertex_group.name in rename_map:
                vertex_group.name = rename_map[vertex_group.name]

    return rename_map


def _push_action_to_nla(armature_obj, action, strip_name=None):
    if action is None:
        return None

    armature_obj.animation_data_create()
    track = armature_obj.animation_data.nla_tracks.new()
    track.name = strip_name or action.name

    frame_start = int(round(float(action.frame_range[0])))
    frame_end = int(round(float(action.frame_range[1])))
    if frame_end <= frame_start:
        frame_end = frame_start + 1

    strip = track.strips.new(strip_name or action.name, frame_start, action)
    try:
        strip.action_frame_start = frame_start
        strip.action_frame_end = frame_end
    except Exception:
        pass
    try:
        strip.frame_start = frame_start
        strip.frame_end = frame_end
    except Exception:
        pass
    return strip


def _sanitize_anim_dict_name(name):
    raw = (name or "").strip()
    if not raw:
        return "default"
    return bpy.path.clean_name(raw)


def _get_or_create_anim_dict(settings, dict_name):
    dict_name = _sanitize_anim_dict_name(dict_name)
    for item in settings.animation_dictionaries:
        if item.name == dict_name:
            return item
    item = settings.animation_dictionaries.add()
    item.name = dict_name
    settings.active_anim_dict_index = max(0, len(settings.animation_dictionaries) - 1)
    return item


def _dict_has_clip(dict_item, clip_name):
    for clip in dict_item.clips:
        if clip.clip_name == clip_name:
            return True
    return False


def _add_clip_to_dict(settings, dict_name, clip_name):
    clip_name = (clip_name or "").strip()
    if not clip_name:
        return False
    dict_item = _get_or_create_anim_dict(settings, dict_name)
    if _dict_has_clip(dict_item, clip_name):
        return False
    clip = dict_item.clips.add()
    clip.clip_name = clip_name
    dict_item.active_clip_index = max(0, len(dict_item.clips) - 1)
    return True


def _sanitize_ik_chain_name(name):
    raw = (name or "").strip()
    if not raw:
        return "ik_chain"
    return bpy.path.clean_name(raw)


def _get_or_create_ik_chain(settings, chain_name):
    chain_name = _sanitize_ik_chain_name(chain_name)
    for item in settings.ik_chains:
        if item.name == chain_name:
            return item
    item = settings.ik_chains.add()
    item.name = chain_name
    settings.active_ik_chain_index = max(0, len(settings.ik_chains) - 1)
    return item


def _collect_armature_bone_names(armature_obj):
    if armature_obj is None or armature_obj.type != 'ARMATURE':
        return set()
    return {bone.name for bone in armature_obj.data.bones}


def _default_biped_ik_specs():
    return [
        {
            'name': 'leg_l',
            'parent_bone_name': 'DEF_thigh_l',
            'ik_target_name': 'IK_foot_l',
            'effector_bone_name': 'DEF_foot_l',
            'pole_bone_name': 'IK_knee_l',
            'chain_length': 1.0,
            'solver_iterations': 2,
            'min_knee_angle': 5.0,
            'max_knee_angle': 175.0,
        },
        {
            'name': 'leg_r',
            'parent_bone_name': 'DEF_thigh_r',
            'ik_target_name': 'IK_foot_r',
            'effector_bone_name': 'DEF_foot_r',
            'pole_bone_name': 'IK_knee_r',
            'chain_length': 1.0,
            'solver_iterations': 2,
            'min_knee_angle': 5.0,
            'max_knee_angle': 175.0,
        },
    ]


def _collect_ik_export_data(settings):
    chains = []
    for chain in settings.ik_chains:
        chains.append({
            'name': _sanitize_ik_chain_name(chain.name),
            'enabled': bool(chain.enabled),
            'parent_bone_name': (chain.parent_bone_name or '').strip(),
            'ik_target_name': (chain.ik_target_name or '').strip(),
            'effector_bone_name': (chain.effector_bone_name or '').strip(),
            'pole_bone_name': (chain.pole_bone_name or '').strip(),
            'chain_length': float(chain.chain_length),
            'solver_iterations': int(chain.solver_iterations),
            'min_knee_angle': float(chain.min_knee_angle),
            'max_knee_angle': float(chain.max_knee_angle),
        })
    return chains


def _write_ik_sidecar(directory, asset_name, armature_obj, settings):
    if not settings.ik_export_sidecar:
        return None, 0

    chains = _collect_ik_export_data(settings)
    if not chains:
        return None, 0

    sidecar_path = os.path.join(directory, f"{asset_name}.ik.toml")
    lines = [
        'version = "1.0"',
        f'asset = "{asset_name}"',
    ]
    if armature_obj is not None:
        lines.append(f'armature = "{armature_obj.name}"')
    lines.append('')

    for chain in chains:
        lines.append('[[chains]]')
        lines.append(f'name = "{chain["name"]}"')
        lines.append(f'enabled = {str(chain["enabled"]).lower()}')
        lines.append(f'parent_bone = "{chain["parent_bone_name"]}"')
        lines.append(f'ik_target = "{chain["ik_target_name"]}"')
        lines.append(f'effector_bone = "{chain["effector_bone_name"]}"')
        if chain['pole_bone_name']:
            lines.append(f'pole_bone = "{chain["pole_bone_name"]}"')
        lines.append(f'chain_length = {chain["chain_length"]:.6g}')
        lines.append(f'solver_iterations = {chain["solver_iterations"]}')
        lines.append(f'min_knee_angle = {chain["min_knee_angle"]:.6g}')
        lines.append(f'max_knee_angle = {chain["max_knee_angle"]:.6g}')
        lines.append('')

    with open(sidecar_path, 'w', encoding='utf-8') as handle:
        handle.write('\n'.join(lines))

    return sidecar_path, len(chains)


class BEVY_OT_InitProject(bpy.types.Operator):
    bl_idname     = "bevy.init_project"
    bl_label      = "Initialize Masks"
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
    bl_idname     = "bevy.init_masks2"
    bl_label      = "Initialize bevy_masks2"
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
    bl_label  = "Set Paint Mask"
    mode: bpy.props.StringProperty()

    def execute(self, context):
        obj = context.active_object
        if not obj or obj.type != "MESH":
            self.report({"WARNING"}, "Active object must be a mesh")
            return {"CANCELLED"}

        brush = context.tool_settings.vertex_paint.brush

        if self.mode in ("INIT2", "AO", "EMISSIVE", "ERASE2"):
            ensure_masks2_attribute(obj.data)
            if context.object.mode != "VERTEX_PAINT":
                bpy.ops.object.mode_set(mode="VERTEX_PAINT")
            if self.mode == "AO":
                brush.color = (1.0, 0.0, 0.0)
            elif self.mode == "EMISSIVE":
                brush.color = (0.0, 1.0, 0.0)
            elif self.mode == "ERASE2":
                brush.color = (1.0, 0.0, 0.0)
            return {"FINISHED"}

        ensure_mask_attribute(obj.data)
        if context.object.mode != "VERTEX_PAINT":
            bpy.ops.object.mode_set(mode="VERTEX_PAINT")

        if self.mode == "NORMAL_SUPP":
            brush.color = (1.0, 0.0, 0.0)
        elif self.mode == "L1":
            brush.color = (1.0, 0.0, 0.0)
        elif self.mode == "DIRT":
            brush.color = (0.0, 1.0, 0.0)
        elif self.mode == "BLOOD":
            brush.color = (0.0, 1.0, 0.0)
        elif self.mode == "WET":
            brush.color = (0.0, 0.0, 1.0)
        elif self.mode == "ERASE":
            brush.color = (0.0, 0.0, 0.0)
        else:
            self.report({"WARNING"}, f"Unknown paint mode: {self.mode}")
            return {"CANCELLED"}
        return {"FINISHED"}


class BEVY_OT_FillAlphaMask(bpy.types.Operator):
    bl_idname     = "bevy.fill_alpha_mask"
    bl_label      = "Fill Tint Alpha"
    bl_description = "Write a constant alpha value to bevy_masks for selected meshes"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        targets  = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets and context.active_object and context.active_object.type == "MESH":
            targets = [context.active_object]
        if not targets:
            self.report({"WARNING"}, "No mesh object selected")
            return {"CANCELLED"}
        for obj in targets:
            fill_alpha_channel(obj.data, settings.alpha_fill_value)
        self.report({"INFO"}, f"Filled alpha for {len(targets)} mesh object(s)")
        return {"FINISHED"}


class BEVY_OT_ApplyVertexPreset(bpy.types.Operator):
    bl_idname     = "bevy.apply_vertex_preset"
    bl_label      = "Apply Vertex Preset"
    bl_description = "Fill bevy_masks on selected meshes with preset RGBA values (R=layer, G=dirt, B=wet, A=palette)"

    preset: bpy.props.StringProperty()

    def execute(self, context):
        from .constants import VERTEX_PRESETS
        if self.preset not in VERTEX_PRESETS:
            self.report({"WARNING"}, f"Unknown preset: {self.preset}")
            return {"CANCELLED"}
        targets = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets and context.active_object and context.active_object.type == "MESH":
            targets = [context.active_object]
        if not targets:
            self.report({"WARNING"}, "No mesh object selected")
            return {"CANCELLED"}
        rgba = VERTEX_PRESETS[self.preset]
        for obj in targets:
            fill_vertex_preset(obj.data, rgba)
        self.report({"INFO"}, f"Applied '{self.preset}' preset to {len(targets)} mesh object(s)")
        return {"FINISHED"}


class BEVY_OT_GenerateCol(bpy.types.Operator):
    bl_idname = "bevy.gen_col"
    bl_label  = "Create Collision Proxy"

    def execute(self, context):
        src = context.active_object
        if not src or src.type != "MESH":
            self.report({"WARNING"}, "Active object must be a mesh")
            return {"CANCELLED"}
        new_obj = src.copy()
        new_obj.data = src.data.copy()
        new_obj.name = f"COL_{src.name}"
        new_obj.bevy_toolkit_obj.is_col    = True
        new_obj.bevy_toolkit_obj.col_shape = "CONVEX"
        new_obj.bevy_toolkit_obj.col_climbable = False
        new_obj.bevy_toolkit_obj.col_ladder = False
        new_obj.bevy_toolkit_obj.col_material = "CONCRETE"
        new_obj.bevy_toolkit_obj.lock_tx = False
        new_obj.bevy_toolkit_obj.lock_ty = False
        new_obj.bevy_toolkit_obj.lock_tz = False
        new_obj.bevy_toolkit_obj.lock_rx = False
        new_obj.bevy_toolkit_obj.lock_ry = False
        new_obj.bevy_toolkit_obj.lock_rz = False
        new_obj.display_type = "WIRE"
        new_obj.hide_render  = True
        context.collection.objects.link(new_obj)
        self.report({"INFO"}, f"Collision proxy created: {new_obj.name}")
        return {"FINISHED"}


class BEVY_OT_SetupNodes(bpy.types.Operator):
    bl_idname = "bevy.setup_nodes"
    bl_label  = "Setup Preview Nodes"

    def execute(self, context):
        mat = _get_active_material(context)
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        create_bevy_node_tree(mat)
        return {"FINISHED"}


class BEVY_OT_ConvertToDrawableModel(bpy.types.Operator):
    bl_idname = "bevy.convert_to_drawable_model"
    bl_label  = "Convert to Drawable Model"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        targets  = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets:
            self.report({"WARNING"}, "Select at least one mesh object")
            return {"CANCELLED"}

        active     = context.active_object if context.active_object in targets else targets[0]
        model_name = f"{active.name}.model"
        model_obj  = bpy.data.objects.get(model_name)
        if not model_obj:
            model_obj = bpy.data.objects.new(model_name, None)
            context.collection.objects.link(model_obj)

        if settings.center_to_selection:
            center = sum((obj.location for obj in targets), Vector((0.0, 0.0, 0.0))) / len(targets)
            model_obj.location = center
        else:
            model_obj.location = active.location.copy()

        for obj in targets:
            _prefill_export_name(obj)

        for obj in targets:
            world_matrix = obj.matrix_world.copy()
            obj.parent   = model_obj
            obj.matrix_world = world_matrix

        self.report({"INFO"}, f"Created drawable model root '{model_obj.name}'")
        return {"FINISHED"}


class BEVY_OT_ConvertToDrawable(bpy.types.Operator):
    bl_idname = "bevy.convert_to_drawable"
    bl_label  = "Convert to Drawable"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        targets  = [obj for obj in context.selected_objects if obj.type == "MESH"]
        if not targets:
            self.report({"WARNING"}, "Select at least one mesh object")
            return {"CANCELLED"}

        created_col         = 0
        mapped_textures     = 0
        created_proxy_names = []

        for obj in targets:
            if obj.name.startswith("COL_"):
                obj.bevy_toolkit_obj.is_col = True
            else:
                obj.bevy_toolkit_obj.is_col = False
                _prefill_export_name(obj)
                ensure_mask_attribute(obj.data)
                ensure_masks2_attribute(obj.data)
                if settings.auto_embed_collision:
                    proxy_obj, created = duplicate_collision_proxy(obj, context.collection)
                    created_col += 1 if created else 0
                    created_proxy_names.append(proxy_obj.name)

            for slot in obj.material_slots:
                if slot.material:
                    mapped_textures += sync_material_from_nodes(slot.material, only_if_empty=True)

        for proxy_name in created_proxy_names:
            proxy_obj = bpy.data.objects.get(proxy_name)
            if proxy_obj:
                proxy_obj.bevy_toolkit_obj.is_col = True
                _prefill_export_name(proxy_obj)

        self.report(
            {"INFO"},
            f"Converted {len(targets)} object(s); created {created_col} collision proxy object(s); mapped {mapped_textures} texture slot(s)",
        )
        return {"FINISHED"}


class BEVY_OT_CreateDrawable(bpy.types.Operator):
    bl_idname = "bevy.create_drawable"
    bl_label  = "Create Drawable"

    def execute(self, context):
        if "FINISHED" not in bpy.ops.bevy.convert_to_drawable_model():
            return {"CANCELLED"}
        if "FINISHED" not in bpy.ops.bevy.convert_to_drawable():
            return {"CANCELLED"}
        self.report({"INFO"}, "Drawable workflow complete")
        return {"FINISHED"}


class BEVY_OT_CreateDrawableDictionary(bpy.types.Operator):
    bl_idname = "bevy.create_drawable_dictionary"
    bl_label  = "Create Drawable Dictionary"

    def execute(self, context):
        settings  = context.scene.bevy_toolkit_export
        dict_name = settings.drawable_dict_name.strip() or "DrawableDictionary"
        collection = bpy.data.collections.get(dict_name)
        created   = False
        if collection is None:
            collection = bpy.data.collections.new(dict_name)
            context.scene.collection.children.link(collection)
            created = True

        roots = [
            obj for obj in context.selected_objects
            if obj.type == "EMPTY" and obj.name.endswith(".model")
        ]
        for root in roots:
            if root not in collection.objects:
                collection.objects.link(root)

        verb = "Created" if created else "Updated"
        self.report({"INFO"}, f"{verb} drawable dictionary '{dict_name}'")
        return {"FINISHED"}


class BEVY_OT_CreateShaderMaterial(bpy.types.Operator):
    bl_idname = "bevy.create_shader_material"
    bl_label  = "Create Shader Material"

    def execute(self, context):
        active = context.active_object
        if not active or active.type != "MESH":
            self.report({"WARNING"}, "Active object must be a mesh")
            return {"CANCELLED"}

        base_name = "standard"
        idx       = 1
        mat_name  = base_name
        while mat_name in bpy.data.materials:
            idx     += 1
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
    bl_label  = "Convert Active Material"

    def execute(self, context):
        mat = _get_active_material(context)
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        mapped = sync_material_from_nodes(mat, only_if_empty=False)
        create_bevy_node_tree(mat)
        self.report({"INFO"}, f"Mapped {mapped} texture slot(s) and rebuilt shader graph")
        return {"FINISHED"}


class BEVY_OT_ConvertAllMaterials(bpy.types.Operator):
    bl_idname = "bevy.convert_all_materials"
    bl_label  = "Convert All Materials"

    def execute(self, context):
        total = 0
        mats  = [mat for mat in bpy.data.materials if hasattr(mat, "bevy_toolkit")]
        for mat in mats:
            total += sync_material_from_nodes(mat, only_if_empty=False)
            create_bevy_node_tree(mat)
        self.report({"INFO"}, f"Mapped {total} texture slot(s), rebuilt {len(mats)} shader graph(s)")
        return {"FINISHED"}


class BEVY_OT_SetAllTexturesEmbedded(bpy.types.Operator):
    bl_idname = "bevy.set_all_textures_embedded"
    bl_label  = "Set all Textures Embedded"

    def execute(self, context):
        mat = _get_active_material(context)
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        sync_material_from_nodes(mat, only_if_empty=True)
        set_material_texture_source(mat, "embedded")
        self.report({"INFO"}, f"Material '{mat.name}' texture sources set to embedded")
        return {"FINISHED"}


class BEVY_OT_RemoveAllEmbeddedTextures(bpy.types.Operator):
    bl_idname = "bevy.remove_all_embedded_textures"
    bl_label  = "Remove all Embedded Textures"

    def execute(self, context):
        mat = _get_active_material(context)
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        cleared = clear_embedded_images(mat)
        self.report({"INFO"}, f"Removed {cleared} embedded image assignment(s) from '{mat.name}'")
        return {"FINISHED"}


class BEVY_OT_SetAllMaterialsEmbedded(bpy.types.Operator):
    bl_idname = "bevy.set_all_materials_embedded"
    bl_label  = "Set all Materials Embedded"

    def execute(self, context):
        mats = [mat for mat in bpy.data.materials if hasattr(mat, "bevy_toolkit")]
        for mat in mats:
            sync_material_from_nodes(mat, only_if_empty=True)
            set_material_texture_source(mat, "embedded")
        self.report({"INFO"}, f"Set {len(mats)} material(s) to embedded source")
        return {"FINISHED"}


class BEVY_OT_SetAllMaterialsUnembedded(bpy.types.Operator):
    bl_idname = "bevy.set_all_materials_unembedded"
    bl_label  = "Set all Materials Unembedded"

    def execute(self, context):
        mats = [mat for mat in bpy.data.materials if hasattr(mat, "bevy_toolkit")]
        for mat in mats:
            set_material_texture_source(mat, "shared")
        self.report({"INFO"}, f"Set {len(mats)} material(s) to shared source")
        return {"FINISHED"}


class BEVY_OT_Export(bpy.types.Operator):
    bl_idname = "bevy.export_project"
    bl_label  = "Export ADS (GLB + Drawable)"

    def execute(self, context):
        if not bpy.data.filepath:
            self.report({"WARNING"}, "Save .blend file first")
            return {"CANCELLED"}

        settings   = context.scene.bevy_toolkit_export

        target_meshes = gather_target_meshes(context, settings)
        if not target_meshes:
            self.report({"WARNING"}, "No mesh objects match selected export scope")
            return {"CANCELLED"}

        asset_name = _resolve_export_asset_name(context, target_meshes)
        export_dir = os.path.dirname(bpy.data.filepath)
        glb_path   = os.path.join(export_dir, f"{asset_name}.glb")
        toml_path  = os.path.join(export_dir, f"{asset_name}.drawable")

        warnings = validate_export_consistency(target_meshes)
        for warning_text in warnings[:8]:
            self.report({"WARNING"}, warning_text)
        if len(warnings) > 8:
            self.report({"WARNING"}, f"... and {len(warnings) - 8} more consistency warning(s)")

        # Encode bevy_masks2 into a temporary UV channel before GLB export.
        encoded_uv_meshes = []
        for obj in target_meshes:
            if not is_collision_object(obj):
                ensure_mask_attribute(obj.data)
                uv_name = encode_masks2_to_uv(obj.data)
                if uv_name:
                    idx = list(obj.data.uv_layers.keys()).index(uv_name)
                    if idx != 1:
                        self.report(
                            {"WARNING"},
                            f"'{obj.name}': {UV_MASKS2_NAME} is at UV index {idx}, expected 1 "
                            "(TEXCOORD_1). Add exactly one primary UV map before exporting.",
                        )
                    encoded_uv_meshes.append(obj.data)

        original_active    = context.view_layer.objects.active
        original_selection = list(context.selected_objects)
        export_selected    = False
        try:
            if settings.export_scope != "ALL":
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

        # Collect unique materials used by the target meshes.
        used_materials      = []
        seen_material_names = set()
        for obj in target_meshes:
            for slot in obj.material_slots:
                mat = slot.material
                if mat and mat.name not in seen_material_names:
                    used_materials.append(mat)
                    seen_material_names.add(mat.name)

        lines = build_drawable_toml(asset_name, target_meshes, used_materials)
        with open(toml_path, "w", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")

        armature = _find_target_armature(context, target_meshes)
        try:
            ik_path, ik_count = _write_ik_sidecar(export_dir, asset_name, armature, settings)
            if ik_path and ik_count > 0:
                self.report({'INFO'}, f"IK sidecar: {ik_count} chain(s) -> {os.path.basename(ik_path)}")
        except Exception as e:
            self.report({'WARNING'}, f"IK sidecar export selhal: {e}")

        save_companion_textures(self.report, os.path.dirname(toml_path), used_materials)

        self.report({"INFO"}, f"Exported: {os.path.basename(glb_path)} and {os.path.basename(toml_path)}")
        return {"FINISHED"}


_IMAGE_EXTS = (".png", ".jpg", ".jpeg", ".tga", ".dds", ".tif", ".tiff", ".exr", ".bmp")


def _find_image(name: str, search_dir: str = None):
    """Return a Blender image matching `name` (no extension), or None.

    Search order:
      1. Exact name in bpy.data.images
      2. Name + common extension in bpy.data.images
      3. Strip extension from all loaded images and compare base names
      4. Search disk: search_dir, its subdirectories, and its parent directory
    """
    if not name:
        return None

    img = bpy.data.images.get(name)
    if img:
        return img

    for ext in _IMAGE_EXTS:
        img = bpy.data.images.get(name + ext)
        if img:
            return img

    for img in bpy.data.images:
        if os.path.splitext(img.name)[0] == name:
            return img

    if search_dir:
        search_dirs = [search_dir]
        # common subdirectory names used for texture libraries
        for sub in ("textures", "texture", "tex", "maps", "materials"):
            d = os.path.join(search_dir, sub)
            if os.path.isdir(d):
                search_dirs.append(d)
        # also try one level up (e.g. models/ lives next to textures/)
        parent = os.path.dirname(search_dir)
        if parent and parent != search_dir:
            search_dirs.append(parent)
            for sub in ("textures", "texture", "tex"):
                d = os.path.join(parent, sub)
                if os.path.isdir(d):
                    search_dirs.append(d)

        for d in search_dirs:
            for ext in _IMAGE_EXTS:
                path = os.path.join(d, name + ext)
                if os.path.isfile(path):
                    return bpy.data.images.load(path, check_existing=True)

    return None


def _apply_material_from_drawable(mat, mat_data, search_dir=None):
    props = mat.bevy_toolkit
    props.template = mat_data.get("template", "standard_pbr")

    for slot_name, tex_info in mat_data.get("textures", {}).items():
        if hasattr(props, f"{slot_name}_name"):
            name   = tex_info.get("name", "")
            source = tex_info.get("source", "shared")
            setattr(props, f"{slot_name}_name",     name)
            setattr(props, f"{slot_name}_embedded", source == "embedded")
            img = _find_image(name, search_dir)
            if img is not None:
                setattr(props, f"{slot_name}_img", img)

    p = mat_data.get("params", {})
    tint = p.get("tint", [1.0, 1.0, 1.0, 1.0])
    props.tint            = tint[:4]
    props.tiling          = float(p.get("tiling",          1.0))
    props.l0_tiling       = float(p.get("l0_tiling",       1.0))
    props.l1_tiling       = float(p.get("l1_tiling",       1.0))
    props.porosity        = float(p.get("porosity",        0.0))
    props.wetness         = float(p.get("wetness",         0.0))
    props.snow_level      = float(p.get("snow_level",      0.0))
    props.dirt_level      = float(p.get("dirt_level",      0.0))
    props.opacity_mode    = p.get("opacity_mode", "OPAQUE")
    props.alpha_threshold = float(p.get("alpha_threshold", 0.5))

    create_bevy_node_tree(mat)


def _apply_entity_from_drawable(obj, ent_data):
    obj_props = obj.bevy_toolkit_obj
    if ent_data.get("type") == "COLLISION":
        def _bool3(value, default=(False, False, False)):
            if isinstance(value, (list, tuple)) and len(value) == 3:
                return (bool(value[0]), bool(value[1]), bool(value[2]))
            return default

        shape = ent_data.get("shape", "CONVEX")
        obj_props.is_col      = True
        obj_props.col_shape   = shape
        obj_props.mass        = float(ent_data.get("mass",        1.0))
        obj_props.is_static   = bool(ent_data.get("is_static",   False))
        obj_props.col_climbable = bool(ent_data.get("climbable", False))
        obj_props.col_ladder    = bool(ent_data.get("ladder",    False))
        if "material" in ent_data:
            obj_props.col_material = ent_data["material"]
        obj_props.friction    = float(ent_data.get("friction",    0.6))
        obj_props.restitution = float(ent_data.get("restitution", 0.2))
        obj_props.tags_csv    = ",".join(ent_data.get("tags",[]))
        lock_t = _bool3(ent_data.get("lock_translation", (False, False, False)))
        lock_r = _bool3(ent_data.get("lock_rotation", (False, False, False)))
        obj_props.lock_tx, obj_props.lock_ty, obj_props.lock_tz = lock_t
        obj_props.lock_rx, obj_props.lock_ry, obj_props.lock_rz = lock_r
        
        # Aby collider v Blenderu neblokoval výhled, nastavíme zobrazení na 'WIRE'
        obj.display_type      = "WIRE"
        obj.hide_render       = True

        # Vygenerujeme vizuální "proxy" mesh pro primitiva, co z ADM přišla prázdná
        if obj.type == 'MESH' and obj.data and len(obj.data.vertices) == 0:
            import bmesh
            bm = bmesh.new()
            
            hx, hy, hz = 0.5, 0.5, 0.5
            if "half_extents" in ent_data:
                he = ent_data["half_extents"]
                hx, hy, hz = he[0], he[1], he[2]
                
            radius = float(ent_data.get("radius", 0.5))
            height = float(ent_data.get("height", 2.0))
            
            if shape in ("BOX", "CONVEX", "MESH", "NAVMESH"):
                bmesh.ops.create_cube(bm, size=2.0)
                for v in bm.verts:
                    # Převod rozměrů Bevy souřadnic (Y-up) zpět na Blender rozměry (Z-up)
                    v.co.x *= hx
                    v.co.y *= hz  
                    v.co.z *= hy  
            elif shape == "SPHERE":
                bmesh.ops.create_uvsphere(bm, u_segments=16, v_segments=8, radius=radius)
            elif shape in ("CYLINDER", "CAPSULE"):
                bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=16, radius1=radius, radius2=radius, depth=height)
                
            bm.to_mesh(obj.data)
            bm.free()
    else:
        obj_props.is_col       = False
        obj_props.cast_shadows = bool(ent_data.get("cast_shadows", True))


class BEVY_OT_ImportDrawable(bpy.types.Operator):
    bl_idname     = "bevy.import_drawable"
    bl_label      = "Import Drawable"
    bl_description = "Import a .drawable manifest and its matching .glb"

    filepath:    bpy.props.StringProperty(subtype='FILE_PATH')
    filter_glob: bpy.props.StringProperty(default="*.drawable", options={'HIDDEN'})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        import re
        try:
            import tomllib
        except ImportError:
            self.report({'ERROR'}, "tomllib not available — requires Python 3.11 / Blender 4.0+")
            return {'CANCELLED'}

        if not os.path.isfile(self.filepath):
            self.report({'ERROR'}, f"File not found: {self.filepath}")
            return {'CANCELLED'}

        with open(self.filepath, "rb") as fh:
            data = tomllib.load(fh)

        base       = os.path.splitext(self.filepath)[0]
        adm_path   = base + ".adm"
        glb_path   = base + ".glb"
        search_dir = os.path.dirname(self.filepath)

        entities  = data.get("entities",  {})
        materials = data.get("materials", {})

        def base_name(n):
            return re.sub(r'\.\d+$', '', n)

        # --- ADM import ---
        if os.path.isfile(adm_path):
            from .adm_import import import_adm
            try:
                new_objects = import_adm(adm_path)
            except Exception as e:
                self.report({'ERROR'}, f"ADM import selhal: {e}")
                return {'CANCELLED'}

            new_mat_names = {slot.material.name
                             for obj in new_objects if obj.type == 'MESH'
                             for slot in obj.material_slots if slot.material}

            mats_applied = 0
            for mname in new_mat_names:
                mat = bpy.data.materials.get(mname)
                if not mat:
                    continue
                mat_data = materials.get(mname) or materials.get(base_name(mname))
                if not mat_data:
                    continue
                _apply_material_from_drawable(mat, mat_data, search_dir)
                mats_applied += 1

            objs_applied = 0
            for obj in new_objects:
                if obj.type != 'MESH':
                    continue
                fix_imported_vertex_attributes(obj.data)
                ent_data = entities.get(obj.name) or entities.get(base_name(obj.name))
                if ent_data:
                    _apply_entity_from_drawable(obj, ent_data)
                    objs_applied += 1

            self.report({'INFO'},
                f"ADM import: {len(new_objects)} objektů, {mats_applied} materiálů, "
                f"{objs_applied} entit aplikováno ← {os.path.basename(adm_path)}")
            return {'FINISHED'}

        # --- GLB fallback ---
        if not os.path.isfile(glb_path):
            self.report({'ERROR'}, f"Ani .adm ani .glb nenalezeno vedle {os.path.basename(self.filepath)}")
            return {'CANCELLED'}

        before_objects   = set(bpy.data.objects.keys())
        before_materials = set(bpy.data.materials.keys())

        result = bpy.ops.import_scene.gltf(filepath=glb_path)
        if 'FINISHED' not in result:
            self.report({'ERROR'}, "GLB import failed")
            return {'CANCELLED'}

        new_obj_names = set(bpy.data.objects.keys())   - before_objects
        new_mat_names = set(bpy.data.materials.keys()) - before_materials

        mats_applied = 0
        for bname in new_mat_names:
            mat = bpy.data.materials.get(bname)
            if not mat:
                continue
            mat_data = materials.get(bname) or materials.get(base_name(bname))
            if not mat_data:
                continue
            _apply_material_from_drawable(mat, mat_data, search_dir)
            mats_applied += 1

        objs_applied = 0
        for bname in new_obj_names:
            obj = bpy.data.objects.get(bname)
            if not obj or obj.type != "MESH":
                continue
            fix_imported_vertex_attributes(obj.data)
            ent_data = entities.get(bname) or entities.get(base_name(bname))
            if ent_data:
                _apply_entity_from_drawable(obj, ent_data)
                objs_applied += 1

        self.report({'INFO'},
            f"GLB import: {mats_applied} materiálů, {objs_applied} entit ← {os.path.basename(glb_path)}")
        return {'FINISHED'}


class BEVY_OT_AnimDictAdd(bpy.types.Operator):
    bl_idname = "ads.anim_dict_add"
    bl_label = "Add Animation Dictionary"
    bl_description = "Create a new animation dictionary on current scene"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        dict_name = _sanitize_anim_dict_name(settings.new_anim_dict_name)
        _get_or_create_anim_dict(settings, dict_name)
        self.report({'INFO'}, f"Animation dictionary '{dict_name}' ready")
        return {'FINISHED'}


class BEVY_OT_AnimDictRemove(bpy.types.Operator):
    bl_idname = "ads.anim_dict_remove"
    bl_label = "Remove Animation Dictionary"
    bl_description = "Remove active animation dictionary"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.animation_dictionaries:
            self.report({'WARNING'}, "No animation dictionary to remove")
            return {'CANCELLED'}
        idx = min(max(0, settings.active_anim_dict_index), len(settings.animation_dictionaries) - 1)
        name = settings.animation_dictionaries[idx].name
        settings.animation_dictionaries.remove(idx)
        settings.active_anim_dict_index = max(0, min(idx, len(settings.animation_dictionaries) - 1))
        self.report({'INFO'}, f"Removed animation dictionary '{name}'")
        return {'FINISHED'}


class BEVY_OT_AnimDictPrev(bpy.types.Operator):
    bl_idname = "ads.anim_dict_prev"
    bl_label = "Previous Animation Dictionary"
    bl_description = "Select previous animation dictionary"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.animation_dictionaries:
            self.report({'WARNING'}, "No animation dictionary")
            return {'CANCELLED'}
        settings.active_anim_dict_index = max(0, min(settings.active_anim_dict_index - 1, len(settings.animation_dictionaries) - 1))
        return {'FINISHED'}


class BEVY_OT_AnimDictNext(bpy.types.Operator):
    bl_idname = "ads.anim_dict_next"
    bl_label = "Next Animation Dictionary"
    bl_description = "Select next animation dictionary"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.animation_dictionaries:
            self.report({'WARNING'}, "No animation dictionary")
            return {'CANCELLED'}
        settings.active_anim_dict_index = max(0, min(settings.active_anim_dict_index + 1, len(settings.animation_dictionaries) - 1))
        return {'FINISHED'}


class BEVY_OT_AnimDictAddClip(bpy.types.Operator):
    bl_idname = "ads.anim_dict_add_clip"
    bl_label = "Add Clip To Dictionary"
    bl_description = "Add clip name into active animation dictionary"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.animation_dictionaries:
            self.report({'WARNING'}, "Create animation dictionary first")
            return {'CANCELLED'}
        idx = min(max(0, settings.active_anim_dict_index), len(settings.animation_dictionaries) - 1)
        dict_name = settings.animation_dictionaries[idx].name
        clip_name = settings.anim_dict_clip_name.strip()
        if not clip_name:
            active_action = getattr(getattr(context.object, 'animation_data', None), 'action', None) if context.object else None
            if active_action is not None:
                clip_name = active_action.name
        if not clip_name:
            self.report({'WARNING'}, "Set clip name first")
            return {'CANCELLED'}
        added = _add_clip_to_dict(settings, dict_name, clip_name)
        if not added:
            self.report({'INFO'}, f"Clip '{clip_name}' already exists in '{dict_name}'")
            return {'FINISHED'}
        self.report({'INFO'}, f"Added clip '{clip_name}' to '{dict_name}'")
        return {'FINISHED'}


class BEVY_OT_AnimDictRemoveClip(bpy.types.Operator):
    bl_idname = "ads.anim_dict_remove_clip"
    bl_label = "Remove Clip From Dictionary"
    bl_description = "Remove clip from active animation dictionary"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.animation_dictionaries:
            self.report({'WARNING'}, "No animation dictionary")
            return {'CANCELLED'}
        idx = min(max(0, settings.active_anim_dict_index), len(settings.animation_dictionaries) - 1)
        dict_item = settings.animation_dictionaries[idx]
        if not dict_item.clips:
            self.report({'WARNING'}, "No clips in active dictionary")
            return {'CANCELLED'}

        clip_name = settings.anim_dict_clip_name.strip()
        remove_index = -1
        if clip_name:
            for i, clip in enumerate(dict_item.clips):
                if clip.clip_name == clip_name:
                    remove_index = i
                    break
        else:
            remove_index = min(max(0, dict_item.active_clip_index), len(dict_item.clips) - 1)

        if remove_index < 0:
            self.report({'WARNING'}, "Clip not found in active dictionary")
            return {'CANCELLED'}

        removed_name = dict_item.clips[remove_index].clip_name
        dict_item.clips.remove(remove_index)
        dict_item.active_clip_index = max(0, min(remove_index, len(dict_item.clips) - 1))
        self.report({'INFO'}, f"Removed clip '{removed_name}'")
        return {'FINISHED'}


class BEVY_OT_IkChainAdd(bpy.types.Operator):
    bl_idname = "ads.ik_chain_add"
    bl_label = "Add IK Chain"
    bl_description = "Create a new IK chain definition"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        chain_name = _sanitize_ik_chain_name(settings.new_ik_chain_name)
        _get_or_create_ik_chain(settings, chain_name)
        self.report({'INFO'}, f"IK chain '{chain_name}' ready")
        return {'FINISHED'}


class BEVY_OT_IkChainRemove(bpy.types.Operator):
    bl_idname = "ads.ik_chain_remove"
    bl_label = "Remove IK Chain"
    bl_description = "Remove active IK chain definition"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.ik_chains:
            self.report({'WARNING'}, "No IK chain to remove")
            return {'CANCELLED'}
        idx = min(max(0, settings.active_ik_chain_index), len(settings.ik_chains) - 1)
        name = settings.ik_chains[idx].name
        settings.ik_chains.remove(idx)
        settings.active_ik_chain_index = max(0, min(idx, len(settings.ik_chains) - 1))
        self.report({'INFO'}, f"Removed IK chain '{name}'")
        return {'FINISHED'}


class BEVY_OT_IkChainPrev(bpy.types.Operator):
    bl_idname = "ads.ik_chain_prev"
    bl_label = "Previous IK Chain"
    bl_description = "Select previous IK chain"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.ik_chains:
            self.report({'WARNING'}, "No IK chain")
            return {'CANCELLED'}
        settings.active_ik_chain_index = max(0, min(settings.active_ik_chain_index - 1, len(settings.ik_chains) - 1))
        return {'FINISHED'}


class BEVY_OT_IkChainNext(bpy.types.Operator):
    bl_idname = "ads.ik_chain_next"
    bl_label = "Next IK Chain"
    bl_description = "Select next IK chain"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        if not settings.ik_chains:
            self.report({'WARNING'}, "No IK chain")
            return {'CANCELLED'}
        settings.active_ik_chain_index = max(0, min(settings.active_ik_chain_index + 1, len(settings.ik_chains) - 1))
        return {'FINISHED'}


class BEVY_OT_IkChainAutofillBiped(bpy.types.Operator):
    bl_idname = "ads.ik_chain_autofill_biped"
    bl_label = "Autofill Biped IK"
    bl_description = "Populate standard left/right leg IK chain templates"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        prepared = 0
        for spec in _default_biped_ik_specs():
            chain = _get_or_create_ik_chain(settings, spec['name'])
            chain.enabled = True
            chain.parent_bone_name = spec['parent_bone_name']
            chain.ik_target_name = spec['ik_target_name']
            chain.effector_bone_name = spec['effector_bone_name']
            chain.pole_bone_name = spec['pole_bone_name']
            chain.chain_length = spec['chain_length']
            chain.solver_iterations = spec['solver_iterations']
            chain.min_knee_angle = spec['min_knee_angle']
            chain.max_knee_angle = spec['max_knee_angle']
            prepared += 1
        self.report({'INFO'}, f"Prepared {prepared} biped IK chain template(s)")
        return {'FINISHED'}


class BEVY_OT_IkChainValidate(bpy.types.Operator):
    bl_idname = "ads.ik_chain_validate"
    bl_label = "Validate IK Chains"
    bl_description = "Validate IK chain bone names against active armature"

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        armature = _find_target_armature(context)
        if armature is None:
            self.report({'WARNING'}, "No armature selected")
            return {'CANCELLED'}

        if not settings.ik_chains:
            self.report({'WARNING'}, "No IK chains defined")
            return {'CANCELLED'}

        known_bones = _collect_armature_bone_names(armature)
        missing = []
        for chain in settings.ik_chains:
            checks = [
                ('parent', chain.parent_bone_name),
                ('target', chain.ik_target_name),
                ('effector', chain.effector_bone_name),
            ]
            if chain.pole_bone_name.strip():
                checks.append(('pole', chain.pole_bone_name))
            for label, bone_name in checks:
                name = (bone_name or '').strip()
                if not name or name not in known_bones:
                    missing.append(f"{chain.name}:{label}:{name or '<empty>'}")

        if missing:
            self.report({'WARNING'}, f"IK validate: {len(missing)} missing bone reference(s)")
            for msg in missing[:10]:
                self.report({'WARNING'}, msg)
            return {'CANCELLED'}

        self.report({'INFO'}, f"IK validate OK ({len(settings.ik_chains)} chain(s))")
        return {'FINISHED'}


class BEVY_OT_RenameMixamoRig(bpy.types.Operator):
    bl_idname = "ads.rename_mixamo_rig"
    bl_label = "Auto Rename Mixamo Rig"
    bl_description = "Rename Mixamo bones to ADS DEF_ naming and update matching mesh vertex groups"

    def execute(self, context):
        targets = []
        active = getattr(context, 'active_object', None)
        if active and active.type == 'ARMATURE':
            targets.append(active)
        else:
            targets.extend(obj for obj in context.selected_objects if obj.type == 'ARMATURE')

        if not targets:
            target = _find_target_armature(context)
            if target is not None:
                targets.append(target)

        if not targets:
            self.report({'WARNING'}, "No armature selected")
            return {'CANCELLED'}

        renamed = 0
        for armature_obj in targets:
            rename_map = _rename_mixamo_armature(armature_obj)
            renamed += len(rename_map)

        self.report({'INFO'}, f"Renamed {renamed} Mixamo bone name(s)")
        return {'FINISHED'}


class BEVY_OT_ImportMixamoAnimations(bpy.types.Operator):
    bl_idname = "ads.import_mixamo_animations"
    bl_label = "Import Mixamo Animations"
    bl_description = "Import one or more Mixamo FBX animations, auto-rename them and push actions to the target armature"

    directory: bpy.props.StringProperty(subtype='DIR_PATH')
    files: bpy.props.CollectionProperty(type=bpy.types.OperatorFileListElement)
    filepath: bpy.props.StringProperty(subtype='FILE_PATH')
    filter_glob: bpy.props.StringProperty(default="*.fbx;*.FBX", options={'HIDDEN'})
    auto_rename: bpy.props.BoolProperty(name="Auto Rename Rig", default=True)
    merge_to_target: bpy.props.BoolProperty(name="Merge To Target Armature", default=True)
    cleanup_imported: bpy.props.BoolProperty(name="Delete Imported Animation Rig", default=True)
    assign_to_active_dict: bpy.props.BoolProperty(name="Assign To Active Anim Dict", default=True)

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        settings = context.scene.bevy_toolkit_export
        paths = []
        if self.files:
            for file_item in self.files:
                paths.append(os.path.join(self.directory, file_item.name))
        elif self.filepath:
            paths.append(self.filepath)

        if not paths:
            self.report({'WARNING'}, "No FBX files selected")
            return {'CANCELLED'}

        target_armature = _find_target_armature(context)
        if target_armature is not None and self.auto_rename:
            _rename_mixamo_armature(target_armature)

        imported_actions = 0
        for path in paths:
            if not os.path.isfile(path):
                self.report({'WARNING'}, f"File not found: {path}")
                continue

            before_objects = set(bpy.data.objects.keys())
            try:
                bpy.ops.import_scene.fbx(
                    filepath=path,
                    use_anim=True,
                    automatic_bone_orientation=True,
                )
            except TypeError:
                bpy.ops.import_scene.fbx(filepath=path)

            new_objects = [bpy.data.objects[name] for name in (set(bpy.data.objects.keys()) - before_objects) if name in bpy.data.objects]
            imported_armatures = [obj for obj in new_objects if obj.type == 'ARMATURE']
            if not imported_armatures:
                self.report({'WARNING'}, f"No armature found in {os.path.basename(path)}")
                continue

            imported_armature = imported_armatures[0]
            if self.auto_rename:
                _rename_mixamo_armature(imported_armature)

            animation_data = getattr(imported_armature, 'animation_data', None)
            source_action = getattr(animation_data, 'action', None) if animation_data else None
            if source_action is None and animation_data is not None:
                for track in animation_data.nla_tracks:
                    for strip in track.strips:
                        if getattr(strip, 'action', None) is not None:
                            source_action = strip.action
                            break
                    if source_action is not None:
                        break

            if source_action is None:
                self.report({'WARNING'}, f"No action found in {os.path.basename(path)}")
                continue

            clip_name = bpy.path.clean_name(os.path.splitext(os.path.basename(path))[0])
            try:
                source_action.name = clip_name
            except Exception:
                pass

            if self.merge_to_target and target_armature is not None:
                _push_action_to_nla(target_armature, source_action, clip_name)
                if self.assign_to_active_dict and settings.animation_dictionaries:
                    dict_idx = min(max(0, settings.active_anim_dict_index), len(settings.animation_dictionaries) - 1)
                    dict_name = settings.animation_dictionaries[dict_idx].name
                    _add_clip_to_dict(settings, dict_name, clip_name)
                imported_actions += 1

                if self.cleanup_imported:
                    for obj in new_objects:
                        try:
                            bpy.data.objects.remove(obj, do_unlink=True)
                        except Exception:
                            pass
            else:
                imported_actions += 1

        self.report({'INFO'}, f"Imported {imported_actions} animation clip(s)")
        return {'FINISHED'}


class ADS_OT_import_anim_set(bpy.types.Operator):
    bl_idname = "ads.import_anim_set"
    bl_label = "Import ADS Anim Set"
    bl_description = "Import a standalone .ads_anim file and apply its clips to the active armature"

    filepath: bpy.props.StringProperty(subtype='FILE_PATH')
    filter_glob: bpy.props.StringProperty(default="*.ads_anim", options={'HIDDEN'})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        from .adm_import import import_ads_anim

        if not os.path.isfile(self.filepath):
            self.report({'ERROR'}, f"File not found: {self.filepath}")
            return {'CANCELLED'}

        armature = _find_target_armature(context)
        if armature is None:
            self.report({'WARNING'}, "No armature selected for animation import")
            return {'CANCELLED'}

        try:
            clips = import_ads_anim(self.filepath, armature_object=armature, apply_to_target=True, assign_dictionaries=True)
        except Exception as e:
            self.report({'ERROR'}, f"ADS_ANIM import selhal: {e}")
            return {'CANCELLED'}

        self.report({'INFO'}, f"Imported {len(clips)} animation clip(s) from {os.path.basename(self.filepath)}")
        return {'FINISHED'}


class ADS_OT_export_adm(bpy.types.Operator):
    bl_idname  = "ads.export_adm"
    bl_label   = "Export ADM"
    bl_description = "Exportuje geometrii jako .adm + .drawable do vybraného adresáře"

    directory: bpy.props.StringProperty(subtype='DIR_PATH')
    use_selection: bpy.props.BoolProperty(name="Pouze výběr", default=False)

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        from .adm_export import export_adm
        from .export import build_drawable_toml
        import os

        settings = context.scene.bevy_toolkit_export

        if self.use_selection:
            objects = [o for o in context.selected_objects if o.type == 'MESH']
        else:
            objects = [o for o in context.scene.objects if o.type == 'MESH']

        if not objects:
            self.report({'WARNING'}, "Žádné mesh objekty k exportu")
            return {'CANCELLED'}

        export_name  = _resolve_export_asset_name(context, objects)
        adm_path      = os.path.join(self.directory, f"{export_name}.adm")
        drawable_path = os.path.join(self.directory, f"{export_name}.drawable")

        # Export geometrie + embedded DDS textur
        armature = _find_target_armature(context, objects)

        try:
            meshes, nodes = export_adm(adm_path, objects=objects, export_textures=True, armature_object=armature)
        except Exception as e:
            self.report({'ERROR'}, f"ADM export selhal: {e}")
            return {'CANCELLED'}

        # Collect unique materials
        used_materials = []
        seen = set()
        for obj in objects:
            for slot in obj.material_slots:
                mat = slot.material
                if mat and mat.name not in seen:
                    used_materials.append(mat)
                    seen.add(mat.name)

        # Generuj .drawable TOML
        try:
            lines = build_drawable_toml(export_name, objects, used_materials)
            with open(drawable_path, 'w', encoding='utf-8') as f:
                f.write('\n'.join(lines) + '\n')
        except Exception as e:
            self.report({'WARNING'}, f".drawable selhal: {e}")

        try:
            ik_path, ik_count = _write_ik_sidecar(self.directory, export_name, armature, settings)
            if ik_path and ik_count > 0:
                self.report({'INFO'}, f"IK sidecar: {ik_count} chain(s) -> {os.path.basename(ik_path)}")
        except Exception as e:
            self.report({'WARNING'}, f"IK sidecar export selhal: {e}")

        self.report({'INFO'}, f"ADM: {meshes} meshů, {nodes} uzlů → {os.path.basename(adm_path)} + {os.path.basename(drawable_path)}")
        return {'FINISHED'}


class ADS_OT_export_anim_set(bpy.types.Operator):
    bl_idname = "ads.export_anim_set"
    bl_label = "Export ADS Anim Set"
    bl_description = "Exportuje animační klipy do samostatného .ads_anim souboru"

    directory: bpy.props.StringProperty(subtype='DIR_PATH')
    use_selection: bpy.props.BoolProperty(name="Pouze výběr", default=False)

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        from .adm_export import export_ads_anim
        import os

        settings = context.scene.bevy_toolkit_export

        objects = _gather_anim_export_objects(context, self.use_selection)
        armature = _find_target_armature(context, objects)

        if armature is None:
            self.report({'WARNING'}, "Nenašla se žádná armatura pro export animací")
            return {'CANCELLED'}

        export_name = _resolve_anim_export_asset_name(context, armature, objects)
        anim_path = os.path.join(self.directory, f"{export_name}.ads_anim")

        try:
            clip_count, dict_count = export_ads_anim(anim_path, objects=objects, armature_object=armature)
        except Exception as e:
            self.report({'ERROR'}, f"ADS anim export selhal: {e}")
            return {'CANCELLED'}

        try:
            ik_path, ik_count = _write_ik_sidecar(self.directory, export_name, armature, settings)
            if ik_path and ik_count > 0:
                self.report({'INFO'}, f"IK sidecar: {ik_count} chain(s) -> {os.path.basename(ik_path)}")
        except Exception as e:
            self.report({'WARNING'}, f"IK sidecar export selhal: {e}")

        self.report(
            {'INFO'},
            f"ADS_ANIM: {clip_count} clipů, {dict_count} dictionary → {os.path.basename(anim_path)}",
        )
        return {'FINISHED'}


class BEVY_OT_ExportMapManifest(bpy.types.Operator):
    bl_idname = "bevy.export_map_manifest"
    bl_label = "Export Map TOML"
    bl_description = "Export selected (or all) mesh objects to a map TOML manifest"

    filepath: bpy.props.StringProperty(subtype='FILE_PATH')
    filter_glob: bpy.props.StringProperty(default="*.toml", options={'HIDDEN'})
    use_selection: bpy.props.BoolProperty(name="Pouze výběr", default=True)

    def invoke(self, context, event):
        if not self.filepath:
            blend_dir = os.path.dirname(bpy.data.filepath) if bpy.data.filepath else ""
            default = os.path.join(blend_dir, "map.map.toml") if blend_dir else "map.map.toml"
            self.filepath = default
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        if self.use_selection:
            objects = [o for o in context.selected_objects if o.type == 'MESH']
        else:
            objects = [o for o in context.scene.objects if o.type == 'MESH']

        if not objects:
            self.report({'WARNING'}, "Žádné mesh objekty pro map export")
            return {'CANCELLED'}

        instances = []
        for idx, obj in enumerate(objects):
            props = getattr(obj, 'bevy_toolkit_obj', None)
            export_name = props.export_name.strip() if props else ""
            model = export_name or bpy.path.clean_name(obj.name)
            position = _blender_pos_to_map(obj.location)
            rot_deg = obj.rotation_euler.to_matrix().to_euler('XYZ')
            rotation = _blender_rot_to_map_deg(Vector((
                rot_deg.x * (180.0 / 3.141592653589793),
                rot_deg.y * (180.0 / 3.141592653589793),
                rot_deg.z * (180.0 / 3.141592653589793),
            )))
            scale = (float(obj.scale.x), float(obj.scale.z), float(obj.scale.y))
            tags = parse_tags(props.tags_csv) if props else []
            navmesh_only = bool(props and props.is_col and props.col_shape == 'NAVMESH')

            instances.append({
                "id": f"{model}_{idx}",
                "model": model,
                "position": position,
                "rotation_deg": rotation,
                "scale": scale,
                "tags": tags,
                "navmesh_only": navmesh_only,
            })

        lines = _build_map_manifest_lines(instances)
        try:
            with open(self.filepath, 'w', encoding='utf-8') as fh:
                fh.write("\n".join(lines) + "\n")
        except Exception as e:
            self.report({'ERROR'}, f"Nelze zapsat map TOML: {e}")
            return {'CANCELLED'}

        self.report({'INFO'}, f"Map export: {len(instances)} instancí → {os.path.basename(self.filepath)}")
        return {'FINISHED'}


class BEVY_OT_ImportMapManifest(bpy.types.Operator):
    bl_idname = "bevy.import_map_manifest"
    bl_label = "Import Map TOML"
    bl_description = "Import map TOML and create editable scene placeholders"

    filepath: bpy.props.StringProperty(subtype='FILE_PATH')
    filter_glob: bpy.props.StringProperty(default="*.toml", options={'HIDDEN'})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        try:
            import tomllib
        except ImportError:
            self.report({'ERROR'}, "tomllib není dostupné (vyžaduje Blender/Python 3.11+)")
            return {'CANCELLED'}

        if not os.path.isfile(self.filepath):
            self.report({'ERROR'}, f"Soubor nenalezen: {self.filepath}")
            return {'CANCELLED'}

        try:
            with open(self.filepath, 'rb') as fh:
                data = tomllib.load(fh)
        except Exception as e:
            self.report({'ERROR'}, f"Nelze načíst TOML: {e}")
            return {'CANCELLED'}

        instances = data.get('instances', [])
        if not isinstance(instances, list) or not instances:
            self.report({'WARNING'}, "Map TOML neobsahuje žádné [[instances]]")
            return {'CANCELLED'}

        imported = 0
        for idx, entry in enumerate(instances):
            model = str(entry.get('model', f'instance_{idx}')).strip() or f'instance_{idx}'
            obj_name = str(entry.get('id', f'map_{model}_{idx}')).strip() or f'map_{model}_{idx}'

            mesh = bpy.data.meshes.new(obj_name + "_mesh")
            mesh.from_pydata(
                [(-0.5, -0.5, 0.0), (0.5, -0.5, 0.0), (0.5, 0.5, 0.0), (-0.5, 0.5, 0.0)],
                [],
                [(0, 1, 2, 3)],
            )
            obj = bpy.data.objects.new(obj_name, mesh)
            context.collection.objects.link(obj)

            pos = entry.get('position', [0.0, 0.0, 0.0])
            rot = entry.get('rotation_deg', [0.0, 0.0, 0.0])
            scl = entry.get('scale', [1.0, 1.0, 1.0])

            obj.location = _map_pos_to_blender(pos)
            obj.rotation_mode = 'XYZ'
            rot_blender_deg = _map_rot_deg_to_blender(rot)
            obj.rotation_euler = Vector((
                rot_blender_deg.x * (3.141592653589793 / 180.0),
                rot_blender_deg.y * (3.141592653589793 / 180.0),
                rot_blender_deg.z * (3.141592653589793 / 180.0),
            ))
            obj.scale = Vector((float(scl[0]), float(scl[2]), float(scl[1])))

            props = obj.bevy_toolkit_obj
            props.export_name = model
            navmesh_only = bool(entry.get('navmesh_only', False))
            if navmesh_only:
                props.is_col = True
                props.col_shape = 'NAVMESH'
                obj.display_type = 'WIRE'
                obj.hide_render = True

            tags = entry.get('tags', [])
            if isinstance(tags, list):
                props.tags_csv = ",".join(str(t) for t in tags)

            imported += 1

        self.report({'INFO'}, f"Map import: vytvořeno {imported} objektů")
        return {'FINISHED'}


_IMAGE_EXTS_SET = frozenset((".png", ".jpg", ".jpeg", ".tga", ".dds", ".tif", ".tiff", ".exr", ".bmp"))


class BEVY_OT_FindMissingTextures(bpy.types.Operator):
    bl_idname      = "bevy.find_missing_textures"
    bl_label       = "Find Missing Textures"
    bl_description = (
        "Recursively search a directory for missing textures and relink them.\n"
        "Fixes broken image paths in bpy.data.images and fills empty texture slots\n"
        "in Bevy material props where a name is set but no image is assigned."
    )

    directory:   bpy.props.StringProperty(subtype='DIR_PATH')
    # File browser filter — must be a property to silence RNA warnings even for DIR_PATH
    filter_glob: bpy.props.StringProperty(default="*", options={'HIDDEN'})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        search_dir = self.directory.rstrip("\\/")
        if not os.path.isdir(search_dir):
            self.report({'ERROR'}, f"Not a directory: {search_dir}")
            return {'CANCELLED'}

        # ── Build index: stem_lower → first found absolute path ──────────────
        index = {}
        for root, _dirs, files in os.walk(search_dir):
            for fname in files:
                ext = os.path.splitext(fname)[1].lower()
                if ext not in _IMAGE_EXTS_SET:
                    continue
                stem = os.path.splitext(fname)[0].lower()
                if stem not in index:
                    index[stem] = os.path.join(root, fname)

        if not index:
            self.report({'WARNING'}, f"No image files found under: {search_dir}")
            return {'CANCELLED'}

        relinked  = 0
        filled    = 0
        not_found = []

        # ── 1. Fix images in bpy.data.images with missing file paths ─────────
        for img in bpy.data.images:
            if img.source != 'FILE':
                continue
            abspath = bpy.path.abspath(img.filepath)
            if abspath and os.path.isfile(abspath):
                continue  # file is OK

            # Build ordered list of search keys (prefer existing filename stem)
            search_keys = []
            if img.filepath:
                current_stem = os.path.splitext(os.path.basename(bpy.path.abspath(img.filepath)))[0]
                if current_stem:
                    search_keys.append(current_stem.lower())
            name_stem = os.path.splitext(img.name)[0].lower()
            if name_stem not in search_keys:
                search_keys.append(name_stem)

            found_path = next((index[k] for k in search_keys if k in index), None)
            if found_path:
                img.filepath = found_path
                try:
                    img.reload()
                except Exception:
                    pass
                relinked += 1
            else:
                not_found.append(img.name)

        # ── 2. Fill empty _img slots where _name is set but image is missing ──
        _MAT_SLOTS = (
            'albedo', 'mrao', 'normal', 'palette', 'snow',
            'l0_albedo', 'l0_mrao', 'l0_normal',
            'l1_albedo', 'l1_mrao', 'l1_normal',
            'glass_albedo', 'shatter_map',
            'ma', 'mb',
        )
        for mat in bpy.data.materials:
            props = getattr(mat, 'bevy_toolkit', None)
            if props is None:
                continue
            for slot in _MAT_SLOTS:
                img_attr  = f"{slot}_img"
                name_attr = f"{slot}_name"
                if not hasattr(props, img_attr) or not hasattr(props, name_attr):
                    continue
                if getattr(props, img_attr) is not None:
                    continue  # already assigned
                name = getattr(props, name_attr, "").strip()
                if not name:
                    continue
                stem = os.path.splitext(name)[0].lower()
                found_path = index.get(stem)
                if found_path:
                    img = bpy.data.images.load(found_path, check_existing=True)
                    setattr(props, img_attr, img)
                    filled += 1
                else:
                    not_found.append(f"{mat.name}/{slot} ('{name}')")

        # ── Report ────────────────────────────────────────────────────────────
        summary = f"Relinked {relinked} image(s), filled {filled} material slot(s)"
        if not_found:
            summary += f", {len(not_found)} not found"
            self.report({'WARNING'}, summary)
            # Each missing item as a separate WARNING → visible in Info editor
            for item in not_found:
                self.report({'WARNING'}, f"  Not found: {item}")
            # Full list also to system console for easy copy-paste
            print(f"[Find Missing Textures] {summary}")
            for item in not_found:
                print(f"  NOT FOUND: {item}")
        else:
            self.report({'INFO'}, summary)
        return {'FINISHED'}


class BEVY_OT_BrowseTexture(bpy.types.Operator):
    """Select a texture file and assign it to this material slot"""
    bl_idname  = "bevy.browse_texture"
    bl_label   = "Browse Texture"
    bl_options = {'INTERNAL', 'UNDO'}

    filepath:    bpy.props.StringProperty(subtype='FILE_PATH')
    filter_glob: bpy.props.StringProperty(
        default="*.png;*.jpg;*.jpeg;*.tga;*.bmp;*.tif;*.tiff;*.dds;*.exr",
        options={'HIDDEN'},
    )
    slot_name: bpy.props.StringProperty(options={'HIDDEN'})
    mat_name:  bpy.props.StringProperty(options={'HIDDEN'})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def execute(self, context):
        mat = bpy.data.materials.get(self.mat_name)
        if not mat:
            return {'CANCELLED'}
        props = mat.bevy_toolkit
        img   = bpy.data.images.load(self.filepath, check_existing=True)
        setattr(props, f"{self.slot_name}_img", img)
        if not getattr(props, f"{self.slot_name}_name").strip():
            setattr(
                props,
                f"{self.slot_name}_name",
                os.path.splitext(os.path.basename(self.filepath))[0],
            )
        _sync_texture_node(mat, self.slot_name, img)
        return {'FINISHED'}
