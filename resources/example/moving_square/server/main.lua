-- server/main.lua — server cast moving_square resource.
--
-- Kdyz se hrac pripoji, rekne mu jeho player_id ('sq:init').
-- Kazdy tick dostaneme 'onPlayerPosition' z Rust event busu (sim.rs)
-- a broadcastujeme ho vsem klientum jako 'sq:pos'.

assert(IS_SERVER, 'server/main.lua musi bezet na serveru')

-- Kdyz klientsky resource nabehne, explicitne si rekne o init.
-- Vyuzivame `sender` (player_id) a posilame ho jako string, aby se
-- neprojevila ztrata presnosti Lua number u velkych u64.
RegisterEvent('sq:ready', function(_data, sender)
    if not sender then
        return
    end
    local sid = tostring(sender)
    log_info(string.format('[moving_square/server] sq:ready od %s, posilam sq:init', sid))
    TriggerClientEvent('sq:init', sid, { id = sid })
end)

-- Informacni log pri pripojeni hrace (init uz resi sq:ready).
RegisterEvent('playerConnecting', function(data, _sender)
    if type(data) ~= 'table' then return end
    local id = tostring(data.id)
    log_info(string.format('[moving_square/server] hrac %s se pripojil', id))
end)

-- Kazdy FixedUpdate Rust emituje 'onPlayerPosition' do local event busu.
-- Preposle ho vsem klientum.
RegisterEvent('onPlayerPosition', function(data, _sender)
    if type(data) ~= 'table' then return end
    TriggerClientEvent('sq:pos', nil, data)
end)

log_info('[moving_square/server] server ready')
