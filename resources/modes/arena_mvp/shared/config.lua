ArenaConfig = {
    team_names = {
        alpha = 'ALPHA',
        bravo = 'BRAVO',
    },
    score_limit = 30,
    respawn_delay_ms = 3000,
    spawn_delay_join_ms = 500,
    arena_bounds = {
        min = { x = -14.0, y = -2.0, z = -14.0 },
        max = { x = 14.0, y = 8.0, z = 14.0 },
    },
    spawns = {
        alpha = {
            { pos = { x = -10.5, y = 1.25, z = -7.0 }, rot = { x = 0.0, y = 55.0, z = 0.0 } },
            { pos = { x = -10.5, y = 1.25, z = -2.0 }, rot = { x = 0.0, y = 35.0, z = 0.0 } },
            { pos = { x = -10.5, y = 1.25, z = 3.0 }, rot = { x = 0.0, y = -20.0, z = 0.0 } },
            { pos = { x = -10.5, y = 1.25, z = 8.0 }, rot = { x = 0.0, y = -45.0, z = 0.0 } },
        },
        bravo = {
            { pos = { x = 10.5, y = 1.25, z = -8.0 }, rot = { x = 0.0, y = 225.0, z = 0.0 } },
            { pos = { x = 10.5, y = 1.25, z = -3.0 }, rot = { x = 0.0, y = 205.0, z = 0.0 } },
            { pos = { x = 10.5, y = 1.25, z = 2.0 }, rot = { x = 0.0, y = 160.0, z = 0.0 } },
            { pos = { x = 10.5, y = 1.25, z = 7.0 }, rot = { x = 0.0, y = 145.0, z = 0.0 } },
        },
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
