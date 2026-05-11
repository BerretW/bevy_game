-- shared/main.lua
--
-- Test script pro:
-- 1) plynulé přepínání klipů (crossfade přes blend_time)
-- 2) runtime event `onAnimNotify` -> LocalEventBus -> Lua sandbox

log_info(string.format('[%s] anim_notify_test loaded on %s', RESOURCE_ID, SIDE))

if SIDE ~= 'client' then
    return
end

local TEST_MODEL = 'mutant'
local TEST_POS = { x = 6.0, y = 1.0, z = 3.0 }
local TEST_ROT = { x = 0.0, y = 180.0, z = 0.0 }
local CLIP_DURATION_MS = 6000   -- musí být delší než nejdelší notify čas v klipu
local BLEND_TIME_SEC = 0.25

local test_handle = nil
local notify_count = 0
local started_clip = nil

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

RegisterEvent('onAnimNotify', function(payload)
    if type(payload) ~= 'table' then
        return
    end

    local handle = tonumber(payload.handle)
    if not handle or handle ~= test_handle then
        return
    end

    notify_count = notify_count + 1
    local clip_name = tostring(payload.clip_name)
    local notify_name = tostring(payload.notify_name)
    log_info(string.format('[anim_notify_test] notify #%d handle=%d clip=%s name=%s', notify_count, handle, clip_name, notify_name))
end)

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

    local clip_names = Engine.GetModelClipNames(TEST_MODEL)
    if not clip_names or #clip_names == 0 then
        log_warn('[anim_notify_test] model has no clips, test aborted')
        return
    end

    log_info(string.format('[anim_notify_test] model=%s handle=%d clips=%d blend=%.2fs', TEST_MODEL, test_handle, #clip_names, BLEND_TIME_SEC))
    log_info('[anim_notify_test] TIP: pro notify test přidej v Blender Action pose markers a exportuj ADM v4')

    local i = 1
    while true do
        local clip = clip_names[i]
        started_clip = clip
        World.PlayAnimation(test_handle, clip, true, 1.0, BLEND_TIME_SEC)
        log_info(string.format('[anim_notify_test] playing clip %d/%d: %s', i, #clip_names, tostring(clip)))

        Wait(CLIP_DURATION_MS)

        i = i + 1
        if i > #clip_names then
            i = 1
        end
    end
end)

CreateThread(function()
    while true do
        Wait(10000)
        log_info(string.format('[anim_notify_test] heartbeat active_clip=%s notify_count=%d', tostring(started_clip), notify_count))
    end
end)
