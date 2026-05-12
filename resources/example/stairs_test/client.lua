-- Stairs IK Test - Client Side
-- Vizualizace detekce STAIRS collideru a reakce hráče.

local stairs_state = {
    on_stairs = false,
    reacting = false,
    grounded = false,
    hit_distance = -1.0,
    hit_pos = nil,
    player = { x = 0.0, y = 0.0, z = 0.0, vy = 0.0 },
}

local last_on_stairs = false
local hit_marker_handle = nil

local function ensure_hit_marker()
    if hit_marker_handle and World.IsValid(hit_marker_handle) then
        return hit_marker_handle
    end

    hit_marker_handle = World.SpawnLocalDummy(
        'sphere',
        {
            radius = 0.055,
            r = 0.10,
            g = 1.00,
            b = 0.20,
            a = 0.95,
            collider = {
                enabled = false,
            },
        },
        { x = 0.0, y = -9999.0, z = 0.0 },
        { x = 0.0, y = 0.0, z = 0.0 }
    )

    return hit_marker_handle
end

RegisterEvent('stairs:state', function(payload)
    if type(payload) ~= 'table' then
        return
    end

    stairs_state.on_stairs = payload.on_stairs == true
    stairs_state.reacting = payload.reacting == true
    stairs_state.grounded = payload.grounded == true
    stairs_state.hit_distance = tonumber(payload.hit_distance) or -1.0

    if type(payload.hit_pos) == 'table' then
        stairs_state.hit_pos = {
            x = tonumber(payload.hit_pos.x) or 0.0,
            y = tonumber(payload.hit_pos.y) or 0.0,
            z = tonumber(payload.hit_pos.z) or 0.0,
        }
    else
        stairs_state.hit_pos = nil
    end

    if type(payload.player) == 'table' then
        stairs_state.player.x = tonumber(payload.player.x) or 0.0
        stairs_state.player.y = tonumber(payload.player.y) or 0.0
        stairs_state.player.z = tonumber(payload.player.z) or 0.0
        stairs_state.player.vy = tonumber(payload.player.vy) or 0.0
    end

    if stairs_state.on_stairs ~= last_on_stairs then
        if stairs_state.on_stairs then
            log_info('[stairs_test] STAIRS DETECTION: ENTER')
        else
            log_info('[stairs_test] STAIRS DETECTION: EXIT')
        end
        last_on_stairs = stairs_state.on_stairs
    end
end)

CreateThread(function()
    while true do
        Wait(0)

        local on = stairs_state.on_stairs
        local reacting = stairs_state.reacting

        local panel_r, panel_g, panel_b = 60, 60, 60
        if on then
            panel_r, panel_g, panel_b = 30, 110, 40
        end

        Gui.DrawRect(0.185, 0.155, 0.34, 0.16, panel_r, panel_g, panel_b, 170)
        Gui.DrawBorder(0.185, 0.155, 0.34, 0.16, 0.002, 255, 255, 255, 210)

        Gui.DrawText('[stairs_test] STAIRS DEBUG', 0.03, 0.095, 0.42, 230, 230, 230, 240)

        if on then
            Gui.DrawText('Detekce: ON (hrac je na stairs collideru)', 0.03, 0.120, 0.38, 80, 255, 80, 240)
        else
            Gui.DrawText('Detekce: OFF (mimo stairs collider)', 0.03, 0.120, 0.38, 255, 120, 80, 240)
        end

        if reacting then
            Gui.DrawText('Reakce: OK (grounded + stairs hit)', 0.03, 0.143, 0.36, 120, 255, 120, 235)
        else
            Gui.DrawText('Reakce: cekam (chybi grounded nebo hit)', 0.03, 0.143, 0.36, 255, 220, 100, 235)
        end

        Gui.DrawText(string.format('hit_distance: %.3f m', stairs_state.hit_distance), 0.03, 0.166, 0.34, 210, 210, 210, 220)
        Gui.DrawText(string.format('player y: %.3f   vy: %.3f', stairs_state.player.y, stairs_state.player.vy), 0.03, 0.188, 0.34, 210, 210, 210, 220)

        local marker = ensure_hit_marker()
        if stairs_state.hit_pos then
            World.SetPosition(marker, {
                x = stairs_state.hit_pos.x,
                y = stairs_state.hit_pos.y + 0.03,
                z = stairs_state.hit_pos.z,
            })
        else
            World.SetPosition(marker, { x = 0.0, y = -9999.0, z = 0.0 })
        end
    end
end)

print('[stairs_test] Client resource loaded (stairs debug overlay active)')
