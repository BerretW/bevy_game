-- shared/main.lua
-- Test IK (Inverse Kinematics) API
-- Infrastruktura pro Two-Bone IK solver

log_info(string.format('[%s] ik_test loaded on %s', RESOURCE_ID, SIDE))

if SIDE ~= 'client' then
    return
end

local TEST_MODEL = 'mutant'
local TEST_POS = { x = 10.0, y = 1.0, z = 3.0 }
local TEST_ROT = { x = 0.0, y = 180.0, z = 0.0 }

local test_handle = nil

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
        log_warn('[ik_test] failed to spawn test entity')
        return
    end

    log_info(string.format('[ik_test] model=%s handle=%d ready', TEST_MODEL, test_handle))
    log_info('[ik_test] TIP: IK solver bude aktivní jakmile bude implementován runtime')
end)

-- Heartbeat — log každých 10s
CreateThread(function()
    while true do
        Wait(10000)
        log_info(string.format('[ik_test] heartbeat handle=%s', tostring(test_handle)))
    end
end)
