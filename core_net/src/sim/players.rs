use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use core_resources::LocalEventBus;
use core_shared::{NetTransform, PlayerMarker};
use lightyear::prelude::*;

use crate::protocol::PlayerInput;

const POSITION_HISTORY_MAX_SAMPLES: usize = 48;

#[derive(Debug, Clone, Copy)]
struct PositionSample {
    tick: u32,
    translation: Vec3,
}

#[derive(Component, Debug, Default, Clone)]
pub struct PositionHistory {
    samples: VecDeque<PositionSample>,
}

impl PositionHistory {
    pub(super) fn push(&mut self, tick: u32, translation: Vec3) {
        if self
            .samples
            .back()
            .map(|sample| sample.tick == tick)
            .unwrap_or(false)
        {
            if let Some(last) = self.samples.back_mut() {
                last.translation = translation;
            }
            return;
        }

        self.samples.push_back(PositionSample { tick, translation });
        while self.samples.len() > POSITION_HISTORY_MAX_SAMPLES {
            self.samples.pop_front();
        }
    }

    pub(super) fn position_at_or_before(&self, tick: u32) -> Option<Vec3> {
        self.samples
            .iter()
            .rev()
            .find(|sample| sample.tick <= tick)
            .map(|sample| sample.translation)
            .or_else(|| self.samples.front().map(|sample| sample.translation))
    }
}

#[derive(Resource, Debug, Default, Clone, Copy)]
pub(super) struct ServerSimulationTick(pub u32);

#[derive(Resource, Default)]
pub struct LastPlayerInputs(HashMap<u64, PlayerInput>);

impl LastPlayerInputs {
    pub fn update(&mut self, client_id: u64, input: PlayerInput) {
        self.0.insert(client_id, input);
    }

    pub fn get(&self, client_id: u64) -> Option<&PlayerInput> {
        self.0.get(&client_id)
    }

    pub fn remove(&mut self, client_id: u64) {
        self.0.remove(&client_id);
    }
}

pub fn collect_last_inputs(
    mut receivers: Query<(&mut MessageReceiver<PlayerInput>, &RemoteId)>,
    mut last: ResMut<LastPlayerInputs>,
) {
    for (mut receiver, remote_id) in receivers.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => continue,
        };
        for input in receiver.receive() {
            last.update(client_id, input);
        }
    }
}

pub(super) fn trust_client_position(
    last_inputs: Res<LastPlayerInputs>,
    mut players: Query<(&PlayerMarker, &mut NetTransform)>,
) {
    for (marker, mut transform) in players.iter_mut() {
        let Some(input) = last_inputs.get(marker.client_id) else {
            continue;
        };

        let [px, py, pz] = input.position;
        if px.is_finite() && py.is_finite() && pz.is_finite() {
            transform.translation = Vec3::new(px, py, pz);
        }

        let yaw_rad = input.look[0].to_radians();
        if yaw_rad.is_finite() {
            transform.rotation = Quat::from_rotation_y(yaw_rad);
        }
    }
}

pub(super) fn increment_server_simulation_tick(mut tick: ResMut<ServerSimulationTick>) {
    tick.0 = tick.0.wrapping_add(1);
}

pub(super) fn record_position_history(
    tick: Res<ServerSimulationTick>,
    mut players: Query<(&NetTransform, &mut PositionHistory), With<PlayerMarker>>,
) {
    for (transform, mut history) in players.iter_mut() {
        history.push(tick.0, transform.translation);
    }
}

pub(super) fn emit_player_positions(
    players: Query<(&NetTransform, &PlayerMarker)>,
    local_bus: Res<LocalEventBus>,
) {
    let list: Vec<serde_json::Value> = players
        .iter()
        .map(|(transform, marker)| {
            serde_json::json!({
                "id": marker.client_id.to_string(),
                "x": transform.translation.x,
                "z": transform.translation.z,
            })
        })
        .collect();

    if list.is_empty() {
        return;
    }

    let payload = serde_json::to_vec(&serde_json::json!({ "players": list }))
        .unwrap_or_default();
    local_bus.push("onPlayerPosition".to_string(), payload);
}
