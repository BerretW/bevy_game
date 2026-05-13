# Project Context: FiveM-Style Modular Multiplayer Framework

## Vision

Genre-agnostic multiplayer framework v Rustu (Bevy Engine), architektura napodobuje FiveM/CitizenFX:

- **Rust Core** = "Host Shell" — ECS, networking, physics, rendering, DB pooly
- **Lua Resources** = veškerá herní logika, obsah a UI (hot-reloadable)

Podporuje libovolný žánr pouhým swapem Lua resources — bez rekompilace Rust core.
**Po každé změně, splnění roadmapy nebo rozšíření aktualizuj CLAUDE.md.**

---

## Tech Stack

| Oblast      | Technologie                                              |
| ----------- | -------------------------------------------------------- |
| Engine      | Bevy Engine 0.18, headless-first pro server              |
| Networking  | `lightyear` 0.26 — UDP netcode, channels, replication |
| File sync   | `axum` HTTP server + `reqwest::blocking` downloader  |
| Scripting   | `mlua` via `bevy_mod_scripting`                      |
| Database    | `sqlx` — PostgreSQL (prod) / SQLite (dev)             |
| Shadery     | WGSL, hot-reload přes Bevy AssetServer                  |
| WebUI / NUI | Dioxus /`bevy_egui`; Axum (admin API)                  |

---

## Core Architecture: Resource Paradigm

`/resources/[category]/[resource_name]/` — **Nová funkce = nový Resource. NIKDY neupravuj Rust core.**

```lua
-- manifest.lua příklad
resource_type 'script'
dependencies { 'core/inventory', 'core/stats' }
shared_scripts { 'shared/config.lua' }
server_scripts { 'server/decay_loop.lua' }
client_scripts  { 'client/ui_hunger.lua' }
files { 'assets/ui_icons.png' }
```

`ResourceId` = kanonická cesta od `/resources/` rootu ([core_resources/src/types.rs](core_resources/src/types.rs)).

---

## Coding Standards & Guardrails

### KRITICKÉ: Kolize — Drawable Manifest je JEDINOU pravdou

**Kolize MOHOU existovat POUZE jako explicitní `COLLISION` entity v `.drawable` manifestu. Žádný fallback.**

- ADM pipeline, GLTF pipeline i physics zpracovávají POUZE `DrawableCollision` komponenty
- Entita bez `DrawableCollision` = žádná fyzika, bez výjimek; absence v manifestu = GLTF loguje warning
- Blender workflow: `COL_*` uzly → Apparatus Drawable Toolkit → Export → `.drawable` TOML s `[[entities]]` typ="COLLISION"

### Rust / Lua Boundary

- Entity IDs: Lua komunikuje výhradně přes Bevy Entity ID (u64 handle)
- Data Access: žádné raw Rust pointery; safe bridge `SetComponent(entity, "Hunger", 50)`
- Events: globální Event Bus — `TriggerEvent("onPlayerJoin", player_id)`

### Ostatní pravidla

- **DB:** NIKDY neblokuj ECS loop SQL dotazem — používej `IoTaskPool`
- **Stats:** `FinalMaxHealth = (BaseHealth + Sum(Buffs)) * Multiplier(Auras)`
- **Autoritativnost:** Server je jediná pravda — pozice, inventory, stats; klient predikuje pohyb

---

## Roadmap

### Recent Fixes

