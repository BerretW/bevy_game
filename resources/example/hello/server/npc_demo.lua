local AGGRESSIVE_ZOMBIE_BRAIN_ID = 'example/hello_aggressive_zombie'

local zombie_chaser = nil
local zombie_scout = nil
local town_guard = nil
local lure_anchor = nil

local function register_custom_brains()
    World.NpcRegisterBrain(AGGRESSIVE_ZOMBIE_BRAIN_ID, {
        label = 'Hello Aggressive Zombie',
        kind = 'human',
        locomotion = 'biped',
        default_task = 'chase_target',
        allowed_tasks = {
            'idle',
            'wander_zone',
            'investigate',
            'chase_target',
            'combat',
        },
        perception = {
            sight_range = 42.0,
            hearing_range = 24.0,
            alert_range = 28.0,
        },
        motion = {
            cruise_speed = 2.8,
            sprint_speed = 5.8,
            turn_speed = 7.0,
            brake_distance = 0.75,
        },
        navigation = {
            use_navmesh = true,
            use_avoidance = true,
            terrain_snap = true,
            repath_interval_sec = 0.2,
            target_repath_delta = 0.35,
        },
        scenario_tags = {
            'zombie',
            'aggressive',
            'infected',
        },
    })
end

local function spawn_npc(model, pos, ped_profile)
    local handle
    if ped_profile then
        handle = World.SpawnNetworkedNpc(model, pos, { x = 0, y = 0, z = 0 }, ped_profile)
    else
        handle = World.SpawnNetworkedNpc(model, pos, { x = 0, y = 0, z = 0 })
    end

    World.PlayAnimation(handle, 'clip:0', true, 1.0)
    return handle
end

local function ensure_setup()
    if zombie_chaser and World.IsValid(zombie_chaser) then
        return
    end

    register_custom_brains()

    lure_anchor = World.SpawnNetworkedDummy(
        'sphere',
        {
            radius = 0.22,
            r = 1.0,
            g = 0.15,
            b = 0.1,
            a = 1.0,
            collider = { enabled = false },
        },
        { x = 5.0, y = 1.0, z = -1.5 },
        { x = 0.0, y = 0.0, z = 0.0 }
    )

    zombie_chaser = spawn_npc('zombie', { x = 1.5, y = 0.0, z = -1.5 }, 'monster')
    World.NpcSetBrain(zombie_chaser, AGGRESSIVE_ZOMBIE_BRAIN_ID)
    World.NpcConfigure(zombie_chaser, {
        move_speed = 2.8,
        arrive_distance = 0.8,
        turn_speed = 7.0,
    })
    World.NpcSetTask(zombie_chaser, 'chase_target', {
        scenario_id = 'hello/hunt_lure',
        target_handle = lure_anchor,
        stop_distance = 0.8,
        combat_range = 1.4,
        leash_radius = 18.0,
    })

    zombie_scout = spawn_npc('zombie', { x = -2.5, y = 0.0, z = 2.5 }, 'monster')
    World.NpcSetBrain(zombie_scout, AGGRESSIVE_ZOMBIE_BRAIN_ID)
    World.NpcSetTask(zombie_scout, 'wander_zone', {
        scenario_id = 'hello/zombie_perimeter',
        radius = 6.0,
        retarget_sec = 1.4,
        wander_kind = 'random',
    })

    town_guard = spawn_npc('player', { x = -6.0, y = 0.0, z = -3.0 }, 'player')
    World.NpcConfigure(town_guard, {
        move_speed = 2.0,
        arrive_distance = 0.2,
        turn_speed = 9.0,
    })
    World.NpcSetScenario(town_guard, 'hello/watch_post', {
        target_pos = { x = -3.0, y = 0.0, z = -3.0 },
        stop_distance = 0.25,
        idle_clip = 'clip:0',
        facing_yaw = 90.0,
    })

    CreateThread(function()
        local t = 0.0
        while true do
            Wait(120)
            t = t + 0.12
            if lure_anchor and World.IsValid(lure_anchor) then
                World.SetPosition(lure_anchor, {
                    x = 5.0 + math.cos(t) * 3.5,
                    y = 1.0,
                    z = -1.5 + math.sin(t) * 2.0,
                })
            end
        end
    end)

    CreateThread(function()
        while true do
            Wait(9000)
            if zombie_scout and World.IsValid(zombie_scout) then
                World.NpcSetTask(zombie_scout, 'investigate', {
                    scenario_id = 'hello/noise_probe',
                    target_pos = { x = 2.0, y = 0.0, z = 4.0 },
                    stop_distance = 0.5,
                    investigate_hold_sec = 2.0,
                })
            end

            Wait(4000)
            if zombie_scout and World.IsValid(zombie_scout) then
                World.NpcSetTask(zombie_scout, 'wander_zone', {
                    scenario_id = 'hello/zombie_perimeter',
                    radius = 6.0,
                    retarget_sec = 1.4,
                    wander_kind = 'random',
                })
            end
        end
    end)

    log_info('[hello] spawned NPC demo with replicated task/scenario state and custom aggressive zombie brain')
end

ensure_setup()

RegisterEvent('onServerReady', function()
    ensure_setup()
end)