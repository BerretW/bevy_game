-- wet_tool — nastavení výšky namočení objektu.
-- Hráč namíří na objekt, přiblíží se na INTERACT_DIST m, stiskne E.

local INTERACT_DIST = 5.0
local INTERACT_KEY  = "e"

-- ── Stav ──────────────────────────────────────────────────────────────────

local target_handle   = nil
local target_distance = 0.0

local prompt_open   = false
local prompt_handle = nil
local prompt_pos_y  = 0.0
local input_buf     = ""

local last_error    = nil

-- ── Pomocné funkce ─────────────────────────────────────────────────────────

local function apply_wet_height(handle, base_y, height_str)
    local h = tonumber(height_str)
    if not h or h < 0 then return false, "neplatna hodnota: '" .. height_str .. "'" end
    World.SetMaterialParam(handle, "wetness",    1.0)
    World.SetMaterialParam(handle, "wet_height", base_y + h)
    log_info("[wet_tool] wet_height=" .. height_str .. "m -> world Y=" .. tostring(base_y + h))
    return true, nil
end

local function open_prompt(handle)
    local pos = World.GetPosition(handle)
    if not pos then
        last_error = "GetPosition nil pro handle " .. tostring(handle)
        return
    end
    prompt_handle = handle
    prompt_pos_y  = pos.y
    input_buf     = ""
    prompt_open   = true
    last_error    = nil
end

local function close_prompt()
    prompt_open   = false
    prompt_handle = nil
    input_buf     = ""
end

local function can_append(ch)
    if #input_buf >= 7 then return false end
    if ch == "." then return not input_buf:find(".", 1, true) end
    return true
end

-- ── Herní smyčka ──────────────────────────────────────────────────────────

CreateThread(function()
    while true do
        Wait(0)

        local hit = Raycast.GetEntityUnderCrosshair(INTERACT_DIST)
        if hit and World.IsValid(hit.handle) then
            target_handle   = hit.handle
            target_distance = hit.distance
        else
            target_handle   = nil
            target_distance = 0.0
        end

        if not prompt_open then
            if target_handle and Input.IsKeyJustPressed(INTERACT_KEY) then
                open_prompt(target_handle)
            end
        else
            for _, d in ipairs({"0","1","2","3","4","5","6","7","8","9"}) do
                if Input.IsKeyJustPressed(d) and can_append(d) then
                    input_buf = input_buf .. d
                end
            end
            if Input.IsKeyJustPressed("period") and can_append(".") then
                input_buf = input_buf .. "."
            end
            if Input.IsKeyJustPressed("backspace") and #input_buf > 0 then
                input_buf = input_buf:sub(1, -2)
            end
            if Input.IsKeyJustPressed("enter") then
                if #input_buf > 0 and prompt_handle then
                    local ok, err = apply_wet_height(prompt_handle, prompt_pos_y, input_buf)
                    if not ok then last_error = err end
                end
                close_prompt()
            end
            if Input.IsKeyJustPressed("escape") then
                close_prompt()
            end
        end
    end
end)

-- ── GUI ───────────────────────────────────────────────────────────────────
-- DrawText: anchor = TOP_LEFT → x,y je LEVY HORNI roh textu.
-- DrawRect: anchor = CENTER  → x,y je STRED obdelniku.
-- Font scale: font_size = scale * win_h * 0.04, clamp(8,256).
--   Pro 1080p: scale 0.4 = 17px, 0.5 = 22px, 0.6 = 26px.

CreateThread(function()
    while true do
        Wait(0)

        -- ── Debug overlay ────────────────────────────────────────────────
        local dy = 0.10
        local function dbg(text, r, g, b)
            Gui.DrawText(text, 0.01, dy, 0.40, r or 255, g or 255, b or 255, 220)
            dy = dy + 0.024
        end

        dbg("[wet_tool] dist_max=" .. INTERACT_DIST .. "m", 180, 180, 180)
        if target_handle then
            dbg("hit: h=" .. tostring(target_handle) .. "  d=" .. string.format("%.2f", target_distance) .. "m", 80, 255, 80)
        else
            dbg("hit: zadny (>dist / bez fyziky)", 255, 100, 60)
        end
        if prompt_open then
            dbg("prompt: '" .. input_buf .. "'", 255, 220, 50)
        end
        if last_error then
            dbg("ERR: " .. last_error, 255, 60, 60)
        end

        -- ── Prompt ────────────────────────────────────────────────────────
        if prompt_open then
            -- Panel: střed 0.5 × 0.46, rozměr 0.36 × 0.20
            local px, py = 0.5, 0.46
            local pw, ph = 0.36, 0.20
            local lx = px - pw / 2 + 0.012   -- levy okraj textu s marginem

            -- Pozadí a rámeček
            Gui.DrawRect(px, py, pw, ph, 10, 10, 10, 230)
            Gui.DrawBorder(px, py, pw, ph, 0.002, 60, 140, 255, 230)

            -- Nadpis (TOP_LEFT, y = horní hrana panelu + malý mezera)
            Gui.DrawText("Vyska namoceni [m od zakladny]",
                lx, py - ph/2 + 0.015, 0.40, 200, 200, 200, 255)

            -- Vstupní pole
            local field_y = py + 0.015
            Gui.DrawRect(px, field_y, pw - 0.04, 0.05, 20, 20, 20, 245)
            Gui.DrawBorder(px, field_y, pw - 0.04, 0.05, 0.0015, 100, 100, 100, 200)

            local display = (input_buf == "" and "_" or input_buf) .. " m"
            -- text vstupního pole: levý okraj pole + margin, vertikálně centrovat
            Gui.DrawText(display,
                px - (pw - 0.04)/2 + 0.015, field_y - 0.014, 0.55, 255, 255, 255, 255)

            -- Nápověda (dolní část panelu)
            Gui.DrawText("Enter = ok     Escape = zrusit",
                lx, py + ph/2 - 0.035, 0.35, 120, 120, 120, 255)

        elseif target_handle then
            -- Hint dole uprostřed obrazovky
            Gui.DrawRect(0.5, 0.88, 0.34, 0.040, 0, 0, 0, 170)
            Gui.DrawText("[" .. string.upper(INTERACT_KEY) .. "] Nastavit vysku namoceni",
                0.5 - 0.34/2 + 0.012, 0.868, 0.42, 255, 255, 255, 230)
        end
    end
end)
