bl_info = {
    "name": "Bevy Engine Toolkit (Sollumz Style)",
    "author": "Advanced Game Dev",
    "version": (3, 1),
    "blender": (3, 6, 0),
    "location": "View3D > Sidebar > Bevy",
    "description": "Profesionální pipeline pro Bevy Uber-Shader",
    "category": "Import-Export",
}

import bpy
import os

# --- KONSTANTY ---
ATTR_NAME = "bevy_masks"

# --- 1. PROPERTIES ---

class BevyObjectProps(bpy.types.PropertyGroup):
    is_col: bpy.props.BoolProperty(name="Is Collision", default=False)
    col_type: bpy.props.EnumProperty(
        name="Type",
        items=[('BOX', "Box", ""), ('CONVEX', "Convex Hull", ""), ('MESH', "Precise", "")],
        default='CONVEX'
    )

class BevyMaterialProps(bpy.types.PropertyGroup):
    template: bpy.props.EnumProperty(
        name="Template",
        items=[('layered_env', "Industrial Layered", "PBR + Layers + Env")],
        default='layered_env'
    )
    palette_img: bpy.props.PointerProperty(type=bpy.types.Image, name="Palette")
    porosity: bpy.props.FloatProperty(name="Porosity", default=0.5, min=0, max=1)

# --- 2. SHADER ENGINE ---

def create_bevy_node_tree(mat):
    if not mat or not mat.use_nodes:
        return
    nodes = mat.node_tree.nodes
    links = mat.node_tree.links
    nodes.clear()

    attr = nodes.new('ShaderNodeAttribute')
    attr.attribute_name = ATTR_NAME
    attr.location = (-1200, 200)
    
    sep = nodes.new('ShaderNodeSeparateColor')
    sep.location = (-1000, 200)
    links.new(attr.outputs[0], sep.inputs[0])

    l0_alb = nodes.new('ShaderNodeTexImage')
    l0_alb.label = "L0 Albedo"
    l0_alb.location = (-700, 400)
    
    l1_alb = nodes.new('ShaderNodeTexImage')
    l1_alb.label = "L1 Albedo"
    l1_alb.location = (-700, 100)

    mix_alb = nodes.new('ShaderNodeMixRGB')
    mix_alb.location = (-300, 250)
    links.new(sep.outputs[0], mix_alb.inputs[0])
    links.new(l0_alb.outputs[0], mix_alb.inputs[1])
    links.new(l1_alb.outputs[0], mix_alb.inputs[2])

    palette = nodes.new('ShaderNodeTexImage')
    palette.label = "PALETTE_LOOKUP"
    palette.location = (-300, -50)
    if mat.bevy_toolkit.palette_img:
        palette.image = mat.bevy_toolkit.palette_img

    comb_uv = nodes.new('ShaderNodeCombineXYZ')
    comb_uv.location = (-500, -50)
    links.new(sep.outputs[3], comb_uv.inputs[0]) 
    comb_uv.inputs[1].default_value = 0.5        
    links.new(comb_uv.outputs[0], palette.inputs[0])

    tint = nodes.new('ShaderNodeMixRGB')
    tint.blend_type = 'MULTIPLY'
    tint.inputs[0].default_value = 1.0
    tint.location = (0, 250)
    links.new(mix_alb.outputs[0], tint.inputs[1])
    links.new(palette.outputs[0], tint.inputs[2])

    bsdf = nodes.new('ShaderNodeBsdfPrincipled')
    bsdf.location = (400, 250)
    links.new(tint.outputs[0], bsdf.inputs[0])
    
    out = nodes.new('ShaderNodeOutputMaterial')
    out.location = (700, 250)
    links.new(bsdf.outputs[0], out.inputs[0])

# --- 3. OPERATORS ---

class BEVY_OT_InitProject(bpy.types.Operator):
    bl_idname = "bevy.init_project"
    bl_label = "Initialize Bevy Settings"
    def execute(self, context):
        obj = context.active_object
        if obj and obj.type == 'MESH':
            if ATTR_NAME not in obj.data.attributes:
                obj.data.attributes.new(name=ATTR_NAME, type='BYTE_COLOR', domain='CORNER')
            obj.data.attributes.active = obj.data.attributes[ATTR_NAME]
        return {'FINISHED'}

