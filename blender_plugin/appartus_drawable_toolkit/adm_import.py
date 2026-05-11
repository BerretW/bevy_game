"""Import .adm binárního formátu zpět do Blenderu."""

import os
import re
import struct
import tempfile
import bpy
import mathutils

from .constants import ATTR_NAME, ATTR_NAME2, UV_MASKS2_NAME

_C = mathutils.Matrix([
    [1,  0,  0, 0],
    [0,  0,  1, 0],
    [0, -1,  0, 0],
    [0,  0,  0, 1],
])
_C_INV = _C.inverted()

ATTR_POS    = 1 << 0
ATTR_NRM    = 1 << 1
ATTR_TAN    = 1 << 2
ATTR_UV0    = 1 << 3
ATTR_UV1    = 1 << 4
ATTR_MASKS0 = 1 << 5
ATTR_MASKS1 = 1 << 6
ATTR_JOINT_INDICES = 1 << 7
ATTR_JOINT_WEIGHTS = 1 << 8


def _u8(f):   return struct.unpack('<B', f.read(1))[0]
def _u16(f):  return struct.unpack('<H', f.read(2))[0]
def _u32(f):  return struct.unpack('<I', f.read(4))[0]
def _i32(f):  return struct.unpack('<i', f.read(4))[0]
def _f32(f):  return struct.unpack('<f', f.read(4))[0]
def _str(f):  n = _u16(f); return f.read(n).decode('utf-8')
def _f32s(f, n): return struct.unpack(f'<{n}f', f.read(n * 4))
def _u8s(f, n):  return struct.unpack(f'<{n}B', f.read(n))
def _u16s(f, n): return struct.unpack(f'<{n}H', f.read(n * 2))
def _u32s(f, n): return struct.unpack(f'<{n}I', f.read(n * 4))


def _bevy_to_bl_pos(x, y, z):
    """Bevy (Y-up) → Blender (Z-up): (x, y, z) → (x, -z, y)"""
    return (x, -z, y)


def _bevy_to_bl_mat4(floats):
    """Column-major 16 floats (Bevy space) → mathutils.Matrix (Blender space)."""
    m_bevy = mathutils.Matrix([
        [floats[0],  floats[4],  floats[8],  floats[12]],
        [floats[1],  floats[5],  floats[9],  floats[13]],
        [floats[2],  floats[6],  floats[10], floats[14]],
        [floats[3],  floats[7],  floats[11], floats[15]],
    ])
    return _C_INV @ m_bevy @ _C


def _bevy_trs_to_blender(pos, rot_xyzw, scale):
    """Bevy TRS (Y-up) -> Blender TRS (Z-up)."""
    m_pos = mathutils.Matrix.Translation(mathutils.Vector((pos[0], pos[1], pos[2])))
    m_rot = mathutils.Quaternion((rot_xyzw[3], rot_xyzw[0], rot_xyzw[1], rot_xyzw[2])).to_matrix().to_4x4()
    m_scl = mathutils.Matrix.Diagonal((scale[0], scale[1], scale[2], 1.0))
    m_bevy = m_pos @ m_rot @ m_scl
    m_bl = _C_INV @ m_bevy @ _C
    loc, rot, scl = m_bl.decompose()
    return loc, rot, scl


def _parse_mesh(f):
    name        = _str(f)
    vert_count  = _u32(f)
    index_count = _u32(f)
    attr_flags  = _u32(f)

    positions, normals, uv0s, uv1s, masks0s, masks1s = [], [], [], [], [], []
    joint_indices, joint_weights = [], []

    if attr_flags & ATTR_POS:
        raw = _f32s(f, vert_count * 3)
        positions = [_bevy_to_bl_pos(raw[i], raw[i+1], raw[i+2]) for i in range(0, len(raw), 3)]

    if attr_flags & ATTR_NRM:
        raw = _f32s(f, vert_count * 3)
        normals = [_bevy_to_bl_pos(raw[i], raw[i+1], raw[i+2]) for i in range(0, len(raw), 3)]

    if attr_flags & ATTR_TAN:
        f.read(vert_count * 4 * 4)  # skip

    if attr_flags & ATTR_UV0:
        raw = _f32s(f, vert_count * 2)
        uv0s = [(raw[i], 1.0 - raw[i+1]) for i in range(0, len(raw), 2)]  # V-flip

    if attr_flags & ATTR_UV1:
        raw = _f32s(f, vert_count * 2)
        uv1s = [(raw[i], 1.0 - raw[i+1]) for i in range(0, len(raw), 2)]

    if attr_flags & ATTR_MASKS0:
        raw = _u8s(f, vert_count * 4)
        masks0s = [(raw[i]/255, raw[i+1]/255, raw[i+2]/255, raw[i+3]/255)
                   for i in range(0, len(raw), 4)]

    if attr_flags & ATTR_MASKS1:
        raw = _u8s(f, vert_count * 4)
        masks1s = [(raw[i]/255, raw[i+1]/255, raw[i+2]/255, raw[i+3]/255)
                   for i in range(0, len(raw), 4)]

    if attr_flags & ATTR_JOINT_INDICES:
        raw = _u16s(f, vert_count * 4)
        joint_indices = [(raw[i], raw[i+1], raw[i+2], raw[i+3]) for i in range(0, len(raw), 4)]

    if attr_flags & ATTR_JOINT_WEIGHTS:
        raw = _f32s(f, vert_count * 4)
        joint_weights = [(raw[i], raw[i+1], raw[i+2], raw[i+3]) for i in range(0, len(raw), 4)]

    indices = list(_u32s(f, index_count))
    return name, positions, normals, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices


