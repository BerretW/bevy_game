-- /resources/example/hello/manifest.lua
--
-- Demonstruje dependency resolver — depende na 'core/init', takže se
-- *musí* nahrát až po něm. Topological sort to zařídí.
--
-- Důležité: sandbox je izolovaný — `Core.greet` z core/init zde NENÍ
-- dostupný. Cross-resource API jde výhradně přes event bus
-- (`TriggerEvent`/`RegisterEvent`), který napojíme v Phase 3.

resource_type 'script'
author       'Framework'
version      '0.1.0'
description  'Příklad závislého resource — testuje load order a izolaci sandboxu.'

dependencies {
    'core/init',
}

shared_scripts {
    'shared/main.lua',
}

server_scripts {
    'server/npc_demo.lua',
}
