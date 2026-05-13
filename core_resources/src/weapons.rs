use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WeaponSpreadDef {
    #[serde(default)]
    pub base: f32,
    #[serde(default)]
    pub moving: f32,
    #[serde(default)]
    pub sprinting: f32,
    #[serde(default)]
    pub crouch: f32,
    #[serde(default)]
    pub prone: f32,
    #[serde(default)]
    pub ads: f32,
    #[serde(default)]
    pub ads_moving: f32,
    #[serde(default)]
    pub per_shot: f32,
    #[serde(default)]
    pub recovery_rps: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WeaponRecoilPointDef {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WeaponSlotFlags {
    #[serde(default)]
    pub optic: bool,
    #[serde(default)]
    pub muzzle: bool,
    #[serde(default)]
    pub barrel: bool,
    #[serde(default)]
    pub underbarrel: bool,
    #[serde(default)]
    pub stock: bool,
    #[serde(default)]
    pub magazine: bool,
    #[serde(default)]
    pub pistol_grip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WeaponDef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub caliber: String,
    #[serde(default)]
    pub default_ammo: String,
    #[serde(default)]
    pub barrel_length_mm: f32,
    #[serde(default)]
    pub twist_rate_inches: f32,
    #[serde(default)]
    pub fire_modes: Vec<String>,
    #[serde(default)]
    pub default_fire_mode: String,
    #[serde(default)]
    pub rpm: f32,
    #[serde(default)]
    pub fire_from_open_bolt: bool,
    #[serde(default)]
    pub mag_capacity: u32,
    #[serde(default)]
    pub reload_empty_sec: f32,
    #[serde(default)]
    pub reload_tactical_sec: f32,
    #[serde(default)]
    pub spread: WeaponSpreadDef,
    #[serde(default)]
    pub recoil_pattern: Vec<WeaponRecoilPointDef>,
    #[serde(default)]
    pub recoil_loop_from: u32,
    #[serde(default)]
    pub recoil_recovery_dps: f32,
    #[serde(default)]
    pub ads_fov_mult: f32,
    #[serde(default)]
    pub ads_time_sec: f32,
    #[serde(default)]
    pub mass_kg: f32,
    #[serde(default)]
    pub length_folded_mm: f32,
    #[serde(default)]
    pub length_extended_mm: f32,
    #[serde(default)]
    pub slots: WeaponSlotFlags,
    #[serde(default)]
    pub muzzle_thread: String,
    #[serde(default = "default_single_projectile")]
    pub pellet_count: u32,
    #[serde(default)]
    pub choke: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AmmoDef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub caliber: String,
    #[serde(default)]
    pub bullet_mass_g: f32,
    #[serde(default)]
    pub bullet_diameter_mm: f32,
    #[serde(default)]
    pub muzzle_velocity_mps: f32,
    #[serde(default)]
    pub reference_barrel_mm: f32,
    #[serde(default)]
    pub velocity_per_mm: f32,
    #[serde(default)]
    pub ballistic_model: String,
    #[serde(default)]
    pub ballistic_coeff: f32,
    #[serde(default)]
    pub base_damage: f32,
    #[serde(default)]
    pub damage_velocity_ref_mps: f32,
    #[serde(default)]
    pub penetration_class: u32,
    #[serde(default)]
    pub armor_penetration: f32,
    #[serde(default)]
    pub penetration_energy_j: f32,
    #[serde(default)]
    pub after_penetration_mult: f32,
    #[serde(default)]
    pub fragmentation_vel_mps: f32,
    #[serde(default)]
    pub fragmentation_factor: f32,
    #[serde(default = "default_scalar_one")]
    pub wound_mult: f32,
    #[serde(default)]
    pub tracer: bool,
    #[serde(default)]
    pub incendiary: bool,
    #[serde(default)]
    pub explosive: bool,
    #[serde(default)]
    pub subsonic: bool,
    #[serde(default)]
    pub bleed_chance: f32,
    #[serde(default)]
    pub bleed_dps: f32,
    #[serde(default)]
    pub crack_range_m: f32,
    #[serde(default)]
    pub thump_range_m: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AttachmentDef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub slot: String,
    #[serde(default)]
    pub compatible_threads: Vec<String>,
    #[serde(default = "default_scalar_one")]
    pub velocity_mult: f32,
    #[serde(default)]
    pub barrel_length_delta_mm: f32,
    #[serde(default)]
    pub db_reduction: f32,
    #[serde(default)]
    pub changes_sonic_signature: bool,
    #[serde(default)]
    pub ads_time_delta_sec: f32,
    #[serde(default = "default_scalar_one")]
    pub recoil_mult: f32,
    #[serde(default)]
    pub mass_kg: f32,
    #[serde(default)]
    pub ads_fov_mult_override: Option<f32>,
    #[serde(default)]
    pub zero_range_m: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MaterialDef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub thickness_mm: f32,
    #[serde(default)]
    pub hardness_brinell: f32,
    #[serde(default)]
    pub required_energy_j: f32,
    #[serde(default = "default_scalar_one")]
    pub velocity_retention: f32,
    #[serde(default)]
    pub ricochet_angle_deg: f32,
}

fn default_scalar_one() -> f32 {
    1.0
}

fn default_single_projectile() -> u32 {
    1
}

#[derive(Resource, Clone, Default)]
pub struct WeaponRegistry(pub Arc<RwLock<HashMap<String, WeaponDef>>>);

impl WeaponRegistry {
    pub fn insert(&self, id: String, mut def: WeaponDef) {
        if def.id.trim().is_empty() {
            def.id = id.clone();
        }
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, def);
    }

    pub fn get(&self, id: &str) -> Option<WeaponDef> {
        self.0
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    pub fn resolve_default(&self) -> Option<WeaponDef> {
        let guard = self.0.read().unwrap_or_else(|p| p.into_inner());
        if let Some(def) = guard.get("default") {
            return Some(def.clone());
        }
        if guard.len() == 1 {
            return guard.values().next().cloned();
        }
        None
    }
}

#[derive(Resource, Clone, Default)]
pub struct AmmoRegistry(pub Arc<RwLock<HashMap<String, AmmoDef>>>);

impl AmmoRegistry {
    pub fn insert(&self, id: String, mut def: AmmoDef) {
        if def.id.trim().is_empty() {
            def.id = id.clone();
        }
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, def);
    }

    pub fn get(&self, id: &str) -> Option<AmmoDef> {
        self.0
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }
}

#[derive(Resource, Clone, Default)]
pub struct AttachmentRegistry(pub Arc<RwLock<HashMap<String, AttachmentDef>>>);

impl AttachmentRegistry {
    pub fn insert(&self, id: String, mut def: AttachmentDef) {
        if def.id.trim().is_empty() {
            def.id = id.clone();
        }
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, def);
    }

    pub fn get(&self, id: &str) -> Option<AttachmentDef> {
        self.0
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }
}