def _parse_node(f):
    name          = _str(f)
    node_type     = _u8(f)
    mesh_index    = _i32(f)
    parent_index  = _i32(f)
    material_name = _str(f)
    mat_floats    = _f32s(f, 16)
    return name, node_type, mesh_index, parent_index, material_name, mat_floats


def _build_blender_mesh(name, positions, normals, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices):
    mesh = bpy.data.meshes.new(name)
    tri_count = len(indices) // 3
    faces = [(indices[i*3], indices[i*3+1], indices[i*3+2]) for i in range(tri_count)]
    mesh.from_pydata(positions, [], faces)
    mesh.update()

    # Per-loop custom normals (Blender 4.1+ odstranilo use_auto_smooth, ale
    # normals_split_custom_set stále funguje; starší verze mohou vyžadovat jiný setup)
    if normals:
        loop_nrm = []
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                vi = mesh.loops[li].vertex_index
                loop_nrm.append(normals[vi])
        try:
            mesh.normals_split_custom_set(loop_nrm)
        except Exception as _e:
            print(f"[adm_import] Warning: nelze nastavit custom normals ({_e}); použijí se auto-normals")

    # UV0
    if uv0s:
        ul = mesh.uv_layers.new(name="UVMap")
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                ul.data[li].uv = uv0s[mesh.loops[li].vertex_index]

    # UV1 = TEXCOORD_1 = encoded bevy_masks2 (AO=U, emissive=V).
    # Name it _ads_masks2_uv so decode_masks2_from_uv (called by
    # fix_imported_vertex_attributes) will decode it into a proper
    # BYTE_COLOR/CORNER attribute and remove this UV layer.
    if uv1s:
        ul1 = mesh.uv_layers.new(name=UV_MASKS2_NAME)
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                ul1.data[li].uv = uv1s[mesh.loops[li].vertex_index]

    # bevy_masks — BYTE_COLOR/CORNER (matches ensure_mask_attribute in mesh.py)
    if masks0s:
        ca = mesh.color_attributes.new(name=ATTR_NAME, type='BYTE_COLOR', domain='CORNER')
        flat = []
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                vi = mesh.loops[li].vertex_index
                c = masks0s[vi] if vi < len(masks0s) else (0.0, 0.0, 0.0, 1.0)
                flat.extend(c)
        ca.data.foreach_set('color', flat)

    # bevy_masks2 — BYTE_COLOR/CORNER, only when ATTR_MASKS1 is explicitly present.
    # If absent but UV1 was present (renamed _ads_masks2_uv above), the decode
    # path in fix_imported_vertex_attributes handles creation automatically.
    if masks1s:
        ca2 = mesh.color_attributes.new(name=ATTR_NAME2, type='BYTE_COLOR', domain='CORNER')
        flat2 = []
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                vi = mesh.loops[li].vertex_index
                c = masks1s[vi] if vi < len(masks1s) else (1.0, 0.0, 0.0, 1.0)
                flat2.extend(c)
        ca2.data.foreach_set('color', flat2)

    return mesh


def _parse_textures(f):
    """Parsuje texture sekci ADM — vrátí list (name, is_srgb, ext, data)."""
    tex_count = _u32(f)
    textures = []
    for _ in range(tex_count):
        img_name    = _str(f)
        format_byte = _u8(f)
        is_srgb     = _u8(f)
        data_len    = _u32(f)
        data        = f.read(data_len)
        ext = {1: '.jpg', 2: '.dds'}.get(format_byte, '.png')
        textures.append((img_name, bool(is_srgb), ext, data))
    return textures


