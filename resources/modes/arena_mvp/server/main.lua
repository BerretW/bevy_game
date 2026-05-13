assert(IS_SERVER, 'arena_mvp server script must run on server side')

local players = {}
local team_scores = { alpha = 0, bravo = 0 }
local respawn_pending = {}
local world_handles = {}
local spawn_cycle = { alpha = 0, bravo = 0 }

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

local function count_team(team)
    local count = 0
    for _, state in pairs(players) do
        if state.team == team then
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
        attachments = {},
    })
    Weapon.SetEquipped(player_id, 1, {
        weapon_id = 'arena_pistol',
        ammo_in_mag = 12,
        ammo_type_id = 'arena_9mm',
        fire_mode = 'semi',
        attachments = {},
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

local function build_arena()
    if #world_handles > 0 then
        return
    end

    spawn_block({
        pos = { x = 0.0, y = -0.75, z = 0.0 },
        size = { x = 28.0, y = 1.5, z = 28.0 },
        color = { 0.22, 0.24, 0.28, 1.0 },
    })

    spawn_block({ pos = { x = 0.0, y = 2.2, z = -14.5 }, size = { x = 28.0, y = 6.0, z = 1.0 }, color = { 0.18, 0.18, 0.20, 1.0 } })
    spawn_block({ pos = { x = 0.0, y = 2.2, z = 14.5 }, size = { x = 28.0, y = 6.0, z = 1.0 }, color = { 0.18, 0.18, 0.20, 1.0 } })
    spawn_block({ pos = { x = -14.5, y = 2.2, z = 0.0 }, size = { x = 1.0, y = 6.0, z = 28.0 }, color = { 0.18, 0.18, 0.20, 1.0 } })
    spawn_block({ pos = { x = 14.5, y = 2.2, z = 0.0 }, size = { x = 1.0, y = 6.0, z = 28.0 }, color = { 0.18, 0.18, 0.20, 1.0 } })

    spawn_block({ pos = { x = 0.0, y = 1.1, z = 0.0 }, size = { x = 3.0, y = 2.2, z = 8.0 }, color = { 0.33, 0.34, 0.38, 1.0 } })
    spawn_block({ pos = { x = -5.5, y = 0.9, z = -5.0 }, size = { x = 2.0, y = 1.8, z = 4.0 }, color = { 0.30, 0.31, 0.36, 1.0 } })
    spawn_block({ pos = { x = -5.5, y = 0.9, z = 5.0 }, size = { x = 2.0, y = 1.8, z = 4.0 }, color = { 0.30, 0.31, 0.36, 1.0 } })
    spawn_block({ pos = { x = 5.5, y = 0.9, z = -5.0 }, size = { x = 2.0, y = 1.8, z = 4.0 }, color = { 0.30, 0.31, 0.36, 1.0 } })
    spawn_block({ pos = { x = 5.5, y = 0.9, z = 5.0 }, size = { x = 2.0, y = 1.8, z = 4.0 }, color = { 0.30, 0.31, 0.36, 1.0 } })
    spawn_block({ pos = { x = -9.0, y = 0.7, z = 0.0 }, size = { x = 1.6, y = 1.4, z = 3.5 }, color = { 0.28, 0.30, 0.34, 1.0 } })
    spawn_block({ pos = { x = 9.0, y = 0.7, z = 0.0 }, size = { x = 1.6, y = 1.4, z = 3.5 }, color = { 0.28, 0.30, 0.34, 1.0 } })

    spawn_marker({ pos = { x = -11.8, y = 0.15, z = 0.0 }, size = { x = 0.4, y = 0.3, z = 12.0 }, color = { 0.12, 0.45, 0.95, 0.85 } })
    spawn_marker({ pos = { x = 11.8, y = 0.15, z = 0.0 }, size = { x = 0.4, y = 0.3, z = 12.0 }, color = { 0.95, 0.24, 0.18, 0.85 } })
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

build_arena()
register_weapons()
broadcast_state(nil)

RegisterEvent('onPlayerJoin', function(player_id)
    local id = ArenaNormalizePlayerId(player_id)
    if not id then
        return
    end

    local state = ensure_player(id)
    TriggerClientEvent('arena:announcement', id, {
        text = string.format('Assigned to %s. Rifle on slot 1, pistol on slot 2.', ArenaTeamLabel(state.team)),
    })
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
    respawn_pending[id] = nil
    broadcast_state(nil)
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