#[derive(Resource, Clone, Default)]
pub struct MaterialRegistry(pub Arc<RwLock<HashMap<String, MaterialDef>>>);

impl MaterialRegistry {
    pub fn insert(&self, id: String, mut def: MaterialDef) {
        if def.id.trim().is_empty() {
            def.id = id.clone();
        }
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, def);
    }

    pub fn get(&self, id: &str) -> Option<MaterialDef> {
        self.0
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_registry_backfills_id() {
        let registry = WeaponRegistry::default();
        registry.insert(
            "ak47".to_string(),
            WeaponDef {
                display_name: "AK-47".to_string(),
                ..Default::default()
            },
        );

        let stored = registry.get("ak47").expect("weapon should exist");
        assert_eq!(stored.id, "ak47");
        assert_eq!(stored.pellet_count, 1);
    }

    #[test]
    fn ammo_registry_roundtrip() {
        let registry = AmmoRegistry::default();
        registry.insert(
            "762".to_string(),
            AmmoDef {
                caliber: "7.62x39".to_string(),
                ..Default::default()
            },
        );

        let stored = registry.get("762").expect("ammo should exist");
        assert_eq!(stored.id, "762");
        assert_eq!(stored.caliber, "7.62x39");
    }

    #[test]
    fn weapon_registry_prefers_default_id() {
        let registry = WeaponRegistry::default();
        registry.insert(
            "ak47".to_string(),
            WeaponDef {
                display_name: "AK-47".to_string(),
                ..Default::default()
            },
        );
        registry.insert(
            "default".to_string(),
            WeaponDef {
                display_name: "Fallback Rifle".to_string(),
                ..Default::default()
            },
        );

        let stored = registry.resolve_default().expect("default weapon should exist");
        assert_eq!(stored.id, "default");
    }
}
