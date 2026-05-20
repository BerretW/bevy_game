use bevy::prelude::*;
use core_resources::{
    ActiveWeaponSlot, AmmoReserve, ArmorComponent, FireState, Inventory, PlayerEntityMap,
    PlayerStatsCache, ReloadState, Stats, StatsSnapshot, WeaponSlots, WeaponSwapState,
};
use core_shared::{Health, PlayerMarker};
use lightyear::prelude::*;

use crate::net_plugin::StatsChannel;
use crate::protocol::PlayerStatsUpdate;

pub fn broadcast_player_stats(
    players: Query<(
        &PlayerMarker,
        &Health,
        Option<&ArmorComponent>,
        Option<&WeaponSlots>,
        Option<&AmmoReserve>,
        Option<&ActiveWeaponSlot>,
        Option<&FireState>,
        Option<&ReloadState>,
        Option<&WeaponSwapState>,
    )>,
    mut senders: Query<(&mut MessageSender<PlayerStatsUpdate>, &RemoteId)>,
    mut tick: Local<u32>,
) {
    *tick = tick.wrapping_add(1);
    if *tick % 6 != 0 {
        return;
    }

    for (mut sender, remote_id) in senders.iter_mut() {
        let client_id = match remote_id.0 {
            PeerId::Netcode(id) => id,
            _ => continue,
        };

        if let Some((_, health, armor, weapon_slots, ammo_reserve, active_slot, fire_state, reload_state, weapon_swap)) =
            players
                .iter()
                .find(|(m, _, _, _, _, _, _, _, _)| m.client_id == client_id)
        {
            let msg = PlayerStatsUpdate {
                hp: health.current,
                max_hp: health.max,
                armor: armor.cloned().unwrap_or_default(),
                weapon_slots: weapon_slots
                    .map(|value| value.0.iter().cloned().collect())
                    .unwrap_or_else(|| vec![None, None, None, None]),
                ammo_reserve: ammo_reserve.map(|value| value.0.clone()).unwrap_or_default(),
                active_weapon_slot: active_slot.map(|value| value.0).unwrap_or(0),
                fire_cooldown_remaining: fire_state
                    .map(|value| value.cooldown_remaining)
                    .unwrap_or(0.0),
                fire_trigger_held: fire_state.map(|value| value.trigger_held).unwrap_or(false),
                reload_remaining: reload_state.map(|value| value.remaining).unwrap_or(0.0),
                reload_duration: reload_state.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_remaining: weapon_swap.map(|value| value.remaining).unwrap_or(0.0),
                weapon_swap_duration: weapon_swap.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_target_slot: weapon_swap
                    .map(|value| {
                        if value.remaining > 0.0 {
                            Some(value.target_slot)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(None),
            };
            sender.send::<StatsChannel>(msg);
        }
    }
}

pub fn sync_player_state_cache(
    players: Query<
        (
            Entity,
            &PlayerMarker,
            &Health,
            Option<&Stats>,
            Option<&Inventory>,
            Option<&ArmorComponent>,
            Option<&WeaponSlots>,
            Option<&AmmoReserve>,
            Option<&ActiveWeaponSlot>,
            Option<&FireState>,
            Option<&ReloadState>,
            Option<&WeaponSwapState>,
        ),
    >,
    mut player_map: ResMut<PlayerEntityMap>,
    stats_cache: Res<PlayerStatsCache>,
) {
    let mut seen = std::collections::HashSet::new();

    for (
        entity,
        marker,
        health,
        stats,
        inventory,
        armor,
        weapon_slots,
        ammo_reserve,
        active_slot,
        fire_state,
        reload_state,
        weapon_swap,
    ) in &players
    {
        seen.insert(marker.client_id);
        player_map.map.insert(marker.client_id, entity);
        stats_cache.update(
            marker.client_id,
            StatsSnapshot {
                stats: stats.map(|value| value.0.clone()).unwrap_or_default(),
                inventory: inventory.map(|value| value.0.clone()).unwrap_or_default(),
                health: health.current,
                max_health: health.max,
                armor: armor.cloned().unwrap_or_default(),
                weapon_slots: weapon_slots
                    .map(|value| value.0.iter().cloned().collect())
                    .unwrap_or_else(|| vec![None, None, None, None]),
                ammo_reserve: ammo_reserve.map(|value| value.0.clone()).unwrap_or_default(),
                active_weapon_slot: active_slot.map(|value| value.0).unwrap_or(0),
                fire_cooldown_remaining: fire_state
                    .map(|value| value.cooldown_remaining)
                    .unwrap_or(0.0),
                fire_trigger_held: fire_state.map(|value| value.trigger_held).unwrap_or(false),
                reload_remaining: reload_state.map(|value| value.remaining).unwrap_or(0.0),
                reload_duration: reload_state.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_remaining: weapon_swap.map(|value| value.remaining).unwrap_or(0.0),
                weapon_swap_duration: weapon_swap.map(|value| value.duration).unwrap_or(0.0),
                weapon_swap_target_slot: weapon_swap
                    .map(|value| {
                        if value.remaining > 0.0 {
                            Some(value.target_slot)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(None),
            },
        );
    }

    player_map.map.retain(|client_id, _| seen.contains(client_id));
    stats_cache.retain_ids(&seen);
}
