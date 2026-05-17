use bevy::prelude::*;
use core_resources::{
    ActiveWeaponSlot, AmmoReserve, ArmorComponent, FireState, GameBridges, Inventory,
    LocalEventBus, PlayerHitbox, ReloadState, Stats, WeaponSlots, WeaponSwapState,
};
use core_shared::{Health, NetTransform, NetVelocity, PlayerMarker};
use lightyear::prelude::*;
use lightyear::prelude::server::LinkOf;

use super::PositionHistory;

pub(super) fn attach_replication_sender(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .insert(ReplicationSender::default());
    trace!(
        "[sim/server] ReplicationSender attached to link {:?}",
        trigger.entity
    );
}

pub(super) fn spawn_player_on_connect(
    trigger: On<Add, Connected>,
    remote_ids: Query<&RemoteId>,
    mut commands: Commands,
    local_bus: Res<LocalEventBus>,
) {
    let entity = trigger.entity;
    let Ok(remote_id) = remote_ids.get(entity) else {
        warn!(
            "[sim/server] connected entity {:?} has no RemoteId — skipping player spawn",
            entity
        );
        return;
    };

    let client_id = match remote_id.0 {
        PeerId::Netcode(id) => id,
        _ => 0,
    };

    let player = commands
        .spawn((
            NetTransform::default(),
            NetVelocity::default(),
            PlayerMarker { client_id },
            Health::default(),
            Stats::default(),
            Inventory::default(),
            ArmorComponent::default(),
            WeaponSlots::default(),
            AmmoReserve::default(),
            ActiveWeaponSlot::default(),
            PlayerHitbox::default(),
            PositionHistory::default(),
            FireState::default(),
            ReloadState::default(),
            WeaponSwapState::default(),
        ))
        .id();
    commands.entity(player).insert((
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
    ));

    info!(
        "[sim/server] spawned player {:?} for client_id={}",
        player, client_id
    );

    let payload = serde_json::to_vec(&serde_json::json!({
        "id": client_id.to_string(),
        "entity": format!("{:?}", player)
    }))
    .unwrap_or_default();
    local_bus.push("playerConnecting".to_string(), payload.clone());
    local_bus.push("onPlayerJoin".to_string(), payload);
}

pub(super) fn emit_player_disconnect(
    trigger: On<Remove, Connected>,
    remote_ids: Query<&RemoteId>,
    bridges: Res<GameBridges>,
    local_bus: Res<LocalEventBus>,
) {
    let entity = trigger.entity;
    let client_id = remote_ids
        .get(entity)
        .ok()
        .and_then(|remote_id| match remote_id.0 {
            PeerId::Netcode(id) => Some(id),
            _ => None,
        })
        .unwrap_or(0);

    info!("[sim/server] client {} disconnected", client_id);

    bridges.ace.remove_player(client_id);

    let payload = serde_json::to_vec(&serde_json::json!({
        "id": client_id.to_string(),
        "reason": "disconnect"
    }))
    .unwrap_or_default();
    local_bus.push("playerDropped".to_string(), payload);
}

pub(super) fn attach_replication_to_networked_object(
    trigger: On<Add, core_resources::NetworkedObjectMarker>,
    mut commands: Commands,
) {
    commands
        .entity(trigger.entity)
        .insert(Replicate::to_clients(NetworkTarget::All));
    debug!(
        "[sim/server] Replicate attached to networked object {:?}",
        trigger.entity
    );
}
