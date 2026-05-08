import os
import bpy


def toml_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def format_float(value: float) -> str:
    s = f"{float(value):.6g}"
    if "." not in s and "e" not in s and "E" not in s:
        s += ".0"
    return s


def bool_to_toml(value: bool) -> str:
    return "true" if value else "false"


def parse_tags(raw: str):
    if not raw:
        return []
    parts = [part.strip() for part in raw.replace(";", ",").split(",")]
    return [part for part in parts if part]


def first_non_empty(*values):
    for value in values:
        if value:
            return value
    return ""


def image_basename(image: bpy.types.Image) -> str:
    if image is None:
        return ""
    return os.path.splitext(os.path.basename(image.name))[0]
