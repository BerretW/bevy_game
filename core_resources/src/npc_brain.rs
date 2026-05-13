use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

mod replicated_json {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value as Json;

    pub fn serialize<S>(value: &Json, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serde_json::to_string(value)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Json, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        serde_json::from_str(&encoded).map_err(serde::de::Error::custom)
    }
}

pub const CORE_HUMAN_BRAIN_ID: &str = "core/human";
pub const CORE_ANIMAL_BRAIN_ID: &str = "core/animal";
pub const CORE_VEHICLE_BRAIN_ID: &str = "core/vehicle";
pub const CORE_BIRD_BRAIN_ID: &str = "core/bird";
pub const CORE_FISH_BRAIN_ID: &str = "core/fish";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcBrainKind {
    Human,
    Animal,
    Vehicle,
    Bird,
    Fish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcLocomotionMode {
    Biped,
    Quadruped,
    Wheeled,
    Flight,
    Swim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcTaskKind {
    Idle,
    Ambient,
    WanderZone,
    PatrolRoute,
    UseScenarioPoint,
    FollowTarget,
    ChaseTarget,
    Flee,
    Investigate,
    Combat,
    DriveRoute,
    FlyRoute,
    SwimRoute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NpcPerceptionDef {
    pub sight_range: f32,
    pub hearing_range: f32,
    pub alert_range: f32,
}

impl Default for NpcPerceptionDef {
    fn default() -> Self {
        Self {
            sight_range: 30.0,
            hearing_range: 12.0,
            alert_range: 18.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NpcMotionDef {
    pub cruise_speed: f32,
    pub sprint_speed: f32,
    pub turn_speed: f32,
    pub brake_distance: f32,
}

impl Default for NpcMotionDef {
    fn default() -> Self {
        Self {
            cruise_speed: 2.5,
            sprint_speed: 4.5,
            turn_speed: 10.0,
            brake_distance: 0.35,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NpcNavigationDef {
    pub use_navmesh: bool,
    pub use_avoidance: bool,
    pub terrain_snap: bool,
    pub repath_interval_sec: f32,
    pub target_repath_delta: f32,
}

impl Default for NpcNavigationDef {
    fn default() -> Self {
        Self {
            use_navmesh: true,
            use_avoidance: true,
            terrain_snap: true,
            repath_interval_sec: 0.35,
            target_repath_delta: 0.75,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NpcBrainDef {
    pub id: String,
    pub label: String,
    pub kind: NpcBrainKind,
    pub locomotion: NpcLocomotionMode,
    pub default_task: NpcTaskKind,
    pub allowed_tasks: Vec<NpcTaskKind>,
    pub perception: NpcPerceptionDef,
    pub motion: NpcMotionDef,
    pub navigation: NpcNavigationDef,
    pub scenario_tags: Vec<String>,
}

impl Default for NpcBrainDef {
    fn default() -> Self {
        Self::core_human()
    }
}

impl NpcBrainDef {
    pub fn core_human() -> Self {
        Self {
            id: CORE_HUMAN_BRAIN_ID.to_string(),
            label: "Core Human".to_string(),
            kind: NpcBrainKind::Human,
            locomotion: NpcLocomotionMode::Biped,
            default_task: NpcTaskKind::Ambient,
            allowed_tasks: vec![
                NpcTaskKind::Idle,
                NpcTaskKind::Ambient,
                NpcTaskKind::WanderZone,
                NpcTaskKind::PatrolRoute,
                NpcTaskKind::UseScenarioPoint,
                NpcTaskKind::FollowTarget,
                NpcTaskKind::ChaseTarget,
                NpcTaskKind::Flee,
                NpcTaskKind::Investigate,
                NpcTaskKind::Combat,
            ],
            perception: NpcPerceptionDef::default(),
            motion: NpcMotionDef::default(),
            navigation: NpcNavigationDef::default(),
            scenario_tags: vec!["human".to_string(), "ambient".to_string()],
        }
    }

    pub fn core_animal() -> Self {
        Self {
            id: CORE_ANIMAL_BRAIN_ID.to_string(),
            label: "Core Animal".to_string(),
            kind: NpcBrainKind::Animal,
            locomotion: NpcLocomotionMode::Quadruped,
            default_task: NpcTaskKind::WanderZone,
            allowed_tasks: vec![
                NpcTaskKind::Idle,
                NpcTaskKind::Ambient,
                NpcTaskKind::WanderZone,
                NpcTaskKind::FollowTarget,
                NpcTaskKind::Flee,
                NpcTaskKind::Investigate,
            ],
            perception: NpcPerceptionDef {
                sight_range: 24.0,
                hearing_range: 20.0,
                alert_range: 16.0,
            },
            motion: NpcMotionDef {
                cruise_speed: 3.4,
                sprint_speed: 6.5,
                turn_speed: 8.5,
                brake_distance: 0.45,
            },
            navigation: NpcNavigationDef::default(),
            scenario_tags: vec!["animal".to_string(), "herd".to_string()],
        }
    }

    pub fn core_vehicle() -> Self {
        Self {
            id: CORE_VEHICLE_BRAIN_ID.to_string(),
            label: "Core Vehicle".to_string(),
            kind: NpcBrainKind::Vehicle,
            locomotion: NpcLocomotionMode::Wheeled,
            default_task: NpcTaskKind::DriveRoute,
            allowed_tasks: vec![
                NpcTaskKind::Idle,
                NpcTaskKind::Ambient,
                NpcTaskKind::DriveRoute,
                NpcTaskKind::FollowTarget,
                NpcTaskKind::Flee,
                NpcTaskKind::ChaseTarget,
            ],
            perception: NpcPerceptionDef {
                sight_range: 60.0,
                hearing_range: 8.0,
                alert_range: 30.0,
            },
            motion: NpcMotionDef {
                cruise_speed: 9.0,
                sprint_speed: 18.0,
                turn_speed: 4.0,
                brake_distance: 3.0,
            },
            navigation: NpcNavigationDef {
                use_navmesh: false,
                use_avoidance: true,
                terrain_snap: true,
                repath_interval_sec: 0.75,
                target_repath_delta: 2.0,
            },
            scenario_tags: vec!["vehicle".to_string(), "traffic".to_string()],
        }
    }

    pub fn core_bird() -> Self {
        Self {
            id: CORE_BIRD_BRAIN_ID.to_string(),
            label: "Core Bird".to_string(),
            kind: NpcBrainKind::Bird,
            locomotion: NpcLocomotionMode::Flight,
            default_task: NpcTaskKind::FlyRoute,
            allowed_tasks: vec![
                NpcTaskKind::Idle,
                NpcTaskKind::Ambient,
                NpcTaskKind::WanderZone,
                NpcTaskKind::FlyRoute,
                NpcTaskKind::Flee,
                NpcTaskKind::Investigate,
            ],
            perception: NpcPerceptionDef {
                sight_range: 80.0,
                hearing_range: 10.0,
                alert_range: 40.0,
            },
            motion: NpcMotionDef {
                cruise_speed: 5.0,
                sprint_speed: 11.0,
                turn_speed: 6.0,
                brake_distance: 1.0,
            },
            navigation: NpcNavigationDef {
                use_navmesh: false,
                use_avoidance: true,
                terrain_snap: false,
                repath_interval_sec: 0.5,
                target_repath_delta: 1.5,
            },
            scenario_tags: vec!["bird".to_string(), "air".to_string()],
        }
    }

    pub fn core_fish() -> Self {
        Self {
            id: CORE_FISH_BRAIN_ID.to_string(),
            label: "Core Fish".to_string(),
            kind: NpcBrainKind::Fish,
            locomotion: NpcLocomotionMode::Swim,
            default_task: NpcTaskKind::SwimRoute,
            allowed_tasks: vec![
                NpcTaskKind::Idle,
                NpcTaskKind::Ambient,
                NpcTaskKind::WanderZone,
                NpcTaskKind::SwimRoute,
                NpcTaskKind::Flee,
            ],
            perception: NpcPerceptionDef {
                sight_range: 18.0,
                hearing_range: 4.0,
                alert_range: 10.0,
            },
            motion: NpcMotionDef {
                cruise_speed: 2.8,
                sprint_speed: 5.2,
                turn_speed: 7.5,
                brake_distance: 0.4,
            },
            navigation: NpcNavigationDef {
                use_navmesh: false,
                use_avoidance: true,
                terrain_snap: false,
                repath_interval_sec: 0.4,
                target_repath_delta: 1.0,
            },
            scenario_tags: vec!["fish".to_string(), "water".to_string()],
        }
    }
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct NpcBrainState {
    pub brain_id: String,
}

impl Default for NpcBrainState {
    fn default() -> Self {
        Self::human_fallback()
    }
}

impl NpcBrainState {
    pub fn new(brain_id: impl Into<String>) -> Self {
        Self {
            brain_id: brain_id.into(),
        }
    }

    pub fn human_fallback() -> Self {
        Self::new(CORE_HUMAN_BRAIN_ID)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NpcBrainTarget {
    None,
    Position(Vec3),
    Entity(u64),
}

impl Default for NpcBrainTarget {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Component, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplicatedNpcBrain {
    pub brain_id: String,
    pub task: NpcTaskKind,
    pub scenario_id: Option<String>,
    pub target: NpcBrainTarget,
    #[serde(with = "replicated_json")]
    pub params: Json,
}

impl Default for ReplicatedNpcBrain {
    fn default() -> Self {
        Self::human_idle()
    }
}

impl ReplicatedNpcBrain {
    pub fn new(brain_id: impl Into<String>, task: NpcTaskKind) -> Self {
        Self {
            brain_id: brain_id.into(),
            task,
            scenario_id: None,
            target: NpcBrainTarget::None,
            params: Json::Object(Default::default()),
        }
    }

    pub fn human_idle() -> Self {
        Self::new(CORE_HUMAN_BRAIN_ID, NpcTaskKind::Idle)
    }

    pub fn with_target(mut self, target: NpcBrainTarget) -> Self {
        self.target = target;
        self
    }

    pub fn with_scenario(mut self, scenario_id: Option<String>) -> Self {
        self.scenario_id = scenario_id;
        self
    }

    pub fn with_params(mut self, params: Json) -> Self {
        self.params = params;
        self
    }
}

#[derive(Resource, Debug, Clone)]
pub struct NpcBrainRegistry {
    pub fallback_human_id: String,
    pub defs: HashMap<String, NpcBrainDef>,
}

impl Default for NpcBrainRegistry {
    fn default() -> Self {
        let mut registry = Self {
            fallback_human_id: CORE_HUMAN_BRAIN_ID.to_string(),
            defs: HashMap::new(),
        };
        registry.upsert(NpcBrainDef::core_human());
        registry.upsert(NpcBrainDef::core_animal());
        registry.upsert(NpcBrainDef::core_vehicle());
        registry.upsert(NpcBrainDef::core_bird());
        registry.upsert(NpcBrainDef::core_fish());
        registry
    }
}

impl NpcBrainRegistry {
    pub fn upsert(&mut self, mut def: NpcBrainDef) {
        if def.id.trim().is_empty() {
            def.id = self.fallback_human_id.clone();
        }
        self.defs.insert(def.id.clone(), def);
    }

    pub fn get(&self, brain_id: &str) -> Option<&NpcBrainDef> {
        self.defs.get(brain_id)
    }

    pub fn resolve_or_fallback(&self, brain_id: &str) -> &NpcBrainDef {
        self.get(brain_id)
            .or_else(|| self.get(&self.fallback_human_id))
            .expect("NpcBrainRegistry must always contain fallback human brain")
    }

    pub fn canonical_brain_id(&self, brain_id: &str) -> String {
        self.resolve_or_fallback(brain_id).id.clone()
    }
}