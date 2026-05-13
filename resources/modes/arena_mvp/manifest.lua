resource_type 'gamemode'
author 'GitHub Copilot'
version '0.1.0'
description '8-player MVP arena with instant loadout and timed respawns.'

dependencies {
    'core/init',
}

shared_scripts {
    'shared/config.lua',
}

server_scripts {
    'server/main.lua',
}

client_scripts {
    'client/main.lua',
}
