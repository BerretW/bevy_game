assert(IS_CLIENT, 'expected to run on client side')

local SMOKE_DURATION_MS = 16000
local SMOKE_GROW_MS = 1800
local SMOKE_MAX_RADIUS = 12.0
local SMOKE_HEIGHT = 7.0
local SMOKE_DRIFT_Y = 0.85

local smokes = {}
local input_state = {
    interact = false,
}
local startup_spawn_done = false

local function clamp01(value)
    if value < 0.0 then
        return 0.0
    end
    if value > 1.0 then
        return 1.0
    end
    return value
end

local function make_smoke_params(radius)
    return {
        size = { x = 1.0, y = 1.0, z = 1.0 },
        fog_volume = {
            color = { 0.86, 0.86, 0.88, 0.96 },
            density = 0.95,
            absorption = 0.08,
            scattering = 0.84,
            scattering_asymmetry = 0.72,
            light_tint = { 0.96, 0.96, 0.98, 1.0 },
            light_intensity = 1.8,
        },
        collider = {
            enabled = false,
        },
    }
end

local function spawn_smoke_grenade(hit)
    local radius = 0.35
    local handle = World.SpawnLocalDummy(
        'fog_volume',
        make_smoke_params(radius),
        { x = hit.x, y = hit.y + 0.35, z = hit.z },
        { x = 0.0, y = 0.0, z = 0.0 }
    )

    World.SetScale(handle, {
        x = radius * 2.0,
        y = SMOKE_HEIGHT,
        z = radius * 2.0,
    })

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
        handle = handle,
        marker_handle = marker_handle,
        x = hit.x,
        y = hit.y + 0.35,
        z = hit.z,
        age_ms = 0,
    }

    log_info(string.format('[smoke_grenade] spawned local smoke at %.2f %.2f %.2f', hit.x, hit.y, hit.z))
end

CreateThread(function()
    while not startup_spawn_done do
        Wait(250)
        local hit = Raycast.GetGroundPosition()
        if type(hit) == 'table' then
            startup_spawn_done = true
            spawn_smoke_grenade({
                x = tonumber(hit.x) or 0.0,
                y = tonumber(hit.y) or 0.0,
                z = tonumber(hit.z) or 0.0,
            })
            log_info('[smoke_grenade] startup smoke probe spawned automatically')
        end
    end
end)

RegisterEvent('input:state', function(payload)
    if type(payload) ~= 'table' or type(payload.keys) ~= 'table' then
        return
    end

    local pressed = payload.keys.interact == true
    if pressed and not input_state.interact then
        local hit = Raycast.GetGroundPosition()
        if type(hit) == 'table' then
            spawn_smoke_grenade({
                x = tonumber(hit.x) or 0.0,
                y = tonumber(hit.y) or 0.0,
                z = tonumber(hit.z) or 0.0,
            })
        end
    end
    input_state.interact = pressed
end)

CreateThread(function()
    while true do
        Wait(100)

        local next_smokes = {}
        for i = 1, #smokes do
            local smoke = smokes[i]
            smoke.age_ms = smoke.age_ms + 100

            if not World.IsValid(smoke.handle) or smoke.age_ms >= SMOKE_DURATION_MS then
                if World.IsValid(smoke.handle) then
                    World.DeleteObject(smoke.handle)
                end
                if smoke.marker_handle and World.IsValid(smoke.marker_handle) then
                    World.DeleteObject(smoke.marker_handle)
                end
            else
                local grow_t = clamp01(smoke.age_ms / SMOKE_GROW_MS)
                local life_t = clamp01(smoke.age_ms / SMOKE_DURATION_MS)
                local radius = 0.35 + (SMOKE_MAX_RADIUS - 0.35) * grow_t
                local y = smoke.y + SMOKE_DRIFT_Y * life_t

                World.SetPosition(smoke.handle, {
                    x = smoke.x,
                    y = y,
                    z = smoke.z,
                })
                World.SetScale(smoke.handle, {
                    x = radius * 2.0,
                    y = SMOKE_HEIGHT,
                    z = radius * 2.0,
                })
                if smoke.marker_handle and World.IsValid(smoke.marker_handle) then
                    World.SetPosition(smoke.marker_handle, {
                        x = smoke.x,
                        y = smoke.y + 0.18,
                        z = smoke.z,
                    })
                end

                next_smokes[#next_smokes + 1] = smoke
            end
        end

        smokes = next_smokes
    end
end)

CreateThread(function()
    while true do
        Wait(0)
        Gui.DrawText('[smoke_grenade] startup probe auto-spawns once; INTERACT adds more smoke', 0.02, 0.80, 0.24, 210, 220, 230, 175)
        Gui.DrawText('[smoke_grenade] orange sphere marks smoke center if volumetric pass is invisible', 0.02, 0.823, 0.22, 255, 190, 150, 165)
        Gui.DrawText(string.format('[smoke_grenade] active clouds: %d', #smokes), 0.02, 0.846, 0.22, 170, 205, 190, 165)
    end
end)

print('[smoke_grenade] client resource loaded')
