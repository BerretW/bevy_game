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
- [ ] Blend Spaces infrastruktura: `BlendSpaceState` komponenta, `PlayBlendSpace` command, Lua API, ale runtime evaluace vah zatím není (TODO pro Phase 4.x)
- [ ] Integrovat `sqlx` (stub `Database.*` API přítomen)
- [ ] Vlastní WGSL shadery z Lua resources

**ADM v6 migration policy:**

- V5 kompatibilita se dál neudržuje jako cíl.
- Nový formát je `*.adm` pro geometrii a `*.ads_anim` pro animace.
- Runtime má preferovat late binding přes připojené anim-sety, ne embedded klipy v modelu.
- Klientská `ModelAnimationRegistry` pro ADM modely bere clipy/dictionaries pouze z `AttachedAnimSets` (`.ads_anim`), embedded ADM animace se ignorují.

---

### Phase 4.1 — Blend Spaces (Infrastruktura) [✅ Hotovo]

Implementováno: `AdmBlendSpace`, `AdmBlendSpaceClip` struktury v ADM formátu, `BlendSpaceState` komponenta v cmd_queue, `LuaCommand::PlayBlendSpace`, Lua API `World.PlayBlendSpace(handle, blend_space_name, move_x, move_y, speed?, flags?)`, handler v cmd_queue, test resource `resources/example/blend_space_test/` s rotačním move vektorem.

**Zbývá:** Runtime evaluace vah (nový systém `evaluate_blend_spaces`), ADM v5 parser pro blend space definice, aplikace více klipů v `apply_adm_animations`.

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

**Zbývá:** Plná testovací validace na stairs_test resource (ověření raycast heights + IK offset aplikace).

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

**Zbývá:** Runtime evaluace vah (nový systém `evaluate_blend_spaces`), ADM v5 parser pro blend space definice, aplikace více klipů v `apply_adm_animations`.

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

#### 5.1 — Weapon & Ammo Registry [ ]

**Rust side:**

- [ ] `WeaponDef`, `AmmoDef`, `AttachmentDef`, `MaterialDef` structs
- [ ] `WeaponRegistry`, `AmmoRegistry`, `AttachmentRegistry`, `MaterialRegistry` Bevy Resources
- [ ] Lua API: `Weapon.Register(id,def)`, `Weapon.Get(id)`, `Ammo.Register`, `Attachment.Register`, `Material.Register`

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

- [ ] `HitboxDef`, `HitboxRegistry`, `PlayerHitbox`, `PositionHistory` (ring buffer pro lag comp)
- [ ] `LagCompensator` (rewind pozic na spawn_tick), `HitResolver` (kapsle test → `DamageEvent` s hitzone)
- [ ] `Hitbox.Register(model_id, def)` Lua API

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

#### 5.4 — Weapon State [ ]

- [ ] `WeaponSlots` (4 sloty), `EquippedWeapon {weapon_id, ammo_in_mag, ammo_type_id, attachments}`, `AmmoReserve`, `ActiveSlot`
- [ ] `ReloadState` FSM (`Idle|Reloading{elapsed,duration,type}`), `FireState` (`Ready|Firing|Cooling`), `WeaponSwapState`
- [ ] `reload_system`, `fire_system` (FixedUpdate)
- [ ] Nové `PlayerInput` bity: `RELOAD` (2), `ADS` (12), `WEAPON_SLOT_1..4` (13–16)
- [ ] Lua: `Weapon.GetEquipped/SetEquipped/GetAmmoReserve/SetAmmoReserve/GetActiveSlot/ForceReload`

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
| `Weapon.Register/Get`                                 | server/both | WeaponDef registry                              |
| `Weapon.GetEquipped/SetEquipped`                      | server      | Equipment hráče                               |
| `Weapon.GetAmmoReserve/SetAmmoReserve/ForceReload`    | server      | Munice                                          |
| `Ammo/Attachment/Material.Register`                   | server      | Definice registrace                             |
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

