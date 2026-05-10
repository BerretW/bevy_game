bl_info = {
    "name": "Appartu Drawable Toolkit",
    "author": "Advanced Game Dev",
    "version": (4, 0),
    "blender": (3, 6, 0),
    "location": "View3D > Sidebar > Appartu",
    "description": "ADS pipeline: masks, collision metadata, GLB+Drawable export",
    "category": "Import-Export",
}

import bpy

from .properties import BevyObjectProps, BevyMaterialProps, BevyExportProps
from .operators import (
    BEVY_OT_InitProject,
    BEVY_OT_InitMasks2,
    BEVY_OT_SetPaint,
    BEVY_OT_FillAlphaMask,
    BEVY_OT_ApplyVertexPreset,
    BEVY_OT_GenerateCol,
    BEVY_OT_SetupNodes,
    BEVY_OT_ConvertToDrawableModel,
    BEVY_OT_ConvertToDrawable,
    BEVY_OT_CreateDrawable,
    BEVY_OT_CreateDrawableDictionary,
    BEVY_OT_CreateShaderMaterial,
    BEVY_OT_ConvertActiveMaterial,
    BEVY_OT_ConvertAllMaterials,
    BEVY_OT_SetAllTexturesEmbedded,
    BEVY_OT_RemoveAllEmbeddedTextures,
    BEVY_OT_SetAllMaterialsEmbedded,
    BEVY_OT_SetAllMaterialsUnembedded,
    BEVY_OT_Export,
    BEVY_OT_ImportDrawable,
    ADS_OT_export_adm,
    BEVY_OT_BrowseTexture,
)
from .panels import BEVY_PT_MaterialPanel, BEVY_PT_ObjectPanel, BEVY_PT_Panel

classes = (
    BevyObjectProps,
    BevyMaterialProps,
    BevyExportProps,
    BEVY_OT_InitProject,
    BEVY_OT_InitMasks2,
    BEVY_OT_SetPaint,
    BEVY_OT_FillAlphaMask,
    BEVY_OT_ApplyVertexPreset,
    BEVY_OT_GenerateCol,
    BEVY_OT_SetupNodes,
    BEVY_OT_ConvertToDrawableModel,
    BEVY_OT_ConvertToDrawable,
    BEVY_OT_CreateDrawable,
    BEVY_OT_CreateDrawableDictionary,
    BEVY_OT_CreateShaderMaterial,
    BEVY_OT_ConvertActiveMaterial,
    BEVY_OT_ConvertAllMaterials,
    BEVY_OT_SetAllTexturesEmbedded,
    BEVY_OT_RemoveAllEmbeddedTextures,
    BEVY_OT_SetAllMaterialsEmbedded,
    BEVY_OT_SetAllMaterialsUnembedded,
    BEVY_OT_Export,
    BEVY_OT_ImportDrawable,
    ADS_OT_export_adm,
    BEVY_OT_BrowseTexture,
    BEVY_PT_MaterialPanel,
    BEVY_PT_ObjectPanel,
    BEVY_PT_Panel,
)


def register():
    for cls in classes:
        bpy.utils.register_class(cls)
    bpy.types.Object.bevy_toolkit_obj   = bpy.props.PointerProperty(type=BevyObjectProps)
    bpy.types.Material.bevy_toolkit     = bpy.props.PointerProperty(type=BevyMaterialProps)
    bpy.types.Scene.bevy_toolkit_export = bpy.props.PointerProperty(type=BevyExportProps)


def unregister():
    del bpy.types.Scene.bevy_toolkit_export
    del bpy.types.Material.bevy_toolkit
    del bpy.types.Object.bevy_toolkit_obj
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    register()
