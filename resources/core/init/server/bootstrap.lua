-- server/bootstrap.lua — běží jen na serveru (jak vyžaduje manifest).

assert(IS_SERVER, 'expected to run on server side')
log_info(Core.greet('host_server'))

RegisterEvent('onPlayerJoin', function(player_id)
    log_info(string.format('player %s joined (server-side handler)', tostring(player_id)))
end)

-- Phase 2 demo: server odpoví na klientský `ping` broadcastem `pong`.
-- TriggerClientEvent(name, target, payload) — `target = nil` ⇒ broadcast
-- všem připojeným klientům (per-target unicast je Phase 3).
RegisterEvent('ping', function(payload, _sender)
    log_info('[rpc-demo] received ping: ' .. tostring(payload))
    TriggerClientEvent('pong', nil, 'pong from server')
end)
