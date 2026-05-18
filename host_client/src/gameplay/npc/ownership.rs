use bevy::prelude::*;
use core_resources::{
    EntityHandle, NpcAgent, NpcBrainRegistry, NpcBrainState, NpcOwner, NpcPedMarker,
    NpcScenarioRegistry, ReplicatedNpcBrain, ReplicatedNpcSteering, apply_replicated_npc_brain,
    apply_replicated_npc_steering,
};

use crate::gameplay::LocalClientId;

pub(crate) fn bootstrap_owned_npc_agents(
    local_client_id: Option<Res<LocalClientId>>,
    brain_registry: Res<NpcBrainRegistry>,
    scenario_registry: Res<NpcScenarioRegistry>,
    mut commands: Commands,
    npcs: Query<
        (
            Entity,
            &EntityHandle,
            &Transform,
            &NpcOwner,
            &ReplicatedNpcBrain,
            &ReplicatedNpcSteering,
        ),
        (With<NpcPedMarker>, Without<NpcAgent>),
    >,
) {
    let Some(local_id) = local_client_id.map(|value| value.0) else {
        return;
    };

    for (entity, handle, transform, owner, brain, steering) in &npcs {
        if owner.0 != Some(local_id) {
            continue;
        }
        let mut agent = NpcAgent::new(handle.0, transform.translation);
        let mut local_state = NpcBrainState::new(brain.brain_id.clone());
        apply_replicated_npc_brain(
            &brain_registry,
            &scenario_registry,
            brain,
            &mut local_state,
            &mut agent,
        );
        apply_replicated_npc_steering(&mut agent, steering);
        commands.entity(entity).insert((agent, local_state));
    }
}

pub(crate) fn cleanup_unowned_npc_agents(
    local_client_id: Option<Res<LocalClientId>>,
    mut commands: Commands,
    npcs: Query<(Entity, &NpcOwner), (With<NpcPedMarker>, With<NpcAgent>)>,
) {
    let Some(local_id) = local_client_id.map(|value| value.0) else {
        return;
    };

    for (entity, owner) in &npcs {
        if owner.0 == Some(local_id) {
            continue;
        }
        commands.entity(entity).remove::<NpcAgent>();
    }
}
