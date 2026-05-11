-- shared/main.lua
-- Test blend space API (Lua side)
-- Runtime evaluaci blend space vah zatím nemáme, ale infrastruktura je hotova.

log_info(string.format('[%s] blend_space_test loaded on %s', RESOURCE_ID, SIDE))

if SIDE ~= 'client' then
    return
end

local TEST_MODEL = 'mutant'
local TEST_POS = { x = 8.0, y = 1.0, z = 3.0 }
local TEST_ROT = { x = 0.0, y = 180.0, z = 0.0 }
local UPDATE_INTERVAL_MS = 500  -- Aktualizace direction vektoru každých 500ms
local MOVE_SPEED = 2.0

local test_handle = nil
local angle = 0.0

local function ensure_test_entity()
    local handles = World.GetHandlesByModel(TEST_MODEL)
    if handles and #handles > 0 then
        test_handle = handles[1]
        for i = 2, #handles do
            World.DeleteObject(handles[i])
        end
        return
    end

    test_handle = World.SpawnLocalObject(TEST_MODEL, TEST_POS, TEST_ROT)
end

CreateThread(function()
    ensure_test_entity()
    Engine.RequestModel(TEST_MODEL)

    local attempts = 0
    while attempts < 600 do
        local loaded = Engine.HasModelLoaded(TEST_MODEL)
        if loaded then
            break
        end
        attempts = attempts + 1
        Wait(100)
    end

    if not test_handle then
        log_warn('[blend_space_test] failed to spawn test entity')
        return
    end

    log_info(string.format('[blend_space_test] model=%s handle=%d ready', TEST_MODEL, test_handle))
    log_info('[blend_space_test] TIP: blend_space_test bude aktivní jakmile bude blend space evaluace v runtime')
end)

-- Rotační vlákno — aktualizuje direction vektor
CreateThread(function()
    while true do
        Wait(UPDATE_INTERVAL_MS)

        if not test_handle then
            goto continue
        end

        -- Rotuj angle a vypočítej move vektor (kruh)
        angle = (angle + math.pi / 8) % (2 * math.pi)  -- 45° za update
        local move_x = math.cos(angle) * MOVE_SPEED
        local move_y = math.sin(angle) * MOVE_SPEED

        -- Zkus zavolat PlayBlendSpace (režim bez chyby — blend space musí existovat v modelu)
        local ok, err = pcall(function()
            World.PlayBlendSpace(test_handle, 'locomotion', move_x, move_y, 1.0, 1)
        end)

        if not ok then
            log_debug(string.format('[blend_space_test] PlayBlendSpace error: %s', tostring(err)))
        else
            log_debug(string.format('[blend_space_test] blend_space angle=%.2f rad move=(%.2f, %.2f)', angle, move_x, move_y))
        end

        ::continue::
    end
end)

-- Heartbeat — log každých 10s
CreateThread(function()
    while true do
        Wait(10000)
        log_info(string.format('[blend_space_test] heartbeat handle=%s', tostring(test_handle)))
    end
end)
