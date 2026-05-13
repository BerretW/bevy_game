import bpy
from .constants import COLLISION_DEFAULT_MATERIAL, COLLISION_MATERIAL_ITEMS
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
            ("NAVMESH", "NAVMESH", "AI navigation-only surface"),
        ],
        default="CONVEX",
    )
    mass:        bpy.props.FloatProperty(name="Mass",        default=1.0, min=0.0)
    is_static:   bpy.props.BoolProperty(name="Static",       default=False)
    col_climbable: bpy.props.BoolProperty(name="Climbable",  default=False)
    col_ladder:    bpy.props.BoolProperty(name="Ladder",     default=False)
    col_material:  bpy.props.EnumProperty(
        name="Material",
        items=COLLISION_MATERIAL_ITEMS,
        default=COLLISION_DEFAULT_MATERIAL,
    )
    friction:    bpy.props.FloatProperty(name="Friction",    default=0.6, min=0.0)
    restitution: bpy.props.FloatProperty(name="Restitution", default=0.2, min=0.0)
    tags_csv:    bpy.props.StringProperty(name="Tags",       default="")
    lock_tx:     bpy.props.BoolProperty(name="Lock Move X", default=False)
    lock_ty:     bpy.props.BoolProperty(name="Lock Move Y", default=False)
    lock_tz:     bpy.props.BoolProperty(name="Lock Move Z", default=False)
    lock_rx:     bpy.props.BoolProperty(name="Lock Rot X",  default=False)
    lock_ry:     bpy.props.BoolProperty(name="Lock Rot Y",  default=False)
    lock_rz:     bpy.props.BoolProperty(name="Lock Rot Z",  default=False)


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


class BevyAnimDictClipRef(bpy.types.PropertyGroup):
    clip_name: bpy.props.StringProperty(name="Clip Name", default="")


class BevyAnimDictionary(bpy.types.PropertyGroup):
    name: bpy.props.StringProperty(name="Dictionary", default="default")
    clips: bpy.props.CollectionProperty(type=BevyAnimDictClipRef)
    active_clip_index: bpy.props.IntProperty(name="Active Clip", default=0, min=0)


class BevyIkChain(bpy.types.PropertyGroup):
    name: bpy.props.StringProperty(name="Chain", default="leg_l")
    enabled: bpy.props.BoolProperty(name="Enabled", default=True)
    parent_bone_name: bpy.props.StringProperty(name="Parent Bone", default="DEF_thigh_l")
    ik_target_name: bpy.props.StringProperty(name="IK Target", default="IK_foot_l")
    effector_bone_name: bpy.props.StringProperty(name="Effector Bone", default="DEF_foot_l")
    pole_bone_name: bpy.props.StringProperty(name="Pole Bone", default="IK_knee_l")
    chain_length: bpy.props.FloatProperty(name="Chain Length", default=1.0, min=0.001)
    solver_iterations: bpy.props.IntProperty(name="Solver Iterations", default=2, min=1, max=16)
    min_knee_angle: bpy.props.FloatProperty(name="Min Knee Angle", default=5.0, min=0.0, max=180.0)
    max_knee_angle: bpy.props.FloatProperty(name="Max Knee Angle", default=175.0, min=0.0, max=180.0)


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

    # LOD nastavení — exportují se do sekce [lod] v .drawable manifestu
    lod_distance_0: bpy.props.FloatProperty(
        name="LOD0→1",
        description="Vzdálenost (m) přechodu z LOD0 na LOD1",
        default=15.0, min=0.1, unit='LENGTH',
    )
    lod_distance_1: bpy.props.FloatProperty(
        name="LOD1→2",
        description="Vzdálenost (m) přechodu z LOD1 na LOD2",
        default=40.0, min=0.1, unit='LENGTH',
    )
    lod_distance_2: bpy.props.FloatProperty(
        name="LOD2→3",
        description="Vzdálenost (m) přechodu z LOD2 na LOD3",
        default=80.0, min=0.1, unit='LENGTH',
    )
    lod_cull_beyond_last: bpy.props.BoolProperty(
        name="Cull Beyond Last LOD",
        description="Skryj model za poslední LOD vzdáleností",
        default=False,
    )

    ui_show_drawables: bpy.props.BoolProperty(name="Show Drawables", default=False)
    ui_show_convert: bpy.props.BoolProperty(name="Show Convert", default=False)
    ui_show_create: bpy.props.BoolProperty(name="Show Create", default=False)
    ui_show_shader_tools: bpy.props.BoolProperty(name="Show Shader Tools", default=False)
    ui_show_tools: bpy.props.BoolProperty(name="Show Tools", default=False)
    ui_show_mixamo_tools: bpy.props.BoolProperty(name="Show Mixamo Tools", default=False)
    ui_show_animation_dictionaries: bpy.props.BoolProperty(name="Show Animation Dictionaries", default=False)
    ui_show_ik_tools: bpy.props.BoolProperty(name="Show IK Tools", default=False)
    ui_show_entity: bpy.props.BoolProperty(name="Show Entity", default=False)
    ui_show_vertex_masks: bpy.props.BoolProperty(name="Show Vertex Masks", default=False)
    ui_show_import_export: bpy.props.BoolProperty(name="Show Import Export", default=False)
    ui_show_lod_distances: bpy.props.BoolProperty(name="Show LOD Distances", default=False)
    ui_show_map_tools: bpy.props.BoolProperty(name="Show Map Tools", default=False)

    # Animation dictionary workflow (ADM v5)
    animation_dictionaries: bpy.props.CollectionProperty(type=BevyAnimDictionary)
    active_anim_dict_index: bpy.props.IntProperty(name="Active Anim Dict", default=0, min=0)
    new_anim_dict_name: bpy.props.StringProperty(name="New Dict", default="move")
    anim_dict_clip_name: bpy.props.StringProperty(name="Clip", default="")

    # IK authoring workflow
    ik_chains: bpy.props.CollectionProperty(type=BevyIkChain)
    active_ik_chain_index: bpy.props.IntProperty(name="Active IK Chain", default=0, min=0)
    new_ik_chain_name: bpy.props.StringProperty(name="New IK Chain", default="leg_l")
    ik_export_sidecar: bpy.props.BoolProperty(
        name="Export IK Sidecar",
        description="Export IK chains to a sidecar TOML file next to .adm/.ads_anim/.glb",
        default=True,
    )

    # Navmesh generation parameters
    ui_show_navmesh: bpy.props.BoolProperty(name="Show Navmesh Tools", default=False)
    navmesh_walkable_height: bpy.props.FloatProperty(
        name="Walkable Height",
        description="Max height of agent (meters)",
        default=1.8, min=0.1, unit='LENGTH',
    )
    navmesh_walkable_radius: bpy.props.FloatProperty(
        name="Walkable Radius",
        description="Agent radius for navmesh (meters)",
        default=0.35, min=0.05, unit='LENGTH',
    )
    navmesh_climb_height: bpy.props.FloatProperty(
        name="Climb Height",
        description="Max step agent can climb (meters)",
        default=0.5, min=0.0, unit='LENGTH',
    )
    navmesh_include_water: bpy.props.BoolProperty(
        name="Include Water Surfaces",
        description="Generate water walkable surfaces",
        default=True,
    )
    navmesh_include_climbable: bpy.props.BoolProperty(
        name="Include Climbable Surfaces",
        description="Generate steep walkable surfaces (walls, slopes)",
        default=True,
    )
    navmesh_include_ceiling: bpy.props.BoolProperty(
        name="Include Ceiling Surfaces",
        description="Generate inverted surfaces for flying/ceiling traversal",
        default=False,
    )
