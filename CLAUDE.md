# Ruflo — Claude Code Configuration

## Rules

- Do what has been asked; nothing more, nothing less
- NEVER create files unless absolutely necessary — prefer editing existing files
- NEVER create documentation files unless explicitly requested
- NEVER save working files or tests to root — use `/src`, `/tests`, `/docs`, `/config`, `/scripts`
- ALWAYS read a file before editing it
- NEVER commit secrets, credentials, or .env files
- Keep files under 500 lines
- Validate input at system boundaries

## Agent Comms (SendMessage-First Coordination)

Named agents coordinate via `SendMessage`, not polling or shared state.

```
Lead (you) ←→ architect ←→ developer ←→ tester ←→ reviewer
              (named agents message each other directly)
```

### Spawning a Coordinated Team

```javascript
// ALL agents in ONE message, each knows WHO to message next
Agent({ prompt: "Research the codebase. SendMessage findings to 'architect'.",
  subagent_type: "researcher", name: "researcher", run_in_background: true })
Agent({ prompt: "Wait for 'researcher'. Design solution. SendMessage to 'coder'.",
  subagent_type: "system-architect", name: "architect", run_in_background: true })
Agent({ prompt: "Wait for 'architect'. Implement it. SendMessage to 'tester'.",
  subagent_type: "coder", name: "coder", run_in_background: true })
Agent({ prompt: "Wait for 'coder'. Write tests. SendMessage results to 'reviewer'.",
  subagent_type: "tester", name: "tester", run_in_background: true })
Agent({ prompt: "Wait for 'tester'. Review code quality and security.",
  subagent_type: "reviewer", name: "reviewer", run_in_background: true })

// Kick off the pipeline
SendMessage({ to: "researcher", summary: "Start", message: "[task context]" })
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

- [X] **2026-05-18 (Client NPC Ownership + Physics Slice Refactor)**: `host_client::gameplay::npc` byl rozdělen na specializované podmoduly `host_client/src/gameplay/npc/attach.rs`, `ownership.rs`, `network.rs`, `physics.rs`, `animation.rs` s orchestrací v `host_client/src/gameplay/npc/mod.rs`. Praktický dopad: ownership lifecycle (`bootstrap_owned_npc_agents`, `cleanup_unowned_npc_agents`), client-owned transform sync, avoidance + terrain snap fyzika a animation driver už nejsou v jednom dlouhém souboru a změny NPC fyziky/owner flow mají cílené místo.
- [X] **2026-05-18 (Server NPC Owner + Movement Runtime Split)**: Serverová ownership/LOD a NPC pohybová smyčka byly vytaženy z `core_resources/src/cmd_queue.rs` do nového modulu `core_resources/src/npc_runtime.rs` (`assign_npc_owners`, `tick_npc_agents` + LOD helpery). `cmd_queue.rs` zůstává zaměřený na command processing a datové kontrakty, zatímco owner rozhodování a NPC movement runtime žije v dedikované runtime vrstvě.
- [X] **2026-05-18 (Server Sim Combat + NPC Transform + Stats Slice Refactor)**: Monolit `core_net::sim` byl dál rozdělen o tři behaviorální bloky. Combat resolver a všechny jeho helpery (`resolve_combat_weapon_config`, hitbox/armor výpočty, `process_combat`) jsou nově v `core_net/src/sim/combat.rs`; příjem klient-owner NPC transformů (`receive_npc_transform_updates`) je vytažen do `core_net/src/sim/npc.rs`; player stats broadcast + cache sync (`broadcast_player_stats`, `sync_player_state_cache`) jsou přesunuty do `core_net/src/sim/stats.rs`. Praktický dopad: `core_net/src/sim.rs` teď funguje primárně jako orchestrace pluginu a registrace systémů, zatímco combat/NPC/stats maintenance probíhá v cílených modulech podle domény.
- [X] **2026-05-17 (Server Sim Lifecycle Slice Refactor)**: Další blok serverového monolitu `core_net::sim` byl vytažen do `core_net/src/sim/lifecycle.rs`. Nový modul vlastní observer a replication lifecycle helpery `attach_replication_sender`, `spawn_player_on_connect`, `emit_player_disconnect` a `attach_replication_to_networked_object`. Praktický dopad: connection/bootstrap a replication-wiring už neleží mezi movement/cache/combat helpery v jednom souboru a `sim.rs` se dál blíží čisté orchestrace.
- [X] **2026-05-17 (Server Sim Player State Slice Refactor)**: `core_net::sim` se dál rozpadl o player-state/runtime cache blok. `PositionHistory`, `ServerSimulationTick`, `LastPlayerInputs`, `collect_last_inputs`, `trust_client_position`, `increment_server_simulation_tick`, `record_position_history` a `emit_player_positions` byly vytaženy do `core_net/src/sim/players.rs`, přičemž veřejný export `LastPlayerInputs` a `collect_last_inputs` zůstal zachovaný přes `core_net::sim`. Praktický dopad: input cache, trusted transform flow a lag-comp prep/history už mají jedno vlastnické místo mimo hlavní server sim monolit.
- [X] **2026-05-17 (Server Sim Weapon State Slice Refactor)**: Refaktor se posunul i do serverového monolitu `core_net::sim`. Weapon-state runtime a helpery (`tick_fire_states`, `tick_weapon_swap_states`, `tick_reload_states`, `reload_active_weapon`, `requested_weapon_slot`, `weapon_swap_duration`, `reload_duration_for_slot`, `is_reload_active`, `is_weapon_swap_active`) byly vytaženy do `core_net/src/sim/weapons.rs`. Praktický dopad: časovaný reload/swap/fire flow už má vlastní cílový soubor a `core_net/src/sim.rs` se může dál ztenčovat směrem k orchestrace + combat resolveru místo jedné velké směsi všech server gameplay cest.
- [X] **2026-05-17 (Client Physics Stairs + Collider + Navmesh + Attach Slice Refactor)**: Refaktor pokračoval i mimo `host_client::gameplay`. Z `host_client/src/physics.rs` byl stairs collider/raycast slice (`DummyStairsIkSurface`, generated stairs collider cleanup/build, `raycast_stairs_under_player`) vytažen do `host_client/src/physics/stairs.rs`, sada čistých collider helperů (`has_rigidbody_ancestor`, `topmost_ancestor`, `dummy_collider_defaults`, `collider_from_dummy_def`, `locked_axes_from_drawable`, `collider_spec_from_drawable`, `ColliderSpec`) do `host_client/src/physics/colliders.rs`, navmesh + collision-enable runtime (`NavMeshTriangle`, `NavMeshSurfaceCache`, `rebuild_navmesh_surface_cache`, `apply_collision_enabled`) do `host_client/src/physics/navmesh.rs` a `host_client/src/physics/collision_toggle.rs`, a samotné collider attach systémy (`attach_or_update_drawable_colliders`, `attach_or_update_dummy_colliders`, `attach_or_update_collider_objects`) do `host_client/src/physics/attach.rs`. Praktický dopad: `physics.rs` už zůstal hlavně jako plugin orchestrace s debug wiringem a marker typy místo dalšího behaviorálního monolitu.
- [X] **2026-05-17 (Client Gameplay Visuals + Bridge + Environment + Stairs Slice Refactor)**: `host_client::gameplay` se dál ztenčil o render/bridge/environment/stairs kusy, které dřív zůstávaly v orchestration vrstvě. Player/local/dummy visual attach flow (`attach_player_model_to_new_players`, `sync_net_transform_to_render`, predicted-visual preference, local visibility, crosshair entity resolve, local/dummy mesh attach včetně `FogVolume`) byl vytažen do `host_client/src/gameplay/visuals.rs`; input/connection/engine bridge polling (`update_input_bridge`, `update_connection_bridge`, `reset_connection_bridge`, `reset_engine_state`, `handle_engine_cmds`) do `host_client/src/gameplay/bridge.rs`; environment light + volumetric fog apply/debug (`apply_environment_light`, `publish_volumetric_fog_state_to_lua`, marker/debug resource) do `host_client/src/gameplay/environment.rs`; stairs/IK/terrain-snap debug a runtime (`publish_stairs_state_to_lua`, `apply_local_stairs_foot_ik`, `terrain_snap_kinematic`, adaptive IK cache/bone state) do `host_client/src/gameplay/stairs.rs`. Praktický dopad: `gameplay.rs` už zůstal prakticky jen jako plugin orchestrace s malými shared helpery místo dalšího behaviorálního monolitu.
- [X] **2026-05-17 (Client Gameplay Camera + Animation + NPC Slice Refactor)**: Modularizace `host_client::gameplay` pokračovala za hranici movementu. Camera stack (`setup_scene_and_camera`, mouse look, cursor mode, camera follow, raycast bridge) byl vytažen do `host_client/src/gameplay/camera.rs`; player locomotion animation state/registry/Lua animation apply do `host_client/src/gameplay/animation.rs`; client-side NPC lifecycle (visual/capsule attach, ownership bootstrap, transform sync, owned-NPC avoidance/snap/animace) do `host_client/src/gameplay/npc.rs`. Praktický dopad: úpravy kamery, player animací nebo client NPC flow už nevedou do jednoho velkého `gameplay.rs`, ale do samostatných modulů podle domény.
- [X] **2026-05-17 (Client Gameplay Movement Slice Refactor)**: `host_client::gameplay` už není jediný monolit pro player movement/input. Movement fyzika (`apply_player_movement`, ground-contact helpery, post-physics Y damping), síťový input collect/send a Lua `input:state` bridge byly vytaženy do nového modulu `host_client/src/gameplay/movement.rs`, zatímco `host_client/src/gameplay.rs` zůstal jako orchestrace pluginu a ostatních gameplay systémů. Praktický dopad: úpravy pohybu hráče teď vedou primárně do jednoho cílového souboru místo hledání v několika tisících řádcích `gameplay.rs`.
- [X] **2026-05-15 (Arena MVP Equip Payload + Camera Height + NPC Visual Ground Snap Fix)**: `EquippedWeapon.attachments` má nově `#[serde(default)]`, takže server-side Lua `Weapon.SetEquipped(...)` už nespadne, když resource vynechá prázdné attachments pole; `resources/modes/arena_mvp/server/main.lua` tomu odpovídá a přestal posílat prázdnou mapu místo sekvence. `host_client::update_camera_follow()` zároveň už nenuluje `Y` lokálního predicted hráče a používá `ped.capsule.eye_height`, takže kamera skutečně sleduje výšku postavy/terénu. `host_client::sync_npc_net_transform()` navíc po síťové synchronizaci provádí stejný raycast-based ground snap i pro neowned NPC, takže zombie vizuálně neleží/nelétají mimo zem jen kvůli serverem poslanému nesnapnutému `NetTransform.y`. Pro samotný arena_mvp mód byly současně zombie spawn body v `resources/modes/arena_mvp/shared/config.lua` srovnány z `y=1.25` na ground plane `y=0.0`, protože elevovaný spawn byl vhodný pro player spawn/drop flow, ale pro NPC bez gravity vedl k perzistentnímu floatingu hned od vzniku. `resources/modes/arena_mvp/server/main.lua` navíc nově čte `onPlayerJoin` kompatibilně z payloadu `{ id, entity }`, takže join bootstrap už neposílá equip/health/ammo příkazy na falešný `player_id = 0`.
- [X] **2026-05-13 (Join Event Compatibility Fix)**: `core_net::emit_player_connect()` teď po server spawn hráče neposílá jen `playerConnecting`, ale i alias `onPlayerJoin` se stejným payloadem. Tím se opravilo, že resources spoléhající na dokumentovaný join event (`core/init`, `modes/arena_mvp`, další demo resources) dřív vůbec nechytly příchod hráče, takže se nespouštěl team assignment, spawn/equip flow ani initial per-player bootstrap.
- [X] **2026-05-13 (Arena MVP Gamemode + Server Resource Filter)**: Přidán nový root resource `resources/modes/arena_mvp/` jako malé 8-player PvP MVP: dvě auto-balancované teamy (`alpha`/`bravo`), de_dust-like arena poskládaná čistě z networked dummy objektů, timed respawn po smrti, server-side score tracking a dvouzbraňový loadout (`arena_rifle` + `arena_pistol`). Mód teď zároveň automaticky doplňuje lobby do 8 slotů zombie NPC boty: spawnují se do středu mapy, periodicky retargetují nejbližšího živého hráče a při kontaktu udělují damage/killují hráče s následným respawnem. Zároveň přidán `core_resources::ResourceLoadFilter` napojený na `server.toml [gameplay].gamemode`, takže server i handshake/digest pipeline teď umí načítat a inzerovat jen kořenový gamemode resource a jeho dependency closure místo všech demo resources najednou. Pro respawn flow přibylo i server-side Lua API `Player.SetHealth(player_id, current, max?)`.
- [X] **2026-05-13 (Armor Component + Basic Damage Absorption Foundation)**: `core_resources` teď obsahuje sdílené `ArmorClass`, `ArmorPiece` a `ArmorComponent { helmet, vest }`, nové server-side Lua API `Player.GetArmor/SetArmor`, a `StatsSnapshot`/`PlayerStatsUpdate` nově přenášejí i armor snapshot do `Player.GetLocalStats()`. `core_net::process_combat()` zároveň už respektuje `HitboxDef.armor_zones`, `HitboxBoneDef.armor_bypass` a ammo `penetration_class`/`armor_penetration`: zásahy do `helmet`/`vest` teď absorbují část damage, ubírají durability a event `onPlayerDamage` nově nese `armor_absorbed` a `penetrated_armor`. Zatím jde o jednoduchou foundation, ne finální NIJ absorption table ani plný status-effect pipeline.
- [X] **2026-05-13 (Lua Weapon API Uses Timed Server States)**: Server-side `Weapon.SetActiveSlot` a `Weapon.ForceReload` už neprovádějí instantní mutaci aktivního slotu nebo zásobníku bokem přes `cmd_queue`. Obě cesty teď plánují stejný `WeaponSwapState` / `ReloadState` jako standardní input flow a resetují `FireState`, takže resource-side weapon kontrola už neobchází autoritativní timed server state.
- [X] **2026-05-13 (Position History + Coarse Rewind Foundation)**: `core_net` teď na každém server player entity drží `PositionHistory` ring buffer recentních fixed-tick pozic a používá ho v combat resolveru pro coarse rewind cílů před hitbox testem. Aktuální foundation zatím používá krátký fixní rewind window místo RTT-aware lag compensation, ale hit detection už není čistě „jen poslední aktuální transform“ a technický základ pro plný `LagCompensator` je připravený.
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