def _parse_animations_v2(f, version=2):
    """Parsuje (a zatím ignoruje) animační sekci ADM v2/v3."""
    clips = []
    clip_count = _u32(f)
    for _ in range(clip_count):
        name = _str(f)
        duration = _f32(f)
        track_count = _u32(f)
        tracks = []
        for _ in range(track_count):
            node = _str(f)
            flags = _u32(f) if version >= 3 else 1
            key_count = _u32(f)
            keys = []
            for _ in range(key_count):
                t = _f32(f)
                pos = (_f32(f), _f32(f), _f32(f))
                rot = (_f32(f), _f32(f), _f32(f), _f32(f))
                scale = (_f32(f), _f32(f), _f32(f))
                keys.append((t, pos, rot, scale))
            tracks.append((node, flags, keys))
        clips.append((name, duration, tracks))
    return clips


def _find_view3d_context():
    """Return (window, area, region) for a VIEW_3D area, or (None,None,None)."""
    for window in bpy.context.window_manager.windows:
        for area in window.screen.areas:
            if area.type == 'VIEW_3D':
                for region in area.regions:
                    if region.type == 'WINDOW':
                        return window, area, region
    return None, None, None


def _enter_edit_mode(arm_obj, view_layer) -> bool:
    """Switch arm_obj into EDIT mode.  Returns True on success.

    mode_set requires a VIEW_3D context.  When the importer runs from a
    FILE_BROWSER dialog, bpy.context.area is the file-browser area.
    We try two approaches in order:

    1. temp_override with a discovered VIEW_3D window/screen/area/region.
    2. Temporarily patch the *current* area type to VIEW_3D (well-known
       add-on pattern; Blender restores it after the with-block).
    """
    # --- method 1: find an existing VIEW_3D area and override ---
    window, area, region = _find_view3d_context()
    if window is not None:
        override = {
            'window': window, 'screen': window.screen,
            'area': area, 'region': region,
            'scene': bpy.context.scene,
            'view_layer': view_layer,
            'active_object': arm_obj,
        }
        with bpy.context.temp_override(**override):
            result = bpy.ops.object.mode_set(mode='EDIT')
        if 'FINISHED' in result:
            return True

    # --- method 2: temporarily change the current area type ---
    ctx_area = bpy.context.area
    if ctx_area is not None:
        old_type = ctx_area.type
        try:
            ctx_area.type = 'VIEW_3D'
            result = bpy.ops.object.mode_set(mode='EDIT')
            return 'FINISHED' in result
        except Exception:
            return False
        finally:
            ctx_area.type = old_type

    return False


def _exit_edit_mode(arm_obj, view_layer):
    """Switch arm_obj back to OBJECT mode (mirrors _enter_edit_mode)."""
    window, area, region = _find_view3d_context()
    if window is not None:
        override = {
            'window': window, 'screen': window.screen,
            'area': area, 'region': region,
            'scene': bpy.context.scene,
            'view_layer': view_layer,
            'active_object': arm_obj,
        }
        with bpy.context.temp_override(**override):
            try:
                bpy.ops.object.mode_set(mode='OBJECT')
                return
            except Exception:
                pass

    ctx_area = bpy.context.area
    if ctx_area is not None:
        old_type = ctx_area.type
        try:
            ctx_area.type = 'VIEW_3D'
            bpy.ops.object.mode_set(mode='OBJECT')
        except Exception:
            pass
        finally:
            ctx_area.type = old_type


def _find_armature_root_index(node_data):
    bone_indices = [i for i, (_, ntype, _, _, _, _) in enumerate(node_data) if ntype == 3]
    if not bone_indices:
        return None
    root_candidates = []
    for i in bone_indices:
        parent_idx = node_data[i][3]
        if 0 <= parent_idx < len(node_data):
            if node_data[parent_idx][1] != 3:
                root_candidates.append(parent_idx)
    if root_candidates:
        return root_candidates[0]

    for i, (_, ntype, _, _, _, _) in enumerate(node_data):
        if ntype == 2:
            return i
    return None


