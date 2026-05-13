# shader_override_test

Opt-in klientský demo resource pro per-entitní shader profile na drawable materiálu.

## Co dělá

- počká na lokální player handle z `player:anim_state`
- zavolá `World.SetEntityShaderProfile(handle, profile)` jen na lokální entitu
- přes `F8` cykluje profily `debug_stripes`, `hologram`, `heat`, `dissolve`

## Co máš vidět

- overlay ukazuje aktivní `profile`, `status` a `handle`
- efekt se aplikuje jen na lokální hráčovu entitu, ne na všechny drawable stejné template
- `F8` okamžitě přepíná mezi čtyřmi odlišnými vzhledy

## Profily

- `debug_stripes`: cyan/orange pruhy
- `hologram`: cyan scanlines + fresnel glow
- `heat`: žhavý oranžovo-žlutý pulse
- `dissolve`: procedurální rozpad s emissive hranou

## Poznámka

Aktuálně jsou tyto profily implementované v `standard_pbr` shaderu. API zůstává per-entitní, ale další template by potřebovaly stejnou branch doplnit zvlášť.