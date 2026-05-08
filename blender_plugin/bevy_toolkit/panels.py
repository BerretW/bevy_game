import bpy
from .constants import SLOT_COLORSPACE
from .mesh import is_collision_object
from .material import get_template_texture_specs


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
    bl_label      = "Bevy ADS"
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


class BEVY_PT_Panel(bpy.types.Panel):
    bl_label      = "Bevy ADS Toolkit"
    bl_idname     = "BEVY_PT_main"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category   = "Bevy"

    def draw(self, context):
        layout   = self.layout
        obj      = context.active_object
        settings = context.scene.bevy_toolkit_export

        layout.label(text="Drawables", icon="OUTLINER_COLLECTION")

        convert_box = layout.box()
        convert_box.label(text="Convert", icon="FILE_REFRESH")
        convert_box.row(align=True).operator("bevy.convert_to_drawable_model", text="Convert to Drawable Model")
        convert_box.row(align=True).operator("bevy.convert_to_drawable",       text="Convert to Drawable")
        options_row = convert_box.row(align=True)
        options_row.prop(settings, "auto_embed_collision")
        options_row.prop(settings, "center_to_selection")

        create_box = layout.box()
        create_box.label(text="Create", icon="ADD")
        create_row = create_box.row(align=True)
        create_row.operator("bevy.create_drawable",            text="Create Drawable")
        create_row.operator("bevy.create_drawable_dictionary", text="Create Drawable Dictionary")
        create_box.prop(settings, "drawable_dict_name", text="Dictionary")

        shader_box = layout.box()
        shader_box.label(text="Shader Tools", icon="SHADING_RENDERED")
        shader_box.operator("bevy.create_shader_material", text="Create Shader Material")
        shader_row = shader_box.row(align=True)
        shader_row.operator("bevy.convert_active_material", text="Convert Active Material")
        shader_row.operator("bevy.convert_all_materials",   text="Convert All Materials")

        tools_box = layout.box()
        tools_box.label(text="Tools", icon="TOOL_SETTINGS")
        tool_row = tools_box.row(align=True)
        tool_row.operator("bevy.set_all_textures_embedded",    text="Set all Textures Embedded")
        tool_row.operator("bevy.remove_all_embedded_textures", text="Remove all Embedded Textures")
        tool_row = tools_box.row(align=True)
        tool_row.operator("bevy.set_all_materials_embedded",   text="Set all Materials Embedded")
        tool_row.operator("bevy.set_all_materials_unembedded", text="Set all Materials Unembedded")

        if not obj:
            self._draw_export_box(layout, settings)
            return

        if obj.type == "MESH":
            physics_box = layout.box()
            physics_box.label(text="Entity", icon="PHYSICS")
            physics_box.prop(obj.bevy_toolkit_obj, "is_col")
            if is_collision_object(obj):
                physics_box.prop(obj.bevy_toolkit_obj, "col_shape")
                physics_box.prop(obj.bevy_toolkit_obj, "mass")
                physics_box.prop(obj.bevy_toolkit_obj, "is_static")
                physics_box.prop(obj.bevy_toolkit_obj, "friction")
                physics_box.prop(obj.bevy_toolkit_obj, "restitution")
                physics_box.prop(obj.bevy_toolkit_obj, "tags_csv")
            else:
                physics_box.prop(obj.bevy_toolkit_obj, "cast_shadows")
                physics_box.operator("bevy.gen_col", icon="MESH_CUBE", text="Generate COL_ proxy")

            paint_box = layout.box()
            paint_box.label(text="Vertex Masks", icon="VPAINT_HLT")

            paint_box.label(text="bevy_masks  (COLOR_0)")
            paint_box.label(text="R=NormSupp/L1  G=dirt  B=wet  A=palette")
            paint_box.operator("bevy.init_project", text="Initialize bevy_masks")
            row = paint_box.row(align=True)
            row.operator("bevy.set_paint", text="Norm/L1 (R)").mode = "NORMAL_SUPP"
            row.operator("bevy.set_paint", text="Dirt (G)").mode    = "DIRT"
            row.operator("bevy.set_paint", text="Wet (B)").mode     = "WET"
            row.operator("bevy.set_paint", text="Erase").mode       = "ERASE"
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
        export_box = layout.box()
        export_box.label(text="Import / Export", icon="EXPORT")
        export_box.operator("bevy.import_drawable", text="Import Drawable", icon="IMPORT")
        export_box.separator(factor=0.5)
        export_box.prop(settings, "export_scope")
        export_box.prop(settings, "apply_modifiers")
        export_box.operator("bevy.export_project", text="Export ADS", icon="EXPORT")
        export_box.operator("ads.export_adm", text="Export ADM + Drawable", icon="MESH_DATA")