def _build_armature_from_nodes(node_data, collection):
    armature_root_idx = _find_armature_root_index(node_data)
    bone_node_indices = [i for i, (_, ntype, _, _, _, _) in enumerate(node_data) if ntype == 3]
    if armature_root_idx is None or not bone_node_indices:
        return None, [], {}

    root_name, _, _, _, _, root_mat_floats = node_data[armature_root_idx]
    root_mat = _bevy_to_bl_mat4(root_mat_floats)
    arm_data = bpy.data.armatures.new(f"{root_name}_ArmatureData")
    arm_obj = bpy.data.objects.new(f"{root_name}_Armature", arm_data)
    collection.objects.link(arm_obj)
    arm_obj.matrix_local = root_mat

    view_layer = bpy.context.view_layer
    prev_active = view_layer.objects.active
    view_layer.objects.active = arm_obj

    entered_edit = False
    try:
        entered_edit = _enter_edit_mode(arm_obj, view_layer)
        if not entered_edit:
            print("[adm_import] Warning: cannot enter EDIT mode for armature — no 3D viewport found")
            return arm_obj, [], {}

        edit_bones = arm_data.edit_bones
        eb_by_node = {}
        global_by_node = {}
        children_by_node = {}
        for node_idx in bone_node_indices:
            parent_idx = node_data[node_idx][3]
            children_by_node.setdefault(parent_idx, []).append(node_idx)

        # 1) Přepočet bone globálních matic v armature-local prostoru.
        for node_idx in bone_node_indices:
            _name, _ntype, _mesh, parent_idx, _mat_name, mat_floats = node_data[node_idx]
            local = _bevy_to_bl_mat4(mat_floats)
            parent_global = global_by_node.get(parent_idx, mathutils.Matrix.Identity(4))
            global_by_node[node_idx] = parent_global @ local

        # 2) Vytvoření všech bone objektů.
        for node_idx in bone_node_indices:
            name = node_data[node_idx][0]
            eb_by_node[node_idx] = edit_bones.new(name)

        # 3) Nastavení parentingu a head/tail podle hierarchy.
        #    Díky tomu nevznikají nulové délky, které se ve viewportu zobrazí jen jako body.
        for node_idx in bone_node_indices:
            eb = eb_by_node[node_idx]
            parent_idx = node_data[node_idx][3]
            if parent_idx in eb_by_node:
                eb.parent = eb_by_node[parent_idx]

            global_mat = global_by_node[node_idx]
            head = global_mat.translation.copy()
            child_indices = children_by_node.get(node_idx, [])
            local_y_axis = (global_mat.to_3x3() @ mathutils.Vector((0.0, 1.0, 0.0)))
            if local_y_axis.length < 1e-6:
                local_y_axis = mathutils.Vector((0.0, 1.0, 0.0))
            else:
                local_y_axis.normalize()

            if child_indices:
                # Směr kosti bereme VŽDY z exportované lokální +Y osy.
                # Child uzly slouží jen k odhadu délky.
                best_len = 0.0
                for child_idx in child_indices:
                    vec = global_by_node[child_idx].translation - head
                    if vec.length < 1e-6:
                        continue
                    proj = abs(vec.dot(local_y_axis))
                    if proj > best_len:
                        best_len = proj

                if best_len > 1e-5:
                    tail = head + local_y_axis * best_len
                else:
                    tail = head + local_y_axis * 0.08
            else:
                # Leaf bone: použij lokální +Y osu a konzervativní délku.
                axis = local_y_axis.copy()
                if axis.length < 1e-6:
                    axis = mathutils.Vector((0.0, 1.0, 0.0))
                else:
                    axis.normalize()

                parent_len = 0.0
                if parent_idx in eb_by_node:
                    parent_eb = eb_by_node[parent_idx]
                    parent_len = (parent_eb.tail - parent_eb.head).length
                fallback_len = parent_len * 0.4 if parent_len > 1e-5 else 0.08
                tail = head + axis * fallback_len

            if (tail - head).length < 1e-5:
                axis = local_y_axis.copy()
                if axis.length < 1e-6:
                    axis = mathutils.Vector((0.0, 1.0, 0.0))
                else:
                    axis.normalize()
                tail = head + axis * 0.08

            eb.head = head
            eb.tail = tail

            # Roll: referenční Z osu promítneme do roviny kolmé na skutečný
            # head->tail směr. Bez projekce může align_roll kost přetočit.
            z_axis = (global_mat.to_3x3() @ mathutils.Vector((0.0, 0.0, 1.0)))
            bone_y = (eb.tail - eb.head)
            if z_axis.length > 1e-6 and bone_y.length > 1e-6:
                bone_y = bone_y.normalized()
                z_proj = z_axis - bone_y * z_axis.dot(bone_y)
                if z_proj.length > 1e-6:
                    z_axis = z_proj.normalized()
                try:
                    eb.align_roll(z_axis)
                except Exception:
                    pass
    finally:
        if entered_edit:
            _exit_edit_mode(arm_obj, view_layer)
        view_layer.objects.active = prev_active

    joint_bone_names = [node_data[i][0] for i in bone_node_indices]
    bone_by_node_index = {idx: node_data[idx][0] for idx in bone_node_indices}
    return arm_obj, joint_bone_names, bone_by_node_index


