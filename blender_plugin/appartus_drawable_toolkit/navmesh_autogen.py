"""
Automatic navmesh generation from collision meshes.

Detects COL_* objects, classifies faces by slope angle and normal direction,
and generates walkable/water/climbable/ceiling surfaces.
"""

import bpy
import bmesh
import mathutils
import math
from typing import Dict, List, Tuple, Optional


class NavmeshAutoGenerator:
    """
    Analyzes collision meshes and generates navmesh surfaces.
    
    Supported surface types:
    - ground: slope < 45° (normal heading up)
    - water: explicit water collider (name/material), not angle-based
    - climbable: 45-80° slope (steep walkable surfaces, walls, ladders)
    - ceiling: slope >= 80° (mostly vertical/inverted; useful for flying NPC)
    """

    def __init__(self, context):
        self.context = context
        self.scene = context.scene

    def _resolve_target_collection(self, fallback_obj=None):
        candidates = []
        if fallback_obj is not None:
            candidates.append(fallback_obj)
        active_object = getattr(self.context, "active_object", None)
        if active_object is not None and active_object not in candidates:
            candidates.append(active_object)

        for candidate in candidates:
            user_collections = getattr(candidate, "users_collection", None) or ()
            if user_collections:
                return user_collections[0]

        active_layer = getattr(getattr(self.context, "view_layer", None), "active_layer_collection", None)
        if active_layer is not None and getattr(active_layer, "collection", None) is not None:
            return active_layer.collection

        if getattr(self.context, "collection", None) is not None:
            return self.context.collection
        return self.scene.collection

    def _collection_mesh_objects(self, collection):
        if collection is None:
            return []
        return [obj for obj in collection.all_objects if obj.type == 'MESH']

    @staticmethod
    def _is_water_object(obj) -> bool:
        name = (obj.name or "").upper()
        if "WATER" in name:
            return True
        toolkit = getattr(obj, "bevy_toolkit_obj", None)
        mat = getattr(toolkit, "col_material", None)
        return str(mat).upper() == "WATER"

    def get_scene_settings(self) -> Dict[str, float]:
        """Retrieve navmesh walkable parameters from scene properties."""
        return {
            'walkable_height': getattr(self.scene, 'navmesh_walkable_height', 1.8),
            'walkable_radius': getattr(self.scene, 'navmesh_walkable_radius', 0.35),
            'climb_height': getattr(self.scene, 'navmesh_climb_height', 0.5),
        }

    @staticmethod
    def get_face_normal_and_slope(face: bmesh.types.BMFace) -> Tuple[mathutils.Vector, float]:
        """
        Calculate face normal and slope angle from Blender vertical (Z-up, up = 0 radians).
        
        Returns:
            (normal_vector, slope_angle_radians)
        """
        normal = face.normal.normalized()
        up_vector = mathutils.Vector((0.0, 0.0, 1.0))
        
        # Clamp to avoid numerical errors
        dot_product = max(-1.0, min(1.0, normal.dot(up_vector)))
        slope_angle = math.acos(dot_product)
        
        return normal, slope_angle

    @staticmethod
    def classify_face_type(slope_angle: float) -> str:
        """
        Classify face type based on slope angle from vertical.
        
        Args:
            slope_angle: radians (0 = horizontal floor, π/2 = vertical wall)
        
        Returns:
            'ground' | 'climbable' | 'ceiling'
        """
        angle_deg = math.degrees(slope_angle)
        
        if angle_deg < 45:
            return 'ground'
        elif angle_deg < 80:
            return 'climbable'
        return 'ceiling'

    def extract_walkable_surfaces(self) -> Dict[str, Dict]:
        """
        Scan all COL_* objects and extract walkable surfaces by type.
        
        Returns:
            dict mapping 'ground' | 'water' | 'climbable' | 'ceiling' to
            {'verts': [(x,y,z), ...], 'faces': [(i,j,k), ...]}
        """
        include_water = bool(getattr(self.scene, 'navmesh_include_water', True))
        include_climbable = bool(getattr(self.scene, 'navmesh_include_climbable', True))
        include_ceiling = bool(getattr(self.scene, 'navmesh_include_ceiling', False))
        target_collection = self._resolve_target_collection()

        col_objects = [
            obj for obj in self._collection_mesh_objects(target_collection)
            if obj.name.startswith("COL_") and obj.type == 'MESH'
        ]
        
        if not col_objects:
            print(f"[navmesh_autogen] WARNING: No COL_* objects found in collection '{target_collection.name}'!")
            return {}
        
        surface_meshes = {
            'ground': {'verts': [], 'faces': []},
            'water': {'verts': [], 'faces': []},
            'climbable': {'verts': [], 'faces': []},
            'ceiling': {'verts': [], 'faces': []},
        }
        
        for col_obj in col_objects:
            print(f"[navmesh_autogen] Processing {col_obj.name}...")
            force_water = include_water and self._is_water_object(col_obj)
            
            # Apply object transformations and get evaluated mesh
            depsgraph = self.context.evaluated_depsgraph_get()
            obj_eval = col_obj.evaluated_get(depsgraph)
            mesh = obj_eval.to_mesh()
            bm = bmesh.new()
            try:
                # Use BMesh for iteration and triangulation
                bm.from_mesh(mesh)
                bm.normal_update()

                triangulated_faces = []
                for face in bm.faces:
                    # Triangulate n-gons
                    try:
                        tris = bmesh.ops.triangulate(bm, faces=[face])['faces']
                        triangulated_faces.extend(tris)
                    except Exception:
                        triangulated_faces.append(face)

                for tri_face in triangulated_faces:
                    normal, slope = self.get_face_normal_and_slope(tri_face)
                    face_type = 'water' if force_water else self.classify_face_type(slope)

                    if face_type == 'water' and not include_water:
                        continue
                    if face_type == 'climbable' and not include_climbable:
                        continue
                    if face_type == 'ceiling' and not include_ceiling:
                        continue

                    # Transform vertices to world space and collect indices
                    vert_indices = []
                    for vert in tri_face.verts:
                        # World-space coordinate
                        world_co = col_obj.matrix_world @ vert.co
                        idx = len(surface_meshes[face_type]['verts'])
                        surface_meshes[face_type]['verts'].append(tuple(world_co))
                        vert_indices.append(idx)

                    if len(vert_indices) >= 3:
                        surface_meshes[face_type]['faces'].append(tuple(vert_indices[:3]))
            finally:
                bm.free()
                # Temporary evaluated mesh must be released via to_mesh_clear().
                obj_eval.to_mesh_clear()

            print(f"  ✓ {col_obj.name} processed")
        
        return surface_meshes

    def create_navmesh_in_blender(self, surface_meshes: Dict[str, Dict]) -> None:
        """
        Create visible navmesh geometry in Blender (NAV_AUTO_*) for user refinement.
        Assigns color materials by type for visual distinction.
        """
        color_map = {
            'ground': (0.2, 0.8, 0.2, 0.6),       # Green
            'water': (0.2, 0.6, 1.0, 0.4),        # Blue
            'climbable': (1.0, 0.8, 0.0, 0.5),    # Yellow/Orange
            'ceiling': (0.6, 0.2, 0.8, 0.6),      # Purple
        }
        target_collection = self._resolve_target_collection()
        
        for surface_type, data in surface_meshes.items():
            if not data['verts']:
                continue
            
            # Create mesh data
            mesh = bpy.data.meshes.new(f"nav_{surface_type}_mesh")
            mesh.from_pydata(data['verts'], [], data['faces'])
            mesh.update()
            
            # Create object
            obj = bpy.data.objects.new(f"NAV_AUTO_{surface_type}", mesh)
            target_collection.objects.link(obj)
            
            # Create and assign material
            mat = bpy.data.materials.new(f"mat_nav_{surface_type}")
            mat.use_nodes = True
            mat.shadow_method = 'NONE'  # Don't cast shadows
            mat.blend_method = 'BLEND'  # Allow transparency
            
            # Set color
            principled = mat.node_tree.nodes.get("Principled BSDF")
            if principled:
                principled.inputs[0].default_value = color_map.get(surface_type, (1, 1, 1, 1))
            
            obj.data.materials.append(mat)
            
            # Store surface type as custom property for later export
            obj['navmesh_type'] = surface_type
            
            print(f"  ✓ Created {obj.name} ({len(data['verts'])} verts, {len(data['faces'])} faces)")

    def generate_and_create(self) -> bool:
        """
        Main entry point: extract surfaces and create navmesh objects in Blender.
        
        Returns:
            True if successful, False otherwise
        """
        print("\n[navmesh_autogen] Starting navmesh auto-generation...")
        
        surfaces = self.extract_walkable_surfaces()
        total_surfaces = sum(1 for data in surfaces.values() if data['verts'])
        
        if total_surfaces == 0:
            print("[navmesh_autogen] ❌ No walkable surfaces generated!")
            return False
        
        self.create_navmesh_in_blender(surfaces)
        print(f"\n[navmesh_autogen] ✅ Generated {total_surfaces} surface(s)!")
        print("[navmesh_autogen] You can now edit NAV_AUTO_* meshes in Blender.")
        print("[navmesh_autogen] Export when ready.\n")
        
        return True


def cleanup_autogenerated_navmesh(context) -> int:
    """
    Delete all NAV_AUTO_* objects (for regeneration).
    
    Returns:
        Number of objects deleted
    """
    active_object = getattr(context, "active_object", None)
    if active_object is not None and getattr(active_object, "users_collection", None):
        target_collection = active_object.users_collection[0]
    else:
        active_layer = getattr(getattr(context, "view_layer", None), "active_layer_collection", None)
        if active_layer is not None and getattr(active_layer, "collection", None) is not None:
            target_collection = active_layer.collection
        elif getattr(context, "collection", None) is not None:
            target_collection = context.collection
        else:
            target_collection = context.scene.collection

    nav_auto_objects = [
        obj for obj in target_collection.all_objects
        if obj.name.startswith("NAV_AUTO_") and obj.type == 'MESH'
    ]
    
    for obj in nav_auto_objects:
        bpy.data.objects.remove(obj, do_unlink=True)
    
    if nav_auto_objects:
        print(f"[navmesh_autogen] Cleaned up {len(nav_auto_objects)} NAV_AUTO_* object(s) in collection '{target_collection.name}'")
    
    return len(nav_auto_objects)
