import re
import bpy
from .constants import SLOT_COLORSPACE
from .mesh import is_collision_object
from .material import get_template_texture_specs


def _lod_level_from_name(name: str) -> int:
    m = re.search(r'_LOD(\d+)$', name, re.IGNORECASE)
    return int(m.group(1)) if m else 0


def _collapsible_box(layout, settings, toggle_name, title, icon):
    box = layout.box()
    header = box.row(align=True)
    is_open = bool(getattr(settings, toggle_name, False))
    header.prop(
        settings,
        toggle_name,
        text="",
        emboss=False,
        icon='TRIA_DOWN' if is_open else 'TRIA_RIGHT',
    )
    header.label(text=title, icon=icon)
    return box, is_open


class BEVY_UL_AnimationDictionaries(bpy.types.UIList):
    def draw_item(self, context, layout, data, item, icon, active_data, active_propname, index):
        dict_item = item
        row = layout.row(align=True)
        row.label(text=dict_item.name or "(unnamed)", icon="ACTION")
        row.label(text=str(len(dict_item.clips)))


def _draw_bevy_material_props(layout, mat):
    props    = mat.bevy_toolkit
    template = props.template

    header = layout.row(align=True)
    header.prop(props, "template")
    header.operator("bevy.setup_nodes", text="", icon="NODETREE")

    tex_box = layout.box()
    tex_box.label(text="Texture Parameters")

    for slot_name, label, _ in get_template_texture_specs(template):
        col = tex_box.column(align=True)

        row   = col.row(align=True)
        split = row.split(factor=0.32, align=True)
        split.label(text=label)
        split.template_ID(props, f"{slot_name}_img", open="image.open")

        img = getattr(props, f"{slot_name}_img")
        if img is not None:
            sub = col.row(align=False)
            sub.separator(factor=3.5)
            sub.prop(props, f"{slot_name}_embedded", text="Embedded")
            cs = sub.row()
            cs.label(text=f"Color Space:   {SLOT_COLORSPACE.get(slot_name, 'sRGB')}")

        col.separator(factor=0.3)

    params_box = layout.box()
    params_box.label(text="Value Parameters")
    col = params_box.column(align=True)
    col.prop(props, "snow_level",  slider=True)
    col.prop(props, "dirt_level",  slider=True)
    col.prop(props, "wetness",     slider=True)
    col.prop(props, "porosity",    slider=True)
    col.separator()
    col.prop(props, "tiling")
    if template == "layered_env":
        col.prop(props, "l0_tiling")
        col.prop(props, "l1_tiling")
    col.separator()
    col.prop(props, "tint")

    opacity_box = layout.box()
    opacity_box.label(text="Opacity")
    oc = opacity_box.column(align=True)
    oc.prop(props, "opacity_mode")
    if props.opacity_mode != "OPAQUE" or props.mb_img is not None:
        oc.prop(props, "alpha_threshold", slider=True)


class BEVY_PT_MaterialPanel(bpy.types.Panel):
    bl_label      = "Appartu Drawable Toolkit"
    bl_idname     = "MATERIAL_PT_bevy_ads"
    bl_space_type = 'PROPERTIES'
    bl_region_type = 'WINDOW'
    bl_context    = 'material'

    @classmethod
    def poll(cls, context):
        return (
            context.active_object is not None
            and context.active_object.active_material is not None
        )

    def draw(self, context):
        _draw_bevy_material_props(self.layout, context.active_object.active_material)


