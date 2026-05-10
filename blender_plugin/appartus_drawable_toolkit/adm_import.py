"""Import .adm binárního formátu zpět do Blenderu."""

import os
import re
import struct
import tempfile
import bpy
import mathutils

from .constants import ATTR_NAME, ATTR_NAME2

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


def _load_drawable_templates(adm_path):
    """Parsuje template per-materiál z .drawable TOML manifestu vedle .adm souboru."""
    drawable_path = os.path.splitext(adm_path)[0] + '.drawable'
    if not os.path.isfile(drawable_path):
        return {}
    try:
        try:
            import tomllib
        except ImportError:
            try:
                import tomli as tomllib  # pip-installed fallback
            except ImportError:
                return {}
        with open(drawable_path, 'rb') as f:
            data = tomllib.load(f)
        templates = {}
        for mat_name, mat_data in data.get('materials', {}).items():
            if isinstance(mat_data, dict) and 'template' in mat_data:
                templates[mat_name] = mat_data['template']
        return templates
    except Exception as e:
        print(f"[adm_import] Warning: nelze načíst .drawable manifest: {e}")
        return {}


def _guess_slot(img_name):
    """Odhadne slot z názvu textury pomocí TEXTURE_KEYWORDS."""
    from .constants import TEXTURE_KEYWORDS
    lowered = img_name.lower().replace(" ", "").replace("-", "").replace("_", "")
    for slot_name, keywords in TEXTURE_KEYWORDS.items():
        for kw in keywords:
            if kw in lowered:
                return slot_name
    return None


def _assign_textures_to_material(mat, loaded_images):
    """Přiřadí embedded textury do bevy_toolkit slotů materiálu."""
    props = getattr(mat, 'bevy_toolkit', None)
    if props is None:
        return
    from .utils import image_basename
    for img_name, img in loaded_images.items():
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

    # Vytvoř Blender mesh assets
    bl_meshes = []
    for (mname, positions, normals, uv0s, uv1s, masks0s, masks1s, indices) in mesh_data:
        bl_meshes.append(_build_blender_mesh(mname, positions, normals, uv0s, uv1s, masks0s, masks1s, indices))

    # Vytvoř objekty
    node_objects = []
    created = []
    imported_mats = set()
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
            imported_mats.add(mat_name)

        node_objects.append(obj)
        created.append(obj)

    # Parent-child vztahy
    for i, (_, _, _, parent_idx, _, _) in enumerate(node_data):
        if 0 <= parent_idx < len(node_objects):
            node_objects[i].parent = node_objects[parent_idx]

    # Přiřaď textury do materiálů importovaných z tohoto ADM
    if loaded_images:
        for mat_name in imported_mats:
            mat = bpy.data.materials.get(mat_name)
            if mat:
                _assign_textures_to_material(mat, loaded_images)

    # Načti šablony z .drawable (stejné jméno vedle .adm)
    drawable_templates = _load_drawable_templates(filepath)

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

    # Spoj sub-objekty vzniklé multi-material exportem zpět do jednoho objektu
    created = _merge_material_splits(created)

    return created
