-- /resources/example/anim_notify_test/manifest.lua
--
-- Testovací resource pro ADM crossfade + onAnimNotify event flow.

resource_type 'script'
author       'Framework'
version      '0.1.0'
description  'Anim notify + crossfade test pro ADM v4 pipeline.'

dependencies {
    'core/init',
}

shared_scripts {
    'shared/main.lua',
}
