-- shared/main.lua
-- Test Root Motion — pohyb řízený animací
-- Root Motion extrahuje translaci kořenové kosti a aplikuje ji na entity

log_info(string.format('[%s] root_motion_test loaded on %s', RESOURCE_ID, SIDE))

if SIDE ~= 'client' then
    return
end

local TEST_MODEL = 'mutant'
local TEST_POS = { x = 12.0, y = 1.0, z = 3.0 }
local TEST_ROT = { x = 0.0, y = 0.0, z = 0.0 }
local ANIM_NAME = 'mutant_run'  -- Animace s root motion translací

local test_handle = nil
local current_pos = { x = 12.0, y = 1.0, z = 3.0 }

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
        local clip_count = Engine.GetModelClipCount(TEST_MODEL)
        if loaded and clip_count > 0 then
            break
        end
        attempts = attempts + 1
        Wait(100)
    end

    if not test_handle then
        log_warn('[root_motion_test] failed to spawn test entity')
        return
    end

    -- Spustit animaci s root motion — bez loopingu, aby se entita pohybovala
    World.PlayAnimation(test_handle, ANIM_NAME, false, 1.0, 0.15)
    current_pos = { x = 12.0, y = 1.0, z = 3.0 }

    log_info(string.format('[root_motion_test] model=%s handle=%d anim=%s', TEST_MODEL, test_handle, ANIM_NAME))
    log_info('[root_motion_test] TIP: Root motion translator extrahuje pohyb z animace')
end)

-- Monitoruj pozici entity — měla by se pohybovat podle animace
CreateThread(function()
    while true do
        Wait(2000)

        if not test_handle then
            goto continue
        end

        local new_pos = World.GetPosition(test_handle)
        if new_pos then
            local dx = new_pos.x - current_pos.x
            local dy = new_pos.y - current_pos.y
            local dz = new_pos.z - current_pos.z
            local dist = math.sqrt(dx * dx + dy * dy + dz * dz)

            log_debug(string.format('[root_motion_test] position change: %.3f units (x=%.2f, z=%.2f)', dist, dx, dz))
            current_pos = new_pos
        end

        ::continue::
    end
end)

-- Heartbeat — log každých 10s
CreateThread(function()
    while true do
        Wait(10000)
        log_info(string.format('[root_motion_test] heartbeat handle=%s', tostring(test_handle)))
    end
end)
