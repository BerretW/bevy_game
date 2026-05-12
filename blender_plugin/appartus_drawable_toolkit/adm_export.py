"""Export meshů do binárního .adm formátu pro Apparatus Drawable System."""

import math
import re
import struct
import os
import bpy
import bmesh
import mathutils
from .constants import ATTR_NAME, ATTR_NAME2

# Koordinátová transformace Blender (Z-up) → Bevy (Y-up)
_C = mathutils.Matrix([
    [1,  0,  0, 0],
    [0,  0,  1, 0],
    [0, -1,  0, 0],
    [0,  0,  0, 1],
])
_C_INV = _C.inverted()


def _to_bevy_vec3(v):
    """Konvertuje Blender Vector na Bevy souřadnice (x, z, -y)."""
    return (v.x, v.z, -v.y)


def _to_bevy_mat4(mat):
    """Konvertuje Blender matrix do Bevy coordinate space, vrátí 16 float column-major."""
    m = _C @ mat @ _C_INV
    return [f for col in m.col for f in col]


def _to_bevy_trs(mat):
    """Konvertuje Blender matici na Bevy TRS tuple (pos3, rot4, scale3)."""
    m = _C @ mat @ _C_INV
    loc, rot, scale = m.decompose()
    pos = (loc.x, loc.y, loc.z)
    quat = (rot.x, rot.y, rot.z, rot.w)
    scl = (scale.x, scale.y, scale.z)
    return pos, quat, scl


def _pack_str(s):
    """u16 length prefix + utf8 bytes."""
    b = s.encode('utf-8')
    return struct.pack('<H', len(b)) + b


def _get_color(attr, loop_idx, vert_idx):
    """Vrátí RGBA float tuple z color_attribute (CORNER nebo POINT domain)."""
    if attr.domain == 'CORNER':
        c = attr.data[loop_idx].color
    else:
        c = attr.data[vert_idx].color
    return (c[0], c[1], c[2], c[3] if len(c) > 3 else 1.0)


def _collect_vertex_skinning(obj, vertex_index, bone_index_lookup):
    if not bone_index_lookup or vertex_index >= len(obj.data.vertices):
        return (0, 0, 0, 0), (0.0, 0.0, 0.0, 0.0)

    weighted = []
    for group in obj.data.vertices[vertex_index].groups:
        if group.group >= len(obj.vertex_groups):
            continue
        group_name = obj.vertex_groups[group.group].name
        bone_index = bone_index_lookup.get(group_name)
        if bone_index is None:
            bone_index = bone_index_lookup.get(_canonical_bone_name(group_name))
        if bone_index is None or group.weight <= 0.0:
            continue
        weighted.append((group.weight, bone_index))

    if not weighted:
        return (0, 0, 0, 0), (0.0, 0.0, 0.0, 0.0)

    weighted.sort(key=lambda item: item[0], reverse=True)
    weighted = weighted[:4]
    total = sum(weight for weight, _ in weighted)
    if total <= 0.0:
        return (0, 0, 0, 0), (0.0, 0.0, 0.0, 0.0)

    norm = [(bone_index, weight / total) for weight, bone_index in weighted]
    while len(norm) < 4:
        norm.append((0, 0.0))

    joints = tuple(int(bone_index) for bone_index, _ in norm)
    weights = tuple(float(weight) for _, weight in norm)
    return joints, weights


def _object_uses_armature(obj, armature_object):
    if obj is None or armature_object is None or obj.type != 'MESH' or armature_object.type != 'ARMATURE':
        return False
    if obj.parent == armature_object:
        return True
    for modifier in getattr(obj, 'modifiers', []):
        if modifier.type == 'ARMATURE' and getattr(modifier, 'object', None) == armature_object:
            return True
    return False


def _canonical_bone_name(name):
    token = name.split(':')[-1].strip()
    token = token.replace('.', '_').replace('-', '_').lower()
    if token.startswith('def_'):
        token = token[4:]
    token = re.sub(r'_(l|r)$', r'\1', token)
    token = re.sub(r'[^a-z0-9]+', '', token)
    return token


def _build_bone_index_lookup(ordered_bones):
    lookup = {}

    def register(key, index):
        if not key:
            return
        if key not in lookup:
            lookup[key] = index

    for idx, bone in enumerate(ordered_bones):
        name = bone.name
        register(name, idx)
        register(name.split(':')[-1], idx)
        register(_canonical_bone_name(name), idx)

    return lookup


