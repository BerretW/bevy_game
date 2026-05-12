-- Stairs IK Test - Server Side
-- Vytváří testovací schody přes parametrický dummy spawn

local stairs = nil
local stairs_spawned = false

function RegisterStairs()
    if stairs_spawned then
        return
    end
    stairs_spawned = true

    -- Vytvoří 3 schodiště s neblokujícím STAIRS trigger colliderem.
    -- IK systém pak může detekovat, že je hráč na schodech.
    
    stairs = {}
    
    local total_height = 1.2
    local total_depth = 2.4
    local width = 2.0

    local pos = {
        x = 0.0,
        y = total_height * 0.5,
        z = 0.0,
    }
    local rot = { x = 0.0, y = 0.0, z = 0.0 }

    local handle = World.SpawnNetworkedDummy(
        'stairs',
        {
            size = { x = width, y = total_height, z = total_depth },
            height = total_height,
            steps = 6,
            r = 0.55,
            g = 0.62,
            b = 0.72,
            a = 1.0,
            collider = {
                enabled = true,
                shape = 'box',
                is_static = true,
                is_trigger = true,
                stairs = true,
                size = { x = width, y = total_height, z = total_depth },
            },
        },
        pos,
        rot
    )
    table.insert(stairs, handle)
    
    print("[stairs_test] Stairs created: " .. #stairs .. " steps")
end

RegisterStairs()

RegisterEvent('onServerReady', function()
    print("[stairs_test] Server resource loaded")
    RegisterStairs()
end)

RegisterEvent('onPlayerJoin', function(player_id)
    print("[stairs_test] Player " .. player_id .. " joined")
end)
