assert(IS_CLIENT, 'arena_mvp client script must run on client side')

local local_player_handle = nil
local pending_spawn = nil
local clock_ms = 0
local announcement = nil
local announcement_deadline = 0

local arena_state = {
    team = nil,
    scores = { alpha = 0, bravo = 0 },
    players = {},
    score_limit = ArenaConfig.score_limit,
    respawn_delay_ms = ArenaConfig.respawn_delay_ms,
    respawn_deadline = nil,
    humans = 0,
    zombies = 0,
    slots = ArenaConfig.max_slots,
}

local function set_announcement(text, duration_ms)
    announcement = text
    announcement_deadline = clock_ms + (duration_ms or 2500)
end

local function color_for_team(team, alpha)
    if team == 'alpha' then
        return 70, 145, 255, alpha
    end
    if team == 'bravo' then
        return 255, 95, 72, alpha
    end
    return 210, 210, 210, alpha
end

local function apply_spawn(payload)
    if not payload or type(payload) ~= 'table' then
        return
    end

    arena_state.team = payload.team or arena_state.team
    arena_state.respawn_deadline = nil

    if not local_player_handle or not World.IsValid(local_player_handle) then
        pending_spawn = payload
        return
    end

    local pos = payload.position or { x = 0.0, y = 1.25, z = 0.0 }
    local rot = payload.rotation or { x = 0.0, y = 0.0, z = 0.0 }
    World.SetTransform(local_player_handle, pos, rot)
    pending_spawn = nil
end

Camera.SetMode("first_person")

RegisterEvent('player:anim_state', function(payload)
    if type(payload) ~= 'table' or payload.is_local ~= true then
        return
    end

    local handle = tonumber(payload.handle)
    if not handle or handle <= 0 then
        return
    end

    local_player_handle = handle
    if pending_spawn then
        apply_spawn(pending_spawn)
    end
end)

RegisterEvent('arena:state', function(payload)
    if type(payload) ~= 'table' then
        return
    end

    if type(payload.scores) == 'table' then
        arena_state.scores.alpha = tonumber(payload.scores.alpha) or 0
        arena_state.scores.bravo = tonumber(payload.scores.bravo) or 0
    end
    arena_state.players = type(payload.players) == 'table' and payload.players or {}
    arena_state.score_limit = tonumber(payload.score_limit) or arena_state.score_limit
    arena_state.respawn_delay_ms = tonumber(payload.respawn_delay_ms) or arena_state.respawn_delay_ms
    arena_state.humans = tonumber(payload.humans) or arena_state.humans
    arena_state.zombies = tonumber(payload.zombies) or arena_state.zombies
    arena_state.slots = tonumber(payload.slots) or arena_state.slots
end)

RegisterEvent('arena:respawn_timer', function(payload)
    if type(payload) ~= 'table' then
        return
    end
    arena_state.team = payload.team or arena_state.team
    local delay_ms = tonumber(payload.respawn_ms) or arena_state.respawn_delay_ms
    arena_state.respawn_deadline = clock_ms + delay_ms
end)

RegisterEvent('arena:respawn', function(payload)
    apply_spawn(payload)
end)

RegisterEvent('arena:announcement', function(payload)
    if type(payload) == 'table' then
        set_announcement(tostring(payload.text or ''), tonumber(payload.duration_ms) or 3000)
    elseif payload ~= nil then
        set_announcement(tostring(payload), 3000)
    end
end)

CreateThread(function()
    while true do
        Wait(50)
        clock_ms = clock_ms + 50
        if announcement and clock_ms >= announcement_deadline then
            announcement = nil
        end
    end
end)

local function get_sorted_players()
    local rows = {}
    for player_id, info in pairs(arena_state.players or {}) do
        rows[#rows + 1] = {
            id = tostring(player_id),
            team = info.team,
            kills = tonumber(info.kills) or 0,
            deaths = tonumber(info.deaths) or 0,
            alive = info.alive == true,
        }
    end
    table.sort(rows, function(a, b)
        if a.team ~= b.team then
            return tostring(a.team) < tostring(b.team)
        end
        if a.kills ~= b.kills then
            return a.kills > b.kills
        end
        if a.deaths ~= b.deaths then
            return a.deaths < b.deaths
        end
        return a.id < b.id
    end)
    return rows
end

local function draw_crosshair()
    local cx = 0.5
    local cy = 0.5
    local gap = 0.010
    local len = 0.014
    Gui.DrawLine(cx - gap - len, cy, cx - gap, cy, 255, 255, 255, 180)
    Gui.DrawLine(cx + gap, cy, cx + gap + len, cy, 255, 255, 255, 180)
    Gui.DrawLine(cx, cy - gap - len, cx, cy - gap, 255, 255, 255, 180)
    Gui.DrawLine(cx, cy + gap, cx, cy + gap + len, 255, 255, 255, 180)
