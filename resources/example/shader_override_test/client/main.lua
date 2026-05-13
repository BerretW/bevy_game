assert(IS_CLIENT, 'expected to run on client side')

local PROFILES = {
    'debug_stripes',
    'hologram',
    'heat',
    'dissolve',
}

local local_player_handle = nil
local status_text = 'waiting_for_local_player'
local active_index = 1

local function active_profile()
    return PROFILES[active_index]
end

local function apply_profile()
    if not local_player_handle then
        return
    end
    World.SetEntityShaderProfile(local_player_handle, active_profile())
    status_text = 'active'
    log_info(string.format('[shader_override_test] shader profile %s active on handle=%d', active_profile(), local_player_handle))
end

local function cycle_profile()
    active_index = active_index + 1
    if active_index > #PROFILES then
        active_index = 1
    end
    apply_profile()
end

RegisterEvent('player:anim_state', function(payload)
    if type(payload) ~= 'table' or payload.is_local ~= true then
        return
    end

    local handle = tonumber(payload.handle)
    if not handle or handle <= 0 then
        return
    end

    if local_player_handle == handle then
        return
    end

    local_player_handle = handle
    apply_profile()
end)

CreateThread(function()
    while true do
        Wait(0)

        if Input.IsKeyJustPressed('f8') then
            cycle_profile()
        end

        Gui.DrawRect(0.185, 0.085, 0.34, 0.092, 22, 28, 34, 170)
        Gui.DrawBorder(0.185, 0.085, 0.34, 0.092, 0.002, 255, 255, 255, 210)
        Gui.DrawText('[shader_override_test] Entity shader profile', 0.03, 0.050, 0.34, 230, 230, 230, 240)
        Gui.DrawText(string.format('profile: %s   status: %s', active_profile(), status_text), 0.03, 0.074, 0.30, 120, 255, 180, 235)
        Gui.DrawText(string.format('handle: %s   F8 = next profile', tostring(local_player_handle)), 0.03, 0.096, 0.26, 180, 220, 255, 220)
    end
end)

print('[shader_override_test] Client resource loaded')