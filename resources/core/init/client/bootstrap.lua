-- client/bootstrap.lua — běží jen na klientu.

assert(IS_CLIENT, 'expected to run on client side')
log_info(Core.greet('host_client'))

local env_state = nil

local function apply_environment(hour)
    if not env_state then
        return
    end
    local blended = Core.blend_environment(env_state, hour)
    World.ConfigureEnvironmentLight({
        enabled = blended.enabled,
        shadows = blended.shadows,
        hour_of_day = blended.hour_of_day,
        azimuth_deg = blended.azimuth_deg,
        max_elevation_deg = blended.max_elevation_deg,
        color = blended.color,
        illuminance = blended.illuminance,
        ambient_enabled = blended.ambient_enabled,
        ambient_color = blended.ambient_color,
        ambient_brightness = blended.ambient_brightness,
        fog = blended.fog,
    })
end

CreateThread(function()
    while true do
        if not env_state then
            Wait(250)
        else
            local tick_ms = math.max(50, tonumber(env_state.tick_interval_ms) or 100)
            Wait(tick_ms)
            local hours_per_second = 24.0 / math.max(tonumber(env_state.day_length_seconds) or 1200.0, 1.0)
            env_state.hour_of_day = Core.remap_hour(env_state.hour_of_day + (tick_ms / 1000.0) * hours_per_second)
            apply_environment(env_state.hour_of_day)
        end
    end
end)

-- Phase 2 demo: handshake odpověď.
RegisterEvent('pong', function(payload, _sender)
    log_info('[rpc-demo] received pong from server: ' .. tostring(payload))
end)

RegisterEvent(Core.env_sync_event, function(payload, _sender)
    if type(payload) ~= 'table' then
        return
    end
    env_state = Core.clone_default_environment()
    Core.merge_table(env_state, payload)
    env_state.hour_of_day = Core.remap_hour(env_state.hour_of_day)
    apply_environment(env_state.hour_of_day)
end)

-- Klientský sandbox vzniká až po stažení souborů, kdy je lightyear connection
-- už dlouho navázaná, takže TriggerServerEvent v load-time scriptu úspěšně
-- protlačí zprávu serveru. Phase 3 přinese explicitní `onConnect` event,
-- který bude robustnější vstupní bod pro inicializační RPC.
TriggerServerEvent('ping', 'hello server, this is client')
