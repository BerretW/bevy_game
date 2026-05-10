"""Shared node-graph helpers used by all ADS shader builders."""
import bpy
from ..constants import ATTR_NAME2


def _material_image(props, slot_name):
    return getattr(props, f"{slot_name}_img")


def _add_tex_node(nodes, label, image, x, y, colorspace="sRGB"):
    node = nodes.new("ShaderNodeTexImage")
    node.label    = label
    node.location = (x, y)
    if image is not None:
        node.image = image
        if node.image.colorspace_settings:
            node.image.colorspace_settings.name = colorspace
    return node


def _add_value_node(nodes, name, value, x, y):
    node = nodes.new("ShaderNodeValue")
    node.label                    = name
    node.outputs[0].default_value = float(value)
    node.location                 = (x, y)
    return node


def _float_socket(nodes, value, x, y, label="Value"):
    if hasattr(value, "is_linked"):
        return value
    node = _add_value_node(nodes, label, value, x, y)
    return node.outputs[0]


def _mix_float(nodes, links, fac, a, b, x, y, label="MixFloat"):
    """Lerp between two float sockets/values: result = lerp(a, b, fac)."""
    fac_socket = _float_socket(nodes, fac, x - 320, y + 40,  f"{label} Fac")
    a_socket   = _float_socket(nodes, a,   x - 320, y - 40,  f"{label} A")
    b_socket   = _float_socket(nodes, b,   x - 320, y - 120, f"{label} B")

    one = _add_value_node(nodes, f"{label} One", 1.0, x - 180, y + 90)

    one_minus           = nodes.new("ShaderNodeMath")
    one_minus.operation = "SUBTRACT"
    one_minus.location  = (x - 20, y + 80)
    links.new(one.outputs[0], one_minus.inputs[0])
    links.new(fac_socket,     one_minus.inputs[1])

    mul_a           = nodes.new("ShaderNodeMath")
    mul_a.operation = "MULTIPLY"
    mul_a.location  = (x + 160, y)
    links.new(a_socket,             mul_a.inputs[0])
    links.new(one_minus.outputs[0], mul_a.inputs[1])

    mul_b           = nodes.new("ShaderNodeMath")
    mul_b.operation = "MULTIPLY"
    mul_b.location  = (x + 160, y - 100)
    links.new(b_socket,   mul_b.inputs[0])
    links.new(fac_socket, mul_b.inputs[1])

    add           = nodes.new("ShaderNodeMath")
    add.operation = "ADD"
    add.location  = (x + 340, y - 40)
    links.new(mul_a.outputs[0], add.inputs[0])
    links.new(mul_b.outputs[0], add.inputs[1])
    return add.outputs[0]


def build_masks2_ao(nodes, links, x, y):
    """Read bevy_masks2 AO with fallback to 1.0 when the attribute is absent.

    When bevy_masks2 doesn't exist, ShaderNodeAttribute returns (0,0,0,0) which
    would multiply the final color to black.  We use attr.alpha as an
    "initialized" flag: alpha=0 → absent → AO=1.0; alpha=1 → use actual R value.

    Formula: max(ao_raw, 1.0 - attr_alpha)

    Returns (attr2_node, sep2_node, ao_source_socket).
    """
    attr2 = nodes.new("ShaderNodeAttribute")
    attr2.attribute_name = ATTR_NAME2
    attr2.location = (x, y)

    sep2 = nodes.new("ShaderNodeSeparateColor")
    sep2.location = (x + 200, y)
    links.new(attr2.outputs[0], sep2.inputs[0])

    one_minus_a = nodes.new("ShaderNodeMath")
    one_minus_a.operation = "SUBTRACT"
    one_minus_a.location = (x + 330, y - 60)
    one_minus_a.inputs[0].default_value = 1.0
    links.new(attr2.outputs[3], one_minus_a.inputs[1])

    ao_max = nodes.new("ShaderNodeMath")
    ao_max.operation = "MAXIMUM"
    ao_max.location = (x + 330, y)
    links.new(sep2.outputs[0],        ao_max.inputs[0])
    links.new(one_minus_a.outputs[0], ao_max.inputs[1])

    return attr2, sep2, ao_max.outputs[0]


def build_ao_rgb(nodes, links, ao_source, x, y):
    """Convert scalar AO socket to RGB color for MixRGB multiply."""
    ao_rgb = nodes.new("ShaderNodeCombineColor")
    ao_rgb.location = (x, y)
    links.new(ao_source, ao_rgb.inputs[0])
    links.new(ao_source, ao_rgb.inputs[1])
    links.new(ao_source, ao_rgb.inputs[2])
    return ao_rgb


def build_roughness_clamp(nodes, links, rough_socket, min_rough, x, y):
    """Clamp roughness to a minimum so EEVEE preview doesn't go black without HDRI."""
    clamp = nodes.new("ShaderNodeMath")
    clamp.operation = "MAXIMUM"
    clamp.location  = (x, y)
    clamp.inputs[1].default_value = min_rough
    links.new(rough_socket, clamp.inputs[0])
    return clamp.outputs[0]
