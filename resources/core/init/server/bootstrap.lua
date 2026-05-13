-- server/bootstrap.lua — běží jen na serveru (jak vyžaduje manifest).

assert(IS_SERVER, 'expected to run on server side')
log_info(Core.greet('host_server'))

local env_state = Core.clone_default_environment()

local function broadcast_environment(target)
    TriggerClientEvent(Core.env_sync_event, target, {
        hour_of_day = env_state.hour_of_day,
        day_length_seconds = env_state.day_length_seconds,
        enabled = env_state.enabled,
        shadows = env_state.shadows,
        ambient_enabled = env_state.ambient_enabled,
        azimuth_deg = env_state.azimuth_deg,
        max_elevation_deg = env_state.max_elevation_deg,
        sun = Core.deepcopy(env_state.sun),
        ambient = Core.deepcopy(env_state.ambient),
        fog = Core.deepcopy(env_state.fog),
        sync_interval_ms = env_state.sync_interval_ms,
        tick_interval_ms = env_state.tick_interval_ms,
    })
end

local function advance_environment(delta_seconds)
    local hours_per_second = 24.0 / math.max(env_state.day_length_seconds or 1200.0, 1.0)
    env_state.hour_of_day = Core.remap_hour(env_state.hour_of_day + delta_seconds * hours_per_second)
end

CreateThread(function()
    local sync_accum_ms = 0

    broadcast_environment(nil)
    while true do
        local tick_ms = math.max(50, tonumber(env_state.tick_interval_ms) or 100)
        local sync_every_ms = math.max(tick_ms, tonumber(env_state.sync_interval_ms) or 1000)
        Wait(tick_ms)
        advance_environment(tick_ms / 1000.0)
        sync_accum_ms = sync_accum_ms + tick_ms
        if sync_accum_ms >= sync_every_ms then
            sync_accum_ms = 0
            broadcast_environment(nil)
        end
    end
end)

RegisterEvent('onPlayerJoin', function(player_id)
    log_info(string.format('player %s joined (server-side handler)', tostring(player_id)))
    broadcast_environment(player_id)
end)

-- Phase 2 demo: server odpoví na klientský `ping` broadcastem `pong`.
-- TriggerClientEvent(name, target, payload) — `target = nil` ⇒ broadcast
-- všem připojeným klientům (per-target unicast je Phase 3).
RegisterEvent('ping', function(payload, _sender)
    log_info('[rpc-demo] received ping: ' .. tostring(payload))
    TriggerClientEvent('pong', nil, 'pong from server')
end)

RegisterEvent(Core.env_patch_event, function(payload, _sender)
    if type(payload) ~= 'table' then
        return
    end
    Core.merge_table(env_state, payload)
    env_state.hour_of_day = Core.remap_hour(env_state.hour_of_day)
    env_state.day_length_seconds = math.max(1.0, tonumber(env_state.day_length_seconds) or 1200.0)
    env_state.sync_interval_ms = math.max(100, tonumber(env_state.sync_interval_ms) or 1000)
    env_state.tick_interval_ms = math.max(50, tonumber(env_state.tick_interval_ms) or 100)
    broadcast_environment(nil)
end)

RegisterEvent(Core.env_time_event, function(payload, _sender)
    local hour = payload
    if type(payload) == 'table' then
        hour = payload.hour_of_day or payload.hour
    end
    env_state.hour_of_day = Core.remap_hour(hour)
    broadcast_environment(nil)
end)
