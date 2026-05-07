resource_type 'script'
author 'Developer'
version '2.0.0'
description 'In-game ESC pause menu with background, logo and icon sprites.'

dependencies {'core/init'}

client_scripts {'client/esc_menu.lua'}

-- Obrázky k dodání do images/:
--   esc_bg.png    — libovolná textured/blurred fotka na pozadí (doporučeno 1920×1080)
--   esc_logo.png  — název hry / logo (průhledné PNG, libovolné rozměry)
--   esc_icons.png — sprite sheet 2×2: [play | gear / disconnect | power-off]
images {{
    id = 'esc_bg',
    path = 'images/esc_bg.jpg'
}, {
    id = 'esc_logo',
    path = 'images/esc_bg.jpg'
}, {
    id = 'esc_icons',
    path = 'images/esc_bg.jpg'
}}
