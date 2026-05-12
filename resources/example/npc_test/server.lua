local npc_random = nil
local npc_patrol = nil
local npc_orbit = nil
local follow_anchor = nil

local function spawn_npc(model, pos, ped_profile)
    local h
    if ped_profile then
        h = World.SpawnNetworkedNpc(model, pos, { x = 0, y = 0, z = 0 }, ped_profile)
    else
        h = World.SpawnNetworkedNpc(model, pos, { x = 0, y = 0, z = 0 })
    end
    World.NpcConfigure(h, {
        move_speed = 2.2,
        arrive_distance = 0.2,
        turn_speed = 9.0,
    })
    -- Explicitně spustíme animaci pro NPC
    World.PlayAnimation(h, "clip:0", true, 1.0)
    return h
end

local function setup_once()
    if npc_random and World.IsValid(npc_random) then
        return
    end

    -- Anchor entity for follow behavior.
    follow_anchor = World.SpawnNetworkedDummy(
        'sphere',
        {
            radius = 0.25,
            r = 1.0,
            g = 0.35,
            b = 0.2,
            a = 1.0,
            collider = { enabled = false },
        },
        { x = 0.0, y = 1.0, z = 0.0 },
        { x = 0.0, y = 0.0, z = 0.0 }
    )

    npc_random = spawn_npc('zombie', { x = 3.0, y = 0.0, z = 2.0 })
    World.NpcWander(npc_random, 'random', {
        radius = 5.0,
        retarget_sec = 2.0,
    })

    npc_patrol = spawn_npc('player', { x = -5.0, y = 0.0, z = -2.0 }, 'player')
    World.NpcWander(npc_patrol, 'patrol', {
        patrol_point = { x = -1.0, y = 1.0, z = -2.0 },
        radius = 4.0,
        retarget_sec = 1.2,
    })

    npc_orbit = spawn_npc('zombie', { x = 0.0, y = 0.0, z = 6.0 }, 'monster') -- příklad: použij monster.ped.toml
    World.NpcWander(npc_orbit, 'orbit', {
        radius = 3.0,
        orbit_angular_speed = 0.9,
        clockwise = false,
    })

    -- Follow the moving anchor.
    World.NpcGoToEntity(npc_patrol, follow_anchor, 1.5)

    CreateThread(function()
        local t = 0.0
        while true do
            Wait(100)
            t = t + 0.1
            World.SetPosition(follow_anchor, {
                x = math.cos(t) * 5.0,
                y = 1.0,
                z = math.sin(t) * 5.0,
            })
        end
    end)

    -- Sequence: force one NPC to move to explicit coordinates, then resume wander.
    CreateThread(function()
        while true do
            Wait(7000)
            if npc_random and World.IsValid(npc_random) then
                World.NpcGoToCoord(npc_random, { x = 8.0, y = 1.0, z = 0.0 }, 0.3)
            end

            Wait(5000)
            if npc_random and World.IsValid(npc_random) then
                World.NpcWander(npc_random, 'random', { radius = 5.0, retarget_sec = 2.0 })
            end
        end
    end)

    print('[npc_test] spawned NPCs with random/patrol/orbit and follow/coord goals')
end

setup_once()

RegisterEvent('onServerReady', function()
    setup_once()
end)
