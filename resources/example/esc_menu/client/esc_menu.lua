-- ESC menu — pause overlay.
-- DrawRect(x, y, w, h): x,y = CENTER (FiveM/GTA5 convention).
-- DrawText(text, x, y, ...): x,y = TOP-LEFT (Bevy Anchor::TOP_LEFT).

local open = false

-- Panel: center coords
local PANEL_CX, PANEL_CY = 0.50, 0.50
local PANEL_W,  PANEL_H  = 0.28, 0.36

-- Derived edges used for text anchoring and line endpoints
local PANEL_LEFT   = PANEL_CX - PANEL_W * 0.5   -- 0.36
local PANEL_TOP    = PANEL_CY - PANEL_H * 0.5   -- 0.32
local PANEL_RIGHT  = PANEL_CX + PANEL_W * 0.5   -- 0.64
local PANEL_BOTTOM = PANEL_CY + PANEL_H * 0.5   -- 0.68

local BTN_W, BTN_H = 0.20, 0.055
local BTN_CX       = PANEL_CX            -- buttons centered horizontally
local BTN_Y_BASE   = PANEL_TOP + 0.105 + BTN_H * 0.5  -- center y of first button

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

RegisterEvent("input:state", function(payload)
    if not payload then return end
    if payload.keys_just and payload.keys_just.options_menu then
        if open then close_menu() else open_menu() end
    end
end)

CreateThread(function()
    while true do
        if open then
            -- Fullscreen dim overlay (center = screen center, covers everything)
            Gui.DrawRect(0.5, 0.5, 1.0, 1.0,
                C_OVERLAY[1], C_OVERLAY[2], C_OVERLAY[3], C_OVERLAY[4])

            -- Panel background
            Gui.DrawRect(PANEL_CX, PANEL_CY, PANEL_W, PANEL_H,
                C_PANEL[1], C_PANEL[2], C_PANEL[3], C_PANEL[4])

            -- Title (DrawText is top-left anchored)
            Gui.DrawText("PAUSED",
                PANEL_LEFT + 0.035, PANEL_TOP + 0.025, 1.5,
                C_TITLE[1], C_TITLE[2], C_TITLE[3], C_TITLE[4])

            -- Separator
            local sep_y = PANEL_TOP + 0.075
            Gui.DrawLine(
                PANEL_LEFT + 0.010, sep_y,
                PANEL_RIGHT - 0.010, sep_y,
                80, 90, 110, 200)

            -- Buttons
            for i, btn in ipairs(BUTTONS) do
                local bcy  = BTN_Y_BASE + btn.yOff
                local btop = bcy - BTN_H * 0.5

                if Gui.Button(BTN_CX, bcy, BTN_W, BTN_H, btn.label, btn.r, btn.g, btn.b) then
                    do_action(i)
                end

                -- Keyboard shortcut hint (right side of button)
                Gui.DrawText(tostring(i),
                    BTN_CX + BTN_W * 0.5 - 0.025,
                    btop + (BTN_H - 0.018) * 0.5,
                    0.75,
                    120, 120, 120, 200)
            end

            -- Bottom hint
            Gui.DrawText("Press 1 / 2 / 3 to select",
                PANEL_LEFT + 0.015, PANEL_BOTTOM - 0.038, 0.7,
                100, 100, 100, 200)
        end
        Wait(0)
    end
end)
