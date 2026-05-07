-- Ukázkový HUD: health bar + crosshair + fps counter.
--
-- Vzor: CreateThread + nekonečná smyčka + Wait(0) pro per-frame drawing.
-- Gui.Draw* je client-only — na serveru jsou no-ops.

local fps = 0
local frame_count = 0

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
        Gui.DrawText(string.format("FPS: %.1f", fps), 0.88, 0.01, 0.4, 200, 200, 200, 200)

        Wait(0) -- pokračuj v příštím frame
    end
end)

print("HUD loaded")
