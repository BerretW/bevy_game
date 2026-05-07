resource_type 'script'
author       'Developer'
version      '1.0.0'
description  'In-game ESC pause menu with Disconnect and Quit buttons.'

dependencies {
    'core/init',
}

client_scripts { 'client/esc_menu.lua' }
