-- server/bootstrap.lua — běží jen na serveru (jak vyžaduje manifest).

assert(IS_SERVER, 'expected to run on server side')
log_info(Core.greet('host_server'))

RegisterEvent('onPlayerJoin', function(player_id)
    log_info(string.format('player %s joined (server-side handler)', tostring(player_id)))
end)