def _bind_meshes_to_armature(mesh_data, node_data, node_objects, arm_obj, joint_bone_names):
    if arm_obj is None:
        return

    for node_index, (nname, ntype, mesh_idx, _, _, _) in enumerate(node_data):
        if ntype not in (0, 1):
            continue
        if not (0 <= mesh_idx < len(mesh_data)):
            continue
        obj = node_objects[node_index] if node_index < len(node_objects) else None
        if obj is None or obj.type != 'MESH':
            continue

        _, _, _, _, _, _, _, joint_indices, joint_weights, _ = mesh_data[mesh_idx]
        if not joint_indices or not joint_weights:
            continue

        arm_mod = next((m for m in obj.modifiers if m.type == 'ARMATURE' and m.object == arm_obj), None)
        if arm_mod is None:
            arm_mod = obj.modifiers.new(name='Armature', type='ARMATURE')
            arm_mod.object = arm_obj

        # Weighted mesh nemá být zároveň bone-parentnutý, jinak dochází k dvojí transformaci.
        if obj.parent == arm_obj and getattr(obj, 'parent_type', '') == 'BONE':
            obj.parent_type = 'OBJECT'
            obj.parent_bone = ""
            obj.matrix_parent_inverse = mathutils.Matrix.Identity(4)

        vg_by_name = {vg.name: vg for vg in obj.vertex_groups}
        for bone_name in joint_bone_names:
            if bone_name not in vg_by_name:
                vg_by_name[bone_name] = obj.vertex_groups.new(name=bone_name)

        for v_idx, joints in enumerate(joint_indices):
            if v_idx >= len(joint_weights):
                break
            weights = joint_weights[v_idx]
            for i in range(4):
                weight = float(weights[i])
                if weight <= 1e-6:
                    continue
                joint_idx = int(joints[i])
                if joint_idx < 0 or joint_idx >= len(joint_bone_names):
                    continue
                vg_by_name[joint_bone_names[joint_idx]].add([v_idx], weight, 'ADD')


def _import_armature_animations(clips, arm_obj):
    if arm_obj is None or not clips:
        return

    fps = bpy.context.scene.render.fps / max(1.0, float(bpy.context.scene.render.fps_base))
    if fps <= 0.0:
        fps = 30.0

    arm_obj.animation_data_create()
    for track in list(arm_obj.animation_data.nla_tracks):
        arm_obj.animation_data.nla_tracks.remove(track)

    # Blender pose kanály jsou delta vůči rest poze (matrix_basis), zatímco ADM klíče
    # nesou lokální bone transform přímo. Proto před keyframingem převádíme
    # key_local -> basis = rest_local^-1 * key_local.
    rest_local_inv_by_bone = {}
    for bone in arm_obj.data.bones:
        if bone.parent is None:
            rest_local = bone.matrix_local.copy()
        else:
            rest_local = bone.parent.matrix_local.inverted_safe() @ bone.matrix_local
        rest_local_inv_by_bone[bone.name] = rest_local.inverted_safe()

    for clip_name, _duration, tracks in clips:
        action = bpy.data.actions.new(name=f"ADM_{clip_name}")
        has_keys = False

        for node_name, _flags, keys in tracks:
            if node_name not in arm_obj.pose.bones:
                continue
            if not keys:
                continue
            arm_obj.pose.bones[node_name].rotation_mode = 'QUATERNION'

            loc_curves = [action.fcurves.new(data_path=f'pose.bones["{node_name}"].location', index=i) for i in range(3)]
            rot_curves = [action.fcurves.new(data_path=f'pose.bones["{node_name}"].rotation_quaternion', index=i) for i in range(4)]
            scl_curves = [action.fcurves.new(data_path=f'pose.bones["{node_name}"].scale', index=i) for i in range(3)]

            for curve in (*loc_curves, *rot_curves, *scl_curves):
                curve.keyframe_points.add(count=len(keys))

            for key_i, (time_sec, pos, rot_xyzw, scale) in enumerate(keys):
                frame = 1.0 + float(time_sec) * fps

                loc, rot, scl = _bevy_trs_to_blender(pos, rot_xyzw, scale)
                key_local = (
                    mathutils.Matrix.Translation(loc)
                    @ rot.to_matrix().to_4x4()
                    @ mathutils.Matrix.Diagonal((scl.x, scl.y, scl.z, 1.0))
                )
                basis = rest_local_inv_by_bone.get(node_name, mathutils.Matrix.Identity(4)) @ key_local
                b_loc, b_rot, b_scl = basis.decompose()

                l = (b_loc.x, b_loc.y, b_loc.z)
                q = (b_rot.w, b_rot.x, b_rot.y, b_rot.z)
                s = (b_scl.x, b_scl.y, b_scl.z)

                for i in range(3):
                    loc_curves[i].keyframe_points[key_i].co = (frame, l[i])
                    scl_curves[i].keyframe_points[key_i].co = (frame, s[i])
                for i in range(4):
                    rot_curves[i].keyframe_points[key_i].co = (frame, q[i])

            for curve in (*loc_curves, *rot_curves, *scl_curves):
                for kp in curve.keyframe_points:
                    kp.interpolation = 'LINEAR'
                curve.update()
            has_keys = True

        if not has_keys:
            bpy.data.actions.remove(action)
            continue

        track = arm_obj.animation_data.nla_tracks.new()
        track.name = clip_name
        frame_start = 1
        frame_end = max(frame_start + 1, int(round(action.frame_range[1])))
        strip = track.strips.new(clip_name, frame_start, action)
        strip.action_frame_start = action.frame_range[0]
        strip.action_frame_end = action.frame_range[1]
        strip.frame_start = frame_start
        strip.frame_end = frame_end

    if arm_obj.animation_data.nla_tracks:
        arm_obj.animation_data.action = None