- [X] **2026-05-13 (Weapon Fire Mode Lua API)**: Nový server-side weapon state jde teď ovládat i z resources. Přidány Lua API `Weapon.GetFireMode(player_id, slot?)` a `Weapon.SetFireMode(player_id, fire_mode, slot?)`, pod nimi nový `LuaCommand::SetWeaponFireMode`, který mění mód vybavené zbraně v cílovém nebo aktivním slotu a resetuje lokální `FireState`. Resources tak už nemusí měnit celé `EquippedWeapon` tabulky jen kvůli přepnutí `semi/full/burst`.
- [X] **2026-05-13 (Semi-Auto Fire Mode Semantics Foundation)**: `EquippedWeapon` teď nese i `fire_mode` a server při `SetEquippedWeapon` doplňuje výchozí mód z `WeaponDef.default_fire_mode` nebo z prvního `fire_modes` záznamu. `core_net::process_combat()` už fire mode respektuje aspoň v základní podobě: `semi` a zatím i `burst` střílí jen na rising edge triggeru, zatímco ostatní módy fungují jako držení spouště. Tím už `WeaponDef.fire_modes` není mrtvé metadata pole i když plná burst/full-auto FSM ještě zbývá.
- [X] **2026-05-13 (Hitbox Registry + Capsule Hit Resolver Foundation)**: `core_resources` teď obsahuje runtime `HitboxDef` / `HitboxRegistry` s built-in profilem `player_default` a nové server-side Lua API `Hitbox.Register/Get`. Server player entity dostává `PlayerHitbox("player_default")` a `core_net::process_combat()` už pro ranged zásahy nepoužívá jen hrubý 2D cone check, ale nearest-hit kapslový resolver nad hitbox bones. Eventy `onPlayerHit` a `onPlayerDamage` teď nesou `hitzone`, `headshot`, `raw_damage`, `damage`, `distance_m` a `ammo`, takže hit detection konečně rozlišuje head/chest/limb multipliery i bez plné lag-compensation vrstvy.
- [X] **2026-05-13 (Example HUD Weapon State Panel)**: `resources/example/hud/` teď kromě health baru a FPS zobrazuje i lokální weapon snapshot z `Player.GetLocalStats()`: aktivní slot, `weapon_id`, `ammo_in_mag`, reserve ammo a pending `fire/reload/weapon_swap` časy. To dává okamžitý client-side debug panel pro nový server weapon state bez nutnosti psát ad hoc test resource nebo sahat do Rust UI.
- [X] **2026-05-13 (Client Local Weapon State Snapshot)**: `PlayerStatsUpdate` už neposílá jen `hp/max_hp`, ale i lokální weapon snapshot (`weapon_slots`, `ammo_reserve`, `active_weapon_slot`, `fire/reload/swap` timing pole). `core_net::broadcast_player_stats()` ho skládá z autoritativních serverových komponent, `core_net::lua_rpc::receive_player_stats()` ho zapisuje do `LocalPlayerStats` jako plný `StatsSnapshot`, a klientské Lua `Player.GetLocalStats()` teď vrací nejen HP, ale i aktivní slot, zásobníky, reserve ammo a pending fire/reload/swap stav pro HUD/debug resources.
- [X] **2026-05-13 (Shared Fire State Foundation)**: Server weapon flow už nepoužívá izolovaný `WeaponCooldown` mimo zbytek player weapon state. Přidán sdílený ECS komponent `FireState { cooldown_remaining, shot_interval, trigger_held }` do `core_resources`, serverový spawn ho zakládá vedle `ReloadState`/`WeaponSwapState`, `core_net::process_combat()` ho používá jako jediný zdroj pravdy pro gating střelby a `sync_player_state_cache()` teď publikuje i `fire_cooldown_remaining` + `fire_trigger_held` do `StatsSnapshot`. Tím je reload/swap/fire konečně na jedné společné state vrstvě místo mixu shared komponent a lokálního server-only cooldown hacku.
- [X] **2026-05-13 (Timed Reload + Weapon Swap States)**: Weapon flow už není čistě instantní. Přidány ECS komponenty `ReloadState` a `WeaponSwapState` na server player entity, nové fixed-step systémy `tick_reload_states()` a `tick_weapon_swap_states()`, a `core_net::process_combat()` teď při `RELOAD` nebo `WEAPON_SLOT_1..4` pouze zahajuje pending akci místo přímé mutace munice/aktivního slotu. Reload completion používá `WeaponDef.reload_empty_sec` / `reload_tactical_sec`, slot swap má krátké serverové zpoždění odvozené z weapon timings a pending reload se při slot switch requestu ruší.
- [X] **2026-05-13 (Weapon Slot Input Wiring + ADS Bit)**: `PlayerInput.actions` teď skutečně nese i `ADS` a `WEAPON_SLOT_1..4` bity. `host_client::gameplay` je serializuje z existujících bindů `aim` a `weapon_1..weapon_4`, `core_net::protocol::player_action` je má definované na bitech 12 a 13–16 a server `process_combat()` při přijetí inputu okamžitě přepíná `ActiveWeaponSlot` hráče ještě před reload/fire logikou. Tím už weapon state nejde měnit jen přes Lua API, ale i přímo standardním klientským input flow.
- [X] **2026-05-13 (Per-Player Weapon State Foundation)**: Server hráči teď dostávají skutečný weapon state místo čistě implicitního combat defaultu. Přidány ECS komponenty `WeaponSlots` (4 sloty), `EquippedWeapon`, `AmmoReserve` a `ActiveWeaponSlot`, jejich synchronizace do `PlayerStatsCache`/`PlayerEntityMap`, a server-side Lua API `Weapon.GetEquipped/SetEquipped`, `Weapon.GetAmmoReserve/SetAmmoReserve`, `Weapon.GetActiveSlot/SetActiveSlot`, `Weapon.ForceReload`. `core_net::process_combat()` už při střelbě používá aktivní slot konkrétního hráče, odečítá `ammo_in_mag`, respektuje `RELOAD` bit pro okamžité přebití z rezervy a fallbackuje na registry default jen pokud hráč zatím nemá vybavenou žádnou zbraň.
- [X] **2026-05-13 (Combat Reads Default Weapon Registry Entry)**: `core_net::process_combat()` už nepoužívá jen slepý globální `WeaponConfig`. Server combat si nově bere výchozí zbraň z `WeaponRegistry` (`id="default"` nebo jediná registrovaná položka), z ní počítá `fire_rate` z `rpm`, přebírá `weapon_type` z `category` a damage z navázaného `AmmoRegistry` `default_ammo.base_damage`. Event payload `onPlayerHit` teď navíc nese skutečné `weapon` ID z registru. `WeaponConfig` zatím zůstává jako fallback pro legacy/simple combat pole, která nová registry foundation ještě autoritativně neřídí (`range`, `cone_angle` a fallback damage/rate při neúplné definici).
- [X] **2026-05-13 (Weapon/Ammo Registry Foundation)**: `core_resources` teď obsahuje typed runtime registry pro `WeaponDef`, `AmmoDef`, `AttachmentDef` a `MaterialDef` místo původního jediného `core_net::WeaponConfig` defaultu. Přidány Bevy resources `WeaponRegistry`, `AmmoRegistry`, `AttachmentRegistry`, `MaterialRegistry`, nový modul `core_resources/src/weapons.rs` a Lua API `Weapon.Register/Get`, `Ammo.Register/Get`, `Attachment.Register/Get`, `Material.Register/Get`, takže resources už mohou definovat zbraňová data v runtime bez rebuildu Rustu. Zatím jde o lokální registry foundation; navazující balistika, equip state a síťová/gameplay integrace zůstávají v dalších bodech roadmapy.
- [X] **2026-05-13 (Additional Per-Entity Shader Profiles)**: `standard_pbr` per-entitní shader profile branch už neumí jen `debug_stripes`, ale i `hologram`, `heat` a `dissolve`. `resources/example/shader_override_test/` teď na lokálním hráči přes `F8` cykluje mezi čtyřmi profily, takže je možné rychle ověřit několik odlišných shader efektů nad jednou konkrétní entitou bez globálního přepnutí celé materiálové šablony.
- [X] **2026-05-13 (Per-Entity Drawable Shader Profile Override)**: Drawable materiály teď umí i per-entitní shader profile přes existující `LuaMaterialOverride` pipeline místo globálního template switchu. Nové Lua API `World.SetEntityShaderProfile(handle, profile)` / `World.ClearEntityShaderProfile(handle)` zapisuje profile na konkrétní root handle; `core_drawable::hook::apply_material_overrides()` ho promítne do `DrawableParams.profile.x` jen pro danou entitu a `standard_pbr` shader zatím interpretuje `debug_stripes` profile jako výrazný cyan/orange stripe vzhled. To umožňuje nasadit shader efekt na konkrétní entitu ve hře bez přepnutí celé materiálové šablony.
- [X] **2026-05-13 (Example Per-Entity Shader Profile Resource)**: `resources/example/shader_override_test/` teď slouží jako opt-in client-side demo pro per-entitní shader profile. Resource si z `player:anim_state` chytí lokální player handle, zavolá `World.SetEntityShaderProfile(handle, 'debug_stripes')` a v overlay ukáže status + handle, takže je hned vidět, že shader efekt jde nasadit jen na konkrétní entitu místo globálního přepnutí celé template.
- [X] **2026-05-13 (Resource-Driven WGSL Shader Overrides)**: Lua resources už mohou na klientovi přepnout WGSL shader pro existující drawable template bez rebuildu Rustu. Nové API `Engine.SetDrawableShaderOverride(template, rel_path)` / `Engine.ClearDrawableShaderOverride(template)` / `Engine.GetDrawableShaderOverride(template)` resolvuje `.wgsl` soubor relativně k aktuálnímu resource rootu a nastaví absolutní shader override pro `standard_pbr`, `layered_env` nebo `vehicle_glass` materiálovou šablonu. `core_drawable::material` pak při `fragment_shader()`/`deferred_fragment_shader()` preferuje tento resource override před built-in `host_client/assets/shaders/*` fallbackem.
- [X] **2026-05-13 (SQLx Integration Status Verified)**: `core_db` už skutečně poskytuje `sqlx::AnyPool` backend pro Lua `Database.*` API. Server bootstrap v `host_server` předává `DatabaseConfig` + `TokioHandle`, `DatabasePlugin` zakládá pool a vyplňuje `DatabaseBridgeResource`, `core_resources::sandbox` exportuje `Database.execute/query/isConnected`, a `dispatch_db_callbacks()` vrací async výsledky zpět do správného sandboxu. Roadmap bod `Integrovat sqlx` proto už není pending implementace, ale hotová vrstva.
- [X] **2026-05-13 (Stairs Test IK Runtime Diagnostics)**: `host_client::publish_stairs_state_to_lua()` teď do `stairs:state` neposílá jen raycastované `left_foot_y/right_foot_y`, ale i skutečný lokální solver stav z `FootIkBoneState` (`smooth_target_y`, `blend_weight`) pro levou i pravou nohu. `resources/example/stairs_test/` overlay tyto runtime hodnoty zobrazuje, takže ruční průchod už může odlišit „IK se zapnulo“ od „solver opravdu blendí nohy ke schodům“.
- [X] **2026-05-13 (Stairs Test Local IK Auto-Enable)**: `resources/example/stairs_test/` už nevaliduje jen stairs raycast/debug overlay, ale i samotné zapnutí IK. Client resource nově chytá lokální player handle z `player:anim_state`, automaticky volá `World.EnableIk(handle, 1.0)` a v overlay ukazuje `IK enabled` stav + handle, takže runtime ověření foot-placement už není blokované chybějícím test harness.
- [X] **2026-05-13 (Blend Space Weighted Playback)**: `apply_adm_animations` už při aktivním `BlendSpaceState` nesampluje jen jeden dominantní clip. Runtime nově remapuje playback phase do všech aktivních clipů blend space, sampluje jejich tracky po nodech a skládá výsledný keyframe váženě přes `active_clips`, takže blend-space-driven locomotion už používá skutečné multi-clip míchání místo single-clip fallbacku.
- [X] **2026-05-13 (Blend Space Runtime Evaluator Foundation)**: `core_drawable` už nenechává `BlendSpaceState` viset bez efektu. Nový runtime evaluator před `apply_adm_animations` resolveruje blend space z připojených `.ads_anim` setů, spočítá váhy klipů podle pozice v 1D/2D prostoru a zapíše dominantní clip do `AnimationState`, takže `World.PlayBlendSpace(...)` už skutečně rozběhne blend-space-driven locomotion místo pouhého vložení komponenty bez playbacku.
- [X] **2026-05-13 (Client A/D Strafe Axis Fix)**: Default keybindy byly správně (`A=move_left`, `D=move_right`), ale klient měl nekonzistentní orientaci strafe osy vůči aktuální camera/view konvenci. Opraveno jednotně ve všech třech client input paths v `host_client::gameplay` (`apply_player_movement`, `collect_and_send_input`, `publish_input_state_to_lua`), takže `A` a `D` už používají stejnou levou/pravou orientaci jak pro lokální pohyb, tak pro `PlayerInput.move_dir` i Lua resource vstupy.
- [X] **2026-05-13 (Smoke Demo Asymmetric Rotated Puff Cluster)**: `resources/example/smoke_grenade/` teď dál rozbíjí boxovitý vzhled volumetric smoke. Cloud už netvoří soustředné stejnoměrné volume shelly, ale asymetrický cluster různě velkých lobe objemů s odlišným `scale_x/scale_y/scale_z`, růstem, drift offsetem a průběžnou rotací přes `World.SetRotation`, takže výsledný smoke silhouette nepůsobí jako několik vnořených kvádrů.
- [X] **2026-05-13 (Smoke Demo Soft Volumetric Cloud Shape)**: `resources/example/smoke_grenade/` už nespawnuje jeden obří hustý `fog_volume` kvádr. Demo teď skládá kouř z více menších volumetrických lobe objemů s nižší hustotou, nižší absorpcí a jemným drift/wobble offsetem, takže výsledkem není černá krabice, ale podstatně měkčí smoke cloud vhodný jako základ pro grenade/city fog efekty.
- [X] **2026-05-13 (Core Init Volumetric Fog Sync Bool Fix)**: `resources/core/init/` už při klientském blendu normalizuje synchronizované boolean hodnoty (`enabled/shadows/fog.volumetric_enabled/...`) místo striktního `== true`, takže volumetric fog flag nepřepadne na `false`, když payload projde přes síťový Lua/JSON bridge v jiném bool-like tvaru. Client bootstrap navíc posílá volumetric fog parametry i top-level přes `World.ConfigureEnvironmentLight(...)`, takže kamera dostane `VolumetricFog` a env directional light `VolumetricLight` i při nested fog patchi.
- [X] **2026-05-13 (Smoke Demo Reliable Spawn Fallback)**: `resources/example/smoke_grenade/` už není závislý jen na aktuálním `Raycast.GetGroundPosition()` hitu. Startup smoke se spawnuje vždy s fallbackem na bezpečný ground point a resource si navíc při `active clouds == 0` po krátké prodlevě automaticky vytvoří nový demo cloud, takže smoke debug už nezůstane viset na nule jen kvůli chybějícímu raycast hitu nebo jednorázové invalidaci handle.
- [X] **2026-05-13 (Smoke Demo Env Flicker + Fog Volume Scale Fix)**: `resources/example/smoke_grenade/` už každých 500 ms nepřepisuje klientský `EnvironmentLightConfig`, takže se nepere s autoritativním `core/init` a nezpůsobuje blikání světa. Demo zároveň normalizuje spawn `fog_volume` na jednotkový dummy a skutečnou velikost řídí přes `World.SetScale`, takže růst kouřového objemu používá jednu škálovací cestu místo konfliktu mezi marker `size` a parent scale.
- [X] **2026-05-13 (Example Local Smoke Grenade Resource)**: Přidán `resources/example/smoke_grenade/` jako malý client-side demo resource pro nový `fog_volume` dummy shape. Resource přes `input:state` a `Raycast.GetGroundPosition()` spawnuje lokální volumetrický kouř na ground hitu, v čase zvětšuje jeho scale přes `World.SetScale` a po krátké době ho maže, takže slouží jako minimální ukázka pro smoke grenade nebo lokální mlhové kapsy bez zásahu do Rust gameplay systémů.
- [X] **2026-05-13 (Streaming-Boundary Fog + Local Fog Volumes)**: Boundary fog v `core/init` teď může sledovat klientský tile streaming radius místo fixního `fog_start/fog_end`: `host_client::map_loader` publikuje runtime `StreamingVisibilityBoundary`, klientský env apply z něj dopočítá effective fog edge přes `fog.follow_streaming_boundary` + `boundary_inner_distance/boundary_outer_distance`. Zároveň `World.SpawnLocalDummy/SpawnNetworkedDummy` nově podporují shape alias `fog_volume` i `fog_volume={...}` blok na běžných dummy objektech, který spawnuje Bevy `FogVolume` child objem pro lokální mlhu ve městě, kouřové granáty nebo jiné volumetrické kapsy bez zásahu do Rust gameplay logiky.
- [X] **2026-05-13 (Core Init Visibility-Boundary Volumetric Fog)**: Synchronní env profil v `core/init` teď kromě slunce a ambientu řídí i boundary fog preset. `EnvironmentLightConfig` nově obsahuje distance fog (`fog_enabled`, `fog_color`, `fog_start`, `fog_end`, directional fog glow) i volumetric fog nastavení (`volumetric_fog_enabled`, ambient color/intensity, jitter, step_count). Klient aplikuje `DistanceFog` na hlavní kameru, `VolumetricFog` na view a `VolumetricLight` na autoritativní env directional light, takže mlha na hranici viditelnosti i god-rays zůstávají plně resource-driven přes `core:init` snapshoty.
- [X] **2026-05-13 (Env Admin Test Resource + Overlay)**: `resources/example/env_cycle/` nově obsahuje i client overlay s aktuálním synchronním `hour_of_day`, fází dne a základními env parametry ze snapshotů `core:init:env_sync`. Přidán také `resources/example/env_admin/` jako opt-in test resource pro ruční ovládání synchronního env světla: klientský panel se ovládá přes `crouch` toggle a posílá serveru eventy pro výběr presetů a nastavení času, server pak aplikuje změny přes `core:init:set_environment_light` a `core:init:set_environment_time` bez zásahu do Rustu.
- [X] **2026-05-13 (Example Synced Environment Resource)**: Přidán `resources/example/env_cycle/` jako minimální server-side demo resource pro nové synchronní env API v `core/init`. Resource posílá do `core:init:set_environment_light` vlastní preset (`day_length_seconds`, `azimuth_deg`, sun/ambient keyframes) a pak cyklicky přepíná `hour_of_day` přes `core:init:set_environment_time`, takže je hned vidět, jak další resources mají autoritativně řídit den/noc pro všechny klienty.
- [X] **2026-05-13 (Core Init Synced Day/Night Bootstrap)**: `resources/core/init/` teď drží výchozí synchronizovaný env lighting profil pro všechny hráče. Server bootstrap vlastní autoritativní `hour_of_day`, `day_length_seconds` a env preset (`sun` + `ambient` keyframes), pravidelně broadcastuje snapshot přes `core:init:env_sync` a přijímá server-side patch eventy `core:init:set_environment_light` / `core:init:set_environment_time`. Client bootstrap z těchto snapshotů lokálně dopočítává plynulý blend night→dawn→day→dusk→night a aplikuje ho přes `World.ConfigureEnvironmentLight(...)`, takže den/noc běží z jednoho serverového času místo lokálního hardcodu.
- [X] **2026-05-13 (Resource-Driven Environment Light + Blender Light UI)**: `host_client` už při vstupu do hry nespawnuje hardcoded native directional sun. Klientský env light teď běží přes `EnvironmentLightConfig`, který patchují nová client-only Lua API `World.ConfigureEnvironmentLight(opts)` a `World.SetEnvironmentTime(hour)`. Resource skripty tak mohou za běhu měnit `hour_of_day`, `azimuth_deg`, `max_elevation_deg`, barvu, illuminance, shadow toggle i ambient brightness/color. Blender toolkit zároveň dostal light authoring panel pro `LIGHT` objekty: v object panelu jsou přímo vidět/exportovat `color`, `energy`, `use_shadow`, `shadow_soft_size`, `cutoff_distance` a pro spoty i cone parametry (`spot_size`, `spot_blend`), takže authoring už nespoléhá na skryté defaulty mimo toolkit UI.
- [X] **2026-05-13 (Dummy + ADM Lights Foundation)**: Runtime teď umí světla jako součást dummy i drawable pipeline. `World.SpawnLocalDummy/SpawnNetworkedDummy` nově přijímají `point_light`, `spot_light` a `directional_light` shape aliasy i volitelný `light={...}` blok na běžných dummy meshech. `.drawable` manifest podporuje `type="LIGHT"` s parametry `kind/color/intensity/illuminance/range/radius/shadows_enabled/inner_angle_deg/outer_angle_deg`; `hook_drawable_scenes()` i `spawn_adm_scenes()` z něj instancují Bevy Point/Spot/DirectionalLight komponenty. Blender toolkit zároveň zahrnuje `LIGHT` objekty do GLB/ADM export scope, exportuje je jako ADM empty uzly a zapisuje jejich parametry do `.drawable`, takže Blender light entity mohou být embedded přímo v modelu bez nové ADM verze.
- [X] **2026-05-13 (Hello Hunter Zombie Lua Brain Loop)**: `resources/example/hello/server/npc_demo.lua` teď používá jednoho zombie jako server-side hunter demo. Resource sbírá `onPlayerPosition` eventy, vybírá nejbližšího hráče a periodicky přepíná `zombie_chaser` mezi `wander_zone` a `chase_target` s průběžně aktualizovaným `target_pos`, takže demo už nehoní jen statický lure anchor, ale aktivně vyhledává a sleduje hráče.
- [X] **2026-05-13 (Server NPC Ownership Player Query Fix)**: `assign_npc_owners()` na serveru už nepoužívá `Query<(&Transform, &PlayerMarker)>`, ale `Query<(&NetTransform, &PlayerMarker)>`. Serverové player entity totiž při spawnu nemají `Transform`, jen `NetTransform`, takže ownership/LOD systém dřív neviděl žádné hráče, házel všechna NPC do `Background` a ty pak zůstávaly stát bez simulace.
- [X] **2026-05-13 (Owned NPC Bootstrap Query Fix)**: `host_client::bootstrap_owned_npc_agents()` už nevyžaduje nereplikovaný `NpcBrainState` komponent. Lokální state se při převzetí ownershipu vytváří z `ReplicatedNpcBrain`, takže client-owned NPC se skutečně bootstrapnou do `NpcAgent` a po handoffu nezůstávají stát jen proto, že server simulaci vypnul a klient si agenta nikdy nevytvořil.
- [X] **2026-05-13 (NpcRegisterBrain Lua Id Fix)**: `World.NpcRegisterBrain(id, def)` teď doplňuje `def.id` ještě před deserializací. Tím se eliminuje tichý fallback `NpcBrainDef::default()` na `core/human`, který u brain tabulek bez explicitního `id` přepisoval lookup klíč a způsoboval warningy `unknown brain ... falling back to core/human` i hned po úspěšné registraci.
- [X] **2026-05-13 (Replicated NPC JSON Decode Fix)**: `ReplicatedNpcBrain.params` už nepoužívá přímý bincode decode nad `serde_json::Value`. Pole se při síťové replikaci serializuje jako JSON string přes custom serde wrapper, což opravuje klientský lightyear error `SerializationError(BincodeDecode(Serde(AnyNotSupported)))` při zapisování replicated NPC brain komponent.
- [X] **2026-05-13 (NpcRegisterScenario Lua Bridge Fix)**: `World.NpcRegisterScenario(id, def)` už správně doplňuje `def.id` ještě před deserializací do `NpcScenarioDef`, takže resources mohou používat dvouargumentový podpis bez redundantního pole `id` v Lua tabulce. Opraven bootstrap `resources/example/hello/`, který dřív padal na `missing field id` při startu serveru.
- [X] **2026-05-13 (Configurable NPC Runtime Knobs)**: Globální NPC runtime už jde ladit z Lua resources bez zásahu do Rustu. Přidány API `World.NpcConfigureScenarioClock(opts)`, `World.NpcConfigurePopulationDirector(opts)` a `World.NpcConfigureAiLod(opts)`, které patchují `NpcScenarioClockConfig`, `NpcPopulationDirectorConfig` a `NpcAiLodConfig` za běhu. `resources/example/hello/` ukazuje konfiguraci day length, director radius/release chování a LOD budgetů přímo ve skriptu.
- [X] **2026-05-13 (Scenario Clock + Population Director Foundation)**: `core_resources` má nově `NpcScenarioClockConfig` a `advance_npc_scenario_time()`, takže `NpcScenarioTime` běží na serveru automaticky jako day/night clock místo čistě ručního Lua zásahu. Přidán `run_npc_population_director()` a `NpcPopulationAssignment`: scénáře s `auto_assign=true` umí přes `required_tags`, `preferred_brain_kind`, `assignment_radius` a `release_distance` automaticky přidělit volná NPC a zase je uvolnit. `resources/example/hello/` už používá auto-assign watch post bez explicitního `World.NpcSetScenario(...)` bootstrapu.
- [X] **2026-05-13 (Scenario Schedule + LOD Priority Slice)**: `NpcScenarioDef` teď umí `active_from_hour`, `active_until_hour`, `max_occupants` a `lod_priority`. Přidán `NpcScenarioTime` + Lua API `World.NpcSetScenarioTime(hour)`, takže scénáře lze schedule-gatovat i v testech. `sync_npc_scenario_runtime()` počítá per-NPC runtime stav (`active`, `occupancy_granted`, `occupancy_slot`, `lod_priority`) a `sync_npc_brains_to_agents()` fallbackne na `Idle`, když je scénář neaktivní nebo přeobsazený. `assign_npc_owners()` navíc používá scenario priority, task typ a brain archetype pro LOD budget prioritizaci.
- [X] **2026-05-13 (Runtime NPC Scenario Layer Foundation)**: `core_resources` má nově `NpcScenarioRegistry` a Lua API `World.NpcRegisterScenario(id, def)`. `scenario_id` v `ReplicatedNpcBrain` už není jen metadata: `apply_replicated_npc_brain()` mergeuje scenario default params a `UseScenarioPoint` převádí na efektivní scenario task/target před překladem do `NpcMoveGoal`. `resources/example/hello/` registruje první `hello/watch_post` a `hello/zombie_perimeter` scénáře.
- [X] **2026-05-13 (NPC End-State Roadmap)**: V `NPCOwner.md` je nově rozepsaný delivery plán do cíle „NPC populace pro města i divočinu“. Roadmapa je rozdělená na dokončení locomotion core, tile/zone AI LOD, scenario/task layer, population director, reaction/combat AI, debug/telemetrii a zátěžové benchmarky. Má sloužit jako cílový plán nad už hotovým ownership/brain/LOD foundation.
- [X] **2026-05-13 (NPC Steering Cache + AI LOD Foundation)**: `ReplicatedNpcSteering` a `NpcTransformUpdate` teď nesou i lehkou entity-target steering cache (`entity_target_position`, `entity_target_velocity`, `formation_offset`) a nově i obstacle avoidance cache (`avoidance_offset`, `avoidance_timer`), takže chase/follow handoff drží pursuit lead, escort offset i krátkodobý obstacle sidestep bez restartu jen z high-level tasku. `host_client::update_owned_npc_avoidance()` používá forward raycast + side probes pro jednoduchý lokální avoidance impulse. `NpcAiLodState` + `NpcAiLodConfig` zároveň posouvají budgety z čistě per-player modelu i do coarse zone relevance (`zone_size`, `full_budget_per_zone`, `reduced_budget_per_zone`): server v `assign_npc_owners` rozlišuje `Full`/`Reduced`/`Background` podle vzdálenosti hráče, aplikuje per-player i per-zone density budgety a `tick_npc_agents` používá reduced cadence throttling místo plné simulace pro každé NPC v dosahu. Reduced NPC bez ownera už navíc můžou fallbacknout na server simulaci místo úplného freeze.
- [X] **2026-05-13 (Model Viewer NPC Brain Debug)**: `model_viewer` teď umí lokálně simulovat NPC brain/task nad právě načteným root modelem bez serveru a bez Lua. Přidán `NpcBrainDebugState`, trackování aktivního viewer rootu z loaderu, overlay panel s brain/task/motion stavem a gizma pro `home`/current target/path. Ovládání: `F7` toggle debug, `F8` cykluje dostupné brains z `NpcBrainRegistry`, `F9` přepíná task presety (`idle`, `investigate`, `wander_random`, `patrol`, `orbit`) a `F10` otáčí směr orbitu.
- [X] **2026-05-13 (Replicated NPC Brain Contract)**: `core_net` teď replikuje tenký `ReplicatedNpcBrain` komponent (`brain_id`, `task`, `scenario_id`, `target`, `params`) místo plného `NpcAgent`. `core_resources::sync_npc_brains_to_agents()` převádí tento high-level stav na lokální `NpcMoveGoal`, aplikuje motion defaults podle zvoleného brainu a při neplatném tasku fallbackne na `default_task` brain profilu. Lua API rozšířeno o `World.NpcSetTask(handle, task, opts?)` a `World.NpcSetScenario(handle, scenario_id, opts?)`.
- [X] **2026-05-13 (NPC Ownership Handoff Foundation)**: `SpawnNetworkedNpc` nově vždy vkládá `NpcOwner::default()`, takže ownership funguje i pro čerstvě spawnované NPC. `assign_npc_owners` teď používá server-side `NpcOwnershipLease` s hysteresis (`acquire` vs `release` radius), minimální handoff výhodou a cooldownem, takže owner nepřeskakuje mezi klienty při malých změnách vzdálenosti.
- [X] **2026-05-13 (Client-Owned NPC Transform Path)**: Přidán `NpcTransformUpdate` Client→Server message a `NpcTransformChannel`. Server v `core_net::receive_npc_transform_updates()` aplikuje transform jen pokud sender odpovídá `NpcOwner`. Klient bootstrapuje lokální `NpcAgent` pouze pro owned NPC, u těchto NPC ignoruje replicated `NetTransform` writeback a ve `FixedPostUpdate` posílá transform serveru. Server-side `tick_npc_agents` teď NPC s aktivním ownerem přeskočí, takže první client-owned movement loop funguje bez dvojité simulace.
- [X] **2026-05-13 (Client-Owned NPC Fallback + Terrain Snap)**: Server-side `NpcLastClientUpdate` teď sleduje poslední validní NPC update od ownera a `tick_npc_agents` po `NPC_CLIENT_UPDATE_TIMEOUT_SECS` fallbackne zpět na server simulaci, pokud owner umlkne. `host_client::terrain_snap_owned_npcs()` zároveň dělá jednoduchý raycast dolů pro client-owned NPC a koriguje jejich Y vůči terénu před odesláním transformu serveru.
- [X] **2026-05-13 (Owned NPC Handoff Bootstrap)**: `host_client::bootstrap_owned_npc_agents()` už nevkládá prázdný `NpcAgent`, ale hned inicializuje lokálního agenta z `ReplicatedNpcBrain` přes sdílený helper `apply_replicated_npc_brain()`. Nový owner tak přebírá aktuální high-level task/brain stav bez jednokroku v `Idle` a stejné motion defaults se správně reaplikují i po runtime změně brain definice se stejným `brain_id`.
- [X] **2026-05-13 (Owned NPC Steering Snapshot)**: Přidán replikovaný `ReplicatedNpcSteering` snapshot pro handoff continuity. Owning klient posílá přes rozšířený `NpcTransformUpdate` coarse steering stav (`home`, `wander_target`, `wander_timer`, `orbit_angle`, `patrol_to_target`, `current_path`, `waypoint_index`, `map_id`, `last_nav_target`), server ho ukládá do svého `NpcAgent` i replikovaného snapshotu a nový owner z něj při bootstrapu obnoví rozpracovanou corridor/orbit/wander stav místo restartu jen z `ReplicatedNpcBrain`.
- [X] **2026-05-13 (Hello NPC Brain Example)**: `resources/example/hello/` nově obsahuje server-side NPC demo (`server/npc_demo.lua`) ukazující registraci custom brainu `example/hello_aggressive_zombie`, `World.NpcSetBrain`, `World.NpcSetTask` a `World.NpcSetScenario`. Demo spawnuje agresivního zombie chasera, scout zombie a guarda s jednoduchým scenario targetem bez scheduleru.
- [X] **2026-05-13 (Runtime NPC Brains)**: `core_resources` teď má runtime `NpcBrainRegistry` s built-in brain profily `core/human`, `core/animal`, `core/vehicle`, `core/bird`, `core/fish`; `core/human` je integrovaný fallback v jádru. Přidány `NpcBrainState`, Lua API `World.NpcRegisterBrain(id, def)` a `World.NpcSetBrain(handle, id)`, takže high-level brain kontrakty lze měnit přes hot-reload resources bez rebuildu hry.
- [X] **2026-05-13 (NPC Architecture Decision)**: V [NPCOwner.md](NPCOwner.md) byl zvolen cílový směr pro populaci ala REDM: server-authoritative scenario/task vrstva + client-owned low-level locomotion, tenká replikace `ReplicatedNpcGoal/Brain` místo plného `NpcAgent`, ownership handoff s hysteresis a AI LOD vrstvy pro škálování na stovky NPC.
- [X] **2026-05-13 (NPC Navmesh Repath)**: `NpcAgent` teď drží lehký nav-path stav (`last_nav_target`, `nav_repath_timer`) a změna `NpcGoTo*`/`NpcWander`/`NpcStop` vždy invaliduje starou cestu. `host_client::map_loader::update_npc_nav_paths()` nově dělá repath při změně tile, změně targetu a periodicky pro `GoToEntity`; zároveň používá `agent.wander_target` i pro wander/patrol/orbit, takže NPC nejsou zaseknuté na jednorázově spočtené cestě přes world tiles/navmesh ani při nepřímých movement goal režimech.
- [X] **2026-05-13 (Drawable Diagnostics)**: `core_drawable::hook` teď umí cílenou render diagnostiku pro pády v PBR/light passu. Přidány mesh-layout logy (`vertices/indices/pos/nrm/tan/uv0/uv1/color/joint attrs`), logování patchnutí chybějících `ATTRIBUTE_COLOR/UV_1`, zvýraznění alpha-mask/shadow rizik a nový env přepínač `BEVY_GAME_DISABLE_DRAWABLE_SHADOWS=1`, který vynutí `NotShadowCaster` na všech drawable meshích pro rychlé ověření, zda pád vzniká v shadow passu nad custom drawable materiály.
- [X] **2026-05-13 (Client Diagnostics)**: `host_client` teď instaluje vlastní panic hook už při bootstrapu a zapisuje detailní panic report (`thread`, location, message, force-captured backtrace) do `logs/latest_panic.log` i stderr/console overlay. Přidány breadcrumb logy pro klientský bootstrap (`config/cache/logs/backend/gpu_validation`), `AppState` přechody, `ClientHandshakeState` přechody a detailní `map_loader` instance spawn logy (`map/id/model/navmesh_only/transform`), aby bylo vidět přesně co se děje těsně před pádem po připojení.
- [X] **2026-05-13 (Plugin)**: Opraveno linkování i scope nově vytvářených Blender objektů do kolekcí. `Create Drawable`, `Create Collision Proxy` a `Generate Navmesh` už nevkládají root/proxy/NAV_AUTO objekty slepě do `context.collection` nebo Scene rootu, ale do rodičovské kolekce aktivního objektu. `Generate Navmesh` a `Cleanup` navíc zpracovávají jen `COL_*` a `NAV_AUTO_*` z aktivní tile kolekce místo celé scény, takže více tile v jednom `.blend` se navzájem nemíchá.
- [X] **2026-05-13 (Plugin)**: Blender Apparatus Toolkit export upraven pro tile streaming a runtime navmesh formát. `Export Navmesh` teď zapisuje `surface_type`, per-surface `walkable_height/walkable_radius/climb_height` a převádí vertexy ze souřadnic Blenderu do Bevy world-space. Přidán nový `world.index.toml` export s tile metadaty (`tile_id`, `map`, `center`, `load_radius`, `always_loaded`), nový tile authoring workflow nad Blender kolekcemi (`Export Active Tile To Maps`, `Export All Tile Collections`) a import sousedství (`Import Target + Neighbors`) pro načtení cílového tile a jeho sousedů z `world.index.toml` do samostatných kolekcí v Blenderu. Tile workflow teď umí i one-click single-asset bundle export do `assets/models` + `assets/maps`, takže jedna kolekce může reprezentovat celý tile jako jeden `.adm/.drawable` model a odpovídající `map/navmesh/index` záznam.
- [X] **2026-05-13 (Tooling)**: Přidán nový standalone nástroj `map_viewer` pro vizuální inspekci `map.toml` a `world.index.toml`. Viewer používá stejný `host_client/assets` root jako runtime, načítá map instance, registruje `.adm/.drawable` a `.glb` modely z `assets/models`, a přes gizma overlay umí přepínat zobrazení map meshů, colliderů a navmesh surface wireframe.
- [X] **2026-05-13 (Latest)**: Phase 5 Hierarchical Pathfinding + Server Authority Foundation — Implementován `TileGraph` pro cross-tile routing s adjacency detekcem, dva-úrovňový A* (global přes tile graph + local v navmesh), hierarchický pathfinding v `NavmeshRegistry::find_hierarchical_path()`. `TileGraph::build_adjacency()` automaticky detekuje sousedící tile-y. Přidán `TileStreamingCommand` message type v protocol.rs (server→klient) pro server-side autoritu. Vytvořena HLOD infrastruktura (`HLODLayer`, `HLODState`, `HLODTile`) se třemi distančními vrstvami (0-300m full detail, 300-1000m simplified instanced, 1000m+ culled), distance-based visibility gating. Všechny systémy integrovány do DrawablePlugin a map_loader.
- [X] **2026-05-13**: Large-world tile streaming foundation — `host_client` map loader podporuje volitelný index `assets/maps/world.index.toml` (nebo `map.index.toml`) s tile záznamy (`map`, `center`, `load_radius`, `always_loaded`). Přidán stream-in/stream-out map souborů podle vzdálenosti lokálního hráče, unload ECS entit + `LuaWorldState` handle mapy mimo radius, a unload navmesh přes nové `NavmeshRegistry::unload_navmesh`. Přidán ukázkový index `host_client/assets/maps/world.index.toml`.
- [X] **2026-05-12**: NPC Framework + základní AI pohyb — Přidán `NpcAgent` systém v `core_resources` (`tick_npc_agents` ve `FixedUpdate`) s módy `wander` (`random`/`patrol`/`orbit`), `go to entity` a `go to coord`. Rozšířeno Lua API o `World.NpcConfigure/NpcWander/NpcGoToEntity/NpcGoToCoord/NpcStop`. Přidán demo resource `resources/example/npc_test/`.
- [X] **2026-05-12**: Model Viewer texture browser/export tool — Registrace chybějících systémů `init_texture_browser`, `handle_texture_keys`, `rebuild_panel`, `show_extract_status` do `Update` scheduling. Nástroj na zobrazení a export textur (T pro toggle, E pro export) znovu plně funkční.
- [X] **2026-05-12 (oprava)**: Export textur — Oprava cest: ADM cesty byly relativní. Přidán `ModelSourcePaths` Resource, který si pamatuje absolutní cestu na disk pro každý načtený model. Export teď používá absolutní cesty namísto relativních bevy paths.
- [X] **2026-05-12**: Kinematic pohybový refaktor — IK/Terrain Snap/Root Motion — Přidány `OnStairs` + `IkEnabledComponent` do player spawnu; nový systém `terrain_snap_kinematic` v FixedUpdate po `apply_player_movement` (snappuje Y velocitu hráče k terén height z raycastu); root motion plně implementováno: `RootMotionState` komponent, `extract_root_motion` systém, Lua API `World.EnableRootMotion/DisableRootMotion`.

