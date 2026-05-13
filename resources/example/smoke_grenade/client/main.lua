assert(IS_CLIENT, 'expected to run on client side')

local SMOKE_DURATION_MS = 16000
local SMOKE_GROW_MS = 1800
local SMOKE_MAX_RADIUS = 8.5
local SMOKE_HEIGHT = 4.2
local SMOKE_DRIFT_Y = 0.85
local DEMO_RESPAWN_MS = 2500

local SMOKE_LOBES = {
    { offset = { x = 1.6, y = 0.1, z = 0.2 }, scale_x = 0.86, scale_y = 0.82, scale_z = 0.72, density = 0.70, growth = 0.86, rot = { x = 8.0, y = 24.0, z = -6.0 } },
    { offset = { x = -1.5, y = 0.2, z = -0.8 }, scale_x = 0.80, scale_y = 0.76, scale_z = 0.66, density = 0.66, growth = 0.78, rot = { x = -10.0, y = -32.0, z = 7.0 } },
    { offset = { x = 0.5, y = 0.55, z = 1.8 }, scale_x = 0.72, scale_y = 0.94, scale_z = 0.64, density = 0.60, growth = 0.74, rot = { x = 14.0, y = 58.0, z = -11.0 } },
    { offset = { x = -0.8, y = 0.7, z = -1.9 }, scale_x = 0.70, scale_y = 0.88, scale_z = 0.68, density = 0.58, growth = 0.70, rot = { x = -16.0, y = -47.0, z = 13.0 } },
    { offset = { x = 0.1, y = 1.05, z = -0.2 }, scale_x = 0.60, scale_y = 1.00, scale_z = 0.56, density = 0.52, growth = 0.62, rot = { x = 19.0, y = 12.0, z = -15.0 } },
    { offset = { x = 2.3, y = 0.45, z = -1.2 }, scale_x = 0.62, scale_y = 0.70, scale_z = 0.52, density = 0.44, growth = 0.58, rot = { x = 7.0, y = 83.0, z = 21.0 } },
    { offset = { x = -2.2, y = 0.35, z = 1.3 }, scale_x = 0.58, scale_y = 0.68, scale_z = 0.54, density = 0.42, growth = 0.56, rot = { x = -9.0, y = -75.0, z = -20.0 } },
    { offset = { x = 0.9, y = 1.4, z = 0.9 }, scale_x = 0.48, scale_y = 0.72, scale_z = 0.46, density = 0.34, growth = 0.48, rot = { x = 22.0, y = 39.0, z = 17.0 } },
    { offset = { x = -1.0, y = 1.55, z = -0.7 }, scale_x = 0.46, scale_y = 0.68, scale_z = 0.44, density = 0.32, growth = 0.46, rot = { x = -24.0, y = -21.0, z = -18.0 } },
}

local smokes = {}
local input_state = {
    interact = false,
}
local startup_spawn_done = false
local last_demo_spawn_ms = -DEMO_RESPAWN_MS
local clock_ms = 0
local fog_debug = {
    camera_has_volumetric_fog = false,
    environment_light_count = 0,
    environment_light_with_volumetric = 0,
    fog_volume_count = 0,
    config_volumetric_enabled = false,
    config_shadows_enabled = false,
    config_hour = 0.0,
    config_ambient_intensity = 0.0,
    config_step_count = 0,
}

local function clamp01(value)
    if value < 0.0 then
        return 0.0
    end
    if value > 1.0 then
        return 1.0
    end
    return value
end

local function make_smoke_params(lobe)
    local density = (lobe and lobe.density or 1.0)
    return {
        size = { x = 1.0, y = 1.0, z = 1.0 },
        fog_volume = {
            color = { 0.80, 0.82, 0.84, 0.28 },
            density = 0.12 * density,
            absorption = 0.018,
            scattering = 0.58,
            scattering_asymmetry = 0.18,
            light_tint = { 0.94, 0.95, 0.97, 1.0 },
            light_intensity = 0.42,
        },
        collider = {
            enabled = false,
        },
    }
end

local function any_volume_valid(volumes)
    for i = 1, #volumes do
        if World.IsValid(volumes[i].handle) then
            return true
        end
    end
    return false
end

