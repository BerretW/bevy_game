ATTR_NAME = "bevy_masks"
ATTR_NAME2 = "bevy_masks2"
UV_MASKS2_NAME = "_ads_masks2_uv"
ADS_VERSION = "1.1"

TEXTURE_SLOT_FIELDS = (
    "albedo",
    "mrao",
    "normal",
    "palette",
    "snow",
    "l0_albedo",
    "l0_mrao",
    "l0_normal",
    "l1_albedo",
    "l1_mrao",
    "l1_normal",
    "glass_albedo",
    "shatter_map",
    "ma",
    "mb",
)

# Slots that are purely optional — no validation warning when absent.
OPTIONAL_TEXTURE_SLOTS = frozenset({"palette", "snow", "ma", "mb"})

# Slots not embedded by Blender's GLTF exporter in standard material channels
# (e.g. MB's alpha is ignored when blend_method=OPAQUE). These are saved to disk
# next to the .drawable on export so _find_image can recover them on re-import.
NON_GLTF_SLOTS = frozenset({"ma", "mb", "palette", "snow", "shatter_map"})

TEXTURE_KEYWORDS = {
    "albedo":       ("albedo", "basecolor", "base_color", "diffuse", "color"),
    "mrao":         ("mrao", "orm", "rma", "occlusion", "roughness", "metallic"),
    "normal":       ("normal", "norm", "nrm"),
    "palette":      ("palette", "lut"),
    "snow":         ("snow",),
    "l0_albedo":    ("l0albedo", "layer0albedo", "basealbedo"),
    "l0_mrao":      ("l0mrao", "layer0mrao", "baseorm"),
    "l0_normal":    ("l0normal", "layer0normal"),
    "l1_albedo":    ("l1albedo", "layer1albedo", "overlayalbedo"),
    "l1_mrao":      ("l1mrao", "layer1mrao", "overlayorm"),
    "l1_normal":    ("l1normal", "layer1normal"),
    "glass_albedo": ("glassalbedo", "glasscolor", "windowalbedo"),
    "shatter_map":  ("shatter", "crack", "breakmap", "shattermap"),
    "ma":           ("_ma", "matattr", "materialattr", "material_a"),
    "mb":           ("_mb", "maskblend", "mask_b", "material_b"),
}

SLOT_NODE_LABEL = {
    "albedo":       "Albedo",
    "mrao":         "MRAO",
    "normal":       "Normal",
    "palette":      "Palette",
    "snow":         "Snow",
    "l0_albedo":    "L0 Albedo",
    "l0_mrao":      "L0 MRAO",
    "l0_normal":    "L0 Normal",
    "l1_albedo":    "L1 Albedo",
    "l1_mrao":      "L1 MRAO",
    "l1_normal":    "L1 Normal",
    "glass_albedo": "Glass Albedo",
    "shatter_map":  "Shatter Map",
    "ma":           "MA",
    "mb":           "MB",
}

SLOT_COLORSPACE = {
    "albedo":       "sRGB",
    "mrao":         "Non-Color",
    "normal":       "Non-Color",
    "palette":      "sRGB",
    "snow":         "sRGB",
    "l0_albedo":    "sRGB",
    "l0_mrao":      "Non-Color",
    "l0_normal":    "Non-Color",
    "l1_albedo":    "sRGB",
    "l1_mrao":      "Non-Color",
    "l1_normal":    "Non-Color",
    "glass_albedo": "sRGB",
    "shatter_map":  "Non-Color",
    "ma":           "Non-Color",   # R=AO  G=Roughness  B=Metalness  (RDR2/_ma)
    "mb":           "Non-Color",   # alpha=opacity mask               (RDR2/_mb)
}