### Patterns

| Pattern | Flow | Use When |
|---------|------|----------|
| **Pipeline** | A → B → C → D | Sequential dependencies (feature dev) |
| **Fan-out** | Lead → A, B, C → Lead | Independent parallel work (research) |
| **Supervisor** | Lead ↔ workers | Ongoing coordination (complex refactor) |

### Rules

- ALWAYS name agents — `name: "role"` makes them addressable
- ALWAYS include comms instructions in prompts — who to message, what to send
- Spawn ALL agents in ONE message with `run_in_background: true`
- After spawning: STOP, tell user what's running, wait for results
- NEVER poll status — agents message back or complete automatically

## Swarm & Routing

### Config
- **Topology**: hierarchical-mesh (anti-drift)
- **Max Agents**: 15
- **Memory**: hybrid
- **HNSW**: Enabled
- **Neural**: Enabled

```bash
npx @claude-flow/cli@latest swarm init --topology hierarchical --max-agents 8 --strategy specialized
```

### Agent Routing

| Task | Agents | Topology |
|------|--------|----------|
| Bug Fix | researcher, coder, tester | hierarchical |
| Feature | architect, coder, tester, reviewer | hierarchical |
| Refactor | architect, coder, reviewer | hierarchical |
| Performance | perf-engineer, coder | hierarchical |
| Security | security-architect, auditor | hierarchical |