local function delete_smoke_volumes(volumes)
    for i = 1, #volumes do
        if World.IsValid(volumes[i].handle) then
            World.DeleteObject(volumes[i].handle)
        end
    end
end

local function normalize_hit(hit)
    if type(hit) ~= 'table' then
        return nil
    end
    return {
        x = tonumber(hit.x) or 0.0,
        y = tonumber(hit.y) or 0.0,
        z = tonumber(hit.z) or 0.0,
    }
end

local function resolve_spawn_hit()
    local hit = normalize_hit(Raycast.GetGroundPosition())
    if hit then
        return hit
    end
    return { x = 0.0, y = 0.0, z = 0.0 }
end

local function spawn_smoke_grenade(hit)
    local base_y = hit.y + 0.55
    local volumes = {}
    for i = 1, #SMOKE_LOBES do
        local lobe = SMOKE_LOBES[i]
        local handle = World.SpawnLocalDummy(
            'fog_volume',
            make_smoke_params(lobe),
            {
                x = hit.x + lobe.offset.x,
                y = base_y + lobe.offset.y,
                z = hit.z + lobe.offset.z,
            },
            { x = 0.0, y = 0.0, z = 0.0 }
        )

        World.SetScale(handle, {
            x = 0.78 * lobe.scale_x,
            y = 1.00 * lobe.scale_y,
            z = 0.78 * lobe.scale_z,
        })
        World.SetRotation(handle, lobe.rot)

        volumes[#volumes + 1] = {
            handle = handle,
            offset = lobe.offset,
            scale_x = lobe.scale_x,
            scale_y = lobe.scale_y,
            scale_z = lobe.scale_z,
            growth = lobe.growth,
            rot = lobe.rot,
        }
    end

    local marker_handle = World.SpawnLocalDummy(
        'sphere',
        {
            radius = 0.18,
            r = 1.0,
            g = 0.45,
            b = 0.08,
            a = 1.0,
            collider = {
                enabled = false,
            },
        },
        { x = hit.x, y = hit.y + 0.18, z = hit.z },
        { x = 0.0, y = 0.0, z = 0.0 }
    )

    smokes[#smokes + 1] = {
        handle = volumes[1].handle,
        volumes = volumes,
        marker_handle = marker_handle,
        x = hit.x,
        y = base_y,
        z = hit.z,
        age_ms = 0,
    }

    log_info(string.format('[smoke_grenade] spawned local smoke at %.2f %.2f %.2f', hit.x, hit.y, hit.z))
end

local function spawn_demo_smoke(reason)
    local hit = resolve_spawn_hit()
    startup_spawn_done = true
    last_demo_spawn_ms = clock_ms
    spawn_smoke_grenade(hit)
    log_info(string.format('[smoke_grenade] demo smoke spawn (%s)', tostring(reason)))
end

CreateThread(function()
    while not startup_spawn_done do
        Wait(350)
        spawn_demo_smoke('startup')
    end
end)

RegisterEvent('input:state', function(payload)
    if type(payload) ~= 'table' or type(payload.keys) ~= 'table' then
        return
    end

    local pressed = payload.keys.interact == true
    if pressed and not input_state.interact then
        spawn_smoke_grenade(resolve_spawn_hit())
        last_demo_spawn_ms = clock_ms
    end
    input_state.interact = pressed
end)

RegisterEvent('debug:volumetric_fog_state', function(payload)
    if type(payload) ~= 'table' then
        return
    end

    fog_debug.camera_has_volumetric_fog = payload.camera_has_volumetric_fog == true
    fog_debug.environment_light_count = tonumber(payload.environment_light_count) or 0
    fog_debug.environment_light_with_volumetric = tonumber(payload.environment_light_with_volumetric) or 0
    fog_debug.fog_volume_count = tonumber(payload.fog_volume_count) or 0
    fog_debug.config_volumetric_enabled = payload.config_volumetric_enabled == true
    fog_debug.config_shadows_enabled = payload.config_shadows_enabled == true
    fog_debug.config_hour = tonumber(payload.config_hour) or 0.0
    fog_debug.config_ambient_intensity = tonumber(payload.config_ambient_intensity) or 0.0
    fog_debug.config_step_count = tonumber(payload.config_step_count) or 0
end)

