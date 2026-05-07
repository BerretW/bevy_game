-- ESC menu s volitelnou podporou DrawSprite.
--
-- Obrázky jsou VOLITELNÉ — menu vypadá dobře i bez nich.
-- Jakmile dodáš assety do images/ a nastavíš příznaky níže, zapnou se automaticky.
-- ── příznaky obrázků ─────────────────────────────────────────────────────────
local USE_BG = true -- esc_bg.jpg/png — celoplošná textura pozadí
local USE_LOGO = false -- esc_logo.png   — průhledné logo v headeru
local USE_ICONS = false -- esc_icons.png  — sprite sheet 2×2 ikon tlačítek

-- UV recty v sprite sheetu esc_icons.png (2×2 mřížka)
local UV = {
    resume = {0.0, 0.0, 0.5, 0.5},
    disconnect = {0.0, 0.5, 0.5, 1.0},
    quit = {0.5, 0.5, 1.0, 1.0}
}

-- ── tlačítka ─────────────────────────────────────────────────────────────────
local ITEMS = {{
    t = "btn",
    label = "Resume",
    style = "normal",
    icon = UV.resume,
    cb = function(set_open)
        set_open(false);
        Engine.SetCursorLocked(true)
    end
}, {
    t = "sep"
}, {
    t = "btn",
    label = "Disconnect",
    style = "danger",
    icon = UV.disconnect,
    cb = function(set_open)
        set_open(false);
        Engine.Disconnect()
    end
}, {
    t = "btn",
    label = "Quit to Desktop",
    style = "danger",
    icon = UV.quit,
    cb = function()
        Engine.Quit()
    end
}}

-- ── layout ───────────────────────────────────────────────────────────────────
local CX, CY = 0.50, 0.50
local W = 0.28
local LOGO_H = USE_LOGO and 0.068 or 0.0
local ICON_S = 0.024

-- ── stav ─────────────────────────────────────────────────────────────────────
local fade = 0.0
local open = false
local function set_open(v)
    open = v
end

