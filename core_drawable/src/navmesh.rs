use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, BinaryHeap};
use std::cmp::Ordering;

/// Single walkable surface (ground, water, climbable, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavmeshSurface {
    pub id: String,
    pub surface_type: String, // "ground", "water", "climbable", "ceiling"
    pub vertices: Vec<Vec3>,
    pub faces: Vec<[u32; 3]>,
    
    // Agent constraints
    pub walkable_height: f32,
    pub walkable_radius: f32,
    pub climb_height: f32,
}

/// Complete navmesh for a map
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavmeshData {
    pub surfaces: Vec<NavmeshSurface>,
}

/// Global navmesh registry
#[derive(Resource, Default)]
pub struct NavmeshRegistry {
    pub navmeshes: HashMap<String, NavmeshData>, // map_id -> navmesh
}

/// Path segment for NPC navigation
#[derive(Debug, Clone, PartialEq)]
pub struct PathSegment {
    pub target: Vec3,
    pub face_index: u32,
    pub surface_id: String,
}

/// A* node for pathfinding
#[derive(Debug, Clone)]
struct AstarNode {
    pos: Vec3,
    face_idx: u32,
    g_cost: f32, // cost from start
    h_cost: f32, // heuristic to goal
}

impl PartialEq for AstarNode {
    fn eq(&self, other: &Self) -> bool {
        (self.g_cost + self.h_cost - (other.g_cost + other.h_cost)).abs() < 0.001
    }
}

impl Eq for AstarNode {}

impl PartialOrd for AstarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AstarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_f = self.g_cost + self.h_cost;
        let other_f = other.g_cost + other.h_cost;
        // Reverse for min-heap (BinaryHeap is max-heap by default)
        other_f.partial_cmp(&self_f).unwrap_or(Ordering::Equal)
    }
}

impl NavmeshRegistry {
    /// Load navmesh from TOML data
    pub fn load_navmesh(&mut self, map_id: impl Into<String>, toml_str: &str) -> Result<(), String> {
        let data: NavmeshData = toml::from_str(toml_str)
            .map_err(|e| format!("Failed to parse navmesh TOML: {}", e))?;
        self.navmeshes.insert(map_id.into(), data);
        Ok(())
    }

    /// Unload navmesh for map/tile key.
    pub fn unload_navmesh(&mut self, map_id: &str) {
        self.navmeshes.remove(map_id);
    }

    /// Find shortest path using A* algorithm
    pub fn find_path(
        &self,
        map_id: &str,
        from: Vec3,
        to: Vec3,
        agent_radius: f32,
    ) -> Option<Vec<PathSegment>> {
        let navmesh = self.navmeshes.get(map_id)?;
        
        // Find start and goal surfaces
        let start_surface = find_point_in_surface(&navmesh.surfaces, from, agent_radius)?;
        let goal_surface = find_point_in_surface(&navmesh.surfaces, to, agent_radius)?;
        
        // A* pathfinding
        self.astar_search(navmesh, from, to, start_surface, goal_surface, agent_radius)
    }

    /// Check if a position is walkable
    pub fn is_walkable(&self, map_id: &str, pos: Vec3, agent_radius: f32) -> bool {
        let navmesh = match self.navmeshes.get(map_id) {
            Some(nm) => nm,
            None => return false,
        };
        find_point_in_surface(&navmesh.surfaces, pos, agent_radius).is_some()
    }

    fn astar_search(
        &self,
        navmesh: &NavmeshData,
        start_pos: Vec3,
        goal_pos: Vec3,
        start_surface: (usize, u32), // (surface_idx, face_idx)
        goal_surface: (usize, u32),
        _agent_radius: f32,
    ) -> Option<Vec<PathSegment>> {
        let mut open_set = BinaryHeap::new();
        let mut visited = std::collections::HashSet::new();
        let mut came_from: HashMap<(usize, u32), ((usize, u32), Vec3)> = HashMap::new();

        let start_key = (start_surface.0, start_surface.1);
        let goal_key = (goal_surface.0, goal_surface.1);

        let h = heuristic(start_pos, goal_pos);
        open_set.push(std::cmp::Reverse(AstarNode {
            pos: start_pos,
            face_idx: start_surface.1,
            g_cost: 0.0,
            h_cost: h,
        }));

        while let Some(std::cmp::Reverse(current)) = open_set.pop() {
            if current.face_idx == goal_surface.1 && (start_surface.0 == goal_surface.0) {
                // Reconstruct path
                return Some(reconstruct_path(&came_from, start_key, goal_key, goal_pos));
            }

            let current_key = (start_surface.0, current.face_idx);
            if visited.contains(&current_key) {
                continue;
            }
            visited.insert(current_key);

            // Get neighbors (adjacent faces in same surface)
            let surface = &navmesh.surfaces[start_surface.0];

            // Check adjacent faces (simplified: just use face center as waypoint)
            for (neighbor_face_idx, neighbor_face) in surface.faces.iter().enumerate() {
                let neighbor_key = (start_surface.0, neighbor_face_idx as u32);
                
                if visited.contains(&neighbor_key) {
                    continue;
                }

                // Calculate cost to neighbor
                let neighbor_center = face_center(&surface.vertices, neighbor_face);
                let cost = current.pos.distance(neighbor_center);
                let new_g = current.g_cost + cost;

                let h = heuristic(neighbor_center, goal_pos);
                let neighbor = AstarNode {
                    pos: neighbor_center,
                    face_idx: neighbor_face_idx as u32,
                    g_cost: new_g,
                    h_cost: h,
                };

                came_from.insert(neighbor_key, (current_key, current.pos));
                open_set.push(std::cmp::Reverse(neighbor));
            }
        }

        None // No path found
    }

