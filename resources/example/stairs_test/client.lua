-- Stairs IK Test - Client Side
-- Monitoruje IK stav hráče při chůzi po schodech

local player_handle = nil
local on_stairs = false
local left_foot_height = 0.0
local right_foot_height = 0.0

function UpdateMonitoring()
    -- Monitoruj stav hráče
    if not player_handle then
        return
    end
    
    -- Kontrola zda je hráč na schodech
    -- V budoucnu: přečíst OnStairs komponent hodnoty
    -- Prozatím jen logujeme stav
end

RegisterEvent('input:state', function(state)
    -- State obsahuje input data
    if state.move and (state.move.x ~= 0 or state.move.y ~= 0) then
        -- Hráč se pohybuje - monitoruj IK
        UpdateMonitoring()
    end
end)

RegisterEvent('playerConnecting', function()
    print("[stairs_test] Player connecting")
end)

RegisterEvent('onPlayerSpawn', function(player_data)
    print("[stairs_test] Player spawned, loading IK system")
    
    -- Pozitiv player handle pro budoucí operace
    if player_data and player_data.handle then
        player_handle = player_data.handle
    end
end)

print("[stairs_test] Client resource loaded")