class BEVY_PT_ObjectPanel(bpy.types.Panel):
    bl_label      = "Appartu Drawable Toolkit"
    bl_idname     = "OBJECT_PT_bevy_ads"
    bl_space_type = 'PROPERTIES'
    bl_region_type = 'WINDOW'
    bl_context    = 'object'

    @classmethod
    def poll(cls, context):
        return context.active_object is not None and context.active_object.type == "MESH"

    def draw(self, context):
        layout  = self.layout
        obj     = context.active_object
        settings = context.scene.bevy_toolkit_export

        entity_box, entity_open = _collapsible_box(layout, settings, "ui_show_entity", "Entity", "PHYSICS")
        if entity_open:
            entity_box.prop(obj.bevy_toolkit_obj, "export_name")
            entity_box.prop(obj.bevy_toolkit_obj, "is_col")
            if is_collision_object(obj):
                entity_box.prop(obj.bevy_toolkit_obj, "col_shape")
                entity_box.prop(obj.bevy_toolkit_obj, "mass")
                entity_box.prop(obj.bevy_toolkit_obj, "is_static")
                entity_box.prop(obj.bevy_toolkit_obj, "col_climbable")
                entity_box.prop(obj.bevy_toolkit_obj, "col_ladder")
                entity_box.prop(obj.bevy_toolkit_obj, "col_material")
                entity_box.prop(obj.bevy_toolkit_obj, "friction")
                entity_box.prop(obj.bevy_toolkit_obj, "restitution")
                entity_box.prop(obj.bevy_toolkit_obj, "tags_csv")
                entity_box.label(text="Axis Locks")
                row = entity_box.row(align=True)
                row.prop(obj.bevy_toolkit_obj, "lock_tx", text="Move X")
                row.prop(obj.bevy_toolkit_obj, "lock_ty", text="Move Y")
                row.prop(obj.bevy_toolkit_obj, "lock_tz", text="Move Z")
                row = entity_box.row(align=True)
                row.prop(obj.bevy_toolkit_obj, "lock_rx", text="Rot X")
                row.prop(obj.bevy_toolkit_obj, "lock_ry", text="Rot Y")
                row.prop(obj.bevy_toolkit_obj, "lock_rz", text="Rot Z")
            else:
                entity_box.prop(obj.bevy_toolkit_obj, "cast_shadows")
                entity_box.operator("bevy.gen_col", icon="MESH_CUBE", text="Generate COL_ proxy")

                # LOD info (pouze pro mesh objekty)
                export_name = obj.bevy_toolkit_obj.export_name.strip() or obj.name
                lod_level = _lod_level_from_name(export_name)
                lod_row = entity_box.row()
                lod_row.label(
                    text=f"LOD Level: {lod_level}" + (" (base)" if lod_level == 0 else ""),
                    icon="MOD_DECIM",
                )

        export_box, export_open = _collapsible_box(layout, settings, "ui_show_import_export", "Import / Export", "EXPORT")
        if export_open:
            export_box.operator("bevy.import_drawable", text="Import Drawable", icon="IMPORT")
            export_box.separator(factor=0.5)
            export_box.prop(settings, "export_scope")
            export_box.prop(settings, "apply_modifiers")
            export_box.operator("bevy.export_project", text="Export ADS", icon="EXPORT")
            export_box.operator("ads.export_adm", text="Export ADM + Drawable", icon="MESH_DATA")

            lod_box, lod_open = _collapsible_box(export_box, settings, "ui_show_lod_distances", "LOD Distances", "MOD_DECIM")
            if lod_open:
                col = lod_box.column(align=True)
                col.prop(settings, "lod_distance_0")
                col.prop(settings, "lod_distance_1")
                col.prop(settings, "lod_distance_2")
                lod_box.prop(settings, "lod_cull_beyond_last")

        map_box, map_open = _collapsible_box(layout, settings, "ui_show_map_tools", "Map Tools", "WORLD")
        if map_open:
            map_box.operator("bevy.export_map_manifest", text="Export Map TOML", icon="EXPORT")
            map_box.operator("bevy.import_map_manifest", text="Import Map TOML", icon="IMPORT")


