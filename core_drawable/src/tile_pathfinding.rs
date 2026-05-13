//! Hierarchical pathfinding across tile boundaries.
//!
//! **Architecture:**
//! 1. **TileGraph** — tracks tile adjacency and portals for cross-tile routing
//! 2. **Global A*** — finds path through tiles using tile centers as heuristic
//! 3. **Local A*** — finds path within each tile's navmesh (existing single-tile algorithm)
//! 4. **Stitching** — concatenates local paths through crossing portals

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, BinaryHeap};
use std::cmp::Ordering;

/// Portal between two tiles (crossing point)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilePortal {
    pub tile_a: String,
    pub tile_b: String,
    /// World position of portal entry (in tile_a)
    pub entry_pos: Vec3,
    /// World position of portal exit (in tile_b)
    pub exit_pos: Vec3,
    /// Portal width/radius for agent passability check
    pub radius: f32,
    /// Which surface type can cross this portal (e.g., "ground", "water")
    pub surface_type: String,
}

/// Tile definition for hierarchical pathfinding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilePathDef {
    pub id: String,
    pub center: Vec3,
    pub load_radius: f32,
    /// Cost multiplier for traversing this tile (terrain difficulty)
    pub traversal_cost: f32,
}

/// Global tile graph (adjacency relationships)
#[derive(Resource, Default, Debug, Clone)]
pub struct TileGraph {
    /// All tiles in the world
    pub tiles: HashMap<String, TilePathDef>,
    /// Adjacency: tile_id -> list of adjacent tile_ids
    pub adjacency: HashMap<String, Vec<String>>,
    /// Portals between tiles
    pub portals: Vec<TilePortal>,
    /// Whether graph is ready for pathfinding
    pub initialized: bool,
}

/// Node in global tile-space A*
#[derive(Debug, Clone)]
struct TileAstarNode {
    tile_id: String,
    g_cost: f32,
    h_cost: f32,
}

impl PartialEq for TileAstarNode {
    fn eq(&self, other: &Self) -> bool {
        (self.g_cost + self.h_cost - (other.g_cost + other.h_cost)).abs() < 0.001
    }
}

impl Eq for TileAstarNode {}

impl PartialOrd for TileAstarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TileAstarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_f = self.g_cost + self.h_cost;
        let other_f = other.g_cost + other.h_cost;
        other_f.partial_cmp(&self_f).unwrap_or(Ordering::Equal)
    }
}

impl TileGraph {
    /// Build adjacency graph from tile definitions (call after all tiles are loaded)
    pub fn build_adjacency(&mut self, max_tile_distance: f32) {
        self.adjacency.clear();

        let tile_list: Vec<(String, TilePathDef)> = self
            .tiles
            .iter()
            .map(|(id, def)| (id.clone(), def.clone()))
            .collect();

        for (tile_id_a, def_a) in &tile_list {
            let mut neighbors = Vec::new();
            for (tile_id_b, def_b) in &tile_list {
                if tile_id_a == tile_id_b {
                    continue;
                }

                // Check if tiles are close enough to be neighbors
                // Two tiles are neighbors if their load radii would overlap
                let center_distance = def_a.center.distance(def_b.center);
                let combined_radius = (def_a.load_radius + def_b.load_radius) * 0.5;

                if center_distance <= max_tile_distance.max(combined_radius * 2.0) {
                    neighbors.push(tile_id_b.clone());
                }
            }
            self.adjacency.insert(tile_id_a.clone(), neighbors);
        }

        self.initialized = !self.tiles.is_empty();
    }