CreateThread(function()
    while true do
        Wait(100)
        clock_ms = clock_ms + 100

        local next_smokes = {}
        for i = 1, #smokes do
            local smoke = smokes[i]
            smoke.age_ms = smoke.age_ms + 100

            if not any_volume_valid(smoke.volumes) or smoke.age_ms >= SMOKE_DURATION_MS then
                delete_smoke_volumes(smoke.volumes)
                if smoke.marker_handle and World.IsValid(smoke.marker_handle) then
                    World.DeleteObject(smoke.marker_handle)
                end
            else
                local grow_t = clamp01(smoke.age_ms / SMOKE_GROW_MS)
                local life_t = clamp01(smoke.age_ms / SMOKE_DURATION_MS)
                local base_radius = 0.55 + (SMOKE_MAX_RADIUS - 0.55) * grow_t
                local base_y = smoke.y + SMOKE_DRIFT_Y * life_t

                for volume_index = 1, #smoke.volumes do
                    local volume = smoke.volumes[volume_index]
                    local wobble = math.sin((life_t * 4.0) + volume_index * 0.85) * 0.35
                    local spread = 0.45 + grow_t * volume.growth
                    local drift_x = volume.offset.x * spread + math.sin(life_t * 3.0 + volume_index * 1.7) * 0.22
                    local drift_z = volume.offset.z * spread + math.cos(life_t * 2.5 + volume_index * 1.3) * 0.18
                    if World.IsValid(volume.handle) then
                        World.SetPosition(volume.handle, {
                            x = smoke.x + drift_x,
                            y = base_y + volume.offset.y + wobble,
                            z = smoke.z + drift_z,
                        })
                        World.SetScale(volume.handle, {
                            x = base_radius * volume.scale_x * volume.growth,
                            y = SMOKE_HEIGHT * volume.scale_y,
                            z = base_radius * volume.scale_z * volume.growth,
                        })
                        World.SetRotation(volume.handle, {
                            x = volume.rot.x + math.sin(life_t * 14.0 + volume_index) * 4.0,
                            y = volume.rot.y + life_t * (10.0 + volume_index * 3.0),
                            z = volume.rot.z + math.cos(life_t * 11.0 + volume_index * 0.6) * 4.0,
                        })
                    end
                end

                if smoke.marker_handle and World.IsValid(smoke.marker_handle) then
                    World.SetPosition(smoke.marker_handle, {
                        x = smoke.x,
                        y = smoke.y - 0.22,
                        z = smoke.z,
                    })
                end

                next_smokes[#next_smokes + 1] = smoke
            end
        end

        smokes = next_smokes

        if #smokes == 0 and (clock_ms - last_demo_spawn_ms) >= DEMO_RESPAWN_MS then
            spawn_demo_smoke('keepalive')
        end
    end
end)

CreateThread(function()
    while true do
        Wait(0)
        Gui.DrawText('[smoke_grenade] startup probe auto-spawns once; INTERACT adds more smoke', 0.02, 0.80, 0.24, 210, 220, 230, 175)
        Gui.DrawText('[smoke_grenade] orange sphere marks smoke center if volumetric pass is invisible', 0.02, 0.823, 0.22, 255, 190, 150, 165)
        Gui.DrawText(string.format('[smoke_grenade] active clouds: %d', #smokes), 0.02, 0.846, 0.22, 170, 205, 190, 165)
        Gui.DrawText(string.format('[smoke_grenade] fog debug: camera=%s env=%d env_vol=%d volumes=%d', tostring(fog_debug.camera_has_volumetric_fog), fog_debug.environment_light_count, fog_debug.environment_light_with_volumetric, fog_debug.fog_volume_count), 0.02, 0.869, 0.22, 200, 220, 255, 165)
        Gui.DrawText(string.format('[smoke_grenade] config: volumetric=%s shadows=%s ambient=%.2f steps=%d hour=%.2f', tostring(fog_debug.config_volumetric_enabled), tostring(fog_debug.config_shadows_enabled), fog_debug.config_ambient_intensity, fog_debug.config_step_count, fog_debug.config_hour), 0.02, 0.892, 0.22, 200, 220, 255, 165)
    end
end)

print('[smoke_grenade] client resource loaded')
