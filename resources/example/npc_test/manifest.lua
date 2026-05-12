resource_type 'script'

author 'Engine Team'
version '1.0'
description 'NPC AI test: wander + go-to coord + follow entity'

dependencies {
    'core/init',
}

server_scripts {
    'server.lua'
}

client_scripts {
    'client.lua'
}