### When to Swarm
- **YES**: 3+ files, new features, cross-module refactoring, API changes, security, performance
- **NO**: single file edits, 1-2 line fixes, docs updates, config changes, questions

### 3-Tier Model Routing

| Tier | Handler | Use Cases |
|------|---------|-----------|
| 1 | Agent Booster (WASM) | Simple transforms — skip LLM, use Edit directly |
| 2 | Haiku | Simple tasks, low complexity |
| 3 | Sonnet/Opus | Architecture, security, complex reasoning |

## Memory & Learning

### Before Any Task
```bash
npx @claude-flow/cli@latest memory search --query "[task keywords]" --namespace patterns
npx @claude-flow/cli@latest hooks route --task "[task description]"
```

### After Success
```bash
npx @claude-flow/cli@latest memory store --namespace patterns --key "[name]" --value "[what worked]"
npx @claude-flow/cli@latest hooks post-task --task-id "[id]" --success true --store-results true
```

### MCP Tools (use `ToolSearch("keyword")` to discover)

| Category | Key Tools |
|----------|-----------|
| **Memory** | `memory_store`, `memory_search`, `memory_search_unified` |
| **Bridge** | `memory_import_claude`, `memory_bridge_status` |
| **Swarm** | `swarm_init`, `swarm_status`, `swarm_health` |
| **Agents** | `agent_spawn`, `agent_list`, `agent_status` |
| **Hooks** | `hooks_route`, `hooks_post-task`, `hooks_worker-dispatch` |
| **Security** | `aidefence_scan`, `aidefence_is_safe`, `aidefence_has_pii` |
| **Hive-Mind** | `hive-mind_init`, `hive-mind_consensus`, `hive-mind_spawn` |

