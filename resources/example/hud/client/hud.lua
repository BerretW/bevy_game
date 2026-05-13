-- Ukázkový HUD: health bar + crosshair + fps counter.
--
-- Vzor: CreateThread + nekonečná smyčka + Wait(0) pro per-frame drawing.
-- Gui.Draw* je client-only — na serveru jsou no-ops.

local fps = 0
local frame_count = 0

local function get_active_weapon_snapshot(stats)
    local slot = (stats.active_weapon_slot or 0) + 1
    local slots = stats.weapon_slots or {}
    local equipped = slots[slot]
    if not equipped then
        return slot, nil, 0
    end

    local ammo_type = equipped.ammo_type_id
    if (not ammo_type or ammo_type == '') and equipped.weapon_id then
        ammo_type = equipped.weapon_id
    end

    local reserve = 0
    if ammo_type and stats.ammo_reserve then
        reserve = stats.ammo_reserve[ammo_type] or 0
    end

    return slot, equipped, reserve
end

-- FPS counter thread — počítá framy za sekundu bez os.clock()
CreateThread(function()
    while true do
        local count = frame_count
        frame_count = 0
        fps = count
        Wait(1000)
    end
end)

-- Hlavní HUD thread — běží každý frame
CreateThread(function()
    while true do
        frame_count = frame_count + 1
        local stats = Player.GetLocalStats()
        local hp     = stats.hp     or 100
        local max_hp = stats.max_hp or 100
        local frac   = math.max(0, math.min(1, hp / max_hp))
        local active_slot, equipped, reserve = get_active_weapon_snapshot(stats)
        local fire = stats.fire or {}
        local reload = stats.reload or {}
        local weapon_swap = stats.weapon_swap or {}

        -- Health bar pozadí (tmavé)
        Gui.DrawRect(0.02, 0.94, 0.20, 0.035, 30, 30, 30, 200)

        -- Health bar výplň (zelená → červená podle HP)
        local r = math.floor(255 * (1 - frac))
        local g = math.floor(255 * frac)
        Gui.DrawRect(0.02 + 0.20 * frac / 2, 0.94, 0.20 * frac, 0.035, r, g, 0, 220)

        -- HP text (9. parametr = font_id, nil = výchozí font)
        Gui.DrawText(string.format("HP: %d / %d", math.floor(hp), math.floor(max_hp)),
                     0.022, 0.928, 0.45, 255, 255, 255, 255, 'roboto')

        -- Crosshair (4 krátké čáry)
        local cx, cy = 0.5, 0.5
        local gap = 0.012
        local len = 0.020
        Gui.DrawLine(cx - gap - len, cy,  cx - gap, cy,  255, 255, 255, 200)
        Gui.DrawLine(cx + gap,       cy,  cx + gap + len, cy, 255, 255, 255, 200)
        Gui.DrawLine(cx, cy - gap - len,  cx, cy - gap,   255, 255, 255, 200)
        Gui.DrawLine(cx, cy + gap,        cx, cy + gap + len, 255, 255, 255, 200)
        -- Obrázek jako crosshair: Gui.DrawSprite('crosshair', cx, cy, 0.02, 0.02)

        -- FPS counter (pravý horní roh)
        Gui.DrawText(string.format("FPS: %.1f", fps), 0.88, 0.01, 0.4, 200, 0, 0, 200,'roboto')

        -- Weapon panel (pravý dolní roh)
        Gui.DrawRect(0.79, 0.87, 0.18, 0.11, 20, 20, 24, 205)

        local weapon_name = equipped and equipped.weapon_id or 'unarmed'
        local ammo_in_mag = equipped and (equipped.ammo_in_mag or 0) or 0
        Gui.DrawText(string.format("SLOT %d  %s", active_slot, weapon_name),
                     0.80, 0.855, 0.34, 255, 235, 210, 255, 'roboto')
        Gui.DrawText(string.format("AMMO %d / %d", ammo_in_mag, reserve),
                     0.80, 0.892, 0.32, 240, 240, 240, 255, 'roboto')

        local state_line = string.format(
            "F %.2f  R %.2f  SW %.2f",
            fire.cooldown_remaining or 0,
            reload.remaining or 0,
            weapon_swap.remaining or 0
        )
        Gui.DrawText(state_line,
                     0.80, 0.926, 0.26, 180, 220, 255, 220, 'roboto')

        if weapon_swap.active and weapon_swap.target_slot ~= nil then
            Gui.DrawText(string.format("swap -> slot %d", (weapon_swap.target_slot or 0) + 1),
                         0.80, 0.954, 0.22, 255, 210, 120, 220, 'roboto')
        elseif reload.active then
            local remaining = reload.remaining or 0
            local duration = reload.duration or 0
            Gui.DrawText(string.format("reloading %.2f / %.2f", remaining, duration),
                         0.80, 0.954, 0.22, 255, 210, 120, 220, 'roboto')
        elseif fire.trigger_held then
            Gui.DrawText("trigger held", 0.80, 0.954, 0.22, 255, 210, 120, 220, 'roboto')
        end

        Wait(0) -- pokračuj v příštím frame
    end
end)

print("HUD loaded")
