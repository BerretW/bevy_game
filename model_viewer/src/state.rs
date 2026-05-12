use std::collections::HashMap;
use std::path::PathBuf;

use bevy::prelude::*;
use core_drawable::AnimationSet;

pub use core_drawable::GltfHandleCache;

#[derive(Resource)]
pub struct ModelPaths(pub Vec<PathBuf>);

#[derive(Resource, Default)]
pub struct ModelSourcePaths(pub HashMap<String, PathBuf>);

#[derive(Resource, Default)]
pub struct LodViewerState {
    pub forced: Option<u8>,
    pub panel_active: bool,
}

#[derive(Resource)]
pub struct ViewerState {
    pub grid_visible:      bool,
    pub overlay_visible:   bool,
    pub colliders_visible: bool,
    pub skeleton_visible:  bool,
}

#[derive(Resource, Default)]
pub struct RigViewerState {
    pub ik_mode: bool,
    pub ik_targets: Vec<Entity>,
    pub ik_names: Vec<String>,
    pub selected_ik: usize,
    pub hovered_ik: Option<usize>,
    pub dragging_ik: bool,
    pub drag_distance: f32,
    pub move_step: f32,
}

#[derive(Resource, Default)]
pub struct AdmAnimationBrowser {
    pub root: Option<Entity>,
    pub model_name: Option<String>,
    pub dictionaries: Vec<String>,
    pub selected_dict_idx: usize,
    pub clips: Vec<String>,
    pub selected_idx: usize,
    pub flags: u32,
    pub speed: f32,
    pub looping: bool,
    pub paused: bool,
    /// Notifies aktuálně vybraného klipu: (čas_sec, název)
    pub notifies: Vec<(f32, String)>,
}

#[derive(Resource, Default)]
pub struct ViewerSessionAnimHandles {
    pub handles: HashMap<String, Handle<AnimationSet>>,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self { grid_visible: true, overlay_visible: true, colliders_visible: false, skeleton_visible: false }
    }
}

/// Presety pro weather efekty (snow, dirt, wetness, porosity).
pub const WEATHER_PRESETS: &[(f32, f32, f32, f32, &str)] = &[
    (0.0, 0.0, 0.0, 0.0, "clean"),
    (0.0, 1.0, 0.0, 0.5, "dirty"),
    (0.0, 0.0, 1.0, 0.8, "wet"),
    (1.0, 0.0, 0.0, 0.0, "snowy"),
    (0.2, 0.6, 0.3, 0.5, "combined"),
];

/// Debug kanály pro vertex color visualizaci (tiling.y kódování pro shader).
/// 0.0 = normální render; záporné hodnoty → debug mode.
pub const VCOL_DEBUG_MODES: &[(f32, &str)] = &[
    ( 1.0, "normal"),
    (-1.0, "vcol RGB"),
    (-2.0, "vcol R (normal suppress)"),
    (-3.0, "vcol G (dirt)"),
    (-4.0, "vcol B (wet)"),
    (-5.0, "vcol A (palette)"),
];

pub const ADM_ANIM_MASK_ALL: u32 = 1;
pub const ADM_ANIM_MASK_RIGHT_UPPER_LIMB: u32 = 14;
pub const ADM_ANIM_MASK_LEFT_UPPER_LIMB: u32 = 112;
pub const ADM_ANIM_MASK_LOWER_BODY: u32 = 8064;
pub const ADM_ANIM_MASK_UPPER_BODY: u32 = 24576;

#[derive(Resource, Default)]
pub struct WeatherState {
    pub preset_idx: usize,
    pub vcol_idx:   usize,
}

#[derive(Component)]
pub struct InfoOverlay;

#[derive(Component)]
pub struct DebugStatus;

#[derive(Component)]
pub struct ColliderPanel;

#[derive(Component)]
pub struct LodPanel;

#[derive(Component)]
pub struct AnimPanel;

#[derive(Component)]
pub struct RigPanel;

#[derive(Component)]
pub struct AnimDebugPanel;

#[derive(Component)]
pub struct UiLoadAdmButton;

#[derive(Component)]
pub struct UiLoadAnimButton;

#[derive(Component)]
pub struct UiDiagAnimButton;

#[derive(Resource, Default)]
pub struct AnimDebugState {
    pub text: String,
}

pub struct ViewerIkChainDef {
    pub name: String,
    pub enabled: bool,
    pub parent_bone: String,
    pub ik_target: String,
    pub effector_bone: String,
    pub pole_bone: Option<String>,
}

#[derive(Resource, Default)]
pub struct ViewerIkChains {
    pub chains: Vec<ViewerIkChainDef>,
}