class BEVY_OT_SetPaint(bpy.types.Operator):
    bl_idname = "bevy.set_paint"
    bl_label = "Set Paint Mask"
    mode: bpy.props.StringProperty()
    def execute(self, context):
        if context.object.mode != 'PAINT_VERTEX':
            bpy.ops.object.mode_set(mode='PAINT_VERTEX')
        brush = context.tool_settings.vertex_paint.brush
        if self.mode == "L1": brush.color = (1, 0, 0)
        elif self.mode == "BLOOD": brush.color = (0, 1, 0)
        elif self.mode == "WET": brush.color = (0, 0, 1)
        elif self.mode == "TINT": brush.color = (1, 1, 1)
        elif self.mode == "ERASE": brush.color = (0, 0, 0)
        return {'FINISHED'}

class BEVY_OT_GenerateCol(bpy.types.Operator):
    bl_idname = "bevy.gen_col"
    bl_label = "Create Collision"
    def execute(self, context):
        src = context.active_object
        new_obj = src.copy()
        new_obj.data = src.data.copy()
        new_obj.name = f"COL_{src.name}"
        new_obj.bevy_toolkit_obj.is_col = True
        new_obj.display_type = 'WIRE'
        context.collection.objects.link(new_obj)
        return {'FINISHED'}

class BEVY_OT_Export(bpy.types.Operator):
    bl_idname = "bevy.export_project"
    bl_label = "Export (GLB + TOML)"
    def execute(self, context):
        if not bpy.data.filepath: return {'CANCELLED'}
        path = bpy.data.filepath.replace(".blend", "")
        bpy.ops.export_scene.gltf(filepath=path + ".glb", export_format='GLB')
        with open(path + ".toml", "w") as f:
            f.write("[materials]\n")
            for mat in bpy.data.materials:
                if hasattr(mat, "bevy_toolkit"):
                    f.write(f"\"{mat.name}\" = {{ template = \"{mat.bevy_toolkit.template}\" }}\n")
        return {'FINISHED'}

class BEVY_OT_SetupNodes(bpy.types.Operator):
    bl_idname = "bevy.setup_nodes"
    bl_label = "Update Node Preview"
    def execute(self, context):
        create_bevy_node_tree(context.active_material)
        return {'FINISHED'}

# --- 4. UI PANEL (FIXED) ---

class BEVY_PT_Panel(bpy.types.Panel):
    bl_label = "Bevy Toolkit"
    bl_idname = "BEVY_PT_main"
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = 'Bevy'

    def draw(self, context): # ZDE BYLA CHYBA (chybělo context)
        layout = self.layout
        obj = context.active_object
        
        col = layout.column(align=True)
        col.operator("bevy.export_project", icon='EXPORT', text="Export Project")
        
        if obj:
            # Physics
            box = layout.box()
            box.label(text="Physics & Collision", icon='PHYSICS')
            box.prop(obj.bevy_toolkit_obj, "is_col")
            if not obj.bevy_toolkit_obj.is_col:
                box.prop(obj.bevy_toolkit_obj, "col_type")
                box.operator("bevy.gen_col", icon='MOD_PHYSICS', text="Generate Proxy")

            # Painting
            box = layout.box()
            box.label(text="Vertex Painting", icon='VPAINT_HLT')
            box.operator("bevy.init_project", text="Initialize Masks")
            
            row = box.row(align=True)
            row.operator("bevy.set_paint", text="L1").mode = "L1"
            row.operator("bevy.set_paint", text="Blood").mode = "BLOOD"
            row.operator("bevy.set_paint", text="Wet").mode = "WET"
            
            row = box.row(align=True)
            row.operator("bevy.set_paint", text="Tint (A)").mode = "TINT"
            row.operator("bevy.set_paint", text="Erase").mode = "ERASE"

            # Materials
            if obj.active_material:
                mat = obj.active_material
                box = layout.box()
                box.label(text=f"Material: {mat.name}", icon='MATERIAL')
                box.prop(mat.bevy_toolkit, "palette_img")
                box.prop(mat.bevy_toolkit, "porosity")
                box.operator("bevy.setup_nodes", icon='NODE_SEL', text="Setup Nodes")

# --- REGISTRACE ---

classes = (
    BevyObjectProps, BevyMaterialProps,
    BEVY_OT_InitProject, BEVY_OT_SetPaint, 
    BEVY_OT_GenerateCol, BEVY_OT_Export,
    BEVY_OT_SetupNodes, BEVY_PT_Panel
)

def register():
    for cls in classes:
        bpy.utils.register_class(cls)
    bpy.types.Object.bevy_toolkit_obj = bpy.props.PointerProperty(type=BevyObjectProps)
    bpy.types.Material.bevy_toolkit = bpy.props.PointerProperty(type=BevyMaterialProps)

def unregister():
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)

if __name__ == "__main__":
    register()