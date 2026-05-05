-- client/bootstrap.lua — běží jen na klientu.

assert(IS_CLIENT, 'expected to run on client side')
log_info(Core.greet('host_client'))
