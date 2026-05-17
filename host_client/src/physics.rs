use avian3d::debug_render::PhysicsDebugPlugin;
use avian3d::prelude::*;
use bevy::prelude::*;

mod attach;
mod colliders;
mod collision_toggle;
mod navmesh;
mod stairs;

use attach::{
    attach_or_update_collider_objects, attach_or_update_drawable_colliders,
    attach_or_update_dummy_colliders,
};
use collision_toggle::apply_collision_enabled;
use navmesh::{NavMeshSurfaceCache, rebuild_navmesh_surface_cache};
use stairs::raycast_stairs_under_player;
pub(crate) use stairs::DummyStairsIkSurface;

pub struct ClientPhysicsPlugin;

impl Plugin for ClientPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default());
        app.add_plugins(PhysicsDebugPlugin::default());
        app.init_resource::<NavMeshSurfaceCache>();
        // Vizualizace colliderů je při startu vypnutá — F3 ji přepíná.
        app.add_systems(Startup, disable_physics_debug_on_start);
        app.add_systems(Update, (
            attach_or_update_drawable_colliders,
            attach_or_update_dummy_colliders,
            attach_or_update_collider_objects,
            rebuild_navmesh_surface_cache,
            toggle_physics_debug,
            apply_collision_enabled,
            raycast_stairs_under_player,
        ));
    }
}

fn disable_physics_debug_on_start(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PhysicsGizmos>();
    config.enabled = false;
}

fn toggle_physics_debug(
    keys: Res<ButtonInput<KeyCode>>,
    mut store: ResMut<GizmoConfigStore>,
) {
    if keys.just_pressed(KeyCode::F3) {
        let (config, _) = store.config_mut::<PhysicsGizmos>();
        config.enabled = !config.enabled;
        if config.enabled {
            info!("[physics] Collider debug: ON (F3 pro vypnutí)");
        } else {
            info!("[physics] Collider debug: OFF");
        }
    }
}

#[derive(Component, Debug)]
pub struct StaticWorldCollider;