-- ── renderer ─────────────────────────────────────────────────────────────────
local function render()
    local T = UI.Theme()
    local spd = open and T.fade_in or T.fade_out
    fade = fade + ((open and 1.0 or 0.0) - fade) * spd * 0.016
    fade = math.max(0.0, math.min(1.0, fade))
    if fade < 0.004 then
        return
    end
    local f = fade

    -- ── pozadí obrazovky ─────────────────────────────────────────────────────
    -- POZNÁMKA: obrázky se stahují jen při handshaku. Pokud nevidíš obrázek,
    -- restartuj klienta (reconnect) aby znovu stáhl aktuální digest ze serveru.
    if USE_BG then
        -- fit="fill": vyplní celou obrazovku bez ohledu na rozměry obrázku
        Gui.DrawSprite("esc_bg", 0.5, 0.5, 1.0, 1.0, 255, 255, 255, math.floor(180 * f), {
            fit = "fill"
        })
        -- tmavý overlay přes obrázek (menší než bez obrázku)
        Gui.DrawRect(0.5, 0.5, 1.0, 1.0, 0, 0, 0, math.floor(80 * f))
    else
        Gui.DrawRect(0.5, 0.5, 1.0, 1.0, 0, 0, 0, math.floor(120 * f))
    end

    -- ── výpočet výšky panelu ─────────────────────────────────────────────────
    local hh = T.header_h
    local bh = T.btn_h
    local px = T.pad_x
    local py = T.pad_y

    local ch = hh + LOGO_H + py
    for _, it in ipairs(ITEMS) do
        if it.t == "btn" then
            ch = ch + bh + T.btn_gap
        elseif it.t == "sep" then
            ch = ch + 0.016
        end
    end
    ch = ch + py
    local top = CY - ch * 0.5

    -- ── panel chrome ─────────────────────────────────────────────────────────
    local sc = T.shadow_col
    Gui.DrawShadow(CX, CY, W, ch, T.shadow_size, sc[1], sc[2], sc[3], math.floor(sc[4] * f))
    local brd = T.border
    Gui.DrawRoundedRect(CX, CY, W + T.border_w * 2, ch + T.border_w * 2, T.radius + T.border_w, brd[1], brd[2], brd[3],
        math.floor(brd[4] * f))
    local bg_col = T.bg
    Gui.DrawRoundedRect(CX, CY, W, ch, T.radius, bg_col[1], bg_col[2], bg_col[3], math.floor(bg_col[4] * f))

    -- ── header ───────────────────────────────────────────────────────────────
    local hcy = top + hh * 0.5
    local hb = T.bg_header
    Gui.DrawRoundedRect(CX, hcy, W, hh, T.radius, hb[1], hb[2], hb[3], math.floor(hb[4] * f))
    Gui.DrawRect(CX, top + hh - T.radius * 0.5, W, T.radius * 1.1, hb[1], hb[2], hb[3], math.floor(hb[4] * f))

    local tc = T.text

    if USE_LOGO then
        -- fit="fit": celé logo viditelné, zachová poměr stran
        local logo_cy = top + hh + LOGO_H * 0.5
        Gui.DrawSprite("esc_logo", CX, logo_cy, W - px * 2, LOGO_H - 0.008, 255, 255, 255, math.floor(235 * f), {
            fit = "fit"
        })
        -- oddělovač pod logem
        local logo_bot = top + hh + LOGO_H
        Gui.DrawLine(CX - W * 0.5 + px, logo_bot, CX + W * 0.5 - px, logo_bot, T.sep[1], T.sep[2], T.sep[3],
            math.floor(T.sep[4] * f * 0.5))
    else
        -- bez loga: nadpis "PAUSED" v headeru + oddělovač
        Gui.DrawText("PAUSED", CX - W * 0.5 + px, hcy - 0.009, 0.95, tc[1], tc[2], tc[3], math.floor(tc[4] * f))
        Gui.DrawLine(CX - W * 0.5 + 0.005, top + hh, CX + W * 0.5 - 0.005, top + hh, T.sep[1], T.sep[2], T.sep[3],
            math.floor(T.sep[4] * f))
    end

    -- ── tlačítka ─────────────────────────────────────────────────────────────
    local bw2 = W - px * 2
    local iy = top + hh + LOGO_H + py

    for _, it in ipairs(ITEMS) do
        if it.t == "btn" then
            local bcy = iy + bh * 0.5
            local hov = Gui.IsMouseOver(CX, bcy, bw2, bh)
            local held = hov and Gui.IsMouseDown()
            local clk = hov and Gui.IsMouseClicked()

            local bc
            if it.style == "danger" then
                local d = T.btn_danger
                bc = held and {math.floor(d[1] * .65), math.floor(d[2] * .65), math.floor(d[3] * .65), d[4]} or hov and
                         d or {math.floor(d[1] * .50), math.floor(d[2] * .50), math.floor(d[3] * .50), d[4]}
            else
                bc = held and T.btn_active or hov and T.btn_hover or T.btn
            end
            Gui.DrawRoundedRect(CX, bcy, bw2, bh, T.radius * 0.75, bc[1], bc[2], bc[3], math.floor((bc[4] or 215) * f))

            local text_x = CX - bw2 * 0.5 + px
            if USE_ICONS and it.icon then
                -- UV crop: každá ikona je čtvrtina sprite sheetu
                local ix = CX - bw2 * 0.5 + px * 0.5 + ICON_S * 0.5
                Gui.DrawSprite("esc_icons", ix, bcy, ICON_S, ICON_S, tc[1], tc[2], tc[3], math.floor(200 * f), {
                    uv = it.icon
                })
                text_x = CX - bw2 * 0.5 + px + ICON_S + 0.010
            end

            Gui.DrawText(it.label, text_x, bcy - 0.008, 0.85, tc[1], tc[2], tc[3], math.floor(tc[4] * f))

            if clk and it.cb then
                it.cb(set_open)
            end
            iy = iy + bh + T.btn_gap

        elseif it.t == "sep" then
            local sy = iy + 0.008
            Gui.DrawLine(CX - W * 0.5 + px, sy, CX + W * 0.5 - px, sy, T.sep[1], T.sep[2], T.sep[3],
                math.floor(T.sep[4] * f))
            iy = iy + 0.016
        end
    end
end

-- ── input ────────────────────────────────────────────────────────────────────
RegisterEvent("input:state", function(payload)
    if not payload then
        return
    end
    if payload.keys_just and payload.keys_just.options_menu then
        open = not open
        Engine.SetCursorLocked(not open)
    end
end)

-- ── draw thread ──────────────────────────────────────────────────────────────
CreateThread(function()
    while true do
        render()
        Wait(0)
    end
end)
