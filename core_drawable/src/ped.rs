//! Apparatus Ped Physics Definition — `.ped.toml`
//!
//! Datový soubor popisující fyzikální a pohybové vlastnosti charakteru (peda).
//! Načítá se jako Bevy asset přes `PedPhysicsLoader` (přípona `.ped.toml`).
//!
//! Reference ze `.drawable` manifestu: `ped_physics = "player"` (bez přípony).
//! Loader hledá soubor `models/<id>.ped.toml` v assets.

use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Root struktura
// ---------------------------------------------------------------------------

/// Kompletní fyzikální a pohybový profil charakteru.
#[derive(Asset, TypePath, Debug, Clone, Deserialize)]
pub struct PedPhysicsDef {
    #[serde(default)]
    pub identity: PedIdentity,

    #[serde(default)]
    pub capsule: PedCapsule,

    #[serde(default)]
    pub movement: PedMovement,

    #[serde(default)]
    pub stances: PedStances,

    #[serde(default)]
    pub jump: PedJump,

    #[serde(default)]
    pub vault: PedVault,

    #[serde(default)]
    pub step: PedStep,

    #[serde(default)]
    pub lean: PedLean,

    #[serde(default)]
    pub stamina: PedStamina,

    #[serde(default)]
    pub water: PedWater,

    #[serde(default)]
    pub footstep: PedFootstep,

    #[serde(default)]
    pub ragdoll: PedRagdoll,
}

// ---------------------------------------------------------------------------
// Identita
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedIdentity {
    /// Zobrazovaný název (pro debug a editor).
    pub display_name: String,
    /// Jméno modelu (bez přípony) — musí odpovídat klíči v DrawableManifestRegistry.
    pub model: String,
}

