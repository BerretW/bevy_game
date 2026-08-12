//! Inverse Kinematics (IK) systém pro dynamickou přizpůsobení postav terénu.
//!
//! Umožňuje postavám správně chodit po schodech a jiných nerovných plochách
//! prostřednictvím Two-Bone IK solveru aplikovaného na nohy.

use bevy::prelude::*;
use core_resources::IkEnabledComponent;

// Type alias pro kompatibilitu
pub type IkEnabled = IkEnabledComponent;

// ---------------------------------------------------------------------------
// Komponenty
// ---------------------------------------------------------------------------

/// Marker: entita se nachází na schodech (detekováno kolizí se STAIRS materiálem).
#[derive(Component, Debug, Clone, Copy, Reflect)]
pub struct OnStairs {
    /// Výška terénu pod levou nohou [m] od pevného bodu.
    pub left_foot_height: f32,
    /// Výška terénu pod pravou nohou [m] od pevného bodu.
    pub right_foot_height: f32,
}

impl Default for OnStairs {
    fn default() -> Self {
        Self {
            left_foot_height: 0.0,
            right_foot_height: 0.0,
        }
    }
}

/// Definice IK řetězce pro jednu nohu.
/// 
/// Typicky:
/// - parent_bone: "DEF_upper_leg"
/// - ik_target: "IK_foot_l"
/// - effector_bone: "DEF_foot"
#[derive(Component, Debug, Clone, Reflect)]
pub struct IkChain {
    /// Jméno parent kosti (např. "DEF_upper_leg").
    pub parent_bone_name: String,
    /// Jméno IK target (např. "IK_foot_l").
    pub ik_target_name: String,
    /// Jméno effector kosti na konci chain (např. "DEF_foot").
    pub effector_bone_name: String,
    /// Maximální délka řetězce [m].
    pub chain_length: f32,
    /// Počet iterací Two-Bone IK solveru (typicky 1-3).
    pub solver_iterations: usize,
    /// Minimální úhel v knee joinu [°].
    pub min_knee_angle: f32,
    /// Maximální úhel v knee joinu [°].
    pub max_knee_angle: f32,
}

impl IkChain {
    pub fn new(
        parent_bone: impl Into<String>,
        ik_target: impl Into<String>,
        effector_bone: impl Into<String>,
    ) -> Self {
        Self {
            parent_bone_name: parent_bone.into(),
            ik_target_name: ik_target.into(),
            effector_bone_name: effector_bone.into(),
            chain_length: 1.2,
            solver_iterations: 2,
            min_knee_angle: 15.0,
            max_knee_angle: 170.0,
        }
    }
}

/// State pro řešení IK: ukládá meziresultáty během solvingu.
#[derive(Component, Debug, Clone, Default, Reflect)]
pub struct IkSolverState {
    /// Aktuální pozice effector bodu [world-space].
    pub effector_pos: Vec3,
    /// Cílová pozice [world-space].
    pub target_pos: Vec3,
    /// Pozice middle joint (koleno).
    pub middle_pos: Vec3,
    /// Aktualní práce na výpočtu.
    pub is_solving: bool,
}

// ---------------------------------------------------------------------------
// Two-Bone IK Solver
// ---------------------------------------------------------------------------

/// Implementace Two-Bone IK solveru.
/// 
/// Řeší pozice joints v řetězci:
/// parent_joint -> middle_joint -> effector
/// 
/// Cílem je přesunout effector na target_pos při zachování délky obou segmentů.
pub struct TwoBoneIkSolver;

impl TwoBoneIkSolver {
    /// Řeší IK a vrací nové pozice pro middle a effector joints.
    ///
    /// # Arguments
    /// * `parent_pos` - Pozice parent joinu (např. hip) [world-space]
    /// * `target_pos` - Cílová pozice effector joinu (např. noha) [world-space]
    /// * `segment1_length` - Délka prvního segmentu (parent -> middle) [m]
    /// * `segment2_length` - Délka druhého segmentu (middle -> effector) [m]
    /// * `pole_vector` - Hint vektor pro orientaci kolene
    ///
    /// # Returns
    /// (middle_pos, effector_pos)
    pub fn solve(
        parent_pos: Vec3,
        target_pos: Vec3,
        segment1_length: f32,
        segment2_length: f32,
        pole_vector: Vec3,
    ) -> (Vec3, Vec3) {
        let total_length = segment1_length + segment2_length;
        let distance_to_target = parent_pos.distance(target_pos);

        // Pokud je target moc daleko, natáhne se řetězec do přímky
        if distance_to_target >= total_length * 0.99 {
            let dir = (target_pos - parent_pos).normalize();
            let middle = parent_pos + dir * segment1_length;
            let effector = parent_pos + dir * total_length;
            return (middle, effector);
        }

        // Pokud je target moc blízko, stáhne se řetězec
        if distance_to_target < 0.01 {
            return (parent_pos, parent_pos);
        }

        // Two-Bone IK: law of cosines pro výpočet úhlů
        let a = segment1_length;
        let b = segment2_length;
        let c = distance_to_target;

        // cos(angle_at_parent)
        let cos_parent = ((a * a + c * c - b * b) / (2.0 * a * c)).clamp(-1.0, 1.0);
        let angle_at_parent = cos_parent.acos();

        // Směr od parent k target
        let to_target = (target_pos - parent_pos).normalize();

        // Pole vector pro určení orientace kolene
        let pole_hint = if pole_vector.length() > 0.01 {
            pole_vector.normalize()
        } else {
            // Fallback: koleno směřuje nahoru
            Vec3::Y
        };

        // Počítáme třetí osu pro rotaci
        let axis = to_target.cross(pole_hint);
        let axis = if axis.length() > 0.01 {
            axis.normalize()
        } else {
            // Fallback: pokud pole_hint je paralelní s to_target, použij X osu
            Vec3::X
        };

        // Rotujeme to_target okolo axis o angle_at_parent
        let rotation = Quat::from_axis_angle(axis, angle_at_parent);
        let segment1_dir = rotation * to_target;
        let middle_pos = parent_pos + segment1_dir * a;

        // Effektor je jednoduše vzdálený segment2_length od middle směrem k target
        let effector_dir = (target_pos - middle_pos).normalize();
        let effector_pos = middle_pos + effector_dir * b;

        (middle_pos, effector_pos)
    }
}

