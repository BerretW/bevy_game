-- wet_tool — nastavení výšky namočení objektu.
--
-- Hráč namíří na objekt, přiblíží se na 1.2 m, stiskne E a zadá
-- výšku (v metrech od základny objektu) do které je objekt mokrý.
-- Vlhkost se zobrazí pomocí World.SetMaterialParam.

local INTERACT_DIST = 1.2       -- maximální vzdálenost interakce [m]
local INTERACT_KEY  = "e"       -- klávesa pro otevření promptu

-- ── Stav ──────────────────────────────────────────────────────────────────

local target_handle   = nil     -- handle entity pod crosshairem (nebo nil)
local target_distance = 0.0

local prompt_open     = false
local prompt_handle   = nil     -- entita editovaná v promptu
local prompt_pos_y    = 0.0     -- world Y základny objektu
local input_buf       = ""      -- aktuálně zadaný text (číslo)

-- ── Pomocné funkce ─────────────────────────────────────────────────────────

local function apply_wet_height(handle, base_y, height_str)
    local h = tonumber(height_str)
    if not h or h < 0 then return false end

    -- wet_height = world Y základna + zadaná výška [m]
    World.SetMaterialParam(handle, "wetness",    1.0)
    World.SetMaterialParam(handle, "wet_height", base_y + h)
    return true
end

local function open_prompt(handle)
    local pos = World.GetPosition(handle)
    if not pos then return end

    prompt_handle = handle
    prompt_pos_y  = pos[2]      -- world Y středu objektu (aproximace základny)
    input_buf     = ""
    prompt_open   = true
end

local function close_prompt()
    prompt_open   = false
    prompt_handle = nil
    input_buf     = ""
end

-- Vrátí true pokud je znak povoleno přidat (max délka 7 znaků).
local function can_append(ch)
    if #input_buf >= 7 then return false end
    if ch == "." then
        return not input_buf:find(".", 1, true)
    end
    return true
end

-- ── Herní smyčka ──────────────────────────────────────────────────────────

CreateThread(function()
    while true do
        Wait(0)

        -- Zjisti entitu pod crosshairem (max INTERACT_DIST metrů).
        local hit = Raycast.GetEntityUnderCrosshair(INTERACT_DIST)
        if hit and World.IsValid(hit.handle) then
            target_handle   = hit.handle
            target_distance = hit.distance
        else
            target_handle   = nil
            target_distance = 0.0
        end

        -- Vstup: otevření promptu klávesou E (jen pokud je cíl v dosahu a prompt zavřený).
        if not prompt_open then
            if target_handle and Input.IsKeyJustPressed(INTERACT_KEY) then
                open_prompt(target_handle)
            end
        else
            -- ── Zachycení kláves v promptu ────────────────────────────────
            -- Čísla 0–9
            for _, d in ipairs({"0","1","2","3","4","5","6","7","8","9"}) do
                if Input.IsKeyJustPressed(d) and can_append(d) then
                    input_buf = input_buf .. d
                end
            end
            -- Desetinná tečka (klávesa "period" nebo numpad "decimal")
            if Input.IsKeyJustPressed("period") and can_append(".") then
                input_buf = input_buf .. "."
            end
            -- Backspace
            if Input.IsKeyJustPressed("backspace") and #input_buf > 0 then
                input_buf = input_buf:sub(1, -2)
            end
            -- Enter → potvrdit
            if Input.IsKeyJustPressed("enter") then
                if #input_buf > 0 and prompt_handle then
                    local ok = apply_wet_height(prompt_handle, prompt_pos_y, input_buf)
                    if ok then
                        log_info("[wet_tool] wet_height=" .. input_buf .. "m aplikováno na " .. tostring(prompt_handle))
                    else
                        log_warn("[wet_tool] Neplatná hodnota: '" .. input_buf .. "'")
                    end
                end
                close_prompt()
            end
            -- Escape → zrušit
            if Input.IsKeyJustPressed("escape") then
                close_prompt()
            end
        end
    end
end)

-- ── GUI ───────────────────────────────────────────────────────────────────

CreateThread(function()
    while true do
        Wait(0)

        if prompt_open then
            -- Tmavý panel uprostřed obrazovky
            local px, py = 0.5, 0.46
            local pw, ph = 0.32, 0.16
            Gui.DrawRect(px - pw/2, py - ph/2, pw, ph, 0.05, 0.05, 0.05, 0.88)
            Gui.DrawBorder(px - pw/2, py - ph/2, pw, ph, 0.002, 0.3, 0.6, 1.0, 0.9)

            Gui.DrawText("Výška namočení (metry od základny objektu)",
                px, py - 0.04, 0.016, 0.85, 0.85, 0.85, 1.0, "center")

            -- Vstupní pole
            local display = input_buf == "" and "_" or input_buf
            Gui.DrawRect(px - 0.08, py + 0.005, 0.16, 0.038, 0.1, 0.1, 0.1, 0.95)
            Gui.DrawText(display .. " m",
                px, py + 0.024, 0.022, 1.0, 1.0, 1.0, 1.0, "center")

            Gui.DrawText("Enter = potvrdit   |   Escape = zrušit",
                px, py + 0.062, 0.013, 0.55, 0.55, 0.55, 1.0, "center")

        elseif target_handle then
            -- Interakční hint dole uprostřed
            Gui.DrawText("[" .. string.upper(INTERACT_KEY) .. "] Nastavit výšku namočení",
                0.5, 0.87, 0.017, 1.0, 1.0, 1.0, 0.9, "center")
        end
    end
end)
