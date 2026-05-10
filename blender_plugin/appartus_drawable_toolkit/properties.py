import bpy
from .material import on_template_changed, _on_texture_update, _on_param_update, _on_ma_mb_update


class BevyObjectProps(bpy.types.PropertyGroup):
    is_col:      bpy.props.BoolProperty(name="Is Collision", default=False)
    cast_shadows: bpy.props.BoolProperty(name="Cast Shadows", default=True)
    export_name: bpy.props.StringProperty(name="Export Name", default="")
    col_shape:   bpy.props.EnumProperty(
        name="Shape",
        items=[
            ("BOX",     "BOX",     ""),
            ("SPHERE",  "SPHERE",  ""),
            ("CAPSULE", "CAPSULE", ""),
            ("CYLINDER","CYLINDER",""),
            ("CONVEX",  "CONVEX",  ""),
            ("MESH",    "MESH",    ""),
        ],
        default="CONVEX",
    )
    mass:        bpy.props.FloatProperty(name="Mass",        default=1.0, min=0.0)
    is_static:   bpy.props.BoolProperty(name="Static",       default=False)
    friction:    bpy.props.FloatProperty(name="Friction",    default=0.6, min=0.0)
    restitution: bpy.props.FloatProperty(name="Restitution", default=0.2, min=0.0)
    tags_csv:    bpy.props.StringProperty(name="Tags",       default="")