// ---------------------------------------------------------------------------
// Detekce schodů
// ---------------------------------------------------------------------------

/// Detekuje, když se hráč ocitne na schodech (kolize s STAIRS materiálem).
/// Skutečná detekce se provádí v host_client přes Avian3d collision events.
/// Tento systém je placeholder — nezasahuje do ECS dokud není integrace hotová.
pub fn detect_stairs_on_collision(
    _players: Query<(Entity, &Transform), (Added<Transform>, Without<OnStairs>)>,
) {
}

/// Raycast pod nohama pro zjištění výšky podlahy.
/// POZNÁMKA: Implementace je v host_client/src/physics.rs
/// kde má přístup k Avian3d SpatialQuery.
pub fn raycaster_ground_height(
    mut _on_stairs: Query<(&GlobalTransform, &mut OnStairs)>,
) {
    // Implementace se provádí v host_client/src/physics.rs v systému
    // `raycast_stairs_under_player`, protože SpatialQuery je dostupná
    // pouze v physics plugin.
}

// ---------------------------------------------------------------------------
// Helpers pro vyhledávání kostí
// ---------------------------------------------------------------------------

/// Rekurzivně hledá entitu s konkrétním jménem v hierarchii.
fn find_bone_by_name(
    parent: Entity,
    target_name: &str,
    name_q: &Query<&Name>,
    children_q: &Query<&Children>,
    max_depth: usize,
) -> Option<Entity> {
    if max_depth == 0 {
        return None;
    }

    if let Ok(name) = name_q.get(parent) {
        if name.as_str() == target_name {
            return Some(parent);
        }
    }

    if let Ok(children) = children_q.get(parent) {
        for child in children.iter() {
            if let Some(found) = find_bone_by_name(child, target_name, name_q, children_q, max_depth - 1) {
                return Some(found);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Aplikace IK na transform
// ---------------------------------------------------------------------------

/// Aplikuje IK offsety na transforms kostí.
/// 
/// Tato funkce je jednoduchá verzí IK aplikace:
/// - Hledá left/right foot kosti
/// - Aplikuje Y offset na základě výšek z raycastu
/// - Blenduje s originální pozicí na základě `IkEnabled.blend_weight`
pub fn apply_ik_to_skeleton(
    entities_with_stairs: Query<(Entity, &OnStairs, &IkEnabled), Changed<OnStairs>>,
    mut transforms: Query<&mut Transform>,
    globals: Query<&GlobalTransform>,
    children: Query<&Children>,
    names: Query<&Name>,
) {
    for (entity, stairs, ik_enabled) in &entities_with_stairs {
        // Pokud nejsou žádné výšky terénů, přeskoči
        if stairs.left_foot_height < 0.01 && stairs.right_foot_height < 0.01 {
            continue;
        }

        // Hledej foot kosti - typicky DEF_foot_l a DEF_foot_r
        let left_foot = find_bone_by_name(entity, "DEF_foot_l", &names, &children, 20)
            .or_else(|| find_bone_by_name(entity, "DEF-foot_l", &names, &children, 20))
            .or_else(|| find_bone_by_name(entity, "DEF_foot.L", &names, &children, 20));

        let right_foot = find_bone_by_name(entity, "DEF_foot_r", &names, &children, 20)
            .or_else(|| find_bone_by_name(entity, "DEF-foot_r", &names, &children, 20))
            .or_else(|| find_bone_by_name(entity, "DEF_foot.R", &names, &children, 20));

        // Aplikuj offset na levou nohu
        if let Some(left_entity) = left_foot {
            if let Ok(left_global) = globals.get(left_entity) {
                if let Ok(mut left_local) = transforms.get_mut(left_entity) {
                    let target_y = stairs.left_foot_height;
                    let current_y = left_global.translation().y;
                    let delta = target_y - current_y;
                    let blended_delta = delta * ik_enabled.blend_weight;
                    left_local.translation.y += blended_delta;
                }
            }
        }

        // Aplikuj offset na pravou nohu
        if let Some(right_entity) = right_foot {
            if let Ok(right_global) = globals.get(right_entity) {
                if let Ok(mut right_local) = transforms.get_mut(right_entity) {
                    let target_y = stairs.right_foot_height;
                    let current_y = right_global.translation().y;
                    let delta = target_y - current_y;
                    let blended_delta = delta * ik_enabled.blend_weight;
                    right_local.translation.y += blended_delta;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_two_bone_ik_basic() {
        let parent = Vec3::ZERO;
        let target = Vec3::new(1.0, 0.0, 0.0);
        let seg1 = 0.5;
        let seg2 = 0.5;
        let pole = Vec3::Y;

        let (middle, effector) = TwoBoneIkSolver::solve(parent, target, seg1, seg2, pole);
        
        // Kontrolní vzdálenosti
        let d1 = parent.distance(middle);
        let d2 = middle.distance(effector);
        
        assert!((d1 - seg1).abs() < 0.01, "Segment 1 length mismatch");
        assert!((d2 - seg2).abs() < 0.01, "Segment 2 length mismatch");
    }
}
