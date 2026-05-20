//! Hierarchical LOD (HLOD) — GPU-driven rendering optimization for distant tiles.
//!
//! **Architecture:**
//! - **Close range** (0 to `radius_close`): Full detail mesh, individual entities
//! - **Mid range** (`radius_close` to `radius_mid`): Simplified HLOD mesh with GPU instancing
//! - **Far range** (beyond `radius_mid`): Culled or rendered as billboard markers

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// HLOD layer definition — one LOD tier in the distance hierarchy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HLODLayer {
    pub name: String,
    /// Minimum distance from camera to show this layer [m]
    pub min_distance: f32,
    /// Maximum distance to stop showing this layer [m]
    pub max_distance: f32,
    /// Mesh complexity reduction factor (0.0-1.0)
    /// 1.0 = full detail, 0.1 = 90% simplified
    pub simplification_factor: f32,
    /// Whether to use GPU instancing for this layer
    pub use_instancing: bool,
    /// Max instances per draw call (instancing buffer size)
    pub max_instances: u32,
}

impl Default for HLODLayer {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            min_distance: 0.0,
            max_distance: 1000.0,
            simplification_factor: 1.0,
            use_instancing: false,
            max_instances: 1024,
        }
    }
}

/// Standard HLOD configuration for maps
pub struct StandardHLODConfig;

impl StandardHLODConfig {
    /// Create standard 3-tier HLOD setup:
    /// - Tier 0: 0-300m, full detail, no instancing
    /// - Tier 1: 300-1000m, 60% simplified, GPU instanced
    /// - Tier 2: 1000m+, culled or billboarded
    pub fn create() -> Vec<HLODLayer> {
        vec![
            HLODLayer {
                name: "close".to_string(),
                min_distance: 0.0,
                max_distance: 300.0,
                simplification_factor: 1.0,
                use_instancing: false,
                max_instances: 0,
            },
            HLODLayer {
                name: "mid".to_string(),
                min_distance: 300.0,
                max_distance: 1000.0,
                simplification_factor: 0.6,
                use_instancing: true,
                max_instances: 2048,
            },
            HLODLayer {
                name: "far".to_string(),
                min_distance: 1000.0,
                max_distance: 5000.0,
                simplification_factor: 0.2,
                use_instancing: true,
                max_instances: 4096,
            },
        ]
    }
}

/// GPU instance data for one drawable in HLOD rendering
#[derive(Debug, Clone, Copy)]
pub struct HLODInstanceData {
    /// World position of instance [x, y, z, w=scale]
    pub position_scale: [f32; 4],
    /// Rotation quaternion [x, y, z, w]
    pub rotation: [f32; 4],
    /// Tint color [r, g, b, a]
    pub tint: [f32; 4],
}

/// HLOD state for a single drawable
#[derive(Component, Debug, Clone)]
pub struct HLODState {
    /// Which layer is currently active (index into parent tile's HLOD layers)
    pub active_layer: usize,
    /// Instance data storage (on CPU, would be uploaded to GPU)
    pub instances: Vec<HLODInstanceData>,
    /// Current instance count
    pub instance_count: u32,
    /// Distance to camera (updated every frame)
    pub camera_distance: f32,
    /// Whether visibility should be gated by layer
    pub apply_visibility_gating: bool,
}

impl Default for HLODState {
    fn default() -> Self {
        Self {
            active_layer: 0,
            instances: Vec::new(),
            instance_count: 0,
            camera_distance: f32::INFINITY,
            apply_visibility_gating: true,
        }
    }
}

/// Marker for HLOD-aware map instances
#[derive(Component, Debug, Clone)]
pub struct HLODTile {
    pub tile_id: String,
    /// HLOD layer configuration for this tile
    pub layers: Vec<HLODLayer>,
}

impl HLODTile {
    /// Create HLOD tile with standard 3-tier setup
    pub fn new(tile_id: String) -> Self {
        Self {
            tile_id,
            layers: StandardHLODConfig::create(),
        }
    }

    /// Find which layer should be active for given camera distance
    pub fn layer_at_distance(&self, distance: f32) -> Option<usize> {
        for (i, layer) in self.layers.iter().enumerate() {
            if distance >= layer.min_distance && distance <= layer.max_distance {
                return Some(i);
            }
        }
        None
    }
}

/// System to update HLOD visibility based on camera distance
pub fn update_hlod_visibility(
    camera_q: Query<&GlobalTransform, With<Camera3d>>,
    mut hlod_q: Query<(&GlobalTransform, &HLODTile, &mut HLODState, &mut Visibility)>,
) {
    let Ok(cam_tf) = camera_q.single() else {
        return;
    };

    let cam_pos = cam_tf.translation();

    for (obj_tf, hlod_tile, mut hlod_state, mut visibility) in hlod_q.iter_mut() {
        let obj_pos = obj_tf.translation();
        let distance = cam_pos.distance(obj_pos);
        hlod_state.camera_distance = distance;

        if let Some(layer_index) = hlod_tile.layer_at_distance(distance) {
            hlod_state.active_layer = layer_index;
            if hlod_state.apply_visibility_gating {
                *visibility = Visibility::Visible;
            }
        } else if hlod_state.apply_visibility_gating {
            *visibility = Visibility::Hidden;
        }
    }
}

/// System to build/update instance buffers for mid-tier HLOD rendering
#[allow(dead_code)]
pub fn update_hlod_instance_buffers(
    mut hlod_q: Query<&mut HLODState, Changed<HLODState>>,
) {
    for mut hlod_state in hlod_q.iter_mut() {
        hlod_state.instance_count = hlod_state.instances.len() as u32;
    }
}

/// Culling decision for HLOD mesh simplification
/// Returns true if the mesh should be simplified at the given distance.
#[allow(dead_code)]
pub fn should_simplify_mesh(distance: f32, max_distance: f32, simplification_factor: f32) -> bool {
    if simplification_factor >= 0.99 {
        return false;
    }
    distance > (max_distance * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hlod_config() {
        let layers = StandardHLODConfig::create();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].name, "close");
        assert_eq!(layers[1].name, "mid");
        assert_eq!(layers[2].name, "far");
    }

    #[test]
    fn test_layer_at_distance() {
        let tile = HLODTile::new("test".to_string());
        
        assert_eq!(tile.layer_at_distance(100.0), Some(0));  // close
        assert_eq!(tile.layer_at_distance(500.0), Some(1));  // mid
        assert_eq!(tile.layer_at_distance(2000.0), Some(2)); // far
    }

    #[test]
    fn test_mesh_simplification() {
        assert!(!should_simplify_mesh(100.0, 300.0, 1.0));  // Full detail
        assert!(should_simplify_mesh(200.0, 300.0, 0.6));   // Should simplify
    }
}
