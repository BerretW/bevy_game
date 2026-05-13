-- shared/api.lua — sdílené helpery, které se nahrávají na obou stranách sítě.
--
-- Volá se před side-specific skripty, takže utility tu mohou bez obav existovat
-- a server / client bootstrap je můžou používat.

log_info(string.format('[%s] shared/api.lua loaded on %s side', RESOURCE_ID, SIDE))

-- Triviální namespace pattern — rezervujeme si globál `Core` pro bootstrap helpery.
Core = Core or {}

function Core.greet(who)
    return string.format('hello, %s — from %s sandbox', who, RESOURCE_ID)
end

Core.env_sync_event = 'core:init:env_sync'
Core.env_patch_event = 'core:init:set_environment_light'
Core.env_time_event = 'core:init:set_environment_time'

Core.default_environment = {
    enabled = true,
    shadows = true,
    ambient_enabled = true,
    day_length_seconds = 1200.0,
    sync_interval_ms = 1000,
    tick_interval_ms = 100,
    azimuth_deg = -35.0,
    max_elevation_deg = 78.0,
    hour_of_day = 9.0,
    sun = {
        night = { color = { 0.18, 0.24, 0.42, 1.0 }, illuminance = 40.0 },
        dawn = { color = { 1.00, 0.54, 0.30, 1.0 }, illuminance = 3800.0 },
        day = { color = { 1.00, 0.97, 0.92, 1.0 }, illuminance = 18000.0 },
        dusk = { color = { 1.00, 0.48, 0.26, 1.0 }, illuminance = 3200.0 },
    },
    ambient = {
        night = { color = { 0.05, 0.08, 0.16, 1.0 }, brightness = 18.0 },
        dawn = { color = { 0.26, 0.20, 0.22, 1.0 }, brightness = 90.0 },
        day = { color = { 0.56, 0.60, 0.70, 1.0 }, brightness = 240.0 },
        dusk = { color = { 0.24, 0.18, 0.24, 1.0 }, brightness = 80.0 },
    },
    fog = {
        enabled = true,
        volumetric_enabled = true,
        follow_streaming_boundary = true,
        boundary_inner_distance = 180.0,
        boundary_outer_distance = 36.0,
        directional_exponent = 22.0,
        jitter = 0.02,
        step_count = 48,
        phases = {
            night = {
                color = { 0.05, 0.08, 0.14, 0.82 },
                directional_color = { 0.22, 0.28, 0.40, 0.12 },
                ambient_color = { 0.07, 0.10, 0.18, 1.0 },
                ambient_intensity = 0.02,
                start = 110.0,
                ['end'] = 210.0,
            },
            dawn = {
                color = { 0.46, 0.40, 0.34, 0.56 },
                directional_color = { 1.00, 0.62, 0.36, 0.28 },
                ambient_color = { 0.30, 0.24, 0.22, 1.0 },
                ambient_intensity = 0.05,
                start = 150.0,
                ['end'] = 270.0,
            },
            day = {
                color = { 0.72, 0.78, 0.84, 0.28 },
                directional_color = { 1.00, 0.97, 0.92, 0.18 },
                ambient_color = { 0.60, 0.66, 0.74, 1.0 },
                ambient_intensity = 0.09,
                start = 240.0,
                ['end'] = 430.0,
            },
            dusk = {
                color = { 0.40, 0.30, 0.28, 0.62 },
                directional_color = { 1.00, 0.52, 0.26, 0.30 },
                ambient_color = { 0.26, 0.20, 0.24, 1.0 },
                ambient_intensity = 0.05,
                start = 155.0,
                ['end'] = 275.0,
            },
        },
    },
}

local function deepcopy(value)
    if type(value) ~= 'table' then
        return value
    end
    local out = {}
    for key, inner in pairs(value) do
        out[key] = deepcopy(inner)
    end
    return out
end

function Core.deepcopy(value)
    return deepcopy(value)
end

function Core.merge_table(base, patch)
    if type(patch) ~= 'table' then
        return base
    end
    for key, value in pairs(patch) do
        if type(value) == 'table' and type(base[key]) == 'table' then
            Core.merge_table(base[key], value)
        else
            base[key] = value
        end
    end
    return base
end

function Core.clone_default_environment()
    return deepcopy(Core.default_environment)
end

function Core.remap_hour(hour)
    return ((tonumber(hour) or 0.0) % 24.0 + 24.0) % 24.0
end

function Core.lerp(a, b, t)
    return a + (b - a) * t
end

function Core.lerp_color(a, b, t)
    return {
        Core.lerp(a[1] or 0.0, b[1] or 0.0, t),
        Core.lerp(a[2] or 0.0, b[2] or 0.0, t),
        Core.lerp(a[3] or 0.0, b[3] or 0.0, t),
        Core.lerp(a[4] or 1.0, b[4] or 1.0, t),
    }
