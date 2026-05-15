assert(IS_SERVER, 'arena_mvp server script must run on server side')

local ARENA_ZOMBIE_BRAIN_ID = 'arena_mvp/zombie_brain'

local players = {}
local tracked_players = {}
local team_scores = { alpha = 0, bravo = 0 }
local respawn_pending = {}
local world_handles = {}
local spawn_cycle = { alpha = 0, bravo = 0 }
local zombie_cycle = 0
local zombie_bots = {}
local zombie_contact_clock_ms = 0
local ammo_station_cooldowns = {}

local function make_vec3(pos)
    return {
        x = tonumber(pos.x) or 0.0,
        y = tonumber(pos.y) or 0.0,
        z = tonumber(pos.z) or 0.0,
    }
end

local function make_rot(rot)
    return {
        x = tonumber(rot.x) or 0.0,
        y = tonumber(rot.y) or 0.0,
        z = tonumber(rot.z) or 0.0,
    }
end

local function distance_sq(a, b)
    local dx = (tonumber(a.x) or 0.0) - (tonumber(b.x) or 0.0)
    local dy = (tonumber(a.y) or 0.0) - (tonumber(b.y) or 0.0)
    local dz = (tonumber(a.z) or 0.0) - (tonumber(b.z) or 0.0)
    return dx * dx + dy * dy + dz * dz
end

local function count_team(team)
    local count = 0
    for _, state in pairs(players) do
        if state.team == team then
            count = count + 1
        end
    end
    return count
end

local function extract_player_id(value)
    if type(value) == 'table' then
        return ArenaNormalizePlayerId(value.id or value.player_id or value.client_id)
    end
    return ArenaNormalizePlayerId(value)
end

local function count_humans()
    local count = 0
    for _ in pairs(players) do
        count = count + 1
    end
    return count
end

local function count_live_zombies()
    local count = 0
    for _, zombie in ipairs(zombie_bots) do
        if zombie.handle and World.IsValid(zombie.handle) then
            count = count + 1
        end
    end
    return count
end

local function choose_team()
    local alpha_count = count_team('alpha')
    local bravo_count = count_team('bravo')
    if alpha_count <= bravo_count then
        return 'alpha'
    end
    return 'bravo'
end

local function build_player_payload()
    local payload = {}
    for player_id, state in pairs(players) do
        payload[player_id] = {
            team = state.team,
            kills = state.kills,
            deaths = state.deaths,
            alive = state.alive == true,
        }
    end
    return payload
end

local function broadcast_state(target)
    TriggerClientEvent('arena:state', target, {
        scores = {
            alpha = team_scores.alpha,
            bravo = team_scores.bravo,
        },
        players = build_player_payload(),
        score_limit = ArenaConfig.score_limit,
        respawn_delay_ms = ArenaConfig.respawn_delay_ms,
        humans = count_humans(),
        zombies = count_live_zombies(),
        slots = ArenaConfig.max_slots,
    })
end