### Fáze 1–4 ✅ Dokončeno

| Fáze                                       | Výsledky                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Phase 1** — Shell & VFS            | Cargo workspace, VFS scanner, manifest.lua DSL parser, dependency resolver (Kahn), per-resource Lua sandbox                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| **Phase 2** — Network Handshake      | `core_net`, lightyear UDP, Axum HTTP file server, blake3 digest handshake, Lua RPC bridge                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| **Phase 3.1** — Gameplay Foundations | `PlayerInput`, `NetTransform`, player spawn/render, 1st/3rd person kamera (F6 toggle / `Camera.SetMode`), client-trusted movement (Avian), yaw sync, movement smoothing, dynamic player model resolve z `player.ped.toml` (`identity.model`) místo hardcoded `models/player.adm`, state-driven player animations z `player.ped.toml` (`[animations]`) jako autoritativní selector (`clip:*`/`dict:*`), startup preload + index ADS animačních setů z `player.ped.toml` (`[animation_sets].ads_anim`) a auto-attach `AttachedAnimSets` na player ADM root, **počáteční idle animace nastavená hned při spawnu** (řeší t-pose na připojení)                                                                                                                                                                                                         |
| **Phase 3.2** — Lua Bridge           | `LuaCommand` enum, `CommandQueue`, `LuaWorldState`, `process_lua_commands` (PostUpdate)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| **Phase 3.3** — Combat               | `WeaponConfig`, `Health`, `process_combat`, `PRIMARY_FIRE` bitflag, ACE authority, `onPlayerHit`/`onPlayerDeath`/`playerConnecting`/`playerDropped`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| **Phase 3.4** — Model Registry       | `ModelRegistry`, `scan_stream_models()`, async GPU load, `Engine.RequestModel/HasModelLoaded`, runtime clip metadata cache (`Engine.GetModelClipCount/GetModelClipNames`), ADM v5 animation dictionary metadata cache (`Engine.GetAnimDictNames/GetAnimDictClips`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| **Phase 3.5** — World Objects        | `SpawnNetworkedObject`, `NetworkedObjectMarker`, lightyear replication observer                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| **Phase 3.7** — Raycast API          | `RaycastBridge`, `Raycast.GetGroundPosition()`, yaw v `PlayerInput.look[0]`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| **Phase 3.8** — Event Bus            | `LocalEventBus`, `TriggerEvent`, JSON payloads, `input:state` bridge, `sq:ready` init pattern, Lua-safe string player IDs                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| **Phase 3.9** — Entity State API     | `EntityHandle`, `ModelName`, `AnimationState`, `EntityStateCache`, `World.Get*/Set*` API                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| **Phase 4** — ADS + GUI              | Apparatus Drawable System (`.drawable` TOML, materiály WGSL, LOD systém, kolize), Blender toolkit (`bevy_toolkit.py`), immediate-mode GUI (`Gui.*`), `CreateThread`/`Wait`, `UI.Window`, ADS socket metadata (`AdsSocketMap` + `AdsSocket`), `World.Attach/Detach/GetSocketTransform`, prefix semantika `DEF_/IK_/SOC_/MEC_`, LOD2+ skeletal pruning, ADM v3 animační klipy + runtime playback, ADM skinning export + `SkinnedMesh` binding, track flags pro body-part blending, Mixamo auto-rename/import/export tooling, model_viewer animation browser, ADM importer: rekonstrukce armatury+weights+NLA, pose klíče převáděné rest-relative (bez double-transform deformací), bevy_masks2 fallback import, bone head/tail rekonstrukce z node hierarchy (bez point-bones), COL_ armature skinning parity + ochrana proti bone-parent/weights double-transform |

**Phase 3.6 — YMAP Streaming** (částečně):

- [X] `World.SpawnNetworkedObject` základ
- [ ] YMAP JSON loader, Mapper tool (Lua in-game editor), AABB streaming, GPU Instancing, server culling

**Phase 4 zbývá:**

- [X] ADM runtime crossfade: `apply_adm_animations` respektuje `AnimationState.blend_time` a plynule blenduje předchozí/aktuální klip (`lerp` pozice/scale, `slerp` rotace)
- [X] ADM v4 animation notifies: loader/export/import podporuje `notify_count`, runtime emituje `onAnimNotify { handle, clip_name, notify_name }` přes `LocalEventBus`
- [X] ADM v5 animation dictionaries: export/import/loader podporují sekci dictionary (`dict_name -> clip indices`), runtime podporuje selector `dict:<dict_name>:<clip_name>` a Lua API `Engine.RequestAnimDict/HasAnimDictLoaded/GetAnimDictNames/GetAnimDictClips`
- [X] ADMv6 `.ads_anim` runtime selector parity: `apply_adm_animations` resolver podporuje `dict:*`, `clip:*`, `anim:*`, index i clip-name (fix T-pose při ped selektorech z `player.ped.toml`)
- [X] Blender export split: `.adm` je geometrie a `.ads_anim` je samostatný animační set
- [X] Blender import split: `.adm` importuje geometrii, `.ads_anim` importuje samostatné animace do armatury
- [X] model_viewer ADMv6 path: `.adm` rooty auto-attachují sibling/CLI `.ads_anim`, animation browser čte klipy/dictionaries/notifies z `AnimationSet`
- [X] model_viewer top toolbar: runtime tlačítka pro otevření `.adm/.glb` a `.ads_anim` + rychlé debug přepínače (grid/colliders/skeleton/reset cam)
- [X] model_viewer modularizace: původní `main.rs` rozdělen do menších modulů (`state.rs`, `scene.rs`, `loader.rs`, `animation.rs`, `runtime.rs`) pro lepší čitelnost a údržbu
- [ ] Blend Spaces infrastruktura: `BlendSpaceState`, `PlayBlendSpace`, runtime evaluator i vážené multi-clip playback míchání jsou hotové; zbývá případně jemnější triangulace/interpolace a parser parity podle starších ADM variant.
- [X] Integrovat `sqlx` (`core_db` + `Database.*` Lua bridge + callback dispatch hotové)
- [X] Vlastní WGSL shadery z Lua resources (override existujících drawable template shaderů přes `Engine.SetDrawableShaderOverride`)

**ADM v6 migration policy:**

- Nový formát je `*.adm` pro geometrii a `*.ads_anim` pro animace.
- Runtime má preferovat late binding přes připojené anim-sety, ne embedded klipy v modelu.
- Klientská `ModelAnimationRegistry` pro ADM modely bere clipy/dictionaries pouze z `AttachedAnimSets` (`.ads_anim`), embedded ADM animace se ignorují.

---

### Phase 4.1 — Blend Spaces (Infrastruktura) [✅ Hotovo]

Implementováno: `AdmBlendSpace`, `AdmBlendSpaceClip` struktury v ADM formátu, `BlendSpaceState` komponenta v cmd_queue, `LuaCommand::PlayBlendSpace`, Lua API `World.PlayBlendSpace(handle, blend_space_name, move_x, move_y, speed?, flags?)`, handler v cmd_queue, test resource `resources/example/blend_space_test/` s rotačním move vektorem.

**Zbývá:** Případně jemnější triangulace/interpolace vah pro 2D blend spaces a ADM v5 parser parity pro blend space definice.

---

### Phase 4.2 — Runtime IK (Infrastruktura) [✅ HOTOVO]

**Veškerá implementace dokončena a ověřena kompilací!**

Implementováno:

- `OnStairs` marker komponent pro detekci schodů
- `IkChain` komponenta s definicí IK řetězce (parent bone, IK target, effector bone)
- `IkSolverState` komponenta pro uložení meziresultátů
- `IkEnabledComponent` (v `core_resources`) marker pro aktivaci IK + type alias `IkEnabled` v `core_drawable` pro kompatibilitu
- `TwoBoneIkSolver` - Two-Bone IK solver s law of cosines algoritmem
- `CollisionMaterial::Stairs` varianta v enum - umožňuje značit schodištní kolizory
- `detect_stairs_on_collision` systém - detekuje přítomnost na schodech
- **`raycast_stairs_under_player()` systém** (v `host_client/src/physics.rs`) — **Implementováno**: Raycast pod oběma nohami (levá stopa: -0.10, +0.05m; pravá stopa: +0.10, +0.05m), distance 1.5m downward, Avian3d `SpatialQuery::cast_ray`, populates `OnStairs.left_foot_height` a `.right_foot_height`
- **`apply_ik_to_skeleton()` funkcionalita** (v `core_drawable/src/ik.rs`) — **Implementováno**: Hierarchické vyhledávání kostí (`find_bone_by_name()` rekurzivní), přímá aplikace Y-offsetu na DEF_foot_l/DEF_foot_r (s fallback naming), blended offset via `IkEnabledComponent.blend_weight` (0-1 interpolace), triggery na `Changed<OnStairs>`
- **Lua API** (`World.EnableIk(handle, blend_weight?)` / `World.DisableIk(handle)`) — **Implementováno** v `core_resources/src/sandbox.rs`: `EnableIk` command insertuje `IkEnabledComponent` s blend_weight clamping, `DisableIk` removuje komponentu
- Client stairs locomotion assist: při kontaktu se `StairsCollider` se pohyb projektuje do roviny sklonu a stabilizuje `LinearVelocity.y` pro plynulý výstup
- Client adaptive IK sampling: na schodech full-rate sampling, mimo schody decimovaný sampling; `stairs:state` nese `ik.quality/sample_hz/left_foot_y/right_foot_y`
- Test resource `resources/example/stairs_test/` s demo schodiště a IK monitoring
- Blender IK authoring workflow (`blender_plugin/appartus_drawable_toolkit`): Scene-level `IK Chains` UI, Add/Remove/Autofill/Validate operátory, a sidecar export `*.ik.toml` při `Export ADS` / `Export ADM` / `Export Animation Set`

**Architektura:**

- `core_resources::IkEnabledComponent` — single source of truth pro IK enable/disable state
- `core_drawable::IkEnabled` — type alias pro `IkEnabledComponent` (kompatibilita, bez cyklu)
- `host_client::raycast_stairs_under_player()` — PopulatesOnStairs heights každý frame
- `core_drawable::apply_ik_to_skeleton()` — Aplikuje Y-offset na nohy na základě `OnStairs` + `IkEnabled` blend
- Lua cmd_queue bridge — `EnableIk` / `DisableIk` commands přes `LuaCommand` enum

**Zbývá:** Runtime průchod na `stairs_test` resource (ověření, že overlay hlásí `IK enabled`, raycast heights se mění na schodech a foot offset je vizuálně správně aplikovaný).

---

### Phase 4.3 — Root Motion (Infrastruktura) [✅ HOTOVO]

Implementováno:

- `RootMotionState` komponent v `core_resources/src/cmd_queue.rs` (root_bone_name, prev_root_world_pos, accumulated_delta, lock_y)
- `extract_root_motion` systém v `core_drawable/src/adm.rs` — extrahuje XZ-deltu z root bonu po animačním framu, aplikuje na parent entitu, resetuje bone na origin
- Lua API: `World.EnableRootMotion(handle, opts?)` / `World.DisableRootMotion(handle)`, kde opts = `{ root_bone = "DEF_hips", lock_y = true }`
- `LuaCommand::EnableRootMotion` / `DisableRootMotion` v cmd_queue
- Systém registrován v `DrawablePlugin.build()` s ordering `.after(apply_adm_animations)`
- Test resource: `resources/example/root_motion_test/`

**Architektura:**

- `RootMotionState` na `AdmSceneRoot` entitě; `extract_root_motion` v `core_drawable` Update
- Systém porovnává `GlobalTransform` root bonu mezi framy → delta → přesune ChildOf parent
- `lock_y=true` (výchozí) → pouze XZ delta, Y řídí fyzika/gravitace

---

### Phase 4.1 — Blend Spaces (Infrastruktura) [✅ Hotovo]

Implementováno: `AdmBlendSpace`, `AdmBlendSpaceClip` struktury v ADM formátu, `BlendSpaceState` komponenta v cmd_queue, `LuaCommand::PlayBlendSpace`, Lua API `World.PlayBlendSpace(handle, blend_space_name, move_x, move_y, speed?, flags?)`, handler v cmd_queue, test resource `resources/example/blend_space_test/` s rotačním move vektorem.

**Zbývá:** Případně jemnější triangulace/interpolace vah pro 2D blend spaces a ADM v5 parser parity pro blend space definice.

---

## Phase 5 — Large-World Infrastructure & Streaming

**Obsah:** Hierarchické pathfinding, server-side streaming autorita, HLOD/instancing pro mega-mapy (RDR2-class scénáře).

### Phase 5.0 — Hierarchical Pathfinding & Tile Graph [✅ HOTOVO]

Implementováno:

- `TileGraph` Resource: Adjacency graph tilek, dva-úrovňový A*
- `TilePathDef`: Tile definition s traversal cost (těžkost terénního typu)
- `TilePortal`: Crossing point mezi sousedními tiley
- `NavmeshRegistry::find_hierarchical_path()`: Global A* přes tile graph + local A* v každém tile
- `TileGraph::build_adjacency()`: Auto-detekce sousedství tilek podle center vzdálenosti
- `TileGraph::find_tile_path()`: Bezpečný routing skrz graf (min-heap priority queue)
- Integrace v `host_client/src/map_loader.rs`: TileGraph se buduje automaticky z `world.index.toml` definic
- Wszystkie struktury budovány s `Serialize`/`Deserialize` pro persistence

**Zbývá:**

- [ ] Integrace do `tick_npc_agents` — NPC by měli používat `find_hierarchical_path` místo přímého target
- [ ] Server-side tile path validation (replay protection)
- [ ] Portal locking/dynamic portals (při destruction objektů)

### Phase 5.1 — Server-Side Streaming Authority [🟡 FOUNDATION READY]

Implementováno:

- `TileStreamingCommand` message type v `core_net::protocol` (tile_id, action: "load"/"unload")
- `TileStreamingChannel` v lightyear — SequencedUnreliable server→klient
- Message registration v `net_plugin.rs`

**Zbývá:**

- [ ] `ServerTileStreamingPlugin` — track player positions, validate visible tiles, send commands
- [ ] Client-side command receiver — apply server directives, ignore client-local streaming
- [ ] Anti-cheat: Detect and reject klient-authoritative tile loads (LOD distance abuse)

### Phase 5.2 — HLOD/Instancing Infrastructure [✅ FOUNDATION READY]

Implementováno:

- `HLODLayer`: Distance tier s simplification factor a GPU instancing flags
- `HLODState` Component: Active layer, instance storage, camera distance tracking
- `HLODTile` Marker: Per-tile HLOD configuration (3-tier: 0-300m full, 300-1000m simplified instanced, 1000m+ culled)
- `StandardHLODConfig::create()`: Sensible defaults pro 3-tier setup
- `update_hlod_visibility()` System: Distance-based gating pro visibility
- `HLODInstanceData`: GPU instance struktura (position_scale, rotation, tint)

**Zbývá:**

- [ ] GPU instancing buffer creation/update v render graph
- [ ] Mesh simplification pipeline (Lloyd relaxation či edge-collapse decimation)
- [ ] Billboard rendering pro far-tier (small quads s instanced texturou)
- [ ] Integration s map_loader — auto-attach HLODTile ke spawnutým map instances

---

### Phase 5 — FPS Core Systems

**Filosofie:** Rust = fyzikální engine + datové kontrakty. Lua = vše herně specifické. Žádná zbraň ani herní pravidlo se nesmí hardcodovat do Rustu.

#### 5.0 — Collision Foundation ✅

Implementováno: `DrawableCollision` → Avian `Collider` pipeline, axis-lock flagy (`lock_translation/rotation`), `DisableDrawableCollisions` marker, `StaticWorldCollider` filter pro movement gate, `NAVMESH` shape → `NavMeshSurfaceCache`, `ClientMapPlugin` (`assets/maps/*.map.toml`), Blender toolkit NAVMESH + map TOML workflow, pravidlo RB ownership: hierarchické (child) drawable collidery nedostávají vlastní `RigidBody` (prevence mesh/collider desync), `DummyPrimitiveKind::Stairs` nyní generuje samostatné child collidery: plynulý ramp helper collider pro pohyb, oddělený `StairsCollider` trigger jako svažitá plošina a tenké step-top IK surface sensory pro budoucí foot-placement.

`material` enum: `CONCRETE`, `STONE`, `BRICK`, `WOOD`, `METAL`, `GLASS`, `DIRT`, `GRASS`, `SAND`, `GRAVEL`, `MUD`, `SNOW`, `ICE`, `WATER`, `RUBBER`, `PLASTIC`, `CERAMIC`, `CARPET`, `ASPHALT`, `LADDER_METAL`.

---

#### 5.1 — Weapon & Ammo Registry [🟡 FOUNDATION READY]

**Rust side:**

- [X] `WeaponDef`, `AmmoDef`, `AttachmentDef`, `MaterialDef` structs
- [X] `WeaponRegistry`, `AmmoRegistry`, `AttachmentRegistry`, `MaterialRegistry` Bevy Resources
- [X] Lua API: `Weapon.Register/Get`, `Ammo.Register/Get`, `Attachment.Register/Get`, `Material.Register/Get`
- [ ] Dokončit integraci registrů do equip/ballistics pipeline; simple server combat už čte default z `WeaponRegistry`, ale `range`/`cone_angle` a per-player weapon selection stále fallbackují na starý `WeaponConfig`

**WeaponDef schéma:**

```lua
Weapon.Register('ak47', {
    display_name='AK-47', category='rifle',  -- pistol|smg|rifle|shotgun|lmg|sniper|launcher|melee
    caliber='7.62x39', default_ammo='7.62x39_fmj',
    barrel_length_mm=415, twist_rate_inches=9.45,
    fire_modes={'semi','full'}, default_fire_mode='full',
    rpm=600, fire_from_open_bolt=false,
    mag_capacity=30, reload_empty_sec=3.2, reload_tactical_sec=2.6,
    spread={ base=0.15, moving=0.45, sprinting=1.20, crouch=0.08, prone=0.04,
             ads=0.05, ads_moving=0.18, per_shot=0.06, recovery_rps=3.5 },
    recoil_pattern={ {x=0.00,y=0.28},{x=0.05,y=0.26},{x=-0.08,y=0.25} },
    recoil_loop_from=4, recoil_recovery_dps=8.0,
    ads_fov_mult=0.75, ads_time_sec=0.22,
    mass_kg=3.47, length_folded_mm=645, length_extended_mm=875,
    slots={ optic=true, muzzle=true, barrel=false, underbarrel=true,
            stock=true, magazine=true, pistol_grip=false },
    muzzle_thread='M14x1LH',
    pellet_count=1, choke='none',  -- pro shotguny: buckshot pellets, none|improved_cylinder|modified|full
})
```

**AmmoDef schéma:**

```lua
Ammo.Register('7.62x39_fmj', {
    display_name='7.62×39mm FMJ', caliber='7.62x39',
    bullet_mass_g=8.0, bullet_diameter_mm=7.92,
    muzzle_velocity_mps=715, reference_barrel_mm=415,
    velocity_per_mm=0.55,           -- Δm/s na mm nad/pod referenční délku hlavně
    ballistic_model='G7',           -- G1|G7|custom
    ballistic_coeff=0.255,
    base_damage=42.0, damage_velocity_ref_mps=715,
    -- škálování: dmg = base_damage * (impact_vel / damage_velocity_ref)^1.5
    penetration_class=3,            -- 1=FMJ nízký .. 6=API
    armor_penetration=0.45,         -- frakce poškození skrz brnění
    penetration_energy_j=2100,      -- J potřebné k průniku
    after_penetration_mult=0.65,    -- velocity po průniku
    fragmentation_vel_mps=600, fragmentation_factor=0.0,
    wound_mult=1.0,
    tracer=false, incendiary=false, explosive=false, subsonic=false,
    bleed_chance=0.0, bleed_dps=0.0,
    crack_range_m=400, thump_range_m=1500,
})
```

**AttachmentDef klíčová pole:** `slot`, `compatible_threads[]`, `velocity_mult`, `barrel_length_delta_mm`, `db_reduction`, `changes_sonic_signature`, `ads_time_delta_sec`, `recoil_mult`, `mass_kg`, `ads_fov_mult_override`, `zero_range_m`

**MaterialDef klíčová pole:** `thickness_mm`, `hardness_brinell`, `required_energy_j`, `velocity_retention`, `ricochet_angle_deg`

---

#### 5.2 — Balistický engine [ ]

- [ ] `BallisticsPlugin`, `BulletProjectile` (`pos`, `vel`, `energy_j`, `ammo_id`, `source_entity`, `spawn_tick`)
- [ ] `ProjectileSimulator` (drag G1/G7, gravity `v.y -= 9.81*dt`), `MuzzleVelocityCalc`, `PenetrationResolver`, `RicochetResolver`
- [ ] Hitscan: `v > 700 m/s` → raycast + lag comp; `≤700` nebo `explosive=true` → `BulletProjectile` entity
- [ ] Events: `onBulletImpact {shooter, pos, normal, material, velocity_mps, penetrated}`, `onBulletFlyby {listener, distance_m, velocity_mps}`

---

#### 5.3 — Hitbox & Hit Detection [ ]

- [X] `HitboxDef`, `HitboxRegistry`, `PlayerHitbox` foundation + built-in `player_default` profil
- [ ] `PositionHistory` (ring buffer pro lag comp)
- [ ] `LagCompensator` (rewind pozic na spawn_tick)
- [X] `HitResolver` foundation (kapslový nearest-hit resolver → `hitzone`, `headshot`, damage multiplier)
- [X] `Hitbox.Register(model_id, def)` Lua API

```lua
Hitbox.Register('player_default', {
    bones = {
        head      = { mult=4.0, armor_bypass=0.5,  capsule={r=0.12, hh=0.10, oy=1.75} },
        neck      = { mult=2.5, armor_bypass=0.8,  capsule={r=0.07, hh=0.06, oy=1.55} },
        chest     = { mult=1.0, armor_bypass=0.0,  capsule={r=0.22, hh=0.18, oy=1.30} },
        stomach   = { mult=0.9, armor_bypass=0.1,  capsule={r=0.18, hh=0.12, oy=1.05} },
        pelvis    = { mult=0.8, armor_bypass=0.2,  capsule={r=0.18, hh=0.10, oy=0.90} },
        upper_arm = { mult=0.7, armor_bypass=1.0,  capsule={r=0.07, hh=0.14, oy=1.35} },
        lower_arm = { mult=0.6, armor_bypass=1.0,  capsule={r=0.06, hh=0.12, oy=1.10} },
        upper_leg = { mult=0.75,armor_bypass=1.0,  capsule={r=0.09, hh=0.18, oy=0.60} },
        lower_leg = { mult=0.65,armor_bypass=1.0,  capsule={r=0.07, hh=0.18, oy=0.28} },
    },
    armor_zones = { helmet={bones={'head','neck'}}, vest={bones={'chest','stomach','pelvis'}} },
})
```

---

#### 5.4 — Weapon State [🟡 FOUNDATION READY]

- [X] `WeaponSlots` (4 sloty), `EquippedWeapon {weapon_id, ammo_in_mag, ammo_type_id, attachments}`, `AmmoReserve`, `ActiveSlot`
- [X] `ReloadState`, `WeaponSwapState` a `FireState` foundation (`remaining`, `duration`, pending slot, fire cooldown/trigger`) pro server player entities
- [ ] Plně časovaný `reload_system`, `fire_system` (FixedUpdate); reload a slot swap už běží přes timed server states a střelba už přešla na `FireState`, ale stále chybí bohatší `FireState` FSM (`Ready|Burst|Cooling`) a plná weapon-specific burst/full-auto semantics
- [X] Nové `PlayerInput` bity: `RELOAD` (2), `ADS` (12), `WEAPON_SLOT_1..4` (13–16)
- [X] Lua: `Weapon.GetEquipped/SetEquipped/GetAmmoReserve/SetAmmoReserve/GetActiveSlot/SetActiveSlot/ForceReload`

---

#### 5.5 — Armor & Damage Pipeline [ ]

- [ ] `ArmorComponent {helmet, vest: Option<ArmorPiece{class, durability, max_durability}>`
- [ ] `ArmorClass` (NIJ I/II/IIIa/III/IV), `absorption_table[class][pen_class]`, `ArmorDurabilitySystem`
- [ ] Damage pipeline: energy_j → armor check → `armor_absorption` / `after_penetration_mult` → hitzone mult → `wound_mult` → `health.current -= final`
- [ ] `BleedEffect {dps, remaining_sec}`, `BurnEffect`, `status_effect_system`
- [ ] `onPlayerDamage` event: `{attacker, victim, weapon, ammo, hitzone, final_damage, raw_damage, armor_absorbed, penetrated_armor, distance_m, impact_velocity_mps, headshot, through_wall}`
- [ ] Lua: `Player.GetArmor(pid)`, `Player.SetArmor(pid, slot, class, durability)`

---

#### 5.6 — Rozšířená fyzika hráče [ ]

- [ ] `PlayerStance` (Standing/Crouching/Prone/Vaulting), `LeanState`, `StaminaComponent {current,max}`, `AdsState`, `VaultState`
- [ ] `stance_system`, `lean_system`, `stamina_system`, `ads_system`, `vault_system` (FixedUpdate)
- [ ] Nové input bity: `LEAN_LEFT` (8), `LEAN_RIGHT` (9), `PRONE` (10), `VAULT` (11)
- [ ] Capsule výšky: Standing 1.80m / cam 1.65m, Crouching 1.10m / 0.95m, Prone 0.40m / 0.30m
- [ ] Stamina efekt na balistiku: <30% → spread +0.3°; 0% → nelze sprintovat, spread +0.8°

---

#### 5.7 — Recoil, Spread & ADS [ ]

- [ ] `RecoilAccumulator {offset: Vec2, shot_index}`, `recoil_system`, `spread_calculator` (stance+ADS+pohyb+stamina)
- [ ] `onRecoilUpdate` event → klient animuje vizuální zpětný ráz (odděleno od autoritativního)
- [ ] Shotgun pellet fan: `pellet_count > 1` → N hitscanů v choke kuželi

---

#### 5.8 — Audio Events Bridge [ ]

- [ ] `onGunfire {shooter, pos, dir, weapon, ammo, suppressed, muzzle_velocity_mps, crack_range_m, thump_range_m}`
- [ ] `onFootstep {entity, pos, surface, stance, speed}`, `onExplosion`, `onReload {player, weapon, type}`
- [ ] `FootstepDetector` systém (emituje každých N metrů pohybu)
- [ ] Lua resource `core/audio` zodpovídá za proximity routing → `TriggerClientEvent`

---

#### 5.9 — Kill Feed & Hit Confirmation [ ]

- [ ] `onHitConfirm` unicast: `{victim, hitzone, damage, kill, headshot}`
- [ ] `onPlayerKill` broadcast: `{killer, victim, weapon, ammo, headshot, distance_m, through_wall}`
- [ ] `KillStreak {current_streak, best_streak}`, `kill_streak_system`, `onKillStreak {player, streak}`

---

#### 5.10 — Spawn System [ ]

- [ ] `SpawnPoint {team, active}`, `RespawnTimer`, `onRespawnReady {player}` event
- [ ] Lua: `Spawn.Register/GetFree(team?)/SetActive/GetAll`

---

#### 5.11 — Round State & Game Mode [ ]

- [ ] `RoundState {WarmUp, Active{elapsed_sec}, PostRound{winner_team}}`, `round_timer_system`
- [ ] `TeamAssignment {team: u8}`, `ScoreBoard HashMap<u64, PlayerScore{kills,deaths,assists,score}>`
- [ ] Lua: `Round.GetState/SetTimeLimit/End`, `Player.GetTeam/SetTeam`, `Score.Add/Get/GetAll`
- [ ] Events: `onRoundStart`, `onRoundEnd {winner_team, scores}`, `onRoundTick {elapsed, remaining}`

---

#### Phase 5 — Nová Lua API (přehled)

| Funkce                                                  | Strana      | Popis                                           |
| ------------------------------------------------------- | ----------- | ----------------------------------------------- |
| `Weapon.Register/Get`                                 | both        | Runtime WeaponDef registry na aktuální straně   |
| `Weapon.GetEquipped/SetEquipped`                      | server      | Equipment hráče                               |
| `Weapon.GetAmmoReserve/SetAmmoReserve/GetActiveSlot/SetActiveSlot/GetFireMode/SetFireMode/ForceReload` | server | Munice + aktivní weapon slot + fire mode |
| `Ammo/Attachment/Material.Register/Get`               | both        | Runtime Ammo/Attachment/Material registry       |
| `Hitbox.Register(model,def)`                          | server      | Hitbox definition                               |
| `Player.GetArmor/SetArmor`                            | server      | Brnění                                        |
| `Player.GetTeam/SetTeam`                              | both/server | Team assignment                                 |
| `Player.GetStamina/GetStance`                         | server      | Fyzický stav                                   |
| `Spawn.Register/GetFree/SetActive/GetAll`             | server      | Spawn body                                      |
| `Round.GetState/SetTimeLimit/End`                     | both/server | Stav kola                                       |
| `Score.Add/Get/GetAll`                                | server/both | Scoreboard                                      |
| `Camera.Create/Delete`                                | client      | Vytvoř / smaž pojmenovanou kameru             |
| `Camera.SetActive/GetActive`                          | client      | Přepni aktivní kameru (nil = player kamera)   |
| `Camera.AttachToEntity/AttachToBone/AttachToPosition` | client      | Připoj kameru na entitu, kost nebo pozici      |
| `Camera.SetFOV`                                       | client      | Nastav FOV aktivní kamery (stupně)            |
| `Camera.SetMode/GetMode`                              | client      | `first_person` / `third_person` / custom_id |

---

## ADS — Known Limitations

**Bevy `FogVolume` jako local smoke/grenade efekt:** Aktuální Bevy 0.18 `FogVolume` pipeline se v runtime chová jako transformovaný box volume. Pro broad lokalizovanou mlhu nebo městský haze je to použitelné, ale pro přesvědčivý kouřový granát se i po vrstvení více objemů příliš snadno prozradí stěny/shelly volume. Ber to jako současný technický limit této cesty; pro grenade smoke preferuj částicový/billboard impostor efekt, sprite-based puffs nebo jiný specializovaný volumetric shader/resource místo dalšího ladění `fog_volume` dummy clusteru.

**Více materiálů na jednom mesh objektu:** GLTF exporter rozděluje na primitiva, `process_mesh_node` hledá jedno jméno materiálu. Řešení: rozděl mesh na více objektů v Blenderu (jeden materiál = jeden objekt). Fallback: použije první materiál z manifestu + warning.

**ADM import/export hierarchie:** Importér obnovuje parent-child strom 1:1 podle node sekce ADM (včetně bone parentingu) a implicitně neslučuje multi-material split objekty.

---

## Project Layout

```text
/Cargo.toml                    workspace root
/blender_plugin/bevy_toolkit.py  ADS exporter, Blender UI, COL_*/NAVMESH workflow, map TOML
/core_shared/src/lib.rs          SharedPlugin, LuaEvent, LuaEventRegistry
/core_resources/src/
  types.rs                       ResourceId, Side
  manifest.rs                    Manifest, parse_manifest
  vfs.rs                         Vfs, walkdir scanner, scan_stream_models()
  watcher.rs                     notify watcher + debounce, ResourcesDirty
  resolver.rs                    resolve_load_order (Kahn's)
  sandbox.rs                     LuaSandbox, LocalEventBus, RaycastBridge, CameraBridge, JSON helpers
  plugin.rs                      ResourcesPlugin, SandboxRegistry (NonSend)
  cmd_queue.rs                   LuaCommand, CommandQueue, LuaWorldState, process_lua_commands
  model_registry.rs              ModelRegistry, process_model_commands
  weapons.rs                     Weapon/Ammo/Attachment/Material runtime registry definitions
/core_net/src/
  protocol.rs                    ServerHello, ClientReady, LuaEventMessage
  digest.rs + digest_cache.rs    blake3 digesty, DigestPlugin
  net_plugin.rs                  ProtocolPlugin, ServerNetPlugin, ClientNetPlugin
  handshake.rs                   Server/ClientHandshakePlugin
  lua_rpc.rs                     Server/ClientLuaRpcPlugin
  sim.rs                         ServerSimPlugin, Health, WeaponConfig, combat systémy
/host_server/src/
  main.rs                        MinimalPlugins + Tokio + server plugins
  http_server.rs                 Axum file server (/resources/<id>/<path>)
  config.rs                      ServerConfig (server.toml)
/server.toml                   default server config
/core_drawable/src/
  lib.rs                         DrawablePlugin
  map.rs                         MapManifest, MapInstanceDef
  manifest.rs                    DrawableManifest, CollisionShape + metadata (climbable/ladder/material/lock_*)
  loader.rs                      DrawableManifestLoader (ext=.drawable)
  registry.rs                    DrawableManifestRegistry, TextureRegistry
  material.rs                    DrawableMaterial, shader extensions (standard_pbr/layered_env/vehicle_glass)
  hook.rs                        DrawableSpawnIntent, DrawableCollision, hook systémy, LOD systém
  navmesh.rs                     NavmeshRegistry, A* pathfinding, single-tile routing
  tile_pathfinding.rs            TileGraph, TilePortal, hierarchical cross-tile pathfinding
  hlod.rs                        HLODLayer, HLODState, HLODTile, distance-based visibility gating
/model_viewer/src/
  main.rs                        ADS model viewer (CLI args), grid gizmos, ADM dict browser + clip overlay
  camera.rs                      OrbitCamera (orbit/pan/zoom)
/map_viewer/
  Cargo.toml                     Standalone mapa/collider/navmesh viewer
  src/
    main.rs                      Načítá `map.toml` nebo `world.index.toml`, spawnuje map instance a kreslí collider/navmesh overlay
    camera.rs                    Orbit kamera pro inspekci mapy
/host_client/src/
  main.rs                        DefaultPlugins + client plugins
  config.rs                      ClientConfig (client.toml)
  gameplay.rs                    ClientGameplayPlugin, input, raycast bridge, camera follow
  physics.rs                     ClientPhysicsPlugin, Avian, StaticWorldCollider, NavMeshSurfaceCache
  map_loader.rs                  ClientMapPlugin (assets/maps/*.map.toml)
  native_assets.rs               NativeAssetsPlugin (fonts + models scan)
  assets/shaders/                WGSL shadery (sdíleny s model_viewerem)
/cache/resources/              klientská cache (gitignored)
/resources/
  core/init/                     bootstrap resource
  example/hello/                 demo resource
  example/env_cycle/             demo synchronniho env light/time rizeni pres core/init
  example/env_admin/             test resource pro manualni synchronized env preset/time ovladani
  example/smoke_grenade/         client-side demo localniho fog_volume koure na ground hitu
  example/shader_override_test/  client-side demo resource pro per-entitni shader profile na drawable materialu
  example/anim_notify_test/      test onAnimNotify + crossfade pipeline
  example/blend_space_test/      test PlayBlendSpace (Lua API + infrastruktura)
  example/ik_test/               test IK solver (infrastruktura)
  example/npc_test/              test NPC AI (wander + go-to coord/entity)
  example/root_motion_test/      test root motion — extrakce delty z animací
  example/moving_square/         demo pohybu + input.lua
```

---

## Lua Sandbox Runtime API

Každý resource = vlastní izolovaná `mlua::Lua` instance. **Sandbox isolation:** žádné shared globals — pouze event bus. `SandboxRegistry` je `NonSend` (main thread, `mlua::Lua` je `!Send`).

**Stdlib povolen:** `string`, `table`, `math`, `utf8`, `coroutine`. **Zakázán:** `io`, `os`, `package`, `require`, `debug`, `load*`.

| Symbol                                                                                                                    | Strana      | Popis                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `RESOURCE_ID`, `SIDE`, `IS_SERVER`, `IS_CLIENT`                                                                   | both        | Identita resource                                                                                                                                                                                                                                                                                                                                 |
| `print(...)`, `log_debug/info/warn(s)`                                                                                | both        | Logování                                                                                                                                                                                                                                                                                                                                        |
| `RegisterEvent(name, fn)`                                                                                               | both        | Callback; handler dostane `(payload, sender_id?)`                                                                                                                                                                                                                                                                                               |
| `TriggerServerEvent(name, payload?)`                                                                                    | client      | Pošle event serveru                                                                                                                                                                                                                                                                                                                              |
| `TriggerClientEvent(name, target, payload?)`                                                                            | server      | Unicast (u64/string) nebo broadcast (nil/false)                                                                                                                                                                                                                                                                                                   |
| `TriggerEvent(name, payload?)`                                                                                          | both        | Cross-sandbox bus (in-process)                                                                                                                                                                                                                                                                                                                    |
| `World.SpawnLocalObject(model, pos, rot)`                                                                               | both        | Lokální entita → handle (u64)                                                                                                                                                                                                                                                                                                                  |
| `World.SpawnNetworkedObject(model, pos, rot)`                                                                           | server      | Replikovaná entita → handle                                                                                                                                                                                                                                                                                                                     |
| `World.SpawnNetworkedNpc(model, pos, rot, ped_profile?)`                                                                | server      | Replikované NPC (NPC marker + klientský capsule collider setup, volitelně s explicitním ped profilem, např. "player" nebo "monster")                                                                                                                                                                                                         |
| `World.SpawnLocalDummy(shape, params, pos, rot)`                                                                        | both        | Parametrický dummy objekt (`cuboid/sphere/cube/stairs/arch/point_light/spot_light/directional_light/fog_volume`), volitelně i s `light={...}` nebo `fog_volume={...}` blokem                                                                                                                                                              |
| `World.SpawnNetworkedDummy(shape, params, pos, rot)`                                                                    | server      | Replikovaný parametrický dummy objekt; světelné i fog-volume aliasy podporované stejně jako u lokální varianty                                                                                                                                                                                                                            |
| `World.SpawnLocalCollider(params, pos, rot)`                                                                            | both        | Samostatný collider bez render meshe (`shape/size/radius/height/is_static/is_trigger/stairs/stairs_slope_invert/stairs_clearance_y`)                                                                                                                                                                                                           |
| `World.SpawnNetworkedCollider(params, pos, rot)`                                                                        | server      | Replikovaný samostatný collider bez render meshe                                                                                                                                                                                                                                                                                                |
| `World.DeleteObject(handle)`                                                                                            | both        | Despawn                                                                                                                                                                                                                                                                                                                                           |
| `World.SetTransform/SetPosition/SetRotation/SetScale/SetModel`                                                          | both        | Transformace                                                                                                                                                                                                                                                                                                                                      |
| `World.PlayAnimation(h, name, blend?)` nebo `World.PlayAnimation(h, name, loop?, speed?, blend?)` / `StopAnimation` | both        | Animace (`name` podporuje `clip:N`/`anim:N`/`N`; GLTF = clip index, ADM = clip index nebo clip name; ADM v5 navíc `dict:<dict_name>:<clip_name>`)                                                                                                                                                                                      |
| `World.Attach(child, child_socket, parent, parent_socket)` / `World.Detach(child)`                                    | both        | Socket-to-socket attachment                                                                                                                                                                                                                                                                                                                       |
| `World.AttachWithOffset(child, parent, offset, rot)`                                                                    | both        | Parent attachment přes lokální offset od pivotu parent entity                                                                                                                                                                                                                                                                                  |
| `World.GetSocketTransform(handle, socket)`                                                                              | both        | World-space socket transform                                                                                                                                                                                                                                                                                                                      |
| `World.IsValid/IsAlive/GetHealth/GetModel`                                                                              | both        | State dotazy                                                                                                                                                                                                                                                                                                                                      |
| `World.GetPosition/Rotation/Quaternion/Scale/Transform/Animation/AnimationSpeed`                                        | both        | Gettery                                                                                                                                                                                                                                                                                                                                           |
| `World.NpcConfigure(handle, opts)`                                                                                      | both/server | Nastaví NPC parametry (`move_speed`, `arrive_distance`, `turn_speed`)                                                                                                                                                                                                                                                                      |
| `World.NpcConfigureScenarioClock(opts)`                                                                                 | server      | Runtime patch serverového scenario clocku (`auto_advance`, `day_length_seconds`)                                                                                                                                                                                                                                                             |
| `World.NpcConfigurePopulationDirector(opts)`                                                                            | server      | Runtime patch population directoru (`default_assignment_radius`, `release_distance_multiplier`, `default_release_distance`)                                                                                                                                                                                                                 |
| `World.NpcConfigureAiLod(opts)`                                                                                         | server      | Runtime patch AI LOD budgetů a radiusů (`full/reduced radius`, cadence, player/zone budgets)                                                                                                                                                                                                                                                  |
| `World.ConfigureEnvironmentLight(opts)`                                                                                 | client      | Runtime patch globálního env světla a boundary fogu (`enabled`, `hour_of_day`, `azimuth_deg`, `max_elevation_deg`, `color`, `illuminance`, `shadows`, ambient params, `fog={ enabled, color, directional_color, directional_exponent, start, end, volumetric_enabled, ambient_color, ambient_intensity, jitter, step_count }`) |
| `World.SetEnvironmentTime(hour)`                                                                                        | client      | Nastaví hour-of-day pro env light bez přepisování ostatních parametrů                                                                                                                                                                                                                                                                       |
| `World.SetEntityShaderProfile(handle, profile)` / `World.ClearEntityShaderProfile(handle)`                            | both/client | Nastaví nebo vymaže per-entitní shader profile na drawable materiálu; aktuálně `standard_pbr` podporuje `debug_stripes`, `hologram`, `heat`, `dissolve`                                                                                                                                                                           |
| `World.NpcWander(handle, kind, opts)`                                                                                   | both/server | Wander módy:`random`, `patrol`, `orbit`                                                                                                                                                                                                                                                                                                    |
| `World.NpcGoToEntity(handle, target, stop?)` / `World.NpcGoToCoord(handle, pos, stop?)` / `World.NpcStop(handle)`   | both/server | Přímé AI movement příkazy                                                                                                                                                                                                                                                                                                                    |
| `World.NpcSetBrain(handle, id)` / `World.NpcRegisterBrain(id, def)`                                                   | both/server | Runtime změna/registrace brain profilu bez rebuildu                                                                                                                                                                                                                                                                                              |
| `World.NpcSetTask(handle, task, opts?)` / `World.NpcSetScenario(handle, scenario_id, opts?)`                          | both/server | High-level scenario/task kontrakt; replikuje se jako `ReplicatedNpcBrain` a lokálně se interpretuje na `NpcMoveGoal`                                                                                                                                                                                                                        |

Poznámka: Pro AI postavy používej `World.SpawnNetworkedNpc(...)` místo `World.SpawnNetworkedObject(...)`, aby klientská vrstva vytvořila správný capsule collider pro NPC. Pokud potřebuješ specifický fyzikální profil (např. pro létající monstrum), zadej čtvrtý parametr `ped_profile` (název bez přípony, např. "monster").
| `World.ApplyDamage(target, amount, source?)` | server | Damage intent |
| `Engine.RequestModel/HasModelLoaded/SetModelAsNoLongerNeeded` | both | Model ref-counting |
| `Engine.SetDrawableShaderOverride/ClearDrawableShaderOverride/GetDrawableShaderOverride` | client | Přepne WGSL shader pro existující drawable template (`standard_pbr`/`layered_env`/`vehicle_glass`) na `.wgsl` soubor z aktuálního resource |
| `Engine.GetModelClipCount/GetModelClipNames` | both | Počet a názvy animačních clipů modelu |
| `Engine.RequestAnimDict(model, dict)` / `Engine.HasAnimDictLoaded(model)` | both | Request/load kontrola dictionary (reuse model load pipeline) |
| `Engine.GetAnimDictNames(model)` / `Engine.GetAnimDictClips(model, dict)` | both | Dostupné dictionary a clipy pro konkrétní model |
| `Raycast.GetGroundPosition()` | client | World-pos kurzoru (Y=0 rovina) |
| `Gui.DrawRect/Text/Line/Circle/Disc/RoundedRect/Border/Shadow/Sprite` | client | Immediate-mode GUI (0–1 souřadnice) |
| `Gui.GetCursorPos/IsMouseOver/IsMouseDown/IsMouseClicked` | client | GUI input |
| `UI.Window(opts)` | client | Menu framework (fade, tlačítka, labely) |
| `CreateThread(fn)` / `Wait(ms)` | both | Lua coroutiny |
| `Camera.Create(id, opts?)` | client | Vytvoří pojmenovanou kameru; `opts.fov` (stupně); vrátí `id` |
| `Camera.Delete(id)` | client | Smaže kameru (pokud aktivní → reset na player kameru) |
| `Camera.SetActive(id\|nil)` | client | Přepne aktivní kameru; `nil` = player kamera |
| `Camera.GetActive()` | client | ID aktivní custom kamery nebo `nil` |
| `Camera.AttachToEntity(id, handle, offset?, look_at?)` | client | Sleduje entitu; `look_at=true` = dívá se na ni |
| `Camera.AttachToBone(id, handle, bone_name, offset?)` | client | Připojí na kost (dědí rotaci kosti) |
| `Camera.AttachToPosition(id, pos, look_at?)` | client | Statická pozice + volitelný lookAt bod |
| `Camera.SetFOV(id, degrees)` | client | Nastaví FOV kamery (stupně) |
| `Camera.SetMode(mode)` | client | `"first_person"` nebo `"third_person"` |
| `Camera.GetMode()` | client | `"first_person"` / `"third_person"` / custom camera id |

**input:state** (client-only local event): `{ move={x,y}, keys={...} }` — emitován každý frame přes `LocalEventBus`.

**stairs:state** (client-only local event): `{ on_stairs, reacting, grounded, hit_distance, hit_pos={x,y,z}|nil, ik={quality,sample_hz,sampled_this_frame,left_foot_y,right_foot_y}, player={x,y,z,vy} }` — emitován každý frame pro debug detekce `StairsCollider` pod lokálním hráčem a adaptivní IK sampling.

`payload` = libovolná Lua hodnota, serializována jako JSON. `TriggerClientEvent` target podporuje integer i string.

---

## Client Config — `client.toml`

Generovaný při prvním spuštění: Win `%APPDATA%\bevy_game\client.toml`, Linux `~/.config/bevy_game/client.toml`.

| Sekce                                              | Co řídí                                                                |
| -------------------------------------------------- | ------------------------------------------------------------------------- |
| `[player]`                                       | name, saved_client_id, avatar                                             |
| `[network]`                                      | server, bind, download_concurrency, timeouty                              |
| `[graphics]` / `[graphics.quality]`            | backend (auto/vulkan/dx12/...), resolution, vsync, shadow/AA/LOD/SSAO/SSR |
| `[audio]`                                        | master + 5 kanálů, spatial audio, mute on focus lost                    |
| `[ui]`                                           | jazyk, HUD opacity, crosshair, FPS/ping/minimap                           |
| `[input]` / `[input.keys]` / `[input.mouse]` | sensitivity, invert Y, raw input, 39 keybindings                          |
| `[paths]`                                        | cache/screenshot/savegame/log dir overridy                                |
| `[advanced]`                                     | log level, GPU validation, dev console, preload toggle                    |

## Server Config — `server.toml`

Hledán: CLI arg → `<exe_dir>/server.toml` → `<cwd>/server.toml`. Relativní cesty: nejdřív vedle `.exe`, pak CWD.

| Sekce           | Co řídí                                        |
| --------------- | ------------------------------------------------- |
| `[server]`    | display name, MOTD, tagy                          |
| `[gameplay]`  | max_players, gamemode, idle_kick_sec              |
| `[net]`       | UDP/HTTP bind, tickrate, protocol_id, klíč      |
| `[resources]` | VFS root, hot_reload, debounce                    |
| `[auth]`      | `mode = "open"/"token"/"whitelist"`             |
| `[database]`  | sqlx connection string, pool size (Phase 4)       |
| `[dev]`       | auto_acknowledge_clients, print_digest_on_startup |

`deny_unknown_fields` — překlepy pukají při startu.

---

## Cargo Commands

```powershell
cargo run -p host_server
cargo run -p host_client
cargo run -p host_server --features dynamic_linking   # rychlejší rebuild
cargo run -p host_client --features dynamic_linking
cargo run -p host_server --release
cargo run -p host_client --release
cargo check --workspace
cargo run -p model_viewer -- path/to/model.glb        # ADS model viewer
cargo run -p map_viewer -- host_client\assets\maps\world.index.toml --features dynamic_linking
```

**Porty:** UDP 5000 (lightyear), TCP 8081 (Axum HTTP). Default server: `127.0.0.1:5000` / `http://127.0.0.1:8081`.
