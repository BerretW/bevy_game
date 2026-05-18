mod animation;
mod attach;
mod network;
mod ownership;
mod physics;

use bevy::prelude::*;

#[derive(Component)]
pub(super) struct NpcCapsuleAttached;

#[derive(Component)]
pub(super) struct NpcVisualAttached;

#[derive(Component, Default)]
pub(super) struct NpcMotionTracker {
    pub(super) prev_pos: Vec3,
    pub(super) current_state: u8,
}

pub(super) use animation::drive_npc_animations;
pub(super) use attach::{attach_capsule_to_new_npcs, attach_model_to_new_npcs};
pub(super) use network::{send_owned_npc_transforms, sync_npc_net_transform};
pub(super) use ownership::{bootstrap_owned_npc_agents, cleanup_unowned_npc_agents};
pub(super) use physics::{terrain_snap_owned_npcs, update_owned_npc_avoidance};
