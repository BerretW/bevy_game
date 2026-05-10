"""Export meshů do binárního .adm formátu pro Apparatus Drawable System."""

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


def _collect_mesh_data(obj, material_index=None):
    """
    Získá mesh data ze object v object-local space.
    Pokud je zadán material_index, exportuje pouze trojúhelníky s daným material slotem.
    Vrátí: (positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, indices)
    Vše v Bevy souřadnicovém systému.
    """
    depsgraph = bpy.context.evaluated_depsgraph_get()
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

    positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, indices = [], [], [], [], [], [], [], []
    vert_map = {}

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
                positions.append((px, pz, -py))   # Blender→Bevy Y-up
                normals.append((nx, nz, -ny))

                if has_tangents:
                    tx = loop_tans[loop_idx * 3];  ty = loop_tans[loop_idx * 3 + 1];  tz = loop_tans[loop_idx * 3 + 2]
                    tangents.append((tx, tz, -ty, loop_bsigns[loop_idx]))
                else:
                    tangents.append((1.0, 0.0, 0.0, 1.0))

                uv0s.append(uv0_key)
                uv1s.append((uv1_data[loop_idx * 2], 1.0 - uv1_data[loop_idx * 2 + 1]) if uv1_data else (0.0, 0.0))
                masks0s.append(_sample_color(masks0_colors, loop_idx, vi))
                masks1s.append(_sample_color(masks1_colors, loop_idx, vi))

            indices.append(vert_map[key])

    obj_eval.to_mesh_clear()
    return positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, indices


def _write_mesh(buf, name, positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, indices):
    has_pos = len(positions) > 0
    has_nrm = len(normals) > 0
    has_tan = len(tangents) > 0
    has_uv0 = len(uv0s) > 0
    has_uv1 = len(uv1s) > 0
    has_m0  = any(any(c != 0 for c in m) for m in masks0s)
    has_m1  = any(any(c != 0 for c in m) for m in masks1s)

    flags = 0
    if has_pos: flags |= (1 << 0)
    if has_nrm: flags |= (1 << 1)
    if has_tan: flags |= (1 << 2)
    if has_uv0: flags |= (1 << 3)
    if has_uv1: flags |= (1 << 4)
    if has_m0:  flags |= (1 << 5)
    if has_m1:  flags |= (1 << 6)

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


def export_adm(filepath, objects=None, export_textures=True):
    """
    Exportuje objekty do .adm souboru.
    objects: seznam bpy.types.Object; None = všechny mesh objekty ve scéně.
    """
    if objects is None:
        objects = [o for o in bpy.context.scene.objects if o.type == 'MESH']

    # Seřaď podle hierarchie (parents před children)
    def sort_key(o):
        depth = 0
        p = o.parent
        while p:
            depth += 1
            p = p.parent
        return depth
    objects = sorted(objects, key=sort_key)

    # Unikátní meshe (může více objektů sdílet stejný mesh data)
    mesh_data_list = []  # list of (name, positions, ...) tuples
    mesh_index_map = {}  # object.data.name → mesh_index

    node_list = []  # list of (name, type_byte, mesh_idx, parent_idx, mat_name, transform16)
    obj_to_node = {}  # object → node_index

    for obj in objects:
        # Detekce node type
        name_up = obj.name.upper()
        if 'COL_' in name_up or name_up.startswith('COL'):
            node_type = 1  # COLLISION
        else:
            node_type = 0  # MESH

        # Parent node index
        parent_idx = -1
        if obj.parent and obj.parent in obj_to_node:
            parent_idx = obj_to_node[obj.parent]

        mat_bevy = _to_bevy_mat4(obj.matrix_local)

        if node_type == 0 and len(obj.material_slots) > 1:
            # Multi-material mesh: jeden node per material slot.
            # Pojmenování: slot 0 → obj.name, slot N → "obj.name.N"
            # Tečkový suffix zajišťuje, že base_name() regex \.\d+$ funguje při re-importu.
            first_node_idx = None
            for mat_idx, slot in enumerate(obj.material_slots):
                mat_name = slot.material.name if slot.material else ''
                sub_name = obj.name if mat_idx == 0 else f"{obj.name}.{mat_idx}"

                mesh_key = f"__mat_{obj.data.name if obj.data else obj.name}_{mat_idx}"
                if mesh_key not in mesh_index_map:
                    data = _collect_mesh_data(obj, material_index=mat_idx)
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

            if first_node_idx is not None:
                obj_to_node[obj] = first_node_idx
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
                key = obj.data.name if obj.data else obj.name
                if key not in mesh_index_map:
                    mesh_index_map[key] = len(mesh_data_list)
                    data = _collect_mesh_data(obj)
                    mesh_data_list.append((obj.name, *data))
                mesh_idx = mesh_index_map[key]

            node_idx = len(node_list)
            obj_to_node[obj] = node_idx
            node_list.append((obj.name, node_type, mesh_idx, parent_idx, mat_name, mat_bevy))

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

    # Build binary — bytearray is mutable so += never copies the whole buffer
    buf = bytearray()

    # Header
    buf += b'ADM\x00'
    buf += struct.pack('<I', 1)   # version
    buf += struct.pack('<I', len(mesh_data_list))
    buf += struct.pack('<I', len(node_list))
    buf += struct.pack('<I', 1 if embedded_textures else 0)

    # Mesh sekce
    for entry in mesh_data_list:
        name, positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, indices = entry
        buf = _write_mesh(buf, name, positions, normals, tangents, uv0s, uv1s, masks0s, masks1s, indices)

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

    with open(filepath, 'wb') as f:
        f.write(buf)

    print(f"[adm_export] zapsáno {len(mesh_data_list)} meshů, {len(node_list)} uzlů, "
          f"{len(embedded_textures)} embedded textur → {filepath}")
    return len(mesh_data_list), len(node_list)
