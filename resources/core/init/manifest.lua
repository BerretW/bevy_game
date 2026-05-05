-- /resources/core/init/manifest.lua
--
-- Bootstrap resource. Načítá se jako úplně první (nemá dependencies).
-- Demonstruje plný DSL surface: shared/server/client scripts, files atd.

resource_type 'script'
author       'Framework'
version      '0.1.0'
description  'Bootstrap resource — registruje globální Lua API a event hooks.'

dependencies {
    -- žádné — root manifest
}

shared_scripts {
    'shared/api.lua',
}

server_scripts {
    'server/bootstrap.lua',
}

client_scripts {
    'client/bootstrap.lua',
}

files {
    -- Phase 2 sem přijdou assety, které server zařadí do file digestu pro klienta.
}