end

function Core.boolish(value, default)
    if value == nil then
        return default == true
    end
    if type(value) == 'boolean' then
        return value
    end
    if type(value) == 'number' then
        return value ~= 0
    end
    if type(value) == 'string' then
        local normalized = string.lower(value)
        return normalized == 'true' or normalized == '1' or normalized == 'yes' or normalized == 'on'
    end
    return default == true
end

function Core.blend_environment(config, hour)
    local env = config or Core.default_environment
    local h = Core.remap_hour(hour)

    local from_key = 'night'
    local to_key = 'night'
    local t = 0.0

    if h < 5.0 then
        from_key = 'night'
        to_key = 'night'
        t = 0.0
    elseif h < 8.0 then
        from_key = 'night'
        to_key = 'dawn'
        t = (h - 5.0) / 3.0
    elseif h < 11.0 then
        from_key = 'dawn'
        to_key = 'day'
        t = (h - 8.0) / 3.0
    elseif h < 17.0 then
        from_key = 'day'
        to_key = 'day'
        t = 0.0
    elseif h < 20.0 then
        from_key = 'day'
        to_key = 'dusk'
        t = (h - 17.0) / 3.0
    elseif h < 22.0 then
        from_key = 'dusk'
        to_key = 'night'
        t = (h - 20.0) / 2.0
    else
        from_key = 'night'
        to_key = 'night'
        t = 0.0
    end

    local sun_from = env.sun[from_key]
    local sun_to = env.sun[to_key]
    local ambient_from = env.ambient[from_key]
    local ambient_to = env.ambient[to_key]
    local fog = env.fog or {}
    local fog_phases = fog.phases or {}
    local fog_from = fog_phases[from_key] or fog_phases.night or {
        color = { 0.65, 0.70, 0.76, 0.45 },
        directional_color = { 1.0, 1.0, 1.0, 0.2 },
        ambient_color = { 0.55, 0.60, 0.70, 1.0 },
        ambient_intensity = 0.06,
        start = 180.0,
        ['end'] = 320.0,
    }
    local fog_to = fog_phases[to_key] or fog_from

    return {
        enabled = Core.boolish(env.enabled, true),
        shadows = Core.boolish(env.shadows, true),
        ambient_enabled = Core.boolish(env.ambient_enabled, true),
        azimuth_deg = env.azimuth_deg,
        max_elevation_deg = env.max_elevation_deg,
        hour_of_day = h,
        color = Core.lerp_color(sun_from.color, sun_to.color, t),
        illuminance = Core.lerp(sun_from.illuminance, sun_to.illuminance, t),
        ambient_color = Core.lerp_color(ambient_from.color, ambient_to.color, t),
        ambient_brightness = Core.lerp(ambient_from.brightness, ambient_to.brightness, t),
        fog = {
            enabled = Core.boolish(fog.enabled, true),
            color = Core.lerp_color(fog_from.color, fog_to.color, t),
            directional_color = Core.lerp_color(fog_from.directional_color, fog_to.directional_color, t),
            follow_streaming_boundary = Core.boolish(fog.follow_streaming_boundary, true),
            boundary_inner_distance = tonumber(fog.boundary_inner_distance) or 180.0,
            boundary_outer_distance = tonumber(fog.boundary_outer_distance) or 36.0,
            directional_exponent = tonumber(fog.directional_exponent) or 22.0,
            start = Core.lerp(tonumber(fog_from.start) or 180.0, tonumber(fog_to.start) or 180.0, t),
            ['end'] = Core.lerp(tonumber(fog_from['end']) or 320.0, tonumber(fog_to['end']) or 320.0, t),
            volumetric_enabled = Core.boolish(fog.volumetric_enabled, true),
            ambient_color = Core.lerp_color(fog_from.ambient_color, fog_to.ambient_color, t),
            ambient_intensity = Core.lerp(tonumber(fog_from.ambient_intensity) or 0.06, tonumber(fog_to.ambient_intensity) or 0.06, t),
            jitter = tonumber(fog.jitter) or 0.02,
            step_count = tonumber(fog.step_count) or 48,
        },
    }
end

function Core.patch_environment(patch)
    if not IS_SERVER then
        log_warn('[core/init] Core.patch_environment should be called on server for synchronized env updates')
        return false
    end
    TriggerEvent(Core.env_patch_event, patch)
    return true
end

function Core.set_environment_time(hour)
    if not IS_SERVER then
        log_warn('[core/init] Core.set_environment_time should be called on server for synchronized env updates')
        return false
    end
    TriggerEvent(Core.env_time_event, { hour_of_day = hour })
    return true
end
