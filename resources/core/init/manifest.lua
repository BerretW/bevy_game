-- /resources/core/init/manifest.lua
--
-- Root resource frameworku. Načítá se jako úplně první při startu serveru
-- a nemá žádné závislosti. Ostatní resources na něm můžou stavět přes
-- `dependencies { 'init' }`. V této fázi je úmyslně prázdný — slouží jen
-- jako sanity-check, že VFS scanner v Phase 1 dokáže manifest najít a sparsovat.

resource_type 'script'
author      'Framework'
version     '0.1.0'
description 'Bootstrap resource — registruje globální Lua API a event hooks.'

dependencies {
    -- žádné — root manifest
}

shared_scripts {
    -- 'shared/api.lua',
}

client_scripts {
    -- 'client/bootstrap.lua',
}

server_scripts {
    -- 'server/bootstrap.lua',
}

files {
    -- assety, shadery atd. — prázdné, dokud nemáme co distribuovat
}