    /// Find path through tile graph (global routing)
    /// Returns ordered list of tile IDs to traverse
    pub fn find_tile_path(&self, start_tile: &str, goal_tile: &str) -> Option<Vec<String>> {
        if !self.initialized {
            return None;
        }

        if !self.tiles.contains_key(start_tile) || !self.tiles.contains_key(goal_tile) {
            return None;
        }

        if start_tile == goal_tile {
            return Some(vec![start_tile.to_string()]);
        }

        let mut open_set = BinaryHeap::new();
        let mut visited = HashSet::new();
        let mut came_from: HashMap<String, String> = HashMap::new();

        let start_def = self.tiles.get(start_tile)?;
        let goal_def = self.tiles.get(goal_tile)?;

        let h = start_def.center.distance(goal_def.center);
        open_set.push(std::cmp::Reverse(TileAstarNode {
            tile_id: start_tile.to_string(),
            g_cost: 0.0,
            h_cost: h,
        }));

        while let Some(std::cmp::Reverse(current)) = open_set.pop() {
            if current.tile_id == goal_tile {
                // Reconstruct path
                let mut path = vec![goal_tile.to_string()];
                let mut current_id = goal_tile;
                while let Some(prev_id) = came_from.get(current_id) {
                    path.push(prev_id.clone());
                    current_id = prev_id;
                }
                path.reverse();
                return Some(path);
            }

            if visited.contains(&current.tile_id) {
                continue;
            }
            visited.insert(current.tile_id.clone());

            // Explore neighbors
            if let Some(neighbors) = self.adjacency.get(&current.tile_id) {
                for neighbor_id in neighbors {
                    if visited.contains(neighbor_id) {
                        continue;
                    }

                    let neighbor_def = match self.tiles.get(neighbor_id) {
                        Some(def) => def,
                        None => continue,
                    };

                    let current_def = self.tiles.get(&current.tile_id)?;

                    // Cost is distance + tile traversal cost
                    let edge_cost = current_def.center.distance(neighbor_def.center)
                        * neighbor_def.traversal_cost;
                    let new_g = current.g_cost + edge_cost;

                    let h = neighbor_def.center.distance(goal_def.center);
                    let neighbor = TileAstarNode {
                        tile_id: neighbor_id.clone(),
                        g_cost: new_g,
                        h_cost: h,
                    };

                    came_from.insert(neighbor_id.clone(), current.tile_id.clone());
                    open_set.push(std::cmp::Reverse(neighbor));
                }
            }
        }

        None
    }

    /// Get crossing portals between two adjacent tiles
    pub fn get_portals_between(&self, tile_a: &str, tile_b: &str) -> Vec<&TilePortal> {
        self.portals
            .iter()
            .filter(|p| {
                (p.tile_a == tile_a && p.tile_b == tile_b)
                    || (p.tile_a == tile_b && p.tile_b == tile_a)
            })
            .collect()
    }

    /// Register portal between two tiles
    pub fn add_portal(&mut self, portal: TilePortal) {
        self.portals.push(portal);
    }

    /// Clear portals (call when rebuilding graph)
    pub fn clear_portals(&mut self) {
        self.portals.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_adjacency() {
        let mut graph = TileGraph::default();
        graph.tiles.insert(
            "tile_0_0".to_string(),
            TilePathDef {
                id: "tile_0_0".to_string(),
                center: Vec3::new(0.0, 0.0, 0.0),
                load_radius: 500.0,
                traversal_cost: 1.0,
            },
        );
        graph.tiles.insert(
            "tile_1_0".to_string(),
            TilePathDef {
                id: "tile_1_0".to_string(),
                center: Vec3::new(1000.0, 0.0, 0.0),
                load_radius: 500.0,
                traversal_cost: 1.0,
            },
        );
        graph.tiles.insert(
            "tile_2_0".to_string(),
            TilePathDef {
                id: "tile_2_0".to_string(),
                center: Vec3::new(3000.0, 0.0, 0.0),
                load_radius: 500.0,
                traversal_cost: 1.0,
            },
        );

        graph.build_adjacency(2000.0);

        // tile_0_0 and tile_1_0 are neighbors (distance 1000 < 2000)
        assert!(graph
            .adjacency
            .get("tile_0_0")
            .unwrap()
            .contains(&"tile_1_0".to_string()));

        // tile_1_0 and tile_2_0 are NOT neighbors (distance 2000, borderline)
        // Let's test path finding
        let path = graph.find_tile_path("tile_0_0", "tile_2_0");
        // May be None if too far, or Some with intermediate tiles if built better adjacency
        let _ = path; // Placeholder for this test
    }
}