def _load_image_from_bytes(img_name, is_srgb, ext, data):
    """Vytvoří Blender image z raw bytů, zapackuje ji a nastaví color space."""
    existing = bpy.data.images.get(img_name)
    if existing:
        return existing

    with tempfile.NamedTemporaryFile(suffix=ext, delete=False) as tmp:
        tmp.write(data)
        tmp_path = tmp.name

    try:
        img = bpy.data.images.load(tmp_path)
        img.name = img_name
        img.pack()
        img.filepath_raw = ""
    finally:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass

    try:
        img.colorspace_settings.name = 'sRGB' if is_srgb else 'Non-Color'
    except Exception:
        pass

    return img


def _load_drawable_info(adm_path):
    """Parsuje .drawable TOML manifest vedle .adm souboru.

    Vrátí (templates, tex_slots_by_mat) kde:
      templates        = {mat_name: template_str}
      tex_slots_by_mat = {mat_name: {img_name: slot_name}}
    """
    drawable_path = os.path.splitext(adm_path)[0] + '.drawable'
    if not os.path.isfile(drawable_path):
        return {}, {}
    try:
        try:
            import tomllib
        except ImportError:
            try:
                import tomli as tomllib  # pip-installed fallback
            except ImportError:
                return {}, {}
        with open(drawable_path, 'rb') as f:
            data = tomllib.load(f)
        templates = {}
        tex_slots_by_mat = {}
        for mat_name, mat_data in data.get('materials', {}).items():
            if not isinstance(mat_data, dict):
                continue
            if 'template' in mat_data:
                templates[mat_name] = mat_data['template']
            tex_section = mat_data.get('textures')
            if isinstance(tex_section, dict):
                slot_map = {}
                for slot_name, tex_info in tex_section.items():
                    if isinstance(tex_info, dict) and 'name' in tex_info:
                        slot_map[tex_info['name']] = slot_name
                if slot_map:
                    tex_slots_by_mat[mat_name] = slot_map
        return templates, tex_slots_by_mat
    except Exception as e:
        print(f"[adm_import] Warning: nelze načíst .drawable manifest: {e}")
        return {}, {}


def _load_drawable_templates(adm_path):
    """Zpětně kompatibilní obal — vrátí pouze templates dict."""
    templates, _ = _load_drawable_info(adm_path)
    return templates


def _guess_slot(img_name):
    """Odhadne slot z názvu textury pomocí TEXTURE_KEYWORDS."""
    from .constants import TEXTURE_KEYWORDS
    lowered = img_name.lower().replace(" ", "").replace("-", "").replace("_", "")
    for slot_name, keywords in TEXTURE_KEYWORDS.items():
        for kw in keywords:
            if kw in lowered:
                return slot_name
    return None


def _assign_textures_to_material(mat, loaded_images, tex_slot_map=None):
    """Přiřadí embedded textury do bevy_toolkit slotů materiálu.

    tex_slot_map: volitelná mapa {img_name: slot_name} čtená z .drawable manifestu.
    Pokud je zadána, má přednost před keyword-guessing.
    """
    props = getattr(mat, 'bevy_toolkit', None)
    if props is None:
        return
    from .utils import image_basename
    for img_name, img in loaded_images.items():
        if tex_slot_map:
            slot = tex_slot_map.get(img_name) or _guess_slot(img_name)
        else:
            slot = _guess_slot(img_name)
        if not slot:
            continue
        current = getattr(props, f"{slot}_img", None)
        if current is None:
            setattr(props, f"{slot}_img", img)
            name_field = getattr(props, f"{slot}_name", "").strip()
            if not name_field:
                try:
                    setattr(props, f"{slot}_name", image_basename(img))
                except Exception:
                    pass


_SPLIT_RE = re.compile(r'^(.+)\.(\d+)$')