class BEVY_PT_Panel(bpy.types.Panel):
    bl_label      = "Appartu Drawable Toolkit"
    bl_idname     = "BEVY_PT_main"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category   = "Appartu"

    def draw(self, context):
        layout   = self.layout
        obj      = context.active_object
        settings = context.scene.bevy_toolkit_export

        drawables_box, drawables_open = _collapsible_box(layout, settings, "ui_show_drawables", "Drawables", "OUTLINER_COLLECTION")
        if drawables_open:
            convert_box, convert_open = _collapsible_box(drawables_box, settings, "ui_show_convert", "Convert", "FILE_REFRESH")
            if convert_open:
                convert_box.row(align=True).operator("bevy.convert_to_drawable_model", text="Convert to Drawable Model")
                convert_box.row(align=True).operator("bevy.convert_to_drawable",       text="Convert to Drawable")
                options_row = convert_box.row(align=True)
                options_row.prop(settings, "auto_embed_collision")
                options_row.prop(settings, "center_to_selection")

            create_box, create_open = _collapsible_box(drawables_box, settings, "ui_show_create", "Create", "ADD")
            if create_open:
                create_row = create_box.row(align=True)
                create_row.operator("bevy.create_drawable",            text="Create Drawable")
                create_row.operator("bevy.create_drawable_dictionary", text="Create Drawable Dictionary")
                create_box.prop(settings, "drawable_dict_name", text="Dictionary")

            shader_box, shader_open = _collapsible_box(drawables_box, settings, "ui_show_shader_tools", "Shader Tools", "SHADING_RENDERED")
            if shader_open:
                shader_box.operator("bevy.create_shader_material", text="Create Shader Material")
                shader_row = shader_box.row(align=True)
                shader_row.operator("bevy.convert_active_material", text="Convert Active Material")
                shader_row.operator("bevy.convert_all_materials",   text="Convert All Materials")

            tools_box, tools_open = _collapsible_box(drawables_box, settings, "ui_show_tools", "Tools", "TOOL_SETTINGS")
            if tools_open:
                tool_row = tools_box.row(align=True)
                tool_row.operator("bevy.set_all_textures_embedded",    text="Set all Textures Embedded")
                tool_row.operator("bevy.remove_all_embedded_textures", text="Remove all Embedded Textures")
                tool_row = tools_box.row(align=True)
                tool_row.operator("bevy.set_all_materials_embedded",   text="Set all Materials Embedded")
                tool_row.operator("bevy.set_all_materials_unembedded", text="Set all Materials Unembedded")
                tools_box.operator("bevy.find_missing_textures", text="Find Missing Textures", icon="VIEWZOOM")

        mixamo_box, mixamo_open = _collapsible_box(layout, settings, "ui_show_mixamo_tools", "Mixamo Tools", "ARMATURE_DATA")
        if mixamo_open:
            mixamo_box.operator("ads.rename_mixamo_rig", text="Auto Rename Mixamo Rig", icon="SORTALPHA")
            mixamo_box.operator("ads.import_mixamo_animations", text="Import Mixamo Animations", icon="IMPORT")
            mixamo_box.operator("ads.import_anim_set", text="Import Animation Set (.ads_anim)", icon="ACTION")
            mixamo_box.operator("ads.export_adm", text="Export Geometry (.adm)", icon="EXPORT")
            mixamo_box.operator("ads.export_anim_set", text="Export Animation Set (.ads_anim)", icon="ACTION")

            ik_box, ik_open = _collapsible_box(mixamo_box, settings, "ui_show_ik_tools", "IK Chains", "CONSTRAINT_BONE")
            if ik_open:
                ik_box.prop(settings, "ik_export_sidecar")
                ik_box.prop(settings, "new_ik_chain_name", text="Chain")
                row = ik_box.row(align=True)
                row.operator("ads.ik_chain_add", text="Add Chain", icon="ADD")
                row.operator("ads.ik_chain_remove", text="Remove Chain", icon="REMOVE")
                row = ik_box.row(align=True)
                row.operator("ads.ik_chain_prev", text="", icon="TRIA_LEFT")
                row.operator("ads.ik_chain_next", text="", icon="TRIA_RIGHT")
                row.operator("ads.ik_chain_autofill_biped", text="Autofill Biped", icon="ARMATURE_DATA")
                ik_box.operator("ads.ik_chain_validate", text="Validate Against Armature", icon="CHECKMARK")

                if settings.ik_chains:
                    max_idx = len(settings.ik_chains) - 1
                    active_idx = min(max(0, settings.active_ik_chain_index), max_idx)
                    chain = settings.ik_chains[active_idx]
                    ik_box.label(text=f"Selected: {chain.name}")
                    ik_box.prop(chain, "name")
                    ik_box.prop(chain, "enabled")
                    ik_box.prop(chain, "parent_bone_name")
                    ik_box.prop(chain, "ik_target_name")
                    ik_box.prop(chain, "effector_bone_name")
                    ik_box.prop(chain, "pole_bone_name")
                    ik_box.prop(chain, "chain_length")
                    ik_box.prop(chain, "solver_iterations")
                    ik_box.prop(chain, "min_knee_angle")
                    ik_box.prop(chain, "max_knee_angle")
                else:
                    ik_box.label(text="(no IK chains)")

            dict_box, dict_open = _collapsible_box(mixamo_box, settings, "ui_show_animation_dictionaries", "Animation Dictionaries", "ACTION")
            if dict_open:
                dict_box.template_list(
                    "BEVY_UL_AnimationDictionaries",
                    "",
                    settings,
                    "animation_dictionaries",
                    settings,
                    "active_anim_dict_index",
                    rows=min(4, max(1, len(settings.animation_dictionaries))),
                )
                dict_box.prop(settings, "new_anim_dict_name", text="Dictionary")
                row = dict_box.row(align=True)
                row.operator("ads.anim_dict_add", text="Add Dict", icon="ADD")
                row.operator("ads.anim_dict_remove", text="Remove Dict", icon="REMOVE")

                if settings.animation_dictionaries:
                    max_idx = len(settings.animation_dictionaries) - 1
                    active_idx = min(max(0, settings.active_anim_dict_index), max_idx)
                    active_dict = settings.animation_dictionaries[active_idx]

                    nav_row = dict_box.row(align=True)
                    nav_row.operator("ads.anim_dict_prev", text="", icon="TRIA_LEFT")
                    nav_row.label(text=f"Selected: {active_dict.name}")
                    nav_row.operator("ads.anim_dict_next", text="", icon="TRIA_RIGHT")
                    dict_box.prop(settings, "anim_dict_clip_name", text="Clip")
                    clip_row = dict_box.row(align=True)
                    clip_row.operator("ads.anim_dict_add_clip", text="Add Clip", icon="ADD")
                    clip_row.operator("ads.anim_dict_remove_clip", text="Remove Clip", icon="REMOVE")
                    if active_dict.clips:
                        for clip in active_dict.clips:
                            dict_box.label(text=f"- {clip.clip_name}", icon="ANIM")
                    else:
                        dict_box.label(text="(empty)")

        if not obj:
            self._draw_export_box(layout, settings)
            return

        if obj.type == "MESH":
            entity_box, entity_open = _collapsible_box(layout, settings, "ui_show_entity", "Entity", "PHYSICS")
            if entity_open:
                entity_box.prop(obj.bevy_toolkit_obj, "export_name")
                entity_box.prop(obj.bevy_toolkit_obj, "is_col")
                if is_collision_object(obj):
                    entity_box.prop(obj.bevy_toolkit_obj, "col_shape")
                    entity_box.prop(obj.bevy_toolkit_obj, "mass")
                    entity_box.prop(obj.bevy_toolkit_obj, "is_static")
                    entity_box.prop(obj.bevy_toolkit_obj, "col_climbable")
                    entity_box.prop(obj.bevy_toolkit_obj, "col_ladder")
                    entity_box.prop(obj.bevy_toolkit_obj, "col_material")
                    entity_box.prop(obj.bevy_toolkit_obj, "friction")
                    entity_box.prop(obj.bevy_toolkit_obj, "restitution")
                    entity_box.prop(obj.bevy_toolkit_obj, "tags_csv")
                    entity_box.label(text="Axis Locks")
                    row = entity_box.row(align=True)
                    row.prop(obj.bevy_toolkit_obj, "lock_tx", text="Move X")
                    row.prop(obj.bevy_toolkit_obj, "lock_ty", text="Move Y")
                    row.prop(obj.bevy_toolkit_obj, "lock_tz", text="Move Z")
                    row = entity_box.row(align=True)
                    row.prop(obj.bevy_toolkit_obj, "lock_rx", text="Rot X")
                    row.prop(obj.bevy_toolkit_obj, "lock_ry", text="Rot Y")
                    row.prop(obj.bevy_toolkit_obj, "lock_rz", text="Rot Z")
                else:
                    entity_box.prop(obj.bevy_toolkit_obj, "cast_shadows")
                    entity_box.operator("bevy.gen_col", icon="MESH_CUBE", text="Generate COL_ proxy")

                    export_name = obj.bevy_toolkit_obj.export_name.strip() or obj.name
                    lod_level = _lod_level_from_name(export_name)
                    entity_box.label(
                        text=f"LOD Level: {lod_level}" + (" (base)" if lod_level == 0 else ""),
                        icon="MOD_DECIM",
                    )

            paint_box, paint_open = _collapsible_box(layout, settings, "ui_show_vertex_masks", "Vertex Masks", "VPAINT_HLT")
            if paint_open:
                paint_box.label(text="bevy_masks  (COLOR_0)")
                paint_box.label(text="R=NormSupp/L1  G=dirt  B=wet  A=palette")
                paint_box.operator("bevy.init_project", text="Initialize bevy_masks")
                row = paint_box.row(align=True)
                row.operator("bevy.set_paint", text="Norm/L1 (R)").mode = "NORMAL_SUPP"
                row.operator("bevy.set_paint", text="Dirt (G)").mode    = "DIRT"
                row.operator("bevy.set_paint", text="Wet (B)").mode     = "WET"
                row.operator("bevy.set_paint", text="Erase").mode       = "ERASE"

                paint_box.separator(factor=0.4)
                paint_box.label(text="Presets (fill all vertices):")
                row = paint_box.row(align=True)
                row.operator("bevy.apply_vertex_preset", text="Standard").preset = "standard"
                row.operator("bevy.apply_vertex_preset", text="Metal").preset    = "metal"
                row.operator("bevy.apply_vertex_preset", text="Wood").preset     = "wood"
                row.operator("bevy.apply_vertex_preset", text="Brick").preset    = "brick"
                row = paint_box.row(align=True)
                row.prop(settings, "alpha_fill_value")
                row.operator("bevy.fill_alpha_mask", text="Fill A (palette)")

                paint_box.separator(factor=0.5)
                paint_box.label(text="bevy_masks2  (→ TEXCOORD_1)")
                paint_box.label(text="R=AO (1=none)  G=emissive")
                paint_box.operator("bevy.init_masks2", text="Initialize bevy_masks2")
                row = paint_box.row(align=True)
                row.operator("bevy.set_paint", text="AO (R)").mode       = "AO"
                row.operator("bevy.set_paint", text="Emissive (G)").mode = "EMISSIVE"
                row.operator("bevy.set_paint", text="Erase").mode        = "ERASE2"

        self._draw_export_box(layout, settings)

    @staticmethod
    def _draw_export_box(layout, settings):
        export_box, export_open = _collapsible_box(layout, settings, "ui_show_import_export", "Import / Export", "EXPORT")
        if export_open:
            export_box.operator("bevy.import_drawable", text="Import Drawable", icon="IMPORT")
            export_box.operator("bevy.find_missing_textures", text="Find Missing Textures", icon="VIEWZOOM")
            export_box.separator(factor=0.5)
            export_box.prop(settings, "export_scope")
            export_box.prop(settings, "apply_modifiers")
            export_box.operator("bevy.export_project", text="Export ADS", icon="EXPORT")
            export_box.operator("ads.export_adm", text="Export ADM + Drawable", icon="MESH_DATA")

            lod_box, lod_open = _collapsible_box(export_box, settings, "ui_show_lod_distances", "LOD Distances", "MOD_DECIM")
            if lod_open:
                col = lod_box.column(align=True)
                col.prop(settings, "lod_distance_0")
                col.prop(settings, "lod_distance_1")
                col.prop(settings, "lod_distance_2")
                lod_box.prop(settings, "lod_cull_beyond_last")

        map_box, map_open = _collapsible_box(layout, settings, "ui_show_map_tools", "Map Tools", "WORLD")
        if map_open:
            map_box.operator("bevy.export_map_manifest", text="Export Map TOML", icon="EXPORT")
            map_box.operator("bevy.import_map_manifest", text="Import Map TOML", icon="IMPORT")