### Background Workers

| Worker | When |
|--------|------|
| `audit` | After security changes |
| `optimize` | After performance work |
| `testgaps` | After adding features |
| `map` | Every 5+ file changes |
| `document` | After API changes |

```bash
npx @claude-flow/cli@latest hooks worker dispatch --trigger audit
```

## Agents

**Core**: `coder`, `reviewer`, `tester`, `planner`, `researcher`
**Architecture**: `system-architect`, `backend-dev`, `mobile-dev`
**Security**: `security-architect`, `security-auditor`
**Performance**: `performance-engineer`, `perf-analyzer`
**Coordination**: `hierarchical-coordinator`, `mesh-coordinator`, `adaptive-coordinator`
**GitHub**: `pr-manager`, `code-review-swarm`, `issue-tracker`, `release-manager`

Any string works as a custom agent type.

## Build & Test

- ALWAYS run tests after code changes
- ALWAYS verify build succeeds before committing

```bash
npm run build && npm test
```

## CLI Quick Reference

```bash
npx @claude-flow/cli@latest init --wizard           # Setup
npx @claude-flow/cli@latest swarm init --v3-mode     # Start swarm
npx @claude-flow/cli@latest memory search --query "" # Vector search
npx @claude-flow/cli@latest hooks route --task ""    # Route to agent
npx @claude-flow/cli@latest doctor --fix             # Diagnostics
npx @claude-flow/cli@latest security scan            # Security scan
npx @claude-flow/cli@latest performance benchmark    # Benchmarks
```

26 commands, 140+ subcommands. Use `--help` on any command for details.

## Setup

```bash
claude mcp add claude-flow -- npx -y @claude-flow/cli@latest
npx @claude-flow/cli@latest daemon start
npx @claude-flow/cli@latest doctor --fix
```

**Agent tool** handles execution (agents, files, code, git). **MCP tools** handle coordination (swarm, memory, hooks). **CLI** is the same via Bash.