    /// Find path across multiple tiles using hierarchical A*
    /// Requires TileGraph to be populated with tile definitions
    pub fn find_hierarchical_path(
        &self,
        tile_graph: &crate::TileGraph,
        from_map_id: &str,
        to_map_id: &str,
        from_pos: Vec3,
        to_pos: Vec3,
        agent_radius: f32,
    ) -> Option<Vec<PathSegment>> {
        // If same tile, use single-tile pathfinding
        if from_map_id == to_map_id {
            return self.find_path(from_map_id, from_pos, to_pos, agent_radius);
        }

        // Get tile definitions
        let start_tile_id = from_map_id;
        let goal_tile_id = to_map_id;

        // Find tile path through graph
        let tile_path = tile_graph.find_tile_path(start_tile_id, goal_tile_id)?;

        // Build waypoints across all tiles
        let mut full_waypoints = Vec::new();
        let mut current_pos = from_pos;

        for i in 0..tile_path.len() {
            let current_tile = &tile_path[i];
            let next_tile = if i + 1 < tile_path.len() {
                Some(&tile_path[i + 1])
            } else {
                None
            };

            let target_pos = if let Some(next) = next_tile {
                // Find portal between current and next tile
                let portals = tile_graph.get_portals_between(current_tile, next);
                if !portals.is_empty() {
                    // Use first portal's exit position as intermediate target
                    portals[0].exit_pos
                } else {
                    // Fallback to tile center if no portal
                    if let Some(def) = tile_graph.tiles.get(next) {
                        def.center
                    } else {
                        to_pos
                    }
                }
            } else {
                // Last tile: target final position
                to_pos
            };

            // Find local path within current tile
            if let Some(mut local_path) = self.find_path(current_tile, current_pos, target_pos, agent_radius)
            {
                full_waypoints.append(&mut local_path);
            } else {
                // If no path found, add direct waypoint
                full_waypoints.push(PathSegment {
                    target: target_pos,
                    face_index: 0,
                    surface_id: current_tile.to_string(),
                });
            }

            current_pos = target_pos;
        }

        if full_waypoints.is_empty() {
            None
        } else {
            Some(full_waypoints)
        }
    }
}
fn find_point_in_surface(surfaces: &[NavmeshSurface], point: Vec3, agent_radius: f32) -> Option<(usize, u32)> {
    for (surf_idx, surface) in surfaces.iter().enumerate() {
        // Check if point height is within walkable range
        let min_y = surface.vertices.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
        let max_y = surface.vertices.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max);
        
        if point.y < min_y - agent_radius || point.y > max_y + agent_radius {
            continue;
        }

        // Find containing face via raycasting down
        for (face_idx, face) in surface.faces.iter().enumerate() {
            let v0 = surface.vertices[face[0] as usize];
            let v1 = surface.vertices[face[1] as usize];
            let v2 = surface.vertices[face[2] as usize];

            if point_in_triangle_xz(point, v0, v1, v2) {
                return Some((surf_idx, face_idx as u32));
            }
        }
    }
    None
}

/// Check if point (XZ only) is in triangle
fn point_in_triangle_xz(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> bool {
    let sign = |p1: Vec3, p2: Vec3, p3: Vec3| {
        (p1.x - p3.x) * (p2.z - p3.z) - (p2.x - p3.x) * (p1.z - p3.z)
    };

    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);

    let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);

    !(has_neg && has_pos)
}

fn heuristic(a: Vec3, b: Vec3) -> f32 {
    a.distance(b)
}

fn face_center(vertices: &[Vec3], face: &[u32; 3]) -> Vec3 {
    let v0 = *vertices.get(face[0] as usize).unwrap_or(&Vec3::ZERO);
    let v1 = *vertices.get(face[1] as usize).unwrap_or(&Vec3::ZERO);
    let v2 = *vertices.get(face[2] as usize).unwrap_or(&Vec3::ZERO);
    (v0 + v1 + v2) / 3.0
}

fn reconstruct_path(
    came_from: &HashMap<(usize, u32), ((usize, u32), Vec3)>,
    _start: (usize, u32),
    goal: (usize, u32),
    goal_pos: Vec3,
) -> Vec<PathSegment> {
    let mut path = vec![];
    let mut current = goal;

    while let Some((parent, pos)) = came_from.get(&current) {
        path.push(PathSegment {
            target: *pos,
            face_index: current.1,
            surface_id: format!("surface_{}", current.0),
        });
        current = *parent;
    }

    path.push(PathSegment {
        target: goal_pos,
        face_index: goal.1,
        surface_id: format!("surface_{}", goal.0),
    });

    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_in_triangle() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);
        let c = Vec3::new(0.0, 0.0, 1.0);
        
        let inside = Vec3::new(0.3, 0.0, 0.3);
        let outside = Vec3::new(1.5, 0.0, 0.5);

        assert!(point_in_triangle_xz(inside, a, b, c));
        assert!(!point_in_triangle_xz(outside, a, b, c));
    }
}
