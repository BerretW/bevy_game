ArenaConfig = {
    max_slots = 8,
    team_names = {
        alpha = 'ALPHA',
        bravo = 'BRAVO',
    },
    score_limit = 30,
    respawn_delay_ms = 3000,
    spawn_delay_join_ms = 500,
    zombie = {
        target_bot_count = 8,
        contact_damage = 34,
        contact_radius = 1.25,
        attack_cooldown_ms = 900,
        retarget_interval_ms = 450,
        wander_radius = 4.5,
        move_speed = 3.7,
        arrive_distance = 0.8,
        turn_speed = 8.0,
    },
    ammo = {
        refill_cooldown_ms = 5000,
        stations = {
            {
                id = 'alpha_cache',
                label = 'ALPHA cache',
                pos = { x = -31.0, y = 0.9, z = 18.0 },
                radius = 2.6,
                rifle = 120,
                pistol = 60,
                color = { 0.16, 0.46, 0.95, 0.92 },
            },
            {
                id = 'bravo_cache',
                label = 'BRAVO cache',
                pos = { x = 31.0, y = 0.9, z = -18.0 },
                radius = 2.6,
                rifle = 120,
                pistol = 60,
                color = { 0.94, 0.36, 0.18, 0.92 },
            },
            {
                id = 'mid_cache',
                label = 'MID cache',
                pos = { x = 0.0, y = 0.9, z = 0.0 },
                radius = 2.2,
                rifle = 90,
                pistol = 36,
                color = { 0.92, 0.84, 0.30, 0.92 },
            },
        },
    },
    arena_bounds = {
        min = { x = -42.0, y = -2.0, z = -28.0 },
        max = { x = 42.0, y = 12.0, z = 28.0 },
    },
    spawns = {
        alpha = {
            { pos = { x = -34.0, y = 1.25, z = 18.0 }, rot = { x = 0.0, y = 18.0, z = 0.0 } },
            { pos = { x = -35.0, y = 1.25, z = 12.0 }, rot = { x = 0.0, y = 28.0, z = 0.0 } },
            { pos = { x = -33.0, y = 1.25, z = 6.5 }, rot = { x = 0.0, y = 8.0, z = 0.0 } },
            { pos = { x = -31.5, y = 1.25, z = 1.5 }, rot = { x = 0.0, y = -8.0, z = 0.0 } },
        },
        bravo = {
            { pos = { x = 34.0, y = 1.25, z = -18.0 }, rot = { x = 0.0, y = 198.0, z = 0.0 } },
            { pos = { x = 35.0, y = 1.25, z = -12.0 }, rot = { x = 0.0, y = 208.0, z = 0.0 } },
            { pos = { x = 33.0, y = 1.25, z = -6.5 }, rot = { x = 0.0, y = 188.0, z = 0.0 } },
            { pos = { x = 31.5, y = 1.25, z = -1.5 }, rot = { x = 0.0, y = 172.0, z = 0.0 } },
        },
    },
    zombie_spawns = {
        { x = -6.0, y = 0.0, z = 0.0 },
        { x = -2.0, y = 0.0, z = 4.0 },
        { x = 2.0, y = 0.0, z = -4.0 },
        { x = 6.0, y = 0.0, z = 0.0 },
        { x = -12.0, y = 0.0, z = -14.0 },
        { x = 12.0, y = 0.0, z = 14.0 },
        { x = -16.0, y = 0.0, z = 12.0 },
        { x = 16.0, y = 0.0, z = -12.0 },
    },
}

function ArenaDeepCopy(value)
    if type(value) ~= 'table' then
        return value
    end
    local out = {}
    for key, entry in pairs(value) do
        out[key] = ArenaDeepCopy(entry)
    end
    return out
end

function ArenaNormalizePlayerId(value)
    if value == nil then
        return nil
    end
    local numeric = tonumber(value)
    if numeric then
        return tostring(math.floor(numeric))
    end
    return tostring(value)
end

function ArenaTeamLabel(team)
    if not team then
        return 'UNASSIGNED'
    end
    return ArenaConfig.team_names[team] or string.upper(tostring(team))
end
