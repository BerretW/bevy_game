# Project Context: FiveM-Style Modular Multiplayer Framework

## Vision

Genre-agnostic multiplayer framework v Rustu (Bevy Engine), architektura napodobuje FiveM/CitizenFX:
- **Rust Core** = "Host Shell" — ECS, networking, physics, rendering, DB pooly
- **Lua Resources** = veškerá herní logika, obsah a UI (hot-reloadable)

Podporuje libovolný žánr pouhým swapem Lua resources — bez rekompilace Rust core.
**Po každé změně, splnění roadmapy nebo rozšíření aktualizuj CLAUDE.md.**

---

## Tech Stack

| Oblast | Technologie |
|--------|-------------|
| Engine | Bevy Engine 0.18, headless-first pro server |
| Networking | `lightyear` 0.26 — UDP netcode, channels, replication |
| File sync | `axum` HTTP server + `reqwest::blocking` downloader |
| Scripting | `mlua` via `bevy_mod_scripting` |
| Database | `sqlx` — PostgreSQL (prod) / SQLite (dev) |
| Shadery | WGSL, hot-reload přes Bevy AssetServer |
| WebUI / NUI | Dioxus / `bevy_egui`; Axum (admin API) |

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

### Fáze 1–4 ✅ Dokončeno

| Fáze | Výsledky |
|------|---------|
| **Phase 1** — Shell & VFS | Cargo workspace, VFS scanner, manifest.lua DSL parser, dependency resolver (Kahn), per-resource Lua sandbox |
| **Phase 2** — Network Handshake | `core_net`, lightyear UDP, Axum HTTP file server, blake3 digest handshake, Lua RPC bridge |
| **Phase 3.1** — Gameplay Foundations | `PlayerInput`, `NetTransform`, player spawn/render, 1st/3rd person kamera (F6 toggle / `Camera.SetMode`), client-trusted movement (Avian), yaw sync, movement smoothing |
| **Phase 3.2** — Lua Bridge | `LuaCommand` enum, `CommandQueue`, `LuaWorldState`, `process_lua_commands` (PostUpdate) |
| **Phase 3.3** — Combat | `WeaponConfig`, `Health`, `process_combat`, `PRIMARY_FIRE` bitflag, ACE authority, `onPlayerHit`/`onPlayerDeath`/`playerConnecting`/`playerDropped` |
| **Phase 3.4** — Model Registry | `ModelRegistry`, `scan_stream_models()`, async GPU load, `Engine.RequestModel/HasModelLoaded` |
| **Phase 3.5** — World Objects | `SpawnNetworkedObject`, `NetworkedObjectMarker`, lightyear replication observer |
| **Phase 3.7** — Raycast API | `RaycastBridge`, `Raycast.GetGroundPosition()`, yaw v `PlayerInput.look[0]` |
| **Phase 3.8** — Event Bus | `LocalEventBus`, `TriggerEvent`, JSON payloads, `input:state` bridge, `sq:ready` init pattern, Lua-safe string player IDs |
| **Phase 3.9** — Entity State API | `EntityHandle`, `ModelName`, `AnimationState`, `EntityStateCache`, `World.Get*/Set*` API |
| **Phase 4** — ADS + GUI | Apparatus Drawable System (`.drawable` TOML, materiály WGSL, LOD systém, kolize), Blender toolkit (`bevy_toolkit.py`), immediate-mode GUI (`Gui.*`), `CreateThread`/`Wait`, `UI.Window` |

**Phase 3.6 — YMAP Streaming** (částečně):
- [X] `World.SpawnNetworkedObject` základ
- [ ] YMAP JSON loader, Mapper tool (Lua in-game editor), AABB streaming, GPU Instancing, server culling

**Phase 4 zbývá:**
- [ ] Integrovat `sqlx` (stub `Database.*` API přítomen)
- [ ] Vlastní WGSL shadery z Lua resources

---

### Phase 5 — FPS Core Systems

**Filosofie:** Rust = fyzikální engine + datové kontrakty. Lua = vše herně specifické. Žádná zbraň ani herní pravidlo se nesmí hardcodovat do Rustu.

#### 5.0 — Collision Foundation ✅

Implementováno: `DrawableCollision` → Avian `Collider` pipeline, axis-lock flagy (`lock_translation/rotation`), `DisableDrawableCollisions` marker, `StaticWorldCollider` filter pro movement gate, `NAVMESH` shape → `NavMeshSurfaceCache`, `ClientMapPlugin` (`assets/maps/*.map.toml`), Blender toolkit NAVMESH + map TOML workflow.

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

| Funkce | Strana | Popis |
|--------|--------|-------|
| `Weapon.Register/Get` | server/both | WeaponDef registry |
| `Weapon.GetEquipped/SetEquipped` | server | Equipment hráče |
| `Weapon.GetAmmoReserve/SetAmmoReserve/ForceReload` | server | Munice |
| `Ammo/Attachment/Material.Register` | server | Definice registrace |
| `Hitbox.Register(model,def)` | server | Hitbox definition |
| `Player.GetArmor/SetArmor` | server | Brnění |
| `Player.GetTeam/SetTeam` | both/server | Team assignment |
| `Player.GetStamina/GetStance` | server | Fyzický stav |
| `Spawn.Register/GetFree/SetActive/GetAll` | server | Spawn body |
| `Round.GetState/SetTimeLimit/End` | both/server | Stav kola |
| `Score.Add/Get/GetAll` | server/both | Scoreboard |
| `Camera.Create/Delete` | client | Vytvoř / smaž pojmenovanou kameru |
| `Camera.SetActive/GetActive` | client | Přepni aktivní kameru (nil = player kamera) |
| `Camera.AttachToEntity/AttachToBone/AttachToPosition` | client | Připoj kameru na entitu, kost nebo pozici |
| `Camera.SetFOV` | client | Nastav FOV aktivní kamery (stupně) |
| `Camera.SetMode/GetMode` | client | `first_person` / `third_person` / custom_id |

