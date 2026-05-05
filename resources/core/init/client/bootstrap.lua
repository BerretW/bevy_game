-- client/bootstrap.lua — běží jen na klientu.

assert(IS_CLIENT, 'expected to run on client side')
log_info(Core.greet('host_client'))

-- Phase 2 demo: handshake odpověď.
RegisterEvent('pong', function(payload, _sender)
    log_info('[rpc-demo] received pong from server: ' .. tostring(payload))
end)

-- Klientský sandbox vzniká až po stažení souborů, kdy je lightyear connection
-- už dlouho navázaná, takže TriggerServerEvent v load-time scriptu úspěšně
-- protlačí zprávu serveru. Phase 3 přinese explicitní `onConnect` event,
-- který bude robustnější vstupní bod pro inicializační RPC.
TriggerServerEvent('ping', 'hello server, this is client')