impl Default for PedIdentity {
    fn default() -> Self {
        Self {
            display_name: "Default Ped".into(),
            model: "player".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Fyzikální tvar kapsle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedCapsule {
    /// Poloměr kapsle [m].
    pub radius: f32,
    /// Celková výška peda ve stoje [m].
    pub height: f32,
    /// Výška očí nad zemí (1st person kamera) [m].
    pub eye_height: f32,
    /// Hmotnost [kg] — ovlivňuje impulzy a interakci s physics objekty.
    pub mass: f32,
    /// Třecí koeficient kapsle (0 = žádné tření, pohyb čistě přes LinearVelocity).
    pub friction: f32,
    /// Koeficient odrazu (obvykle 0 — postavy se neodrážejí).
    pub restitution: f32,
    /// Lineární tlumení ve vzduchu (odpor vzduchu).
    pub linear_damping_air: f32,
    /// Lineární tlumení na zemi (okamžité brzdění — doporučeno 0, brzdi movement systém).
    pub linear_damping_ground: f32,
}

impl Default for PedCapsule {
    fn default() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            eye_height: 1.65,
            mass: 80.0,
            friction: 0.0,
            restitution: 0.0,
            linear_damping_air: 0.05,
            linear_damping_ground: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Pohyb
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedMovement {
    /// Rychlost chůze [m/s].
    pub walk_speed: f32,
    /// Rychlost běhu (výchozí pohyb) [m/s].
    pub run_speed: f32,
    /// Rychlost sprintu [m/s].
    pub sprint_speed: f32,
    /// Rychlost v podřepu [m/s].
    pub crouch_speed: f32,
    /// Rychlost v lehu [m/s].
    pub prone_speed: f32,
    /// Rychlost plavání [m/s].
    pub swim_speed: f32,
    /// Akcelerace na zemi [m/s²] — jak rychle se dosáhne cílové rychlosti.
    pub acceleration: f32,
    /// Zpomalení/brzdění na zemi [m/s²].
    pub deceleration: f32,
    /// Akcelerace ve vzduchu [m/s²] — vzdušná kontrola.
    pub air_acceleration: f32,
    /// Vyhlazení pohybu: 0 = okamžitá změna, vyšší = rychlejší dorovnání na cílovou rychlost.
    #[serde(default = "default_movement_smoothing")]
    pub movement_smoothing: f32,
    /// Maximální úhel svahu po kterém lze chodit [°].
    pub slope_max_angle: f32,
    /// Rychlost sklouznutí ze strmého svahu [m/s].
    pub slope_slide_speed: f32,
    /// Rotační rychlost modelu za pohybem (yaw snap) [°/s]. 0 = okamžitý snap.
    pub turn_speed: f32,
    /// Prah rychlosti [m/s] pod nímž se ped považuje za stojícího (animační blend).
    pub idle_threshold: f32,
    /// Multiplikátor rychlosti při ADS (míření).
    pub ads_speed_mult: f32,
    /// Multiplikátor pohybové rychlosti při střelbě od boku.
    pub hipfire_speed_mult: f32,
}

impl Default for PedMovement {
    fn default() -> Self {
        Self {
            walk_speed: 2.8,
            run_speed: 5.0,
            sprint_speed: 8.5,
            crouch_speed: 2.0,
            prone_speed: 1.2,
            swim_speed: 2.5,
            acceleration: 28.0,
            deceleration: 35.0,
            air_acceleration: 8.0,
            movement_smoothing: default_movement_smoothing(),
            slope_max_angle: 46.0,
            slope_slide_speed: 4.0,
            turn_speed: 0.0,
            idle_threshold: 0.1,
            ads_speed_mult: 0.6,
            hipfire_speed_mult: 0.9,
        }
    }
}

fn default_movement_smoothing() -> f32 {
    12.0
}

// ---------------------------------------------------------------------------
// Postoje
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedStances {
    pub standing: StanceDef,
    pub crouching: StanceDef,
    pub prone: StanceDef,
}

impl Default for PedStances {
    fn default() -> Self {
        Self {
            standing: StanceDef {
                capsule_height: 1.8,
                eye_offset_y: 1.65,
                transition_time: 0.12,
                speed_mult: 1.0,
                spread_bonus_deg: 0.0,
            },
            crouching: StanceDef {
                capsule_height: 1.1,
                eye_offset_y: 0.9,
                transition_time: 0.22,
                speed_mult: 0.45,
                spread_bonus_deg: -0.08,
            },
            prone: StanceDef {
                capsule_height: 0.4,
                eye_offset_y: 0.28,
                transition_time: 0.5,
                speed_mult: 0.22,
                spread_bonus_deg: -0.15,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StanceDef {
    /// Výška kapsle v tomto postoji [m].
    pub capsule_height: f32,
    /// Výška očí nad zemí v tomto postoji [m].
    pub eye_offset_y: f32,
    /// Doba přechodu do postoje [s].
    pub transition_time: f32,
    /// Multiplikátor rychlosti pohybu (1.0 = beze změny).
    pub speed_mult: f32,
    /// Korekce spreadu zbraně [°] — záporné = menší spread (přesnější).
    pub spread_bonus_deg: f32,
}

// ---------------------------------------------------------------------------
// Skok
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedJump {
    /// Počáteční vertikální impulz skoku [m/s].
    pub impulse: f32,
    /// Povoluje dvojitý skok.
    pub double_jump: bool,
    /// Impulz druhého skoku [m/s] (relevantní jen pokud double_jump = true).
    pub double_jump_impulse: f32,
    /// Coyote time — sekund po opuštění hrany kdy lze stále skočit [s].
    pub coyote_time: f32,
    /// Jump buffer — sekund před dopadem kdy se skok předregistruje [s].
    pub jump_buffer_time: f32,
    /// Gravitační multiplikátor při držení skokového tlačítka (floatier vzestup).
    pub hold_gravity_scale: f32,
    /// Gravitační multiplikátor při pádu (snappier dopad).
    pub fall_gravity_scale: f32,
    /// Gravitační multiplikátor při neutrálním vertikálním pohybu.
    pub base_gravity_scale: f32,
    /// Terminální rychlost pádu [m/s — absolutní hodnota].
    pub max_fall_speed: f32,
    /// Minimální vertikální rychlost [m/s] pro detekci "je na zemi".
    pub grounded_vel_threshold: f32,
    /// Maximální výška nárazu při přistání bez poškození [m] — základ pro fall damage.
    pub safe_fall_height: f32,
    /// Poškození za každý metr nad safe_fall_height.
    pub fall_damage_per_meter: f32,
    /// Zda lze skákat v podřepu.
    pub allow_crouch_jump: bool,
    /// Zda lze skákat v lehu.
    pub allow_prone_jump: bool,
    /// Vliv horizontální rychlosti na maximální výšku skoku (0 = žádný, 1 = lineární).
    pub speed_jump_bonus: f32,
}

impl Default for PedJump {
    fn default() -> Self {
        Self {
            impulse: 6.5,
            double_jump: false,
            double_jump_impulse: 5.0,
            coyote_time: 0.12,
            jump_buffer_time: 0.10,
            hold_gravity_scale: 0.65,
            fall_gravity_scale: 1.8,
            base_gravity_scale: 1.0,
            max_fall_speed: 28.0,
            grounded_vel_threshold: 0.25,
            safe_fall_height: 4.0,
            fall_damage_per_meter: 8.0,
            allow_crouch_jump: true,
            allow_prone_jump: false,
            speed_jump_bonus: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Vault / Mantle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedVault {
    /// Zapnutí vault systému.
    pub enabled: bool,
    /// Maximální výška překážky přes kterou lze vaultovat [m].
    pub max_obstacle_height: f32,
    /// Minimální výška pro vault (pod touto = auto step-up) [m].
    pub min_obstacle_height: f32,
    /// Vzdálenost dopředu pro detekci překážky [m].
    pub detection_dist: f32,
    /// Šířka detection raycastu (tolerance) [m].
    pub detection_width: f32,
    /// Délka vaultové animace/pohybu [s].
    pub duration: f32,
    /// Výška oblouku při vaultování [m].
    pub arc_height: f32,
    /// Horizontální vzdálenost přes kterou se ped přesune [m].
    pub forward_dist: f32,
    /// Minimální stamina pro vault (0 = vždy povoleno).
    pub stamina_cost: f32,
    /// Lze vaultovat ve sprintu.
    pub allow_while_sprint: bool,
    /// Lze vaultovat ve vzduchu (parkour).
    pub allow_while_airborne: bool,
}

impl Default for PedVault {
    fn default() -> Self {
        Self {
            enabled: true,
            max_obstacle_height: 1.3,
            min_obstacle_height: 0.3,
            detection_dist: 0.55,
            detection_width: 0.25,
            duration: 0.5,
            arc_height: 0.25,
            forward_dist: 0.7,
            stamina_cost: 8.0,
            allow_while_sprint: true,
            allow_while_airborne: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Step-up (automatický výstup na nízké hrany)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedStep {
    /// Maximální výška automatického výstupu [m].
    pub max_step_height: f32,
    /// Vzdálenost dopředu pro detekci hrany [m].
    pub search_dist: f32,
    /// Zda je step-up povolen.
    pub enabled: bool,
}

impl Default for PedStep {
    fn default() -> Self {
        Self {
            max_step_height: 0.32,
            search_dist: 0.28,
            enabled: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Náklon (lean)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedLean {
    pub enabled: bool,
    /// Maximální úhel náklonu [°].
    pub max_angle_deg: f32,
    /// Rychlost přechodu do/z náklonu [°/s].
    pub speed: f32,
    /// Horizontální offset těla při maximálním náklonu [m].
    pub body_offset_max: f32,
    /// Zda se kamera naklání (roll) při leanu.
    pub camera_roll: bool,
    /// Maximální roll kamery při plném náklonu [°].
    pub camera_roll_max_deg: f32,
    /// Head-bob při náklonu.
    pub head_bob: bool,
}

impl Default for PedLean {
    fn default() -> Self {
        Self {
            enabled: true,
            max_angle_deg: 22.0,
            speed: 10.0,
            body_offset_max: 0.3,
            camera_roll: true,
            camera_roll_max_deg: 5.0,
            head_bob: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Stamina
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedStamina {
    /// Maximální hodnota staminy.
    pub max: f32,
    /// Spotřeba staminy za sekundu při sprintu.
    pub sprint_drain: f32,
    /// Spotřeba staminy za skok.
    pub jump_cost: f32,
    /// Spotřeba staminy za vault.
    pub vault_cost: f32,
    /// Regenerace staminy za sekundu.
    pub regen_rate: f32,
    /// Prodleva [s] před začátkem regenerace po spotřebě.
    pub regen_delay: f32,
    /// Pod touto hranicí je ped "vyčerpaný" (debuffs).
    pub exhausted_threshold: f32,
    pub exhausted: StaminaExhausted,
}

impl Default for PedStamina {
    fn default() -> Self {
        Self {
            max: 100.0,
            sprint_drain: 14.0,
            jump_cost: 8.0,
            vault_cost: 10.0,
            regen_rate: 9.0,
            regen_delay: 1.5,
            exhausted_threshold: 10.0,
            exhausted: StaminaExhausted::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StaminaExhausted {
    /// Multiplikátor rychlosti v exhausted stavu.
    pub speed_mult: f32,
    /// Penalizace spreadu zbraně [°].
    pub spread_penalty_deg: f32,
    /// Multiplikátor ADS sway.
    pub ads_sway_mult: f32,
    /// Zda je sprint blokován.
    pub block_sprint: bool,
    /// Zda je skok blokován.
    pub block_jump: bool,
}

impl Default for StaminaExhausted {
    fn default() -> Self {
        Self {
            speed_mult: 0.55,
            spread_penalty_deg: 0.9,
            ads_sway_mult: 1.6,
            block_sprint: true,
            block_jump: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Voda
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedWater {
    /// Hloubka vody při které ped začne plavat [m].
    pub swim_depth_threshold: f32,
    /// Hloubka vody pro brodění (zpomalení pohybu) [m].
    pub wade_depth_threshold: f32,
    /// Multiplikátor rychlosti při brodění.
    pub wade_speed_mult: f32,
    /// Povoluje potápění.
    pub dive_enabled: bool,
    /// Délka dechu pod vodou [s].
    pub breath_capacity: f32,
    /// Poškození za sekundu bez dechu.
    pub drowning_dps: f32,
    /// Rychlost plavání pod vodou [m/s].
    pub underwater_speed: f32,
}

impl Default for PedWater {
    fn default() -> Self {
        Self {
            swim_depth_threshold: 1.2,
            wade_depth_threshold: 0.5,
            wade_speed_mult: 0.7,
            dive_enabled: true,
            breath_capacity: 30.0,
            drowning_dps: 5.0,
            underwater_speed: 3.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Kroky (footstep eventy)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedFootstep {
    /// Každých N metrů pohybu se emituje footstep event.
    pub emit_distance_walk: f32,
    /// Stride při běhu [m].
    pub emit_distance_run: f32,
    /// Stride při sprintu [m].
    pub emit_distance_sprint: f32,
    /// Stride v podřepu [m].
    pub emit_distance_crouch: f32,
    /// Zda se generují události pro AI slyšitelnost.
    pub emit_audio_events: bool,
}

impl Default for PedFootstep {
    fn default() -> Self {
        Self {
            emit_distance_walk: 0.75,
            emit_distance_run: 0.90,
            emit_distance_sprint: 1.10,
            emit_distance_crouch: 0.65,
            emit_audio_events: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Ragdoll
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PedRagdoll {
    /// Zapnutí ragdoll systému při smrti.
    pub enabled: bool,
    /// Síla impulzu při zásahu (pro hit reactions) [N·s].
    pub hit_impulse: f32,
    /// Minimální rychlost dopadu při které přejde do ragdollu (fall KO) [m/s].
    pub ko_fall_speed: f32,
    /// Jak dlouho zůstane ragdoll aktivní po přistání [s].
    pub active_duration: f32,
    /// Lineární tlumení ragdoll těla.
    pub angular_damping: f32,
}

impl Default for PedRagdoll {
    fn default() -> Self {
        Self {
            enabled: true,
            hit_impulse: 200.0,
            ko_fall_speed: 12.0,
            active_duration: 5.0,
            angular_damping: 0.5,
        }
    }
}

// ---------------------------------------------------------------------------
// AssetLoader
// ---------------------------------------------------------------------------

#[derive(Default, bevy::reflect::TypePath)]
pub struct PedPhysicsLoader;

#[derive(Debug, thiserror::Error)]
pub enum PedPhysicsError {
    #[error("IO error")]
    Io,
    #[error("TOML parse error: {0}")]
    Toml(String),
}

impl AssetLoader for PedPhysicsLoader {
    type Asset = PedPhysicsDef;
    type Settings = ();
    type Error = PedPhysicsError;

    async fn load(
        &self,
        reader: &mut dyn bevy::asset::io::Reader,
        _settings: &Self::Settings,
        _ctx: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        use bevy::asset::io::Reader as _;
        reader.read_to_end(&mut bytes).await.map_err(|_| PedPhysicsError::Io)?;
        let text = std::str::from_utf8(&bytes).map_err(|e| PedPhysicsError::Toml(e.to_string()))?;
        toml::from_str(text).map_err(|e| PedPhysicsError::Toml(e.to_string()))
    }

    fn extensions(&self) -> &[&str] {
        &["ped.toml"]
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Mapuje jméno peda (= bez přípony, např. `"player"`) na `Handle<PedPhysicsDef>`.
#[derive(Resource, Default)]
pub struct PedPhysicsRegistry(pub std::collections::HashMap<String, Handle<PedPhysicsDef>>);
