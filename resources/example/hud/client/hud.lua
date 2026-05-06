log_info(string.format('[%s] loaded on %s', RESOURCE_ID, SIDE))

-- Periodicky čte HP z nativního Rust API a posílá do NUI.
-- input:state přichází každý frame; throttlujeme na ~10 Hz pomocí counter.
local frame = 0
RegisterEvent('input:state', function(_payload)
    frame = frame + 1
    if frame % 6 ~= 0 then return end  -- ~10 Hz při 60 fps

    local stats = Player.GetLocalStats()
    SendNUIMessage({ type = 'hp_update', hp = stats.hp, max = stats.max_hp })
end)