def _merge_material_splits(objects):
    """
    Sloučí sub-objekty vzniklé multi-material exportem (např. 'kostka.1', 'kostka.2')
    zpět do jejich base objektu ('kostka') pomocí Blender join operátoru.
    Base objekt dostane všechny material sloty; sub-objekty jsou odstraněny.
    """
    # Zachytit všechna jména PŘED jakýmikoliv mutacemi — po join() jsou stare
    # bpy reference na smazané objekty neplatné (StructRNA has been removed).
    all_names_ordered = [obj.name for obj in objects]
    mesh_by_name = {obj.name: obj for obj in objects if obj.type == 'MESH'}

    # Skupiny: base_name → seřazený list (index, sub_obj)
    groups = {}
    for obj in objects:
        if obj.type != 'MESH':
            continue
        m = _SPLIT_RE.match(obj.name)
        if not m:
            continue
        base = m.group(1)
        if base not in mesh_by_name:
            continue
        groups.setdefault(base, []).append((int(m.group(2)), obj))

    if not groups:
        return objects

    merged_names = set()
    for base_name, sub_list in groups.items():
        base_obj = mesh_by_name[base_name]
        sub_list.sort(key=lambda x: x[0])
        sub_objs = [o for _, o in sub_list]
        sub_names = {o.name for o in sub_objs}  # uložit před join

        all_objs = [base_obj] + sub_objs
        try:
            with bpy.context.temp_override(
                active_object=base_obj,
                selected_objects=all_objs,
                selected_editable_objects=all_objs,
            ):
                bpy.ops.object.join()
            merged_names.update(sub_names)
        except Exception as exc:
            print(f"[adm_import] merge '{base_name}' failed: {exc}")

    # Lookup přes bpy.data.objects — nevyužívá stale Python reference na smazané objekty
    return [
        bpy.data.objects[name]
        for name in all_names_ordered
        if name not in merged_names and name in bpy.data.objects
    ]


def _set_object_parent_with_local(child, parent, local_matrix):
    """Set OBJECT parent while preserving exporter-authored local matrix exactly."""
    child.parent = parent
    child.parent_type = 'OBJECT'
    child.parent_bone = ""
    child.matrix_parent_inverse = mathutils.Matrix.Identity(4)
    child.matrix_local = local_matrix


def _set_bone_parent_with_local(child, arm_obj, bone_name, local_matrix):
    """Set BONE parent while preserving exporter-authored local matrix exactly."""
    child.parent = arm_obj
    child.parent_type = 'BONE'
    child.parent_bone = bone_name
    child.matrix_parent_inverse = mathutils.Matrix.Identity(4)
    child.matrix_local = local_matrix


