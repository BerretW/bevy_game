resource_type 'script'
author       'Developer'
version      '1.0.0'
description  'Minimální HP HUD — ukázka NUI overlay.'

dependencies {
    'core/init',
}

client_scripts { 'client/hud.lua' }

ui_page 'ui/index.html'

files {
    'client/hud.lua',
    'ui/index.html',
}
