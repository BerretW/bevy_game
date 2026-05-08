"""Import .adm binárního formátu zpět do Blenderu."""

import struct
import bpy
import mathutils

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


def _u8(f):   return struct.unpack('<B', f.read(1))[0]
def _u16(f):  return struct.unpack('<H', f.read(2))[0]
def _u32(f):  return struct.unpack('<I', f.read(4))[0]
def _i32(f):  return struct.unpack('<i', f.read(4))[0]
def _str(f):  n = _u16(f); return f.read(n).decode('utf-8')
def _f32s(f, n): return struct.unpack(f'<{n}f', f.read(n * 4))
def _u8s(f, n):  return struct.unpack(f'<{n}B', f.read(n))
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


def _parse_mesh(f):
    name        = _str(f)
    vert_count  = _u32(f)
    index_count = _u32(f)
    attr_flags  = _u32(f)

    positions, normals, uv0s, uv1s, masks0s, masks1s = [], [], [], [], [], []

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

    indices = list(_u32s(f, index_count))
    return name, positions, normals, uv0s, uv1s, masks0s, masks1s, indices


def _parse_node(f):
    name          = _str(f)
    node_type     = _u8(f)
    mesh_index    = _i32(f)
    parent_index  = _i32(f)
    material_name = _str(f)
    mat_floats    = _f32s(f, 16)
    return name, node_type, mesh_index, parent_index, material_name, mat_floats


def _build_blender_mesh(name, positions, normals, uv0s, uv1s, masks0s, masks1s, indices):
    mesh = bpy.data.meshes.new(name)
    tri_count = len(indices) // 3
    faces = [(indices[i*3], indices[i*3+1], indices[i*3+2]) for i in range(tri_count)]
    mesh.from_pydata(positions, [], faces)
    mesh.update()

    # Per-loop custom normals
    if normals:
        loop_nrm = []
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                vi = mesh.loops[li].vertex_index
                loop_nrm.append(normals[vi])
        mesh.normals_split_custom_set(loop_nrm)

    # UV0
    if uv0s:
        ul = mesh.uv_layers.new(name="UVMap")
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                ul.data[li].uv = uv0s[mesh.loops[li].vertex_index]

    # UV1
    if uv1s:
        ul1 = mesh.uv_layers.new(name="UVMap.001")
        for poly in mesh.polygons:
            for li in poly.loop_indices:
                ul1.data[li].uv = uv1s[mesh.loops[li].vertex_index]

    # bevy_masks (první sada vertex colors)
    if masks0s:
        ca = mesh.color_attributes.new(name="bevy_masks", type='FLOAT_COLOR', domain='POINT')
        for vi, c in enumerate(masks0s):
            ca.data[vi].color = c

    # bevy_masks2 (druhá sada vertex colors)
    if masks1s:
        ca2 = mesh.color_attributes.new(name="bevy_masks2", type='FLOAT_COLOR', domain='POINT')
        for vi, c in enumerate(masks1s):
            ca2.data[vi].color = c

    return mesh


def import_adm(filepath):
    """
    Importuje .adm soubor do aktuální Blender scény.
    Vrátí seznam nově vytvořených bpy.types.Object.
    """
    with open(filepath, 'rb') as f:
        if f.read(4) != b'ADM\x00':
            raise ValueError("Neplatný soubor: špatné magic bytes (očekáváno ADM\\0)")

        version = _u32(f)
        if version != 1:
            raise ValueError(f"Nepodporovaná verze ADM: {version}")

        mesh_count   = _u32(f)
        node_count   = _u32(f)
        has_textures = _u32(f)

        mesh_data = [_parse_mesh(f) for _ in range(mesh_count)]
        node_data = [_parse_node(f) for _ in range(node_count)]
        # Embedded DDS textury přeskočíme — v Blenderu se přiřadí z disku přes .drawable

    # Vytvoř Blender mesh assets
    bl_meshes = []
    for (mname, positions, normals, uv0s, uv1s, masks0s, masks1s, indices) in mesh_data:
        bl_meshes.append(_build_blender_mesh(mname, positions, normals, uv0s, uv1s, masks0s, masks1s, indices))

    # Vytvoř objekty
    node_objects = []
    created = []
    collection = bpy.context.collection

    for (nname, ntype, mesh_idx, parent_idx, mat_name, mat_floats) in node_data:
        bl_mat = _bevy_to_bl_mat4(mat_floats)

        if ntype == 0 and 0 <= mesh_idx < len(bl_meshes):
            obj = bpy.data.objects.new(nname, bl_meshes[mesh_idx])
        elif ntype == 1:
            col_name = nname if nname.upper().startswith('COL') else f"COL_{nname}"
            obj = bpy.data.objects.new(col_name, None)
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

        node_objects.append(obj)
        created.append(obj)

    # Parent-child vztahy
    for i, (_, _, _, parent_idx, _, _) in enumerate(node_data):
        if 0 <= parent_idx < len(node_objects):
            node_objects[i].parent = node_objects[parent_idx]

    return created