def import_adm(filepath, merge_material_splits=False):
    """
    Importuje .adm soubor do aktuální Blender scény.
    Vrátí seznam nově vytvořených bpy.types.Object.
    """
    with open(filepath, 'rb') as f:
        if f.read(4) != b'ADM\x00':
            raise ValueError("Neplatný soubor: špatné magic bytes (očekáváno ADM\\0)")

        version = _u32(f)
        if version not in (1, 2, 3):
            raise ValueError(f"Nepodporovaná verze ADM: {version}")

        mesh_count   = _u32(f)
        node_count   = _u32(f)
        has_textures = _u32(f)

        mesh_data = [_parse_mesh(f) for _ in range(mesh_count)]
        node_data = [_parse_node(f) for _ in range(node_count)]

        # Načti embedded textury z ADM a zapackuj je do Blenderu
        loaded_images = {}
        if has_textures == 1:
            for img_name, is_srgb, ext, data in _parse_textures(f):
                try:
                    img = _load_image_from_bytes(img_name, is_srgb, ext, data)
                    loaded_images[img_name] = img
                    print(f"[adm_import] textura '{img_name}' ({ext[1:]}, sRGB={is_srgb})")
                except Exception as e:
                    print(f"[adm_import] nelze načíst texturu '{img_name}': {e}")

        animations = []
        if version >= 2:
            try:
                animations = _parse_animations_v2(f, version=version)
            except Exception as e:
                print(f"[adm_import] warning: animační sekce je poškozená: {e}")

    # Vytvoř Blender mesh assets
    bl_meshes = []
    for (mname, positions, normals, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices) in mesh_data:
        bl_meshes.append(_build_blender_mesh(
            mname, positions, normals, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices
        ))

    # Vytvoř objekty
    node_objects = []
    created = []
    imported_mats = set()
    collection = bpy.context.collection

    node_local_mats = []
    for (nname, ntype, mesh_idx, parent_idx, mat_name, mat_floats) in node_data:
        bl_mat = _bevy_to_bl_mat4(mat_floats)

        # Bone nody (type=3) reprezentujeme přes skutečnou Blender armaturu,
        # ne přes pomocné Empty objekty.
        if ntype == 3:
            node_objects.append(None)
            node_local_mats.append(bl_mat.copy())
            continue

        if ntype == 0 and 0 <= mesh_idx < len(bl_meshes):
            obj = bpy.data.objects.new(nname, bl_meshes[mesh_idx])
        elif ntype == 1:
            # Pokud collider obsahuje vlastní mesh geometrii, použijeme ji.
            if 0 <= mesh_idx < len(bl_meshes):
                obj = bpy.data.objects.new(nname, bl_meshes[mesh_idx])
            else:
                # Pokud mesh nemá, vytvoříme zatím prázdný "MESH" (ne Empty!),
                # do kterého v operators.py za chvíli vygenerujeme náhradní tvar.
                dummy_mesh = bpy.data.meshes.new(nname + "_mesh")
                obj = bpy.data.objects.new(nname, dummy_mesh)
            obj.bevy_toolkit_obj.is_col = True
        else:
            obj = bpy.data.objects.new(nname, None)

        collection.objects.link(obj)
        obj.matrix_local = bl_mat

        # Přiřaď materiál
        if mat_name and obj.type == 'MESH':
            mat = bpy.data.materials.get(mat_name) or bpy.data.materials.new(mat_name)
            if obj.data.materials:
                obj.data.materials[0] = mat
            else:
                obj.data.materials.append(mat)
            imported_mats.add(mat_name)

        node_objects.append(obj)
        node_local_mats.append(bl_mat.copy())
        created.append(obj)

    # Parent-child vztahy
    for i, (_, _, _, parent_idx, _, _) in enumerate(node_data):
        child = node_objects[i] if i < len(node_objects) else None
        if child is None:
            continue
        if 0 <= parent_idx < len(node_objects):
            parent = node_objects[parent_idx]
            if parent is not None:
                _set_object_parent_with_local(child, parent, node_local_mats[i])

    # Rekonstrukce armatury + skinningu + animací
    arm_obj, joint_bone_names, bone_by_node_index = _build_armature_from_nodes(node_data, collection)
    if arm_obj is not None:
        created.append(arm_obj)

        armature_root_idx = _find_armature_root_index(node_data)
        if armature_root_idx is not None and 0 <= armature_root_idx < len(node_objects):
            placeholder = node_objects[armature_root_idx]
            if placeholder is not None and placeholder != arm_obj:
                root_parent_idx = node_data[armature_root_idx][3]
                if 0 <= root_parent_idx < len(node_objects):
                    root_parent = node_objects[root_parent_idx]
                    if root_parent is not None:
                        _set_object_parent_with_local(arm_obj, root_parent, node_local_mats[armature_root_idx])

                for child_idx, child in enumerate(node_objects):
                    if child is not None and child.parent == placeholder:
                        _set_object_parent_with_local(child, arm_obj, node_local_mats[child_idx])
                if placeholder in created:
                    created.remove(placeholder)
                try:
                    bpy.data.objects.remove(placeholder, do_unlink=True)
                except Exception:
                    pass
            node_objects[armature_root_idx] = arm_obj

        # Objekt parentnutý na bone node přepojíme na skutečný bone parent.
        for i, (_, _, _, parent_idx, _, mat_floats) in enumerate(node_data):
            if i >= len(node_objects):
                continue
            obj = node_objects[i]
            if obj is None:
                continue
            bone_name = bone_by_node_index.get(parent_idx)
            if not bone_name:
                continue
            _set_bone_parent_with_local(obj, arm_obj, bone_name, _bevy_to_bl_mat4(mat_floats))

        _bind_meshes_to_armature(mesh_data, node_data, node_objects, arm_obj, joint_bone_names)
        _import_armature_animations(animations, arm_obj)

    # Načti info z .drawable (šablony + texture slot mapování)
    drawable_templates, drawable_tex_slots = _load_drawable_info(filepath)

    # Přiřaď textury do materiálů importovaných z tohoto ADM.
    # Priorita: .drawable slot mapping → keyword guessing
    if loaded_images:
        for mat_name in imported_mats:
            mat = bpy.data.materials.get(mat_name)
            if mat:
                _assign_textures_to_material(
                    mat, loaded_images,
                    tex_slot_map=drawable_tex_slots.get(mat_name),
                )

    # Vytvoř shader node preview pro každý nově importovaný materiál
    from .material import create_bevy_node_tree
    for mat_name in imported_mats:
        mat = bpy.data.materials.get(mat_name)
        if not mat:
            continue
        props = getattr(mat, 'bevy_toolkit', None)
        if props is not None:
            template = drawable_templates.get(mat_name, 'standard_pbr')
            try:
                props.template = template
            except Exception:
                pass
        create_bevy_node_tree(mat)

    # Oprav vertex atributy na importovaných meshích (přejmenuj COLOR_0 → bevy_masks atd.)
    from .mesh import fix_imported_vertex_attributes
    for obj in created:
        if obj.type == 'MESH' and obj.data:
            fix_imported_vertex_attributes(obj.data)

    # Optional compatibility mode: merge split mesh parts created by multi-material export.
    # Disabled by default so importer reproduces exported hierarchy 1:1.
    if merge_material_splits:
        created = _merge_material_splits(created)

    return created