def _collect_mesh_data(obj, material_index=None, bone_index_lookup=None, bind_matrix=None, disable_armature_modifiers=False):
    """
    Získá mesh data ze object v object-local space.
    Pokud je zadán material_index, exportuje pouze trojúhelníky s daným material slotem.
    Vrátí: (positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices)
    Vše v Bevy souřadnicovém systému.
    """
    depsgraph = bpy.context.evaluated_depsgraph_get()
    mod_restore = []
    if disable_armature_modifiers:
        for modifier in getattr(obj, 'modifiers', []):
            if modifier.type == 'ARMATURE':
                mod_restore.append((modifier, bool(modifier.show_viewport)))
                modifier.show_viewport = False
        if mod_restore:
            bpy.context.view_layer.update()

    obj_eval = obj.evaluated_get(depsgraph)
    mesh = obj_eval.to_mesh()

    # Triangulate
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.triangulate(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()

    # calc_normals_split() removed in Blender 4.1; loop normals are always split
    try:
        mesh.calc_tangents()
    except Exception:
        pass

    uv_layers = mesh.uv_layers
    uv0_layer = None
    uv1_layer = None
    if uv_layers:
        uv0_layer = getattr(uv_layers, "active_render", None)
        if uv0_layer is None:
            uv0_layer = getattr(uv_layers, "active", None)
        if uv0_layer is None:
            for layer in uv_layers:
                if getattr(layer, "active_render", False):
                    uv0_layer = layer
                    break
        if uv0_layer is None:
            uv0_layer = uv_layers[0]
        for layer in uv_layers:
            if layer != uv0_layer:
                uv1_layer = layer
                break

    col_attrs = getattr(mesh, 'color_attributes', [])
    masks0_attr = None
    masks1_attr = None
    for ca in col_attrs:
        if ca.name == ATTR_NAME:
            masks0_attr = ca
        elif ca.name == ATTR_NAME2:
            masks1_attr = ca
    if masks0_attr is None:
        for ca in col_attrs:
            if ca is not masks1_attr:
                masks0_attr = ca
                break
    if masks1_attr is None:
        for ca in col_attrs:
            if ca is not masks0_attr:
                masks1_attr = ca
                break

    # Pre-fetch all mesh data in bulk — vastly faster than per-element RNA access
    loop_count = len(mesh.loops)
    vert_count = len(mesh.vertices)

    vert_cos = [0.0] * (vert_count * 3)
    mesh.vertices.foreach_get('co', vert_cos)

    loop_vert_idx = [0] * loop_count
    mesh.loops.foreach_get('vertex_index', loop_vert_idx)

    loop_nrms = [0.0] * (loop_count * 3)
    mesh.loops.foreach_get('normal', loop_nrms)

    has_tangents = loop_count > 0 and hasattr(mesh.loops[0], 'tangent')
    loop_tans = loop_bsigns = None
    if has_tangents:
        loop_tans   = [0.0] * (loop_count * 3)
        loop_bsigns = [0.0] * loop_count
        mesh.loops.foreach_get('tangent', loop_tans)
        mesh.loops.foreach_get('bitangent_sign', loop_bsigns)

    uv0_data = uv1_data = None
    if uv0_layer:
        uv0_data = [0.0] * (loop_count * 2)
        uv0_layer.data.foreach_get('uv', uv0_data)
    if uv1_layer:
        uv1_data = [0.0] * (loop_count * 2)
        uv1_layer.data.foreach_get('uv', uv1_data)

    # Color attributes: flat RGBA arrays indexed by loop or vertex
    def _prefetch_colors(attr):
        if attr is None:
            return None
        n = loop_count if attr.domain == 'CORNER' else vert_count
        flat = [0.0] * (n * 4)
        attr.data.foreach_get('color', flat)
        return (flat, attr.domain)

    masks0_colors = _prefetch_colors(masks0_attr)
    masks1_colors = _prefetch_colors(masks1_attr)

    def _sample_color(colors_tuple, loop_idx, vi):
        if colors_tuple is None:
            return (0, 0, 0, 0)
        flat, domain = colors_tuple
        base = (loop_idx if domain == 'CORNER' else vi) * 4
        return tuple(min(255, int(flat[base + k] * 255)) for k in range(4))

    # Pre-fetch triangle data
    mesh.calc_loop_triangles()
    num_tris = len(mesh.loop_triangles)
    tri_mat_idx   = [0]     * num_tris
    tri_loops_flat = [0]    * (num_tris * 3)
    tri_use_smooth = [False] * num_tris
    tri_nrms_flat  = [0.0]  * (num_tris * 3)
    mesh.loop_triangles.foreach_get('material_index', tri_mat_idx)
    mesh.loop_triangles.foreach_get('loops',          tri_loops_flat)
    mesh.loop_triangles.foreach_get('use_smooth',     tri_use_smooth)
    mesh.loop_triangles.foreach_get('normal',         tri_nrms_flat)

    positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices = [], [], [], [], [], [], [], [], [], []
    vert_map = {}
    normal_matrix = None
    if bind_matrix is not None:
        normal_matrix = bind_matrix.to_3x3().inverted().transposed()

    for ti in range(num_tris):
        if material_index is not None and tri_mat_idx[ti] != material_index:
            continue
        use_smooth = tri_use_smooth[ti]
        tnx = tri_nrms_flat[ti * 3];  tny = tri_nrms_flat[ti * 3 + 1];  tnz = tri_nrms_flat[ti * 3 + 2]

        for k in range(3):
            loop_idx = tri_loops_flat[ti * 3 + k]
            vi = loop_vert_idx[loop_idx]

            if use_smooth:
                nx = loop_nrms[loop_idx * 3];  ny = loop_nrms[loop_idx * 3 + 1];  nz = loop_nrms[loop_idx * 3 + 2]
            else:
                nx, ny, nz = tnx, tny, tnz
            nrm_key = (round(nx, 4), round(ny, 4), round(nz, 4))

            if uv0_data:
                u0 = uv0_data[loop_idx * 2];  v0 = 1.0 - uv0_data[loop_idx * 2 + 1]
                uv0_key = (round(u0, 5), round(v0, 5))
            else:
                uv0_key = (0.0, 0.0)

            if uv1_data:
                u1 = uv1_data[loop_idx * 2];  v1 = 1.0 - uv1_data[loop_idx * 2 + 1]
                uv1_key = (round(u1, 5), round(v1, 5))
            else:
                uv1_key = (0.0, 0.0)

            key = (vi, nrm_key, uv0_key, uv1_key)
            if key not in vert_map:
                new_idx = len(positions)
                vert_map[key] = new_idx

                px = vert_cos[vi * 3];  py = vert_cos[vi * 3 + 1];  pz = vert_cos[vi * 3 + 2]
                if bind_matrix is not None:
                    pos = bind_matrix @ mathutils.Vector((px, py, pz))
                    nrm = (normal_matrix @ mathutils.Vector((nx, ny, nz))).normalized()
                else:
                    pos = mathutils.Vector((px, py, pz))
                    nrm = mathutils.Vector((nx, ny, nz))

                positions.append((pos.x, pos.z, -pos.y))   # Blender→Bevy Y-up
                normals.append((nrm.x, nrm.z, -nrm.y))

                if has_tangents:
                    tx = loop_tans[loop_idx * 3];  ty = loop_tans[loop_idx * 3 + 1];  tz = loop_tans[loop_idx * 3 + 2]
                    if bind_matrix is not None:
                        tan = (normal_matrix @ mathutils.Vector((tx, ty, tz))).normalized()
                        tangents.append((tan.x, tan.z, -tan.y, loop_bsigns[loop_idx]))
                    else:
                        tangents.append((tx, tz, -ty, loop_bsigns[loop_idx]))
                else:
                    tangents.append((1.0, 0.0, 0.0, 1.0))

                uv0s.append(uv0_key)
                uv1s.append((uv1_data[loop_idx * 2], 1.0 - uv1_data[loop_idx * 2 + 1]) if uv1_data else (0.0, 0.0))
                masks0s.append(_sample_color(masks0_colors, loop_idx, vi))
                masks1s.append(_sample_color(masks1_colors, loop_idx, vi))
                joints, weights = _collect_vertex_skinning(obj, vi, bone_index_lookup)
                joint_indices.append(joints)
                joint_weights.append(weights)

            indices.append(vert_map[key])

    obj_eval.to_mesh_clear()

    if mod_restore:
        for modifier, old_show in mod_restore:
            modifier.show_viewport = old_show
        bpy.context.view_layer.update()

    return positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices


def _write_mesh(buf, name, positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices):
    has_pos = len(positions) > 0
    has_nrm = len(normals) > 0
    has_tan = len(tangents) > 0
    has_uv0 = len(uv0s) > 0
    has_uv1 = len(uv1s) > 0
    has_m0  = any(any(c != 0 for c in m) for m in masks0s)
    has_m1  = any(any(c != 0 for c in m) for m in masks1s)
    has_weights = any(any(v != 0.0 for v in weight) for weight in joint_weights)
    has_joints = len(joint_indices) > 0 and has_weights

    flags = 0
    if has_pos: flags |= (1 << 0)
    if has_nrm: flags |= (1 << 1)
    if has_tan: flags |= (1 << 2)
    if has_uv0: flags |= (1 << 3)
    if has_uv1: flags |= (1 << 4)
    if has_m0:  flags |= (1 << 5)
    if has_m1:  flags |= (1 << 6)
    if has_joints: flags |= (1 << 7)
    if has_weights: flags |= (1 << 8)

    buf += _pack_str(name)
    buf += struct.pack('<III', len(positions), len(indices), flags)

    # Batch-pack entire arrays at once — avoids O(n²) buffer copies from per-vertex +=
    if has_pos:
        buf += struct.pack(f'<{len(positions) * 3}f', *[v for p in positions for v in p])
    if has_nrm:
        buf += struct.pack(f'<{len(normals) * 3}f',   *[v for n in normals   for v in n])
    if has_tan:
        buf += struct.pack(f'<{len(tangents) * 4}f',  *[v for t in tangents  for v in t])
    if has_uv0:
        buf += struct.pack(f'<{len(uv0s) * 2}f',      *[v for u in uv0s      for v in u])
    if has_uv1:
        buf += struct.pack(f'<{len(uv1s) * 2}f',      *[v for u in uv1s      for v in u])
    if has_m0:
        buf += struct.pack(f'{len(masks0s) * 4}B',    *[v for m in masks0s   for v in m])
    if has_m1:
        buf += struct.pack(f'{len(masks1s) * 4}B',    *[v for m in masks1s   for v in m])
    if has_joints:
        buf += struct.pack(f'<{len(joint_indices) * 4}H', *[v for joint in joint_indices for v in joint])
    if has_weights:
        buf += struct.pack(f'<{len(joint_weights) * 4}f', *[v for weight in joint_weights for v in weight])

    buf += struct.pack(f'<{len(indices)}I', *indices)
    return buf


def _write_node(buf, name, node_type_byte, mesh_index, parent_index, material_name, mat_bevy):
    buf += _pack_str(name)
    buf += struct.pack('<b', node_type_byte)
    buf += struct.pack('<i', mesh_index)
    buf += struct.pack('<i', parent_index)
    buf += _pack_str(material_name)
    buf += struct.pack('<16f', *mat_bevy)
    return buf


def _write_animations(buf, clips):
    """Zapíše animační sekci ADM v4.

    Formát:
      u32 clip_count
      for clip:
        str name
        f32 duration
        u32 track_count
        for track:
          str node_name
                u32 flags
          u32 key_count
          for key:
            f32 time
            vec3 pos
            quat rot_xyzw
            vec3 scale
                u32 notify_count
                for notify:
                    f32 time
                    str name
    """
    buf += struct.pack('<I', len(clips))
    for clip in clips:
        buf += _pack_str(clip['name'])
        buf += struct.pack('<f', clip['duration'])
        tracks = clip['tracks']
        buf += struct.pack('<I', len(tracks))
        for tr in tracks:
            buf += _pack_str(tr['node'])
            buf += struct.pack('<I', tr.get('flags', 1))
            keys = tr['keys']
            buf += struct.pack('<I', len(keys))
            for key in keys:
                t = key['time']
                p = key['pos']
                r = key['rot']
                s = key['scale']
                buf += struct.pack(
                    '<f3f4f3f',
                    t,
                    p[0], p[1], p[2],
                    r[0], r[1], r[2], r[3],
                    s[0], s[1], s[2],
                )
        notifies = clip.get('notifies', [])
        buf += struct.pack('<I', len(notifies))
        for notify in notifies:
            buf += struct.pack('<f', float(notify['time']))
            buf += _pack_str(str(notify['name']))
    return buf


def _write_animation_dictionaries(buf, dictionaries):
    """Zapíše animation dictionaries sekci pro ADM v5.

    Formát:
      u32 dict_count
      for dict:
        str name
        u32 clip_count
        u32 clip_index[clip_count]
    """
    buf += struct.pack('<I', len(dictionaries))
    for dictionary in dictionaries:
        buf += _pack_str(dictionary['name'])
        clip_indices = dictionary['clip_indices']
        buf += struct.pack('<I', len(clip_indices))
        for clip_index in clip_indices:
            buf += struct.pack('<I', int(clip_index))
    return buf


def _collect_animation_dictionaries_from_scene():
    scene = getattr(bpy.context, 'scene', None)
    if scene is None:
        return []
    settings = getattr(scene, 'bevy_toolkit_export', None)
    if settings is None or not hasattr(settings, 'animation_dictionaries'):
        return []

    dictionaries = []
    for dict_item in settings.animation_dictionaries:
        dict_name = bpy.path.clean_name((dict_item.name or '').strip())
        if not dict_name:
            continue
        clips = []
        seen = set()
        for clip in dict_item.clips:
            clip_name = (clip.clip_name or '').strip()
            if not clip_name or clip_name in seen:
                continue
            seen.add(clip_name)
            clips.append(clip_name)
        dictionaries.append({
            'name': dict_name,
            'clips': clips,
        })
    return dictionaries


def _bone_track_flags(node_name):
    name = node_name.split(':')[-1]
    low = name.lower()

    if low.startswith('def_'):
        low = low[4:]

    side = None
    if low.endswith('_r'):
        side = 'right'
        low = low[:-2]
    elif low.endswith('_l'):
        side = 'left'
        low = low[:-2]

    if low in ('hips', 'pelvis', 'root'):
        return 8064
    if 'spine' in low or 'chest' in low or 'neck' in low or low == 'head' or low in ('clavicle', 'shoulder'):
        return 24576

    if side == 'right':
        if 'clavicle' in low or 'shoulder' in low:
            return 14
        if 'arm' in low:
            return 14
        if 'hand' in low or 'finger' in low or 'thumb' in low:
            return 14
        if 'forearm' in low or 'lowerarm' in low:
            return 14
        if 'upleg' in low or 'thigh' in low or 'leg' in low or 'calf' in low or 'shin' in low or 'foot' in low or 'toe' in low:
            return 8064

    if side == 'left':
        if 'clavicle' in low or 'shoulder' in low:
            return 112
        if 'arm' in low:
            return 112
        if 'hand' in low or 'finger' in low or 'thumb' in low:
            return 112
        if 'forearm' in low or 'lowerarm' in low:
            return 112
        if 'upleg' in low or 'thigh' in low or 'leg' in low or 'calf' in low or 'shin' in low or 'foot' in low or 'toe' in low:
            return 8064

    if 'arm' in low:
        return 14
    if 'leg' in low or 'thigh' in low or 'calf' in low or 'shin' in low or 'foot' in low or 'toe' in low:
        return 8064
    if 'spine' in low or 'chest' in low or 'neck' in low or low == 'head':
        return 24576
    if 'clavicle' in low or 'shoulder' in low:
        return 112

    return 1


def _ordered_bones(armature_obj):
    bones = list(armature_obj.data.bones)
    ordered = []
    seen = set()

    def visit(bone):
        if bone.name in seen:
            return
        if bone.parent is not None:
            visit(bone.parent)
        seen.add(bone.name)
        ordered.append(bone)

    for bone in bones:
        visit(bone)

    return ordered


def _bone_rest_local_matrix(bone):
    if bone.parent is None:
        return bone.matrix_local.copy()
    return bone.parent.matrix_local.inverted() @ bone.matrix_local


def _bone_pose_local_matrix(pose_bone):
    if pose_bone.parent is None:
        return pose_bone.matrix.copy()
    return pose_bone.parent.matrix.inverted() @ pose_bone.matrix


def _build_armature_clip_specs(scene, armature_obj):
    animation_data = getattr(armature_obj, 'animation_data', None)
    if not animation_data:
        return []

    clip_specs = []
    for track in animation_data.nla_tracks:
        if getattr(track, 'mute', False):
            continue
        for strip in track.strips:
            action = getattr(strip, 'action', None)
            if action is None or getattr(strip, 'mute', False):
                continue
            clip_name = strip.name.strip() or action.name.strip() or armature_obj.name
            frame_start = float(getattr(strip, 'action_frame_start', action.frame_range[0]))
            frame_end = float(getattr(strip, 'action_frame_end', action.frame_range[1]))
            clip_specs.append((clip_name, action, frame_start, frame_end))

    action = getattr(animation_data, 'action', None)
    if not clip_specs and action is not None:
        clip_name = action.name.strip() or armature_obj.name
        clip_specs.append((clip_name, action, float(action.frame_range[0]), float(action.frame_range[1])))

    return clip_specs


def _collect_action_notifies(action, frame_start, frame_end, fps):
    notifies = []
    if action is None:
        return notifies

    for marker in getattr(action, 'pose_markers', []):
        marker_frame = float(getattr(marker, 'frame', 0.0))
        if marker_frame < frame_start or marker_frame > frame_end:
            continue
        time_sec = max(0.0, (marker_frame - frame_start) / max(1e-6, fps))
        name = getattr(marker, 'name', '').strip()
        if not name:
            continue
        notifies.append({'time': float(time_sec), 'name': name})

    notifies.sort(key=lambda item: item['time'])
    return notifies


def _collect_armature_animation(armature_obj):
    scene = bpy.context.scene
    fps = scene.render.fps / max(1.0, float(scene.render.fps_base))
    if fps <= 0.0:
        fps = 30.0

    animation_data = getattr(armature_obj, 'animation_data', None)
    if not animation_data:
        return []

    clip_specs = _build_armature_clip_specs(scene, armature_obj)
    if not clip_specs:
        return []

    bones = _ordered_bones(armature_obj)
    old_frame = scene.frame_current
    old_action = getattr(animation_data, 'action', None)
    old_use_nla = getattr(animation_data, 'use_nla', None)
    old_track_mutes = [bool(getattr(track, 'mute', False)) for track in animation_data.nla_tracks]
    clips = []

    try:
        for track in animation_data.nla_tracks:
            track.mute = True
        if old_use_nla is not None:
            animation_data.use_nla = False

        for clip_name, action, frame_start, frame_end in clip_specs:
            animation_data.action = action
            start_frame = int(math.floor(frame_start))
            end_frame = max(start_frame + 1, int(math.ceil(frame_end)))
            tracks = []
            notifies = _collect_action_notifies(action, frame_start, frame_end, fps)

            for bone in bones:
                pose_bone = armature_obj.pose.bones.get(bone.name)
                if pose_bone is None:
                    continue

                keys = []
                for frame in range(start_frame, end_frame + 1):
                    scene.frame_set(frame)
                    bpy.context.view_layer.update()
                    pose_bone = armature_obj.pose.bones.get(bone.name)
                    if pose_bone is None:
                        continue
                    pos, rot, scale = _to_bevy_trs(_bone_pose_local_matrix(pose_bone))
                    keys.append({
                        'time': float((frame - start_frame) / fps),
                        'pos': pos,
                        'rot': rot,
                        'scale': scale,
                    })

                if not keys:
                    continue

                first = keys[0]
                varying = any(
                    (k['pos'] != first['pos']) or
                    (k['rot'] != first['rot']) or
                    (k['scale'] != first['scale'])
                    for k in keys[1:]
                )
                if varying or len(keys) == 1:
                    tracks.append({'node': bone.name, 'flags': _bone_track_flags(bone.name), 'keys': keys})

            if tracks:
                duration = max(0.0, (end_frame - start_frame) / fps)
                clips.append({
                    'name': clip_name,
                    'duration': float(duration),
                    'tracks': tracks,
                    'notifies': notifies,
                })
    finally:
        scene.frame_set(old_frame)
        animation_data.action = old_action
        if old_use_nla is not None:
            animation_data.use_nla = old_use_nla
        for track, old_mute in zip(animation_data.nla_tracks, old_track_mutes):
            track.mute = old_mute
        bpy.context.view_layer.update()

    return clips


def _collect_scene_animation(objects):
    """Vrátí 0 nebo 1 clip se sampled keyframes z frame range aktuální scény.

    Exportuje objektové transform animace pro všechny exportované objekty,
    které mají animation_data (action/NLA). Název clipu: 'scene'.
    """
    if not objects:
        return []

    scene = bpy.context.scene
    animated = [o for o in objects if getattr(o, 'animation_data', None)]
    if not animated:
        return []

    frame_start = int(scene.frame_start)
    frame_end = int(scene.frame_end)
    if frame_end < frame_start:
        return []

    fps = scene.render.fps / max(1.0, float(scene.render.fps_base))
    if fps <= 0.0:
        fps = 30.0

    old_frame = scene.frame_current
    tracks = []
    try:
        for obj in animated:
            keys = []
            for frame in range(frame_start, frame_end + 1):
                scene.frame_set(frame)
                pos, rot, scale = _to_bevy_trs(obj.matrix_local)
                t = (frame - frame_start) / fps
                keys.append({
                    'time': float(t),
                    'pos': pos,
                    'rot': rot,
                    'scale': scale,
                })

            # Vyhoď konstantní tracky (1 klíč nebo všechny klíče stejné)
            if not keys:
                continue
            first = keys[0]
            varying = any(
                (k['pos'] != first['pos']) or
                (k['rot'] != first['rot']) or
                (k['scale'] != first['scale'])
                for k in keys[1:]
            )
            if varying or len(keys) == 1:
                tracks.append({'node': obj.name, 'flags': 1, 'keys': keys})
    finally:
        scene.frame_set(old_frame)

    if not tracks:
        return []

    duration = max(0.0, (frame_end - frame_start) / fps)
    return [{
        'name': 'scene',
        'duration': float(duration),
        'tracks': tracks,
        'notifies': [],
    }]


def _get_image_bytes(img):
    """Přečte DDS texturu — vždy formát DDS (format_byte=2).

    is_srgb: čteme z Blenderova colorspace_settings — sRGB = 1, Non-Color/Linear = 0.
    Bevy DDS loader ignoruje DXGI format v headeru a řídí se výhradně is_srgb bytem.
    1. Packed file → img.packed_file.data přímo
    2. Soubor na disku → přečte raw bytes
    """
    cs = getattr(getattr(img, 'colorspace_settings', None), 'name', 'sRGB')
    is_srgb = cs == 'sRGB'

    # 1. Packed v .blend
    if img.packed_file:
        return 2, is_srgb, bytes(img.packed_file.data)

    # 2. Disk
    abspath = bpy.path.abspath(img.filepath_raw)
    if abspath and os.path.isfile(abspath):
        with open(abspath, 'rb') as f:
            return 2, is_srgb, f.read()

    raise RuntimeError(f"Textura '{img.name}' není na disku ani packed")


def export_adm(filepath, objects=None, export_textures=True, armature_object=None, animation_dictionaries=None):
    """
    Exportuje objekty do .adm souboru.
    objects: seznam bpy.types.Object; None = všechny mesh objekty ve scéně.
    """
    if objects is None:
        objects = [o for o in bpy.context.scene.objects if o.type == 'MESH']

    if armature_object is not None and armature_object not in objects:
        objects = list(objects) + [armature_object]

    # Seřaď podle hierarchie (parents před children)
    def sort_key(o):
        depth = 0
        p = o.parent
        while p:
            depth += 1
            p = p.parent
        return depth
    objects = sorted(objects, key=sort_key)

    ordered_bones = []
    bone_weight_index_lookup = {}
    if armature_object is not None and armature_object.type == 'ARMATURE':
        ordered_bones = _ordered_bones(armature_object)
        bone_weight_index_lookup = _build_bone_index_lookup(ordered_bones)

    # Unikátní meshe (může více objektů sdílet stejný mesh data)
    mesh_data_list = []  # list of (name, positions, ...) tuples
    mesh_index_map = {}  # object.data.name → mesh_index

    node_list = []  # list of (name, type_byte, mesh_idx, parent_idx, mat_name, transform16)
    obj_to_node = {}  # object → node_index
    original_local_by_obj = {}
    exported_local_by_obj = {}
    pending_bone_parents = []  # list of (node_idx, bone_name)

    for obj in objects:
        # Detekce node type
        name_up = obj.name.upper()
        if obj.type == 'ARMATURE':
            node_type = 2  # EMPTY (armature root)
        elif 'COL_' in name_up or name_up.startswith('COL'):
            node_type = 1  # COLLISION
        else:
            node_type = 0  # MESH

        original_local = obj.matrix_local.copy()
        adjusted_local = original_local.copy()
        is_bone_parent = (
            armature_object is not None
            and obj.parent == armature_object
            and getattr(obj, 'parent_type', '') == 'BONE'
            and bool(getattr(obj, 'parent_bone', ''))
        )
        bone_parent_name = getattr(obj, 'parent_bone', '') if is_bone_parent else ''

        if is_bone_parent:
            bone = armature_object.data.bones.get(bone_parent_name)
            if bone is not None:
                # Object parented to a bone uses rest-bone space as local parent space.
                adjusted_local = bone.matrix_local.inverted() @ original_local
        elif obj.parent and obj.parent in exported_local_by_obj:
            parent_original = original_local_by_obj[obj.parent]
            parent_exported = exported_local_by_obj[obj.parent]
            adjusted_local = parent_exported.inverted() @ parent_original @ original_local

        # Parent node index
        parent_idx = -1
        if obj.parent and obj.parent in obj_to_node:
            parent_idx = obj_to_node[obj.parent]

        skinned_mesh = node_type in (0, 1) and _object_uses_armature(obj, armature_object)
        bind_matrix = adjusted_local.copy() if skinned_mesh else None
        exported_local = mathutils.Matrix.Identity(4) if skinned_mesh else adjusted_local
        mat_bevy = _to_bevy_mat4(exported_local)

        if node_type == 0 and len(obj.material_slots) > 1:
            # Multi-material mesh: jeden node per material slot.
            # Pojmenování: slot 0 → obj.name, slot N → "obj.name.N"
            # Tečkový suffix zajišťuje, že base_name() regex \.\d+$ funguje při re-importu.
            first_node_idx = None
            for mat_idx, slot in enumerate(obj.material_slots):
                mat_name = slot.material.name if slot.material else ''
                sub_name = obj.name if mat_idx == 0 else f"{obj.name}.{mat_idx}"

                if skinned_mesh:
                    mesh_key = f"__skinned_mat_{obj.name}_{mat_idx}"
                else:
                    mesh_key = f"__mat_{obj.data.name if obj.data else obj.name}_{mat_idx}"
                if mesh_key not in mesh_index_map:
                    data = _collect_mesh_data(
                        obj,
                        material_index=mat_idx,
                        bone_index_lookup=bone_weight_index_lookup,
                        bind_matrix=bind_matrix,
                        disable_armature_modifiers=skinned_mesh,
                    )
                    if not data[0]:  # žádné trojúhelníky nepoužívají tento material slot
                        continue
                    mesh_index_map[mesh_key] = len(mesh_data_list)
                    mesh_data_list.append((sub_name, *data))

                if mesh_key not in mesh_index_map:
                    continue  # byl přeskočen (prázdný slot)

                mesh_idx = mesh_index_map[mesh_key]
                node_idx = len(node_list)
                if first_node_idx is None:
                    first_node_idx = node_idx
                node_list.append((sub_name, node_type, mesh_idx, parent_idx, mat_name, mat_bevy))
                if is_bone_parent and bone_parent_name and not skinned_mesh:
                    pending_bone_parents.append((node_idx, bone_parent_name))

            if first_node_idx is not None:
                obj_to_node[obj] = first_node_idx
                original_local_by_obj[obj] = original_local
                exported_local_by_obj[obj] = exported_local
        else:
            # Single material nebo COLLISION
            mat_name = obj.active_material.name if obj.active_material else ''

            mesh_idx = -1
            
            # Zjistíme, zda potřebujeme exportovat reálnou 3D geometrii
            needs_mesh = False
            if node_type == 0: # Vizuální mesh
                needs_mesh = True
            elif node_type == 1: # Kolizní mesh
                shape = obj.bevy_toolkit_obj.col_shape
                if shape in ("MESH", "CONVEX", "NAVMESH"):
                    needs_mesh = True

            if needs_mesh:
                key = obj.name if skinned_mesh else (obj.data.name if obj.data else obj.name)
                if key not in mesh_index_map:
                    mesh_index_map[key] = len(mesh_data_list)
                    data = _collect_mesh_data(
                        obj,
                        bone_index_lookup=bone_weight_index_lookup,
                        bind_matrix=bind_matrix,
                        disable_armature_modifiers=skinned_mesh,
                    )
                    mesh_data_list.append((obj.name, *data))
                mesh_idx = mesh_index_map[key]

            node_idx = len(node_list)
            obj_to_node[obj] = node_idx
            node_list.append((obj.name, node_type, mesh_idx, parent_idx, mat_name, mat_bevy))
            if is_bone_parent and bone_parent_name and not skinned_mesh:
                pending_bone_parents.append((node_idx, bone_parent_name))
            original_local_by_obj[obj] = original_local
            exported_local_by_obj[obj] = exported_local

    bone_index_map = {}
    if armature_object is not None and armature_object.type == 'ARMATURE':
        armature_node_idx = obj_to_node.get(armature_object)
        if armature_node_idx is not None:
            for bone in ordered_bones:
                parent_idx = armature_node_idx
                if bone.parent is not None:
                    parent_idx = bone_index_map.get(bone.parent.name, armature_node_idx)
                bone_idx = len(node_list)
                node_list.append((bone.name, 3, -1, parent_idx, '', _to_bevy_mat4(_bone_rest_local_matrix(bone))))
                bone_index_map[bone.name] = bone_idx

            # Rebind objects parented to bones from armature root to concrete bone nodes.
            for node_idx, bone_name in pending_bone_parents:
                resolved_parent = bone_index_map.get(bone_name, armature_node_idx)
                name, type_b, mesh_idx, _, mat_name, mat16 = node_list[node_idx]
                node_list[node_idx] = (name, type_b, mesh_idx, resolved_parent, mat_name, mat16)

    # Sbíráme embedded textury (ze všech materiálů s bevy_toolkit props)
    # V ADM se balí VŠECHNY přiřazené textury bez ohledu na _embedded flag
    # (_embedded flag řídí sdílené DDS textury v .drawable workflow, ne ADM)
    embedded_textures = {}  # img_name → (format, is_srgb, bytes)
    if export_textures:
        from .utils import image_basename

        # Explicitní seznam _img slotů z BevyMaterialProps
        IMG_SLOTS = (
            'albedo_img', 'mrao_img', 'normal_img', 'palette_img', 'snow_img',
            'l0_albedo_img', 'l0_mrao_img', 'l0_normal_img',
            'l1_albedo_img', 'l1_mrao_img', 'l1_normal_img',
            'glass_albedo_img', 'shatter_map_img',
            'ma_img', 'mb_img',
        )

        seen_mats = set()
        for obj in objects:
            for slot in obj.material_slots:
                mat = slot.material
                if not mat or mat.name in seen_mats:
                    continue
                seen_mats.add(mat.name)
                props = getattr(mat, 'bevy_toolkit', None)
                if props is None:
                    continue
                for attr in IMG_SLOTS:
                    img = getattr(props, attr, None)
                    if img is None:
                        continue
                    img_name = image_basename(img)
                    if img_name and img_name not in embedded_textures:
                        try:
                            embedded_textures[img_name] = _get_image_bytes(img)
                            print(f"[adm_export] embed '{img_name}'")
                        except Exception as e:
                            print(f"[adm_export] nelze exportovat texturu '{img_name}': {e}")

    # Animace (ADM v2)
    clips = []
    if armature_object is not None and armature_object.type == 'ARMATURE':
        clips = _collect_armature_animation(armature_object)
    if not clips:
        clips = _collect_scene_animation(objects)

    if animation_dictionaries is None:
        animation_dictionaries = _collect_animation_dictionaries_from_scene()

    clip_index_by_name = {clip['name']: idx for idx, clip in enumerate(clips)}
    resolved_dictionaries = []
    for dictionary in animation_dictionaries:
        dict_name = bpy.path.clean_name((dictionary.get('name', '') or '').strip())
        if not dict_name:
            continue
        clip_indices = []
        seen = set()
        for clip_name in dictionary.get('clips', []):
            idx = clip_index_by_name.get(clip_name)
            if idx is None:
                print(f"[adm_export] warning: dictionary '{dict_name}' references unknown clip '{clip_name}'")
                continue
            if idx in seen:
                continue
            seen.add(idx)
            clip_indices.append(idx)
        if clip_indices:
            resolved_dictionaries.append({'name': dict_name, 'clip_indices': clip_indices})

    adm_version = 5 if resolved_dictionaries else 4

    # Build binary — bytearray is mutable so += never copies the whole buffer
    buf = bytearray()

    # Header
    buf += b'ADM\x00'
    buf += struct.pack('<I', adm_version)
    buf += struct.pack('<I', len(mesh_data_list))
    buf += struct.pack('<I', len(node_list))
    buf += struct.pack('<I', 1 if embedded_textures else 0)

    # Mesh sekce
    for entry in mesh_data_list:
        name, positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices = entry
        buf = _write_mesh(buf, name, positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, joint_indices, joint_weights, indices)

    # Node sekce
    for (name, type_b, mesh_idx, parent_idx, mat_name, mat16) in node_list:
        buf = _write_node(buf, name, type_b, mesh_idx, parent_idx, mat_name, mat16)

    # Texture sekce
    if embedded_textures:
        buf += struct.pack('<I', len(embedded_textures))
        for img_name, (fmt, is_srgb, data) in embedded_textures.items():
            buf += _pack_str(img_name)
            buf += struct.pack('BB', fmt, 1 if is_srgb else 0)
            buf += struct.pack('<I', len(data))
            buf += data

    # Animation sekce (v4)
    buf = _write_animations(buf, clips)

    # Animation dictionaries sekce (v5)
    if resolved_dictionaries:
        buf = _write_animation_dictionaries(buf, resolved_dictionaries)

    with open(filepath, 'wb') as f:
        f.write(buf)

        print(f"[adm_export] zapsáno {len(mesh_data_list)} meshů, {len(node_list)} uzlů, "
            f"{len(embedded_textures)} embedded textur, {len(clips)} clipů, "
            f"{len(resolved_dictionaries)} dictionary → {filepath}")
    return len(mesh_data_list), len(node_list)