---

## ADS — Known Limitations

**Více materiálů na jednom mesh objektu:** GLTF exporter rozděluje na primitiva, `process_mesh_node` hledá jedno jméno materiálu. Řešení: rozděl mesh na více objektů v Blenderu (jeden materiál = jeden objekt). Fallback: použije první materiál z manifestu + warning.

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
/model_viewer/src/
  main.rs                        ADS model viewer (CLI args), grid gizmos
  camera.rs                      OrbitCamera (orbit/pan/zoom)
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
  example/moving_square/         demo pohybu + input.lua
```

---

## Lua Sandbox Runtime API

Každý resource = vlastní izolovaná `mlua::Lua` instance. **Sandbox isolation:** žádné shared globals — pouze event bus. `SandboxRegistry` je `NonSend` (main thread, `mlua::Lua` je `!Send`).

**Stdlib povolen:** `string`, `table`, `math`, `utf8`, `coroutine`. **Zakázán:** `io`, `os`, `package`, `require`, `debug`, `load*`.

| Symbol | Strana | Popis |
|--------|--------|-------|
| `RESOURCE_ID`, `SIDE`, `IS_SERVER`, `IS_CLIENT` | both | Identita resource |
| `print(...)`, `log_debug/info/warn(s)` | both | Logování |
| `RegisterEvent(name, fn)` | both | Callback; handler dostane `(payload, sender_id?)` |
| `TriggerServerEvent(name, payload?)` | client | Pošle event serveru |
| `TriggerClientEvent(name, target, payload?)` | server | Unicast (u64/string) nebo broadcast (nil/false) |
| `TriggerEvent(name, payload?)` | both | Cross-sandbox bus (in-process) |
| `World.SpawnLocalObject(model, pos, rot)` | both | Lokální entita → handle (u64) |
| `World.SpawnNetworkedObject(model, pos, rot)` | server | Replikovaná entita → handle |
| `World.DeleteObject(handle)` | both | Despawn |
| `World.SetTransform/SetPosition/SetRotation/SetScale/SetModel` | both | Transformace |
| `World.PlayAnimation(h, name, loop?, speed?)` / `StopAnimation` | both | Animace |
| `World.IsValid/IsAlive/GetHealth/GetModel` | both | State dotazy |
| `World.GetPosition/Rotation/Quaternion/Scale/Transform/Animation/AnimationSpeed` | both | Gettery |
| `World.ApplyDamage(target, amount, source?)` | server | Damage intent |
| `Engine.RequestModel/HasModelLoaded/SetModelAsNoLongerNeeded` | both | Model ref-counting |
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

`payload` = libovolná Lua hodnota, serializována jako JSON. `TriggerClientEvent` target podporuje integer i string.

---

## Client Config — `client.toml`

Generovaný při prvním spuštění: Win `%APPDATA%\bevy_game\client.toml`, Linux `~/.config/bevy_game/client.toml`.

| Sekce | Co řídí |
|-------|---------|
| `[player]` | name, saved_client_id, avatar |
| `[network]` | server, bind, download_concurrency, timeouty |
| `[graphics]` / `[graphics.quality]` | backend (auto/vulkan/dx12/...), resolution, vsync, shadow/AA/LOD/SSAO/SSR |
| `[audio]` | master + 5 kanálů, spatial audio, mute on focus lost |
| `[ui]` | jazyk, HUD opacity, crosshair, FPS/ping/minimap |
| `[input]` / `[input.keys]` / `[input.mouse]` | sensitivity, invert Y, raw input, 39 keybindings |
| `[paths]` | cache/screenshot/savegame/log dir overridy |
| `[advanced]` | log level, GPU validation, dev console, preload toggle |

## Server Config — `server.toml`

Hledán: CLI arg → `<exe_dir>/server.toml` → `<cwd>/server.toml`. Relativní cesty: nejdřív vedle `.exe`, pak CWD.

| Sekce | Co řídí |
|-------|---------|
| `[server]` | display name, MOTD, tagy |
| `[gameplay]` | max_players, gamemode, idle_kick_sec |
| `[net]` | UDP/HTTP bind, tickrate, protocol_id, klíč |
| `[resources]` | VFS root, hot_reload, debounce |
| `[auth]` | `mode = "open"/"token"/"whitelist"` |
| `[database]` | sqlx connection string, pool size (Phase 4) |
| `[dev]` | auto_acknowledge_clients, print_digest_on_startup |

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
```

**Porty:** UDP 5000 (lightyear), TCP 8081 (Axum HTTP). Default server: `127.0.0.1:5000` / `http://127.0.0.1:8081`.
