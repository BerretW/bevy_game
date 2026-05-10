import os
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


def _prefill_export_name(obj):
    if not obj or obj.type != "MESH":
        return
    props = getattr(obj, "bevy_toolkit_obj", None)
    if props is None:
        return
    if not props.export_name.strip():
        props.export_name = bpy.path.clean_name(obj.name)


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
        new_obj.display_type = "WIRE"
        new_obj.hide_render  = True
        context.collection.objects.link(new_obj)
        self.report({"INFO"}, f"Collision proxy created: {new_obj.name}")
        return {"FINISHED"}


class BEVY_OT_SetupNodes(bpy.types.Operator):
    bl_idname = "bevy.setup_nodes"
    bl_label  = "Setup Preview Nodes"

    def execute(self, context):
        mat = context.active_material
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
        mat = context.active_material
        if not mat:
            self.report({"WARNING"}, "No active material")
            return {"CANCELLED"}
        mapped = sync_material_from_nodes(mat, only_if_empty=False)
        self.report({"INFO"}, f"Mapped {mapped} texture slot(s) from material nodes")
        return {"FINISHED"}


class BEVY_OT_ConvertAllMaterials(bpy.types.Operator):
    bl_idname = "bevy.convert_all_materials"
    bl_label  = "Convert All Materials"

    def execute(self, context):
        total = 0
        mats  = [mat for mat in bpy.data.materials if hasattr(mat, "bevy_toolkit")]
        for mat in mats:
            total += sync_material_from_nodes(mat, only_if_empty=False)
        self.report({"INFO"}, f"Mapped {total} texture slot(s) across {len(mats)} material(s)")
        return {"FINISHED"}


class BEVY_OT_SetAllTexturesEmbedded(bpy.types.Operator):
    bl_idname = "bevy.set_all_textures_embedded"
    bl_label  = "Set all Textures Embedded"

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
    bl_label  = "Remove all Embedded Textures"

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
        obj_props.is_col      = True
        obj_props.col_shape   = ent_data.get("shape",       "CONVEX")
        obj_props.mass        = float(ent_data.get("mass",        1.0))
        obj_props.is_static   = bool(ent_data.get("is_static",   False))
        obj_props.friction    = float(ent_data.get("friction",    0.6))
        obj_props.restitution = float(ent_data.get("restitution", 0.2))
        obj_props.tags_csv    = ",".join(ent_data.get("tags", []))
        obj.display_type      = "WIRE"
        obj.hide_render       = True
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
        try:
            meshes, nodes = export_adm(adm_path, objects=objects, export_textures=True)
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

        self.report({'INFO'}, f"ADM: {meshes} meshů, {nodes} uzlů → {os.path.basename(adm_path)} + {os.path.basename(drawable_path)}")
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