class BevyMaterialProps(bpy.types.PropertyGroup):
    template: bpy.props.EnumProperty(
        name="Template",
        items=[
            ("standard_pbr",  "standard_pbr",  "Standard PBR template"),
            ("layered_env",   "layered_env",   "Layered environment template"),
            ("vehicle_glass", "vehicle_glass", "Vehicle glass template"),
        ],
        default="standard_pbr",
        update=on_template_changed,
    )

    albedo_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="Albedo Image",      update=_on_texture_update)
    albedo_name:     bpy.props.StringProperty(name="Albedo Name",    default="")
    albedo_embedded: bpy.props.BoolProperty(name="Embedded",         default=True)

    mrao_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="MRAO Image",      update=_on_texture_update)
    mrao_name:     bpy.props.StringProperty(name="MRAO Name",    default="")
    mrao_embedded: bpy.props.BoolProperty(name="Embedded",       default=False)

    normal_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="Normal Image",      update=_on_texture_update)
    normal_name:     bpy.props.StringProperty(name="Normal Name",    default="")
    normal_embedded: bpy.props.BoolProperty(name="Embedded",         default=False)

    palette_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="Palette Image",      update=_on_texture_update)
    palette_name:     bpy.props.StringProperty(name="Palette Name",   default="")
    palette_embedded: bpy.props.BoolProperty(name="Embedded",          default=False)

    snow_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="Snow Image",      update=_on_texture_update)
    snow_name:     bpy.props.StringProperty(name="Snow Name",    default="")
    snow_embedded: bpy.props.BoolProperty(name="Embedded",       default=False)

    l0_albedo_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="L0 Albedo Image",      update=_on_texture_update)
    l0_albedo_name:     bpy.props.StringProperty(name="L0 Albedo Name", default="")
    l0_albedo_embedded: bpy.props.BoolProperty(name="Embedded",          default=True)

    l0_mrao_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="L0 MRAO Image",      update=_on_texture_update)
    l0_mrao_name:     bpy.props.StringProperty(name="L0 MRAO Name",   default="")
    l0_mrao_embedded: bpy.props.BoolProperty(name="Embedded",          default=False)

    l0_normal_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="L0 Normal Image",      update=_on_texture_update)
    l0_normal_name:     bpy.props.StringProperty(name="L0 Normal Name", default="")
    l0_normal_embedded: bpy.props.BoolProperty(name="Embedded",          default=False)

    l1_albedo_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="L1 Albedo Image",      update=_on_texture_update)
    l1_albedo_name:     bpy.props.StringProperty(name="L1 Albedo Name", default="")
    l1_albedo_embedded: bpy.props.BoolProperty(name="Embedded",          default=True)

    l1_mrao_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="L1 MRAO Image",      update=_on_texture_update)
    l1_mrao_name:     bpy.props.StringProperty(name="L1 MRAO Name",   default="")
    l1_mrao_embedded: bpy.props.BoolProperty(name="Embedded",          default=False)

    l1_normal_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="L1 Normal Image",      update=_on_texture_update)
    l1_normal_name:     bpy.props.StringProperty(name="L1 Normal Name", default="")
    l1_normal_embedded: bpy.props.BoolProperty(name="Embedded",          default=False)

    glass_albedo_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="Glass Albedo Image",      update=_on_texture_update)
    glass_albedo_name:     bpy.props.StringProperty(name="Glass Albedo Name", default="")
    glass_albedo_embedded: bpy.props.BoolProperty(name="Embedded",             default=True)

    shatter_map_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="Shatter Map Image",      update=_on_texture_update)
    shatter_map_name:     bpy.props.StringProperty(name="Shatter Map Name", default="")
    shatter_map_embedded: bpy.props.BoolProperty(name="Embedded",            default=True)

    ma_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="MA Image",  update=_on_ma_mb_update)
    ma_name:     bpy.props.StringProperty(name="MA Name",   default="")
    ma_embedded: bpy.props.BoolProperty(name="Embedded",    default=False)

    mb_img:      bpy.props.PointerProperty(type=bpy.types.Image, name="MB Image",  update=_on_ma_mb_update)
    mb_name:     bpy.props.StringProperty(name="MB Name",   default="")
    mb_embedded: bpy.props.BoolProperty(name="Embedded",    default=False)

    opacity_mode: bpy.props.EnumProperty(
        name="Opacity Mode",
        items=[
            ("OPAQUE",  "Opaque",  "Fully opaque"),
            ("BLEND",   "Blend",   "Alpha blend (sorted)"),
            ("HASHED",  "Hashed",  "Alpha hash (no sorting)"),
            ("CLIP",    "Clip",    "Alpha clip (sharp cutout)"),
        ],
        default="OPAQUE",
        update=_on_ma_mb_update,
    )
    alpha_threshold: bpy.props.FloatProperty(
        name="Alpha Threshold",
        default=0.5, min=0.0, max=1.0,
        update=_on_ma_mb_update,
    )

    tint:       bpy.props.FloatVectorProperty(name="Tint",       size=4, default=(1.0, 1.0, 1.0, 1.0), min=0.0, max=1.0, update=_on_param_update)
    tiling:     bpy.props.FloatProperty(name="Tiling",           default=1.0, update=_on_param_update)
    l0_tiling:  bpy.props.FloatProperty(name="L0 Tiling",        default=1.0, update=_on_param_update)
    l1_tiling:  bpy.props.FloatProperty(name="L1 Tiling",        default=1.0, update=_on_param_update)
    porosity:   bpy.props.FloatProperty(name="Porosity",  default=0.0, min=0.0, max=1.0, update=_on_param_update)
    wetness:    bpy.props.FloatProperty(name="Wetness",   default=0.0, min=0.0, max=1.0, update=_on_param_update)
    snow_level: bpy.props.FloatProperty(name="Snow Level",default=0.0, min=0.0, max=1.0, update=_on_param_update)
    dirt_level: bpy.props.FloatProperty(name="Dirt Level",default=0.0, min=0.0, max=1.0, update=_on_param_update)


class BevyExportProps(bpy.types.PropertyGroup):
    export_scope: bpy.props.EnumProperty(
        name="Scope",
        items=[
            ("ALL",              "All Meshes",        "Export all mesh objects"),
            ("SELECTED",         "Selected",          "Export only selected mesh objects"),
            ("ACTIVE_COLLECTION","Active Collection", "Export meshes from active collection"),
        ],
        default="ALL",
    )
    apply_modifiers:     bpy.props.BoolProperty(name="Apply Modifiers",     default=True)
    auto_embed_collision: bpy.props.BoolProperty(name="Auto-Embed Collision", default=True)
    center_to_selection: bpy.props.BoolProperty(name="Center To Selection", default=True)
    drawable_dict_name:  bpy.props.StringProperty(name="Dictionary Name",   default="DrawableDictionary")
    alpha_fill_value:    bpy.props.FloatProperty(name="Tint Alpha",          default=1.0, min=0.0, max=1.0)
