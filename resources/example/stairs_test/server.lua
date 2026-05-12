-- Stairs IK Test - Server Side
-- Vytvořá testovací schody objekty

local stairs = nil

function RegisterStairs()
    -- Vytvořit 3-stupňová schodiště
    -- Každý schod je 0.3m vysoký a 0.4m hluboko
    
    stairs = {}
    
    for i = 1, 3 do
        local height = i * 0.3
        local depth = -i * 0.4
        
        local pos = Vec3.new(0, height, depth)
        local rot = Quat.identity()
        
        -- Vytvoří collider schodů s STAIRS materiálem
        -- Tento objekt by měl mít DrawableCollision s material='STAIRS'
        -- V manifestu .drawable by mělo být: [[entities]] type="COLLISION", shape="BOX", material="STAIRS"
        
        local handle = World.SpawnNetworkedObject("stairs_step", pos, rot)
        table.insert(stairs, handle)
    end
    
    print("[stairs_test] Stairs created: " .. #stairs .. " steps")
end

RegisterEvent('onServerReady', function()
    print("[stairs_test] Server resource loaded")
    RegisterStairs()
end)

RegisterEvent('onPlayerJoin', function(player_id)
    print("[stairs_test] Player " .. player_id .. " joined")
end)