end

CreateThread(function()
    while true do
        Wait(0)

        local stats = Player.GetLocalStats()
        local hp = stats.hp or 100
        local max_hp = stats.max_hp or 100
        local frac = math.max(0.0, math.min(1.0, hp / math.max(max_hp, 1)))
        local team = arena_state.team
        local tr, tg, tb, ta = color_for_team(team, 220)
        local team_label = ArenaTeamLabel(team)

        Gui.DrawRect(0.15, 0.07, 0.26, 0.085, 10, 12, 14, 170)
        Gui.DrawBorder(0.15, 0.07, 0.26, 0.085, 0.002, tr, tg, tb, 220)
        Gui.DrawText(string.format('%s  %d / %d', team_label, arena_state.scores.alpha, arena_state.scores.bravo), 0.035, 0.045, 0.42, 245, 245, 245, 245)
        Gui.DrawText(string.format('Limit %d   Humans %d   Zombies %d / %d', arena_state.score_limit, arena_state.humans, arena_state.zombies, arena_state.slots), 0.035, 0.074, 0.22, 185, 185, 185, 220)

        Gui.DrawRect(0.13, 0.94, 0.22, 0.032, 22, 22, 26, 210)
        Gui.DrawRect(0.13 - (0.22 * (1.0 - frac) / 2.0), 0.94, 0.22 * frac, 0.032, math.floor(255 * (1.0 - frac)), math.floor(200 * frac), 48, 230)
        Gui.DrawText(string.format('HP %d / %d', math.floor(hp), math.floor(max_hp)), 0.025, 0.925, 0.36, 255, 255, 255, 255)

        local rows = get_sorted_players()
        Gui.DrawRect(0.84, 0.18, 0.26, 0.34, 12, 12, 16, 185)
        Gui.DrawBorder(0.84, 0.18, 0.26, 0.34, 0.002, 255, 255, 255, 110)
        Gui.DrawText('Roster', 0.73, 0.018, 0.34, 240, 240, 240, 240)
        Gui.DrawText('ID   K   D   Team', 0.73, 0.050, 0.24, 175, 175, 175, 220)
        for index = 1, math.min(#rows, 8) do
            local row = rows[index]
            local rr, rg, rb, _ = color_for_team(row.team, 255)
            local alive_flag = row.alive and '*' or 'x'
            Gui.DrawText(
                string.format('%s %2d %2d %s %s', alive_flag, row.kills, row.deaths, ArenaTeamLabel(row.team), row.id),
                0.73,
                0.050 + index * 0.028,
                0.20,
                rr,
                rg,
                rb,
                230
            )
        end

        local active_slot = (stats.active_weapon_slot or 0) + 1
        local weapon_slots = stats.weapon_slots or {}
        local weapon = weapon_slots[active_slot]
        local weapon_name = weapon and weapon.weapon_id or 'unarmed'
        local ammo_in_mag = weapon and (weapon.ammo_in_mag or 0) or 0
        local ammo_type = weapon and weapon.ammo_type_id or nil
        local reserve = 0
        if ammo_type and stats.ammo_reserve then
            reserve = stats.ammo_reserve[ammo_type] or 0
        end

        Gui.DrawRect(0.85, 0.91, 0.24, 0.10, 10, 12, 14, 185)
        Gui.DrawText(string.format('Slot %d  %s', active_slot, weapon_name), 0.74, 0.895, 0.26, 245, 245, 245, 240)
        Gui.DrawText(string.format('Ammo %d / %d', ammo_in_mag, reserve), 0.74, 0.923, 0.24, 220, 220, 220, 230)
        Gui.DrawText('1 = rifle   2 = pistol', 0.74, 0.950, 0.20, 170, 170, 170, 220)
        Gui.DrawText('Orange sphere = ammo cache', 0.74, 0.972, 0.18, 255, 176, 82, 235)

        if arena_state.respawn_deadline and arena_state.respawn_deadline > clock_ms then
            local remaining = math.ceil((arena_state.respawn_deadline - clock_ms) / 1000.0)
            Gui.DrawRect(0.5, 0.18, 0.28, 0.06, 0, 0, 0, 160)
            Gui.DrawBorder(0.5, 0.18, 0.28, 0.06, 0.002, tr, tg, tb, 220)
            Gui.DrawText(string.format('Respawn in %ds', remaining), 0.415, 0.158, 0.42, 255, 255, 255, 240)
        end

        if announcement then
            Gui.DrawRect(0.5, 0.10, 0.40, 0.05, 0, 0, 0, 150)
            Gui.DrawText(announcement, 0.32, 0.082, 0.34, 255, 236, 180, 245)
        end

        draw_crosshair()
    end
end)
