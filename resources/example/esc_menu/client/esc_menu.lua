-- ESC menu — pause overlay shown when the player presses Escape in-game.
-- Engine.SetCursorLocked(false) releases the mouse while the menu is open.

local open = false

-- Layout (normalised 0-1 screen coords, origin top-left)
local PANEL_W, PANEL_H = 0.28, 0.36
local PANEL_X = (1.0 - PANEL_W) / 2.0
local PANEL_Y = (1.0 - PANEL_H) / 2.0

local BTN_W, BTN_H = 0.20, 0.055
local BTN_X = PANEL_X + (PANEL_W - BTN_W) / 2.0

-- Colours (r, g, b, a)
local C_OVERLAY = { 0,   0,   0,   120 }
local C_PANEL   = { 20,  22,  28,  230 }
local C_TITLE   = { 220, 220, 220, 255 }
local C_TXT     = { 215, 215, 215, 255 }

local BUTTONS = {
    { label = "Resume",          yOff = 0,     r=35,  g=80,  b=55  },
    { label = "Disconnect",      yOff = 0.075, r=160, g=90,  b=30  },
    { label = "Quit to Desktop", yOff = 0.150, r=140, g=30,  b=30  },
}

local function open_menu()
    open = true
    Engine.SetCursorLocked(false)
end

local function close_menu()
    open = false
    Engine.SetCursorLocked(true)
end

local function do_action(idx)
    if idx == 1 then
        close_menu()
    elseif idx == 2 then
        close_menu()
        Engine.Disconnect()
    elseif idx == 3 then
        Engine.Quit()
    end
end

-- Toggle on Escape and handle keyboard shortcuts while open
RegisterEvent("input:state", function(payload)
    if not payload then return end

    -- Toggle menu on Escape just-pressed
    if payload.keys_just and payload.keys_just.options_menu then
        if open then close_menu() else open_menu() end
        return
    end
end)

-- Render loop
CreateThread(function()
    while true do
        if open then
            -- Fullscreen dim overlay
            Gui.DrawRect(0, 0, 1, 1,
                C_OVERLAY[1], C_OVERLAY[2], C_OVERLAY[3], C_OVERLAY[4])

            -- Panel background
            Gui.DrawRect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H,
                C_PANEL[1], C_PANEL[2], C_PANEL[3], C_PANEL[4])

            -- Title
            Gui.DrawText("PAUSED",
                PANEL_X + 0.035, PANEL_Y + 0.025, 1.5,
                C_TITLE[1], C_TITLE[2], C_TITLE[3], C_TITLE[4])

            -- Separator
            local sep_y = PANEL_Y + 0.075
            Gui.DrawLine(
                PANEL_X + 0.01, sep_y, PANEL_X + PANEL_W - 0.01, sep_y,
                80, 90, 110, 200)

            -- Buttons
            local by_base = PANEL_Y + 0.105
            for i, btn in ipairs(BUTTONS) do
                local by = by_base + btn.yOff
                Gui.DrawRect(BTN_X, by, BTN_W, BTN_H, btn.r, btn.g, btn.b, 230)
                Gui.DrawText(btn.label,
                    BTN_X + 0.018, by + (BTN_H - 0.018) / 2.0, 0.9,
                    C_TXT[1], C_TXT[2], C_TXT[3], C_TXT[4])
                -- Button number hint (keyboard shortcut)
                Gui.DrawText(tostring(i),
                    BTN_X + BTN_W - 0.025, by + (BTN_H - 0.018) / 2.0, 0.75,
                    120, 120, 120, 200)
            end

            -- Hint text
            Gui.DrawText("Press 1 / 2 / 3 to select",
                PANEL_X + 0.015, PANEL_Y + PANEL_H - 0.035, 0.7,
                100, 100, 100, 200)
        end
        Wait(0)
    end
end)

-- Keyboard number shortcuts while menu is open
RegisterEvent("input:state", function(payload)
    if not open then return end
    -- Number keys are not in input:state yet; handle via a separate polling thread
    -- until key bindings are extended (Phase 5).
    -- For now the menu can be closed with Escape (handled above).
end)