| Symbol                                                                                                                    | Strana      | Popis                                                                                                                                                        |
| ------------------------------------------------------------------------------------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `RESOURCE_ID`, `SIDE`, `IS_SERVER`, `IS_CLIENT`                                                                   | both        | Identita resource                                                                                                                                            |
| `print(...)`, `log_debug/info/warn(s)`                                                                                | both        | Logování                                                                                                                                                   |
| `RegisterEvent(name, fn)`                                                                                               | both        | Callback; handler dostane `(payload, sender_id?)`                                                                                                          |
| `TriggerServerEvent(name, payload?)`                                                                                    | client      | Pošle event serveru                                                                                                                                         |
| `TriggerClientEvent(name, target, payload?)`                                                                            | server      | Unicast (u64/string) nebo broadcast (nil/false)                                                                                                              |
| `TriggerEvent(name, payload?)`                                                                                          | both        | Cross-sandbox bus (in-process)                                                                                                                               |
| `World.SpawnLocalObject(model, pos, rot)`                                                                               | both        | Lokální entita → handle (u64)                                                                                                                             |
| `World.SpawnNetworkedObject(model, pos, rot)`                                                                           | server      | Replikovaná entita → handle                                                                                                                                |
| `World.SpawnNetworkedNpc(model, pos, rot, ped_profile?)`                                                                | server      | Replikované NPC (NPC marker + klientský capsule collider setup, volitelně s explicitním ped profilem, např. "player" nebo "monster")                    |
| `World.SpawnLocalDummy(shape, params, pos, rot)`                                                                        | both        | Parametrický dummy objekt (cuboid/sphere/cube/stairs/arch)                                                                                                  |
| `World.SpawnNetworkedDummy(shape, params, pos, rot)`                                                                    | server      | Replikovaný parametrický dummy objekt                                                                                                                      |
| `World.SpawnLocalCollider(params, pos, rot)`                                                                            | both        | Samostatný collider bez render meshe (`shape/size/radius/height/is_static/is_trigger/stairs/stairs_slope_invert/stairs_clearance_y`)                      |
| `World.SpawnNetworkedCollider(params, pos, rot)`                                                                        | server      | Replikovaný samostatný collider bez render meshe                                                                                                           |
| `World.DeleteObject(handle)`                                                                                            | both        | Despawn                                                                                                                                                      |
| `World.SetTransform/SetPosition/SetRotation/SetScale/SetModel`                                                          | both        | Transformace                                                                                                                                                 |
| `World.PlayAnimation(h, name, blend?)` nebo `World.PlayAnimation(h, name, loop?, speed?, blend?)` / `StopAnimation` | both        | Animace (`name` podporuje `clip:N`/`anim:N`/`N`; GLTF = clip index, ADM = clip index nebo clip name; ADM v5 navíc `dict:<dict_name>:<clip_name>`) |
| `World.Attach(child, child_socket, parent, parent_socket)` / `World.Detach(child)`                                    | both        | Socket-to-socket attachment                                                                                                                                  |
| `World.AttachWithOffset(child, parent, offset, rot)`                                                                    | both        | Parent attachment přes lokální offset od pivotu parent entity                                                                                             |
| `World.GetSocketTransform(handle, socket)`                                                                              | both        | World-space socket transform                                                                                                                                 |
| `World.IsValid/IsAlive/GetHealth/GetModel`                                                                              | both        | State dotazy                                                                                                                                                 |
| `World.GetPosition/Rotation/Quaternion/Scale/Transform/Animation/AnimationSpeed`                                        | both        | Gettery                                                                                                                                                      |
| `World.NpcConfigure(handle, opts)`                                                                                      | both/server | Nastaví NPC parametry (`move_speed`, `arrive_distance`, `turn_speed`)                                                                                 |
| `World.NpcWander(handle, kind, opts)`                                                                                   | both/server | Wander módy:`random`, `patrol`, `orbit`                                                                                                               |
| `World.NpcGoToEntity(handle, target, stop?)` / `World.NpcGoToCoord(handle, pos, stop?)` / `World.NpcStop(handle)`   | both/server | Přímé AI movement příkazy                                                                                                                               |
| `World.NpcSetBrain(handle, id)` / `World.NpcRegisterBrain(id, def)`                                                    | both/server | Runtime změna/registrace brain profilu bez rebuildu                                                                                                    |
| `World.NpcSetTask(handle, task, opts?)` / `World.NpcSetScenario(handle, scenario_id, opts?)`                         | both/server | High-level scenario/task kontrakt; replikuje se jako `ReplicatedNpcBrain` a lokálně se interpretuje na `NpcMoveGoal`                                |

Poznámka: Pro AI postavy používej `World.SpawnNetworkedNpc(...)` místo `World.SpawnNetworkedObject(...)`, aby klientská vrstva vytvořila správný capsule collider pro NPC. Pokud potřebuješ specifický fyzikální profil (např. pro létající monstrum), zadej čtvrtý parametr `ped_profile` (název bez přípony, např. "monster").
| `World.ApplyDamage(target, amount, source?)` | server | Damage intent |
| `Engine.RequestModel/HasModelLoaded/SetModelAsNoLongerNeeded` | both | Model ref-counting |
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
