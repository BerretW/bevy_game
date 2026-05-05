-- /resources/example/moving_square/manifest.lua
--
-- Demo resource: server dostava pozice hracu z Rust event busu a posilá je
-- klientum. Klient spawne zeleny ctverec a pohybuje jim pres World.SetTransform.
-- Pohyb ctveree je rizen klavesami WASD (pres server-authoritativni pohyb hrace).

resource_type 'script'
author       'Example'
version      '1.0.0'
description  'Moving square demo — zeleny ctverec sleduje pohyb hrace (WASD).'

dependencies {
    'core/init',
}

shared_scripts {
    'shared/input.lua',
}
files {
    'stream/blacksmith.glb',
}

server_scripts {
    'server/main.lua',
}

client_scripts {
    'client/main.lua',
}