local function next_spawn(team)
    local list = ArenaConfig.spawns[team] or ArenaConfig.spawns.alpha
    if #list == 0 then
        return {
            pos = { x = 0.0, y = 1.25, z = 0.0 },
            rot = { x = 0.0, y = 0.0, z = 0.0 },
        }
    end
    spawn_cycle[team] = ((spawn_cycle[team] or 0) % #list) + 1
    local selected = list[spawn_cycle[team]]
    return {
        pos = make_vec3(selected.pos),
        rot = make_rot(selected.rot),
    }
end

local function next_zombie_spawn()
    local list = ArenaConfig.zombie_spawns or {}
    if #list == 0 then
        return { x = 0.0, y = 1.25, z = 0.0 }
    end
    zombie_cycle = (zombie_cycle % #list) + 1
    return make_vec3(list[zombie_cycle])
end

local function register_weapons()
    Ammo.Register('arena_556', {
        display_name = '5.56 Arena Ball',
        caliber = '5.56x45',
        muzzle_velocity_mps = 890.0,
        ballistic_model = 'G7',
        ballistic_coeff = 0.205,
        base_damage = 28.0,
        damage_velocity_ref_mps = 890.0,
        penetration_class = 2,
        armor_penetration = 0.18,
        crack_range_m = 500.0,
        thump_range_m = 1100.0,
    })

    Ammo.Register('arena_9mm', {
        display_name = '9mm Arena Ball',
        caliber = '9x19',
        muzzle_velocity_mps = 365.0,
        ballistic_model = 'G1',
        ballistic_coeff = 0.145,
        base_damage = 19.0,
        damage_velocity_ref_mps = 365.0,
        penetration_class = 1,
        armor_penetration = 0.08,
        crack_range_m = 120.0,
        thump_range_m = 600.0,
    })

    Weapon.Register('arena_rifle', {
        display_name = 'MR-21',
        category = 'rifle',
        caliber = '5.56x45',
        default_ammo = 'arena_556',
        fire_modes = { 'semi', 'full' },
        default_fire_mode = 'full',
        rpm = 660.0,
        mag_capacity = 30,
        reload_empty_sec = 2.4,
        reload_tactical_sec = 2.0,
        ads_fov_mult = 0.84,
        ads_time_sec = 0.14,
        spread = {
            base = 0.12,
            moving = 0.30,
            sprinting = 0.75,
            crouch = 0.09,
            prone = 0.07,
            ads = 0.05,
            ads_moving = 0.12,
            per_shot = 0.04,
            recovery_rps = 4.5,
        },
    })

    Weapon.Register('arena_pistol', {
        display_name = 'PX-9',
        category = 'pistol',
        caliber = '9x19',
        default_ammo = 'arena_9mm',
        fire_modes = { 'semi' },
        default_fire_mode = 'semi',
        rpm = 320.0,
        mag_capacity = 12,
        reload_empty_sec = 1.8,
        reload_tactical_sec = 1.5,
        ads_fov_mult = 0.92,
        ads_time_sec = 0.10,
        spread = {
            base = 0.10,
            moving = 0.24,
            sprinting = 0.60,
            crouch = 0.08,
            prone = 0.06,
            ads = 0.04,
            ads_moving = 0.10,
            per_shot = 0.03,
            recovery_rps = 5.5,
        },
    })
end

local function equip_player(player_id)
    Weapon.SetEquipped(player_id, 0, {
        weapon_id = 'arena_rifle',
        ammo_in_mag = 30,
        ammo_type_id = 'arena_556',
        fire_mode = 'full',
        attachments = nil,
    })
    Weapon.SetEquipped(player_id, 1, {
        weapon_id = 'arena_pistol',
        ammo_in_mag = 12,
        ammo_type_id = 'arena_9mm',
        fire_mode = 'semi',
        attachments = nil,
    })
    Weapon.SetEquipped(player_id, 2, nil)
    Weapon.SetEquipped(player_id, 3, nil)
    Weapon.SetAmmoReserve(player_id, 'arena_556', 90)
    Weapon.SetAmmoReserve(player_id, 'arena_9mm', 48)
    Weapon.SetActiveSlot(player_id, 0)
    Player.SetArmor(player_id, 'helmet', nil)
    Player.SetArmor(player_id, 'vest', nil)
    Player.SetHealth(player_id, 100.0, 100.0)
end

local function send_spawn(player_id, reason)
    local state = players[player_id]
    if not state then
        return
    end

    local spawn = next_spawn(state.team)
    state.alive = true
    tracked_players[player_id] = make_vec3(spawn.pos)
    equip_player(player_id)

    TriggerClientEvent('arena:respawn', player_id, {
        team = state.team,
        reason = reason or 'respawn',
        position = spawn.pos,
        rotation = spawn.rot,
    })
    broadcast_state(nil)
end

local function schedule_spawn(player_id, delay_ms, reason)
    if respawn_pending[player_id] then
        return
    end
    respawn_pending[player_id] = true

    local state = players[player_id]
    if state then
        state.alive = false
        TriggerClientEvent('arena:respawn_timer', player_id, {
            team = state.team,
            respawn_ms = delay_ms,
        })
    end
    broadcast_state(nil)

    CreateThread(function()
        Wait(delay_ms)
        respawn_pending[player_id] = nil
        if players[player_id] then
            send_spawn(player_id, reason)
        end
    end)
end

local function spawn_block(def)
    local handle = World.SpawnNetworkedDummy(
        def.shape or 'cuboid',
        {
            size = { x = def.size.x, y = def.size.y, z = def.size.z },
            r = def.color[1],
            g = def.color[2],
            b = def.color[3],
            a = def.color[4],
            collider = {
                enabled = def.collider ~= false,
                shape = 'box',
                is_static = true,
                size = { x = def.size.x, y = def.size.y, z = def.size.z },
            },
        },
        def.pos,
        def.rot or { x = 0.0, y = 0.0, z = 0.0 }
    )
    world_handles[#world_handles + 1] = handle
end

local function spawn_marker(def)
    local handle = World.SpawnNetworkedDummy(
        'cuboid',
        {
            size = { x = def.size.x, y = def.size.y, z = def.size.z },
            r = def.color[1],
            g = def.color[2],
            b = def.color[3],
            a = def.color[4],
            collider = {
                enabled = false,
            },
        },
        def.pos,
        def.rot or { x = 0.0, y = 0.0, z = 0.0 }
    )
    world_handles[#world_handles + 1] = handle
end

local function spawn_ramp(def)
    local handle = World.SpawnNetworkedDummy(
        'stairs',
        {
            size = { x = def.size.x, y = def.size.y, z = def.size.z },
            height = def.size.y,
            steps = def.steps or 5,
            r = def.color[1],
            g = def.color[2],
            b = def.color[3],
            a = def.color[4],
            collider = {
                enabled = true,
                shape = 'box',
                is_static = true,
                size = { x = def.size.x, y = def.size.y, z = def.size.z },
            },
        },
        def.pos,
        def.rot or { x = 0.0, y = 0.0, z = 0.0 }
    )
    world_handles[#world_handles + 1] = handle
end

local function spawn_ammo_cache(def)
    spawn_block({
        pos = { x = def.pos.x, y = 0.45, z = def.pos.z },
        size = { x = 2.6, y = 0.9, z = 2.6 },
        color = def.color,
        collider = false,
    })
    local glow_color = { 1.0, 0.48, 0.14, 0.96 }
    if def.color and def.color[1] and def.color[1] > 0.85 and def.color[2] > 0.65 then
        glow_color = { 1.0, 0.74, 0.18, 0.96 }
    end
    local handle = World.SpawnNetworkedDummy(
        'sphere',
        {
            radius = 0.85,
            r = glow_color[1],
            g = glow_color[2],
            b = glow_color[3],
            a = glow_color[4],
            collider = {
                enabled = false,
            },
        },
        { x = def.pos.x, y = 1.75, z = def.pos.z },
        { x = 0.0, y = 0.0, z = 0.0 }
    )
    world_handles[#world_handles + 1] = handle
    spawn_marker({
        pos = { x = def.pos.x, y = 2.75, z = def.pos.z },
        size = { x = 0.55, y = 2.8, z = 0.55 },
        color = { 1.0, 0.62, 0.12, 0.68 },
    })
end

local function build_arena()
    if #world_handles > 0 then
        return
    end

    spawn_block({ pos = { x = 0.0, y = -1.0, z = 0.0 }, size = { x = 84.0, y = 2.0, z = 56.0 }, color = { 0.57, 0.50, 0.37, 1.0 } })

    spawn_block({ pos = { x = 0.0, y = 4.0, z = -28.5 }, size = { x = 84.0, y = 10.0, z = 1.0 }, color = { 0.68, 0.58, 0.41, 1.0 } })
    spawn_block({ pos = { x = 0.0, y = 4.0, z = 28.5 }, size = { x = 84.0, y = 10.0, z = 1.0 }, color = { 0.68, 0.58, 0.41, 1.0 } })
    spawn_block({ pos = { x = -42.5, y = 4.0, z = 0.0 }, size = { x = 1.0, y = 10.0, z = 56.0 }, color = { 0.68, 0.58, 0.41, 1.0 } })
    spawn_block({ pos = { x = 42.5, y = 4.0, z = 0.0 }, size = { x = 1.0, y = 10.0, z = 56.0 }, color = { 0.68, 0.58, 0.41, 1.0 } })

    spawn_block({ pos = { x = -24.0, y = 2.2, z = 20.0 }, size = { x = 20.0, y = 4.4, z = 12.0 }, color = { 0.74, 0.65, 0.49, 1.0 } })
    spawn_block({ pos = { x = -12.0, y = 2.0, z = 23.0 }, size = { x = 4.0, y = 4.0, z = 6.0 }, color = { 0.60, 0.51, 0.39, 1.0 } })
    spawn_block({ pos = { x = -6.0, y = 1.2, z = 20.0 }, size = { x = 6.0, y = 2.4, z = 6.0 }, color = { 0.73, 0.63, 0.47, 1.0 } })
    spawn_marker({ pos = { x = -32.0, y = 0.25, z = 20.0 }, size = { x = 5.0, y = 0.5, z = 5.0 }, color = { 0.16, 0.46, 0.95, 0.88 } })

    spawn_block({ pos = { x = 24.0, y = 2.2, z = -20.0 }, size = { x = 20.0, y = 4.4, z = 12.0 }, color = { 0.74, 0.65, 0.49, 1.0 } })
    spawn_block({ pos = { x = 12.0, y = 2.0, z = -23.0 }, size = { x = 4.0, y = 4.0, z = 6.0 }, color = { 0.60, 0.51, 0.39, 1.0 } })
    spawn_block({ pos = { x = 6.0, y = 1.2, z = -20.0 }, size = { x = 6.0, y = 2.4, z = 6.0 }, color = { 0.73, 0.63, 0.47, 1.0 } })
    spawn_marker({ pos = { x = 32.0, y = 0.25, z = -20.0 }, size = { x = 5.0, y = 0.5, z = 5.0 }, color = { 0.95, 0.32, 0.18, 0.88 } })

    spawn_block({ pos = { x = -20.0, y = 2.0, z = 9.0 }, size = { x = 24.0, y = 4.0, z = 2.0 }, color = { 0.70, 0.60, 0.45, 1.0 } })
    spawn_block({ pos = { x = -8.0, y = 2.0, z = 12.0 }, size = { x = 6.0, y = 4.0, z = 8.0 }, color = { 0.60, 0.51, 0.39, 1.0 } })
    spawn_block({ pos = { x = -2.5, y = 2.4, z = 15.0 }, size = { x = 3.0, y = 4.8, z = 4.0 }, color = { 0.54, 0.46, 0.35, 1.0 } })

    spawn_block({ pos = { x = 20.0, y = 2.0, z = -9.0 }, size = { x = 24.0, y = 4.0, z = 2.0 }, color = { 0.70, 0.60, 0.45, 1.0 } })
    spawn_block({ pos = { x = 8.0, y = 2.0, z = -12.0 }, size = { x = 6.0, y = 4.0, z = 8.0 }, color = { 0.60, 0.51, 0.39, 1.0 } })
    spawn_block({ pos = { x = 2.5, y = 2.4, z = -15.0 }, size = { x = 3.0, y = 4.8, z = 4.0 }, color = { 0.54, 0.46, 0.35, 1.0 } })

    spawn_block({ pos = { x = -14.0, y = 2.3, z = -8.0 }, size = { x = 12.0, y = 4.6, z = 2.0 }, color = { 0.66, 0.57, 0.42, 1.0 } })
    spawn_block({ pos = { x = 14.0, y = 2.3, z = 8.0 }, size = { x = 12.0, y = 4.6, z = 2.0 }, color = { 0.66, 0.57, 0.42, 1.0 } })

    spawn_block({ pos = { x = -4.6, y = 2.5, z = 0.0 }, size = { x = 2.0, y = 5.0, z = 6.0 }, color = { 0.55, 0.47, 0.36, 1.0 } })
    spawn_block({ pos = { x = 4.6, y = 2.5, z = 0.0 }, size = { x = 2.0, y = 5.0, z = 6.0 }, color = { 0.55, 0.47, 0.36, 1.0 } })
    spawn_block({ pos = { x = 0.0, y = 4.7, z = 0.0 }, size = { x = 11.2, y = 1.2, z = 6.0 }, color = { 0.63, 0.54, 0.40, 1.0 } })
    spawn_marker({ pos = { x = 0.0, y = 0.25, z = 0.0 }, size = { x = 1.0, y = 0.5, z = 10.0 }, color = { 0.96, 0.88, 0.32, 0.74 } })

    spawn_ramp({ pos = { x = -8.0, y = 1.0, z = 14.0 }, size = { x = 5.0, y = 2.0, z = 8.0 }, steps = 5, color = { 0.66, 0.58, 0.43, 1.0 }, rot = { x = 0.0, y = 180.0, z = 0.0 } })
    spawn_ramp({ pos = { x = 8.0, y = 1.0, z = -14.0 }, size = { x = 5.0, y = 2.0, z = 8.0 }, steps = 5, color = { 0.66, 0.58, 0.43, 1.0 } })

    spawn_block({ pos = { x = -28.0, y = 2.0, z = 4.0 }, size = { x = 4.0, y = 4.0, z = 10.0 }, color = { 0.61, 0.53, 0.40, 1.0 } })
    spawn_block({ pos = { x = 28.0, y = 2.0, z = -4.0 }, size = { x = 4.0, y = 4.0, z = 10.0 }, color = { 0.61, 0.53, 0.40, 1.0 } })
    spawn_block({ pos = { x = -18.0, y = 1.2, z = -18.0 }, size = { x = 14.0, y = 2.4, z = 4.0 }, color = { 0.73, 0.63, 0.47, 1.0 } })
    spawn_block({ pos = { x = 18.0, y = 1.2, z = 18.0 }, size = { x = 14.0, y = 2.4, z = 4.0 }, color = { 0.73, 0.63, 0.47, 1.0 } })

    for _, station in ipairs(ArenaConfig.ammo.stations or {}) do
        spawn_ammo_cache(station)
    end
end

local function refill_ammo_from_station(player_id, station)
    local rifle_target = math.max(0, math.floor(tonumber(station.rifle) or 0))
    local pistol_target = math.max(0, math.floor(tonumber(station.pistol) or 0))
    local current_rifle = tonumber(Weapon.GetAmmoReserve(player_id, 'arena_556')) or 0
    local current_pistol = tonumber(Weapon.GetAmmoReserve(player_id, 'arena_9mm')) or 0
    local changed = false

    if current_rifle < rifle_target then
        Weapon.SetAmmoReserve(player_id, 'arena_556', rifle_target)
        changed = true
    end
    if current_pistol < pistol_target then
        Weapon.SetAmmoReserve(player_id, 'arena_9mm', pistol_target)
        changed = true
    end

    if changed then
        TriggerClientEvent('arena:announcement', player_id, {
            text = string.format('%s restocked ammo.', tostring(station.label or 'Ammo cache')),
            duration_ms = 1600,
        })
    end
end

local function update_ammo_station_loop()
    CreateThread(function()
        while true do
            Wait(200)
            for player_id, state in pairs(players) do
                if state.alive == true and not respawn_pending[player_id] then
                    local player_pos = tracked_players[player_id]
                    if player_pos then
                        local cooldowns = ammo_station_cooldowns[player_id]
                        if not cooldowns then
                            cooldowns = {}
                            ammo_station_cooldowns[player_id] = cooldowns
                        end

                        for _, station in ipairs(ArenaConfig.ammo.stations or {}) do
                            local radius = tonumber(station.radius) or 2.0
                            local radius_sq = radius * radius
                            local expires_at = cooldowns[station.id] or 0
                            if zombie_contact_clock_ms >= expires_at and distance_sq(player_pos, station.pos) <= radius_sq then
                                refill_ammo_from_station(player_id, station)
                                cooldowns[station.id] = zombie_contact_clock_ms + (ArenaConfig.ammo.refill_cooldown_ms or 5000)
                            end
                        end
                    end
                end
            end
        end
    end)
end

local function ensure_player(player_id)
    local state = players[player_id]
    if state then
        return state
    end

    state = {
        team = choose_team(),
        kills = 0,
        deaths = 0,
        alive = false,
    }
    players[player_id] = state
    return state
end

local function announce_score_if_needed()
    local winner = nil
    if team_scores.alpha >= ArenaConfig.score_limit then
        winner = 'alpha'
    elseif team_scores.bravo >= ArenaConfig.score_limit then
        winner = 'bravo'
    end

    if not winner then
        return
    end

    TriggerClientEvent('arena:announcement', nil, {
        text = string.format('%s reached %d frags. Scoreboard reset.', ArenaTeamLabel(winner), ArenaConfig.score_limit),
    })
    team_scores.alpha = 0
    team_scores.bravo = 0
    for _, state in pairs(players) do
        state.kills = 0
        state.deaths = 0
    end
end

local function register_zombie_brain()
    World.NpcRegisterBrain(ARENA_ZOMBIE_BRAIN_ID, {
        label = 'Arena Zombie',
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
            sight_range = 38.0,
            hearing_range = 16.0,
            alert_range = 20.0,
        },
        motion = {
            cruise_speed = ArenaConfig.zombie.move_speed,
            sprint_speed = ArenaConfig.zombie.move_speed + 1.4,
            turn_speed = ArenaConfig.zombie.turn_speed,
            brake_distance = 0.65,
        },
        navigation = {
            use_navmesh = false,
            use_avoidance = true,
            terrain_snap = true,
            repath_interval_sec = 0.2,
            target_repath_delta = 0.35,
        },
        scenario_tags = {
            'zombie',
            'arena',
        },
    })
end

local function configure_zombie(handle)
    World.NpcSetBrain(handle, ARENA_ZOMBIE_BRAIN_ID)
    World.NpcConfigure(handle, {
        move_speed = ArenaConfig.zombie.move_speed,
        arrive_distance = ArenaConfig.zombie.arrive_distance,
        turn_speed = ArenaConfig.zombie.turn_speed,
    })
end

local function make_zombie(index)
    local spawn = next_zombie_spawn()
    local handle = World.SpawnNetworkedNpc('zombie', spawn, { x = 0.0, y = 0.0, z = 0.0 }, 'monster')
    World.PlayAnimation(handle, 'clip:0', true, 1.0)
    configure_zombie(handle)
    local zombie = {
        id = index,
        handle = handle,
        home = spawn,
        target_player_id = nil,
        last_attack_ms = -ArenaConfig.zombie.attack_cooldown_ms,
    }
    World.NpcSetTask(handle, 'wander_zone', {
        target_pos = spawn,
        radius = ArenaConfig.zombie.wander_radius,
        retarget_sec = 1.0,
        wander_kind = 'random',
    })
    return zombie
end

local function trim_zombies(target_count)
    while #zombie_bots > target_count do
        local zombie = table.remove(zombie_bots)
        if zombie and zombie.handle and World.IsValid(zombie.handle) then
            World.DeleteObject(zombie.handle)
        end
    end
end

local function rebalance_zombies()
    local target = math.max(0, math.min(ArenaConfig.max_slots, ArenaConfig.zombie.target_bot_count) - count_humans())
    trim_zombies(target)
    while #zombie_bots < target do
        zombie_bots[#zombie_bots + 1] = make_zombie(#zombie_bots + 1)
    end
    broadcast_state(nil)
end

local function best_target_for_zombie(zombie)
    local best_id = nil
    local best_pos = nil
    local best_dist_sq = nil
    local origin = zombie.handle and World.IsValid(zombie.handle) and World.GetPosition(zombie.handle) or zombie.home
    if not origin then
        origin = zombie.home
    end

    for player_id, state in pairs(players) do
        if state.alive == true and not respawn_pending[player_id] then
            local pos = tracked_players[player_id]
            if pos then
                local d = distance_sq(origin, pos)
                if not best_dist_sq or d < best_dist_sq then
                    best_dist_sq = d
                    best_id = player_id
                    best_pos = pos
                end
            end
        end
    end

    return best_id, best_pos
end

local function kill_player_from_zombie(player_id)
    local state = players[player_id]
    if not state or state.alive ~= true then
        return
    end

    state.deaths = state.deaths + 1
    state.alive = false
    Player.SetHealth(player_id, 0.0, 100.0)
    TriggerClientEvent('arena:announcement', player_id, {
        text = 'Zombie got you. Respawning...',
        duration_ms = 1800,
    })
    broadcast_state(nil)
    schedule_spawn(player_id, ArenaConfig.respawn_delay_ms, 'zombie')
end

local function update_zombie_targets_loop()
    CreateThread(function()
        while true do
            Wait(ArenaConfig.zombie.retarget_interval_ms)
            for _, zombie in ipairs(zombie_bots) do
                if zombie.handle and World.IsValid(zombie.handle) then
                    local target_id, target_pos = best_target_for_zombie(zombie)
                    zombie.target_player_id = target_id
                    if target_pos then
                        World.NpcSetTask(zombie.handle, 'chase_target', {
                            target_pos = make_vec3(target_pos),
                            stop_distance = 0.9,
                            combat_range = 1.1,
                            leash_radius = 18.0,
                        })
                    else
                        World.NpcSetTask(zombie.handle, 'wander_zone', {
                            target_pos = zombie.home,
                            radius = ArenaConfig.zombie.wander_radius,
                            retarget_sec = 1.0,
                            wander_kind = 'random',
                        })
                    end
                end
            end
        end
    end)
end

local function update_zombie_contact_loop()
    CreateThread(function()
        while true do
            Wait(150)
            zombie_contact_clock_ms = zombie_contact_clock_ms + 150
            local now = zombie_contact_clock_ms
            local max_dist_sq = ArenaConfig.zombie.contact_radius * ArenaConfig.zombie.contact_radius
            for _, zombie in ipairs(zombie_bots) do
                if zombie.handle and World.IsValid(zombie.handle) then
                    local zombie_pos = World.GetPosition(zombie.handle)
                    if zombie_pos then
                        for player_id, state in pairs(players) do
                            if state.alive == true then
                                local player_pos = tracked_players[player_id]
                                if player_pos and distance_sq(zombie_pos, player_pos) <= max_dist_sq then
                                    if now - zombie.last_attack_ms >= ArenaConfig.zombie.attack_cooldown_ms then
                                        zombie.last_attack_ms = now
                                        local hp = tonumber(Player.GetHealth(player_id)) or 100.0
                                        local new_hp = hp - ArenaConfig.zombie.contact_damage
                                        if new_hp <= 0.0 then
                                            kill_player_from_zombie(player_id)
                                        else
                                            Player.SetHealth(player_id, new_hp, 100.0)
                                        end
                                        break
                                    end
                                end
                            end
                        end
                    end
                end
            end
        end
    end)
end

build_arena()
register_weapons()
register_zombie_brain()
rebalance_zombies()
broadcast_state(nil)
update_zombie_targets_loop()
update_zombie_contact_loop()
update_ammo_station_loop()

RegisterEvent('onPlayerJoin', function(payload)
    local id = extract_player_id(payload)
    if not id then
        return
    end

    local state = ensure_player(id)
    TriggerClientEvent('arena:announcement', id, {
        text = string.format('Assigned to %s. Rifle on 1, pistol on 2. Refill ammo at your spawn cache or mid doors.', ArenaTeamLabel(state.team)),
    })
    rebalance_zombies()
    broadcast_state(nil)

    CreateThread(function()
        Wait(ArenaConfig.spawn_delay_join_ms)
        if players[id] then
            send_spawn(id, 'join')
        end
    end)
end)

RegisterEvent('playerDropped', function(payload)
    local id = payload
    if type(payload) == 'table' then
        id = payload.id
    end
    id = ArenaNormalizePlayerId(id)
    if not id then
        return
    end

    players[id] = nil
    tracked_players[id] = nil
    respawn_pending[id] = nil
    ammo_station_cooldowns[id] = nil
    rebalance_zombies()
    broadcast_state(nil)
end)

RegisterEvent('onPlayerPosition', function(payload)
    if type(payload) ~= 'table' or type(payload.players) ~= 'table' then
        return
    end

    for _, entry in ipairs(payload.players) do
        local player_id = ArenaNormalizePlayerId(entry.id)
        if player_id then
            tracked_players[player_id] = {
                x = tonumber(entry.x) or 0.0,
                y = 1.25,
                z = tonumber(entry.z) or 0.0,
            }
        end
    end
end)

RegisterEvent('onPlayerDeath', function(payload)
    if type(payload) ~= 'table' then
        return
    end

    local victim_id = ArenaNormalizePlayerId(payload.victim)
    local killer_id = ArenaNormalizePlayerId(payload.killer)
    local victim = victim_id and players[victim_id] or nil
    local killer = killer_id and players[killer_id] or nil

    if victim then
        victim.deaths = victim.deaths + 1
        victim.alive = false
    end

    if killer and victim and killer_id ~= victim_id then
        killer.kills = killer.kills + 1
        if killer.team ~= victim.team then
            team_scores[killer.team] = (team_scores[killer.team] or 0) + 1
        end
    end

    announce_score_if_needed()
    broadcast_state(nil)

    if victim_id then
        schedule_spawn(victim_id, ArenaConfig.respawn_delay_ms, 'death')
    end
end)
