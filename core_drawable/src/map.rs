use serde::{Deserialize, Serialize};

/// TOML map descriptor loaded by host_client and tooling.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MapManifest {
    #[serde(default = "default_map_version")]
    pub version: String,
    #[serde(default)]
    pub instances: Vec<MapInstanceDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapInstanceDef {
    #[serde(default)]
    pub id: String,
    pub model: String,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default)]
    pub rotation_deg: [f32; 3],
    #[serde(default = "default_scale")]
    pub scale: [f32; 3],
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub navmesh_only: bool,
}

fn default_map_version() -> String {
    "1.0".to_string()
}

fn default_scale() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl MapManifest {
    pub fn from_toml_str(input: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(input)
    }

    pub fn to_toml_string_pretty(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}
