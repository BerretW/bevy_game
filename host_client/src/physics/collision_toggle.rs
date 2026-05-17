use avian3d::prelude::*;
use bevy::prelude::*;
use core_resources::CollisionEnabled;

pub(super) fn apply_collision_enabled(
    changed: Query<(Entity, &CollisionEnabled), Changed<CollisionEnabled>>,
    children_q: Query<&Children>,
    has_collider: Query<Has<Collider>>,
    mut commands: Commands,
) {
    for (root, collision_enabled) in &changed {
        toggle_colliders_recursive(root, collision_enabled.0, &children_q, &has_collider, &mut commands);
    }
}

fn toggle_colliders_recursive(
    entity: Entity,
    enabled: bool,
    children_q: &Query<&Children>,
    has_collider: &Query<Has<Collider>>,
    commands: &mut Commands,
) {
    if has_collider.get(entity).unwrap_or(false) {
        let mut entity_commands = commands.entity(entity);
        if enabled {
            entity_commands.remove::<ColliderDisabled>();
        } else {
            entity_commands.insert(ColliderDisabled);
        }
    }

    if let Ok(children) = children_q.get(entity) {
        for &child in children {
            toggle_colliders_recursive(child, enabled, children_q, has_collider, commands);
        }
    }
}
