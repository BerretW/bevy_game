resource_type 'script'
author       'Developer'
version      '1.0.0'
description  'Ukázkový HUD — health bar, crosshair, fps counter a weapon state panel.'

client_scripts { 'client/hud.lua' }

-- Vlastní fonty deklarované v manifestu.
-- Soubor musí existovat v resource root (zde: resources/example/hud/fonts/).
fonts {
    { id = 'roboto', path = 'fonts/Super Beatpop.ttf' },
}

-- Obrázky / sprity deklarované v manifestu.
-- images {
--     { id = 'crosshair', path = 'images/crosshair.png' },
-- }
