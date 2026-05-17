use bevy::prelude::*;
use core_resources::{
    ActiveWeaponSlot, AmmoReserve, FireState, ReloadState, WeaponRegistry, WeaponSlots,
    WeaponSwapState,
};

use crate::protocol::player_action;

use super::{DEFAULT_WEAPON_SWAP_SECS, MIN_WEAPON_ACTION_SECS};

pub(super) fn tick_fire_states(mut q: Query<&mut FireState>, time: Res<Time<Fixed>>) {
    let dt = time.delta_secs();
    for mut fire_state in q.iter_mut() {
        fire_state.cooldown_remaining = (fire_state.cooldown_remaining - dt).max(0.0);
    }
}

pub(super) fn tick_weapon_swap_states(
    mut players: Query<(&mut ActiveWeaponSlot, &mut WeaponSwapState)>,
    time: Res<Time<Fixed>>,
) {
    let dt = time.delta_secs();
    for (mut active_slot, mut swap_state) in players.iter_mut() {
        if swap_state.remaining <= 0.0 {
            continue;
        }

        swap_state.remaining = (swap_state.remaining - dt).max(0.0);
        if swap_state.remaining > 0.0 {
            continue;
        }

        if swap_state.target_slot < 4 {
            active_slot.0 = swap_state.target_slot;
        }
        *swap_state = WeaponSwapState::default();
    }
}

pub(super) fn tick_reload_states(
    weapon_registry: Res<WeaponRegistry>,
    mut players: Query<(&mut WeaponSlots, &mut AmmoReserve, &mut ReloadState)>,
    time: Res<Time<Fixed>>,
) {
    let dt = time.delta_secs();
    for (mut slots, mut reserve, mut reload_state) in players.iter_mut() {
        if reload_state.remaining <= 0.0 {
            continue;
        }

        reload_state.remaining = (reload_state.remaining - dt).max(0.0);
        if reload_state.remaining > 0.0 {
            continue;
        }

        reload_active_weapon(&weapon_registry, &mut slots, &mut reserve, reload_state.slot);
        *reload_state = ReloadState::default();
    }
}

pub(super) fn reload_active_weapon(
    weapon_registry: &WeaponRegistry,
    slots: &mut WeaponSlots,
    reserve: &mut AmmoReserve,
    active_slot: u8,
) -> u32 {
    let Some(equipped) = slots.get_mut(active_slot) else {
        return 0;
    };
    let Some(weapon_def) = weapon_registry.get(&equipped.weapon_id) else {
        return 0;
    };
    let mag_capacity = weapon_def.mag_capacity;
    if mag_capacity == 0 || equipped.ammo_in_mag >= mag_capacity {
        return 0;
    }

    let ammo_id = if equipped.ammo_type_id.trim().is_empty() {
        weapon_def.default_ammo.clone()
    } else {
        equipped.ammo_type_id.clone()
    };
    if ammo_id.trim().is_empty() {
        return 0;
    }

    let reserve_entry = reserve.0.entry(ammo_id.clone()).or_insert(0);
    let needed = mag_capacity.saturating_sub(equipped.ammo_in_mag);
    let loaded = (*reserve_entry).min(needed);
    *reserve_entry = reserve_entry.saturating_sub(loaded);
    equipped.ammo_in_mag = equipped.ammo_in_mag.saturating_add(loaded);
    if equipped.ammo_type_id.trim().is_empty() {
        equipped.ammo_type_id = ammo_id;
    }
    loaded
}

pub(super) fn requested_weapon_slot(actions: u32) -> Option<u8> {
    if actions & player_action::WEAPON_SLOT_1 != 0 {
        Some(0)
    } else if actions & player_action::WEAPON_SLOT_2 != 0 {
        Some(1)
    } else if actions & player_action::WEAPON_SLOT_3 != 0 {
        Some(2)
    } else if actions & player_action::WEAPON_SLOT_4 != 0 {
        Some(3)
    } else {
        None
    }
}

pub(super) fn weapon_swap_duration(
    weapon_registry: &WeaponRegistry,
    slots: &WeaponSlots,
    active_slot: u8,
    target_slot: u8,
) -> f32 {
    let duration = slots
        .get(target_slot)
        .and_then(|weapon| weapon_registry.get(&weapon.weapon_id))
        .map(|weapon| weapon.ads_time_sec)
        .filter(|duration| *duration > 0.0)
        .or_else(|| {
            slots.get(active_slot)
                .and_then(|weapon| weapon_registry.get(&weapon.weapon_id))
                .map(|weapon| weapon.ads_time_sec)
                .filter(|duration| *duration > 0.0)
        })
        .unwrap_or(DEFAULT_WEAPON_SWAP_SECS);
    duration.max(MIN_WEAPON_ACTION_SECS)
}

pub(super) fn reload_duration_for_slot(
    weapon_registry: &WeaponRegistry,
    slots: &WeaponSlots,
    reserve: &AmmoReserve,
    active_slot: u8,
) -> Option<f32> {
    let equipped = slots.get(active_slot)?;
    let weapon_def = weapon_registry.get(&equipped.weapon_id)?;
    let mag_capacity = weapon_def.mag_capacity;
    if mag_capacity == 0 || equipped.ammo_in_mag >= mag_capacity {
        return None;
    }

    let ammo_id = if equipped.ammo_type_id.trim().is_empty() {
        weapon_def.default_ammo.clone()
    } else {
        equipped.ammo_type_id.clone()
    };
    if ammo_id.trim().is_empty() {
        return None;
    }

    if reserve.0.get(&ammo_id).copied().unwrap_or(0) == 0 {
        return None;
    }

    let duration = if equipped.ammo_in_mag == 0 {
        weapon_def.reload_empty_sec
    } else {
        weapon_def.reload_tactical_sec
    };

    Some(duration.max(MIN_WEAPON_ACTION_SECS))
}

pub(super) fn is_reload_active(reload_state: &ReloadState) -> bool {
    reload_state.remaining > 0.0
}

pub(super) fn is_weapon_swap_active(weapon_swap: &WeaponSwapState) -> bool {
    weapon_swap.remaining > 0.0
}
