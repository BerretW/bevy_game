use bevy::prelude::*;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Výchozí LOD vzdálenosti (metry) pro drawable assety bez explicitní `[lod]` sekce.
///
/// Použije se, pokud model obsahuje `_LODn` uzly ale manifest nemá `[lod].distances`.
/// Editovatelné přes `DefaultLodDistances` resource nebo při stavu buildu:
/// ```rust
/// app.insert_resource(DefaultLodDistances(vec![20.0, 60.0, 120.0]));
/// ```
#[derive(Resource)]
pub struct DefaultLodDistances(pub Vec<f32>);

impl Default for DefaultLodDistances {
    fn default() -> Self {
        // LOD0→LOD1 při 15 m, LOD1→LOD2 při 40 m, LOD2→LOD3 při 80 m
        DefaultLodDistances(vec![15.0, 40.0, 80.0])
    }
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// LOD skupina na root entitě drawable (přidaná hook systémem).
///
/// Ukládá entity pro každou LOD úroveň. Systém `update_lod_visibility`
/// každý snímek přepíná viditelnost na základě vzdálenosti kamery.
#[derive(Component, Clone)]
pub struct LodGroup {
    /// Hraniční vzdálenosti na čtverec (optimalizace — vyhne se `sqrt`).
    /// `lod_dist_sq[0]` = přechod LOD0→LOD1 (v metrech²).
    pub lod_dist_sq: Vec<f32>,
    /// Skrýt model za poslední LOD vzdáleností.
    pub cull_beyond_last: bool,
    /// Aktuálně aktivní LOD úroveň. `u8::MAX` = skryto (cull).
    /// Inicializováno na `u8::MAX` aby první frame vždy aktualizoval viditelnost.
    pub active_lod: u8,
    /// Entity pro každou LOD úroveň: `lod_entities[0]` = LOD0, `lod_entities[1]` = LOD1, …
    /// Každá entry je named GLTF uzel; jeho potomci dědí viditelnost.
    pub lod_entities: Vec<Vec<Entity>>,
}

/// Marker na GLTF uzlu — říká na jaké LOD úrovni uzel leží.
/// Přidáno hook systémem při zpracování drawable scény.
#[derive(Component)]
pub struct LodLevel(pub u8);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parsuje LOD úroveň z názvu GLTF uzlu.
///
/// | Název         | Výsledek |
/// |---------------|---------|
/// | `Wall`        | 0       |
/// | `Wall_LOD0`   | 0       |
/// | `Wall_LOD1`   | 1       |
/// | `Barrel_LOD3` | 3       |
pub fn parse_lod_level(name: &str) -> u8 {
    if let Some(pos) = name.rfind("_LOD") {
        let suffix = &name[pos + 4..];
        // Pouze čísla za _LOD (žádné extra znaky)
        if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(level) = suffix.parse::<u8>() {
                return level;
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Přepíná LOD viditelnost na základě vzdálenosti od aktivní kamery.
///
/// Spouští se každý snímek v `Update`. Na serveru (bez `Camera3d`) je no-op.
/// Aktualizuje viditelnost pouze když se LOD úroveň skutečně změní — žádná
/// nadbytečná práce při stabilní vzdálenosti.
pub fn update_lod_visibility(
    camera_q: Query<&GlobalTransform, With<Camera3d>>,
    mut lod_groups: Query<(&GlobalTransform, &mut LodGroup)>,
    mut visibility_q: Query<&mut Visibility>,
) {
    let Ok(cam_tf) = camera_q.single() else { return };
    let cam_pos = cam_tf.translation();

    for (group_tf, mut lod) in &mut lod_groups {
        let dist_sq = group_tf.translation().distance_squared(cam_pos);

        // Zjisti cíl: za poslední hranicí a cull_beyond_last → schovej vše
        let culled = lod.cull_beyond_last
            && lod.lod_dist_sq.last().map(|&d| dist_sq > d).unwrap_or(false);

        let target = if culled {
            u8::MAX
        } else {
            // Najdi první LOD jehož hranice je za vzdáleností kamery
            let mut lvl = lod.lod_entities.len().saturating_sub(1) as u8;
            for (i, &threshold_sq) in lod.lod_dist_sq.iter().enumerate() {
                if dist_sq < threshold_sq {
                    lvl = i as u8;
                    break;
                }
            }
            lvl
        };

        // Beze změny = přeskočit (žádné zbytečné mutace ECS)
        if lod.active_lod == target {
            continue;
        }
        lod.active_lod = target;

        // Aplikuj viditelnost na každou LOD úroveň
        for (level_idx, entities) in lod.lod_entities.iter().enumerate() {
            let show = !culled && level_idx as u8 == target;
            let vis = if show {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            for &entity in entities {
                if let Ok(mut v) = visibility_q.get_mut(entity) {
                    *v = vis;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_lod_level;

    #[test]
    fn parse_lod_no_suffix() {
        assert_eq!(parse_lod_level("Wall"), 0);
        assert_eq!(parse_lod_level("Barrel_Base"), 0);
    }

    #[test]
    fn parse_lod_explicit_levels() {
        assert_eq!(parse_lod_level("Wall_LOD0"), 0);
        assert_eq!(parse_lod_level("Wall_LOD1"), 1);
        assert_eq!(parse_lod_level("Barrel_LOD2"), 2);
        assert_eq!(parse_lod_level("Complex_Name_LOD3"), 3);
    }

    #[test]
    fn parse_lod_no_false_positives() {
        // "_LOD" uprostřed jména (ne na konci) → 0
        assert_eq!(parse_lod_level("LOD_Wall"), 0);
        // Neplatné číslo za _LOD → 0
        assert_eq!(parse_lod_level("Wall_LODx"), 0);
        assert_eq!(parse_lod_level("Wall_LOD"), 0);
    }
}
