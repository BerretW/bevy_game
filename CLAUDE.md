# Project Context: FiveM-Style Modular Multiplayer Framework

---

## Vision

Genre-agnostic, high-performance multiplayer game framework v Rustu (Bevy Engine).
Architektura napodobuje FiveM/CitizenFX:

- **Rust Core** = "Host Shell" — ECS, networking, physics, rendering, DB pooly
- **Lua Resources** = veškerá herní logika, obsah a UI (hot-reloadable)

Podporuje libovolný žánr (FPS, Extraction Shooter, Survival, Turn-based RPG) pouhým swapem Lua resources — bez rekompilace Rust core.
Po každé změně, splnení roadmapy a nebo rozšíření aktualizuj claude.md

---

## Tech Stack

| Oblast      | Technologie                                                                         |
| ----------- | ----------------------------------------------------------------------------------- |
| Engine      | Bevy Engine 0.18, headless-first pro server                                         |
| Networking  | `lightyear` 0.26 — UDP netcode, channels, replication (Phase 3+)                 |
| File sync   | `axum` HTTP server (host_server) + `reqwest::blocking` downloader (host_client) |
| Scripting   | `mlua` via `bevy_mod_scripting`                                                 |
| Database    | `sqlx` — PostgreSQL (prod) / SQLite (dev)                                        |
| Shadery     | WGSL, hot-reload přes Bevy AssetServer                                             |
| WebUI / NUI | Dioxus / local HTML nebo `bevy_egui`; Axum (admin API)                            |

---

## Core Architecture: Resource Paradigm

```
/resources/[category]/[resource_name]/
```

| Vrstva            | Odpovědnost                                               |
| ----------------- | ---------------------------------------------------------- |
| Rust Core         | `lightyear`, WGPU, SQLx pool, Lua Sandbox API (`mlua`) |
| Resource Manifest | `manifest.lua` — metadata, závislosti, seznam souborů |
| Lua Layer         | Herní logika — stats, inventory, items, game loop        |

### Pravidlo: Nová funkce = nový Resource

> **NIKDY neupravuj Rust core**, pokud to není nezbytně nutné. Vždy vytvoř nový Resource.

#### Příklad `manifest.lua`

```lua
-- /resources/survival/metabolism/manifest.lua
resource_type 'script'                    -- 'script' | 'asset' | 'map' | 'gamemode'
author       'Developer'
version      '1.0.0'
description  'Hunger / thirst / temperature loop.'

dependencies {
    'core/inventory',                     -- ResourceId = cesta od /resources/, slash-separated
    'core/stats',
}

shared_scripts { 'shared/config.lua' }    -- nahraje se na obou stranách
server_scripts { 'server/decay_loop.lua' }
client_scripts { 'client/ui_hunger.lua' }

files {
    'assets/ui_icons.png',
    'shaders/screen_blur_low_health.wgsl',
}
```

> `ResourceId` je kanonická cesta od `/resources/` rootu (forward slashes i na Windows). Definováno v [core_resources/src/types.rs](core_resources/src/types.rs).

---

## Coding Standards & Guardrails

### 1. Rust / Lua Boundary

| Pravidlo    | Detail                                                                                                         |
| ----------- | -------------------------------------------------------------------------------------------------------------- |
| Entity IDs  | Lua komunikuje se světem výhradně přes Bevy Entity ID (integer / opaque userdata)                          |
| Data Access | Žádné raw Rust pointery do Lua. Používej safe bridge:`SetComponent(entity, "Hunger", 50)`               |
| Events      | Globální Event Bus. Rust překládá network eventy na Lua hooky:`TriggerEvent("onPlayerJoin", player_id)` |

### 2. Networking & Synchronizace

- **Resource Sync:** Při připojení server pošle digest manifestů. Klient stáhne chybějící `.lua`, `.wgsl` a assety (přes Axum HTTP server) před spawnením.
- **RPC API:**
  ```lua
  TriggerClientEvent(eventName, targetPlayer, args...)
  TriggerServerEvent(eventName, args...)
  ```

### 3. Databáze (Async pravidla)

- **NIKDY neblokuj ECS loop SQL dotazem.**
- Používej `bevy::tasks::IoTaskPool`.
- Async API pro Lua:
  ```lua
  Database.execute(query, params, function(result) ... end)
  ```

### 4. Stats & Survival logika

- Vzorec: `FinalMaxHealth = (BaseHealth + Sum(Buffs)) * Multiplier(Auras)`
- Items, zbraně a buffy jsou definovány v Lua slovnících nebo JSON.
- Rust core zná pouze "Item ID" a jak ho replikovat.

### 5. Multiplayer – autoritativnost

- **Server je jediná pravda** — pozice, inventory, stats.
- **Client Prediction:** Pohyb a střelba (FPS) musí používat rollback mechanismus `lightyear`.

---

## Roadmap

### Phase 1 — Shell & VFS ✅

- [X] Cargo workspace: `core_shared`, `core_resources`, `host_server`, `host_client`
- [X] `SharedPlugin` (event bus prep), `ServerCorePlugin` (headless + Tokio runtime), `ClientCorePlugin` (DefaultPlugins)
- [X] VFS scanner sledující `/resources/` (`walkdir` + `notify` watcher s 150 ms debounce)
- [X] `manifest.lua` DSL parser v izolovaném mlua VM (omezený stdlib: `string`/`table`/`math`)
- [X] Dependency resolver (Kahn's topological sort, detekce cyklů / missing / self-deps)
- [X] Per-resource izolovaný Lua sandbox (vlastní `mlua::Lua` na resource)

### Phase 2 — Network Handshake & File Sync ✅

- [X] `core_net` crate (wire protocol, file digest, lightyear plugin scaffolding)
- [X] Lightyear 0.26 UDP netcode connection (server listen + client connect)
- [X] Axum HTTP file server na `host_server` (`/resources/<id>/<path>`)
- [X] `ServerHello` digest → klient `reqwest::blocking` download → `ClientReady` handshake
- [X] Klient přepnutý na cache mode (`cache/resources/`); `ResourcesPlugin` watcher hot-reloaduje stažené soubory
- [X] Lua RPC bridge: `TriggerServerEvent`, `TriggerClientEvent`, `RegisterEvent` přes lightyear `LuaEventMessage` kanál

### Phase 3 — Universal ECS Bridge & World Streaming

**Hlavní filozofie:** Rust (Core) = slepý sval (rendering, kolize, netcode, paměť). Lua (Resources) = mozek (pravidla, logika, inventář, ukládání). Vše komunikuje asynchronně přes Intents a Events.

#### 3.1 — Gameplay Foundations ✅

- [X] `PlayerInput` struct (move_dir, look, actions bitfield, client_tick)
- [X] `InputChannel` — sequenced unreliable, 60 Hz tick stream
- [X] `ReplicationSender` attach observer (`Add<LinkOf>`) na transport entitu
- [X] Server-side player spawn observer (`spawn_player_on_connect` při `Add<Connected>`)
- [X] `NetTransform`, `NetVelocity`, `PlayerMarker` s lightyear prediction
- [X] `apply_inputs_to_velocity` + `integrate_velocity` v `FixedUpdate` (`sim.rs`)
- [X] Klientský renderer replikovaných hráčů (3D model + fallback capsule per `PlayerMarker`) + WASD input collection (`gameplay.rs`)
- [X] Preferovaná vizualizace predicted entity: fallback duplicate pro stejné `client_id` se automaticky skrývá (`prefer_predicted_player_visuals`)
- [X] Kamera režimy klienta: 3rd person / 1st person toggle (`F6`) se sledováním lokálního hráče
- [X] Kamera stabilizace: look ovládán `MouseMotion` delta (`yaw/pitch` s clamp), cursor lock (`CursorGrabMode::Locked`), bez nelineární rotace podle pozice kurzoru
- [X] GLTF `SceneRoot` stabilizace: registrace reflektovaných typů (`Transform`, `GlobalTransform`, `Visibility`, `InheritedVisibility`, `ViewVisibility`, `TransformTreeChanged`, `Mesh3d`, `MeshMaterial3d<StandardMaterial>`, `Aabb`, `SkinnedMesh`, `GltfExtras`, `GltfSceneExtras`, `GltfMeshExtras`, `GltfMeshName`, `GltfMaterialExtras`, `GltfMaterialName`, `ChildOf`, `Children`, `Name`) v klientském pluginu

#### 3.2 — Command Queue & Bezpečný Lua Bridge ✅

- [X] `LuaCommand` enum: `SpawnLocalObject`, `DespawnEntity`, `SetTransform`, `ApplyDamage`
- [X] Sdílený `CommandQueue` buffer (`Arc<Mutex<Vec<LuaCommand>>>`) — Lua closures zachytí Arc klon
- [X] Bevy systém `process_lua_commands` (PostUpdate) — bezpečně aplikuje příkazy na ECS svět
- [X] `LuaWorldState` Bevy Resource — mapuje Lua handles (`u64`) na Bevy `Entity`
- [X] `LocalObjectMarker` Component — marker pro lokální (non-lightyear) objekty (Phase 3.4 přidá mesh)
- [X] `PendingDamageEvent` Message — Phase 3.3 combat systémy se přihlásí přes `MessageReader`
- [X] Lua API: `World.SpawnLocalObject(model, pos, rot)` → handle, `World.DeleteObject(handle)`, `World.SetTransform(handle, pos, rot)`, `World.ApplyDamage(target, amount, source?)` (server only)

#### 3.3 — Data-Driven Combat & Akce ✅

- [X] `WeaponConfig` Bevy resource (`fire_rate`, `damage`, `range`, `cone_angle`, `weapon_type`) — server-side globální konfigurace
- [X] `Health { current, max }` + `WeaponCooldown { remaining }` komponenty na player entitě
- [X] `collect_last_inputs` systém (Update) — drainuje `MessageReceiver<PlayerInput>` do `LastPlayerInputs` resource
- [X] `process_combat` (FixedUpdate) — čte `LastPlayerInputs`, `PRIMARY_FIRE` bitflag + proximity + angle check, aplikuje dmg na `Health`
- [X] `tick_weapon_cooldowns` (FixedUpdate) — decrementuje `WeaponCooldown.remaining`
- [X] Camera-relative movement input: klient rotuje WASD vektor podle yaw kamery a server dostává world-space `move_dir`
- [X] Player yaw sync: server zapisuje `PlayerInput.look[0]` do `NetTransform.rotation`; klient při render sync aplikuje i rotaci, takže model/sprite míří směrem kamery
- [X] Player movement smoothing: klientský `sync_net_transform_to_render` používá exponenciální lerp/slerp (translation + rotation), aby se snížil jitter při síťových korekcích
- [X] Jump + crouch movement: server sim aplikuje `JUMP` na vertikální rychlost (`NetVelocity.y`) s gravitací/ground clampem; `CROUCH` snižuje rychlost pohybu (a blokuje sprint multiplier)
- [X] Lua eventy `onPlayerHit` + `onPlayerDeath` emitované serverem přes `LocalEventBus` (JSON payload: attacker, victim, damage, weapon, position)
- [X] Lua eventy `playerConnecting` + `playerDropped` při connect/disconnect (observer na `Add<Connected>` / `Remove<Connected>`)

#### 3.4 — Global Model Registry ✅

- [X] `core_resources/src/model_registry.rs` — nový modul: `ModelRegistry`, `ModelEntry { path, ref_count }`, `ModelCommandQueue`
- [X] `vfs.rs::scan_stream_models()` — prochází složku `stream/` každého resource (`.glb`, `.gltf`, `.obj`, `.fbx`, `.col`, `.mesh`; konflikty: logovat + first-wins)
- [X] `process_model_commands` systém (PostUpdate) — drainuje `ModelCommandQueue`, aplikuje `Request`/`Release` na `ModelRegistry`
- [X] Lua API: `Engine.RequestModel(name)`, `Engine.HasModelLoaded(name) -> bool`, `Engine.SetModelAsNoLongerNeeded(name)`
- [ ] Async load modelu z disku do GPU (Phase 4 — vyžaduje Bevy AssetServer integraci)

#### 3.5 — YMAP World Objects ✅ (základ)

- [X] `LuaCommand::SpawnNetworkedObject { handle, model, pos, rot }` — přidáno do `cmd_queue.rs`
- [X] `NetworkedObjectMarker { model }` Component — spawněna entita bez lightyear typů v `process_lua_commands`
- [X] `attach_replication_to_networked_object` observer (`Add<NetworkedObjectMarker>`) v `sim.rs` — přidá `Replicate::to_clients(NetworkTarget::All)` na entitu
- [X] Lua API: `World.SpawnNetworkedObject(model, pos, rot) -> handle` (server only)
- [ ] YMAP JSON loader — dávkový spawn z JSON souboru (`[{"model": ..., "pos": ..., "rot": ...}]`)
- [ ] Mapper tool — Lua in-game editor
- [ ] Oddělit Modelera (`stream/prop.glb`) od Mappera (YMAP JSON)

#### 3.6 — YMAP Streaming & Culling

- [X] `collect_last_inputs` systém odděluje příjem inputů od simulace (předpoklad pro streaming tickování)
- [ ] AABB bounding box per-YMAP (dopočítaný při načtení JSON)
- [ ] Klientský streaming: Load/Unload radius → async YMAP load → GPU Instancing (jeden draw call per model)
- [ ] Serverový culling: fyzika YMAPu se načte jen pokud je v AABB živý hráč nebo AI
- [ ] Server Physics Instancing: `.col` soubory, jeden `Shape` per model type → N instancí (šetří RAM)

#### 3.7 — Raycasting API ✅

- [X] `RaycastBridge(Arc<Mutex<[f32;3]>>)` — Bevy Resource (Clone + Default), definována v `sandbox.rs`, sdílena přes Arc
- [X] `update_raycast_bridge` systém v `gameplay.rs` — camera forward ray → průsečík s rovinou Y=0 → `raycast.set_pos([x,0,z])` každý frame
- [X] Lua API: `Raycast.GetGroundPosition() -> {x, y, z}` — čte z `RaycastBridge` Arc; na serveru vrací `{0,0,0}`
- [X] `collect_and_send_input` v `gameplay.rs` — yaw úhel myši (`atan2`) posílán v `PlayerInput.look[0]`

#### 3.8 — Zbývající ECS Bridge položky ✅ (základ)

- [X] `LocalEventBus(Arc<Mutex<Vec<LocalEvent>>>)` — Bevy Resource; `dispatch_local_events` systém (PostUpdate) drainuje bus a volá `dispatch_incoming` na všechny sandboxy
- [X] `TriggerEvent(name, payload)` — funkční cross-sandwwwwbox bus (nahrazuje Phase 2 no-op stub)
- [X] `TriggerClientEvent(name, target, payload)` — unicast pokud `target` je `u64` player_id; broadcast pokud `nil`/`false`
- [X] JSON payload pro všechny `Trigger*` funkce (LuaTable ↔ `serde_json::Value`; helper funkce `lua_value_to_json` + `json_to_lua_value`)
- [X] `sender` player_id v handlerech — `server_dispatch_incoming` extrahuje `PeerId::Netcode(id)` a předává `Some(id)` do Lua
- [X] Client input bridge: `host_client::gameplay::publish_input_state_to_lua` publikuje `input:state` (move axis + key bools) do `LocalEventBus` každý frame
- [X] Robustní init pro resources: `sq:ready` request/response pattern (client `TriggerServerEvent`, server unicast `sq:init`), odolné vůči missed join eventu po reloadu
- [X] `TriggerClientEvent` nyní akceptuje `target` i jako string u64 (`"123..."`) kvůli Lua number precision limitům
- [X] Lua-safe player identity: server event payloady (`playerConnecting`, `playerDropped`, `onPlayerPosition`, combat eventy) posílají `id`/`attacker`/`victim` jako stringy; klient clampuje `client_id` do i64 rozsahu
- [ ] Vystavit `Stats`, `Inventory` komponenty do Lua přes bridge (Phase 4)

#### 3.9 — Entity State API ✅

- [X] `Health` přesunut do `core_shared` — sdíleno mezi `core_resources` a `core_net`
- [X] `EntityHandle(u64)` Component — embeddovaný handle na všech Lua-spawnutých entitách
- [X] `ModelName(String)` Component — kanonické jméno modelu (oddělené od markerů, mutovatelné)
- [X] `AnimationState { current, speed, looping, paused }` Component — stav animace; Phase 4 napojí na Bevy AnimationPlayer
- [X] `EntitySnapshot` — snapshot stavu entity pro synchronní Lua čtení (pos, rot quaternion, scale, model, alive, health, animation)
- [X] `EntityStateCache(Arc<Mutex<HashMap<u64, EntitySnapshot>>>)` — Bevy Resource, aktualizovaná každý frame
- [X] `sync_entity_state_cache` systém (PostUpdate, po `process_lua_commands`, před `dispatch_local_events`) — naplňuje cache
- [X] Nové `LuaCommand` varianty: `SetModel`, `SetPosition`, `SetRotation`, `SetScale`, `PlayAnimation`, `StopAnimation`
- [X] `process_lua_commands` rozšířen: přidává `EntityHandle` + `ModelName` při spawnu; zpracovává nové příkazy; zachovává rot/scale při `SetPosition` atd.
- [X] Lua API gettery: `World.IsValid`, `World.IsAlive`, `World.GetHealth`, `World.GetModel`, `World.GetPosition`, `World.GetRotation` (Euler°), `World.GetQuaternion`, `World.GetScale`, `World.GetTransform`, `World.GetAnimation`, `World.GetAnimationSpeed`
- [X] Lua API settery: `World.SetModel`, `World.SetPosition`, `World.SetRotation` (Euler°), `World.SetScale` (číslo nebo tabulka), `World.PlayAnimation(h, name, looping?, speed?)`, `World.StopAnimation`

### Phase 4 — WebUI, DB & QOL

- [X] **Dual resource loading** — `resolve_path_relative_to_exe` s třístupňovým fallbackem: exe_dir → CWD → `../resources` (pro `cargo run` z build directory)
- [X] **ADS Blender toolkit refresh** — `blender_plugin/bevy_toolkit.py` exportuje ADS-compliant `.toml` (`asset_name`, `version`, `[materials]`, `[entities]`), podporuje `MESH`/`COLLISION` metadata, texture source (`shared`/`embedded`), vertex mask workflow včetně alpha kanálu, export scope (`ALL`/`SELECTED`/`ACTIVE_COLLECTION`), konzistenční validační warningy před exportem a Sollumz-like workflow (`Convert to Drawable Model`, `Convert to Drawable`, `Create Drawable`, material conversion/embed utility) včetně robustního mapování textur z Principled BSDF graphu, template-specific preview node graphů pro `standard_pbr`/`layered_env`/`vehicle_glass` a deterministického parentingu COL proxy objektů
- [X] **Vlastní GUI framework** — immediate-mode Lua drawing API (`Gui.*`) + Lua threading (`CreateThread` / `Wait`)
  - [X] `GuiDrawBuffer(Arc<Mutex<Vec<DrawCommand>>>)` — sdílený buffer Lua ↔ Bevy
  - [X] `Gui.DrawRect(x, y, w, h, r, g, b, a)` — vyplněný obdélník (normalizované 0–1 souřadnice)
  - [X] `Gui.DrawText(text, x, y, scale, r, g, b, a)` — text s top-left anchoringem
  - [X] `Gui.DrawLine(x1, y1, x2, y2, r, g, b, a)` — čára (renderuje se jako tenký rotovaný Sprite)
  - [X] `Gui.DrawCircle(x, y, radius, r, g, b, a)` — kruh (24 line segmentů)
  - [X] `Gui.DrawSprite(id, x, y, w, h, r?, g?, b?, a?, opts?)` — obrázek z manifestu; opts = `{fit="stretch"|"fit"|"fill", uv={u0,v0,u1,v1}, flip_x=bool, flip_y=bool}`
  - [X] `Gui.DrawDisc(x, y, radius, r, g, b, a)` — vyplněný kruh (GPU texture, anti-aliased)
  - [X] `Gui.DrawRoundedRect(x, y, w, h, radius, r, g, b, a)` — zaoblené rohy
  - [X] `Gui.DrawBorder(x, y, w, h, thickness, r, g, b, a)` — obrys obdélníku
  - [X] `Gui.DrawShadow(x, y, w, h, spread, r, g, b, a)` — vrstvený stín (volat před elementem)
  - [X] `Gui.GetCursorPos()` → `{x, y}`, `Gui.IsMouseOver(x,y,w,h)`, `Gui.IsMouseDown(btn?)`, `Gui.IsMouseClicked(btn?)`
  - [X] `UI.Window(opts)` — Lua menu framework s fade animací, tlačítky, labely a separátory
  - [X] `CreateThread(fn)` — spustí Lua coroutinu (mlua `Thread` uložena přes `RegistryKey`)
  - [X] `Wait(ms)` — alias pro `coroutine.yield(ms)`; 0 = příští frame
  - [X] `tick_lua_threads` systém (PreUpdate) — resumuje thready jejichž wait timer vypršel
  - [X] `GuiRenderPlugin` — `Camera2d` (order 10, no clear, `RenderLayers::layer(31)`) + pool 256 rect + 48 text + 16 image + 128 disc entit
  - [X] `resources/example/hud/` — demo HUD (health bar, crosshair, FPS counter)
  - [X] `resources/example/esc_menu/` — ESC pauza menu (`UI.Window` framework)
- [ ] Integrovat `sqlx` a namapovat Lua Database exporty (základ přítomen jako stub)
- [ ] Umožnit Lua resources registrovat vlastní WGSL shadery a aplikovat je na materiály
- [X] **Apparatus Drawable System (ADS)** — data-driven asset container (`[name].glb` + `[name].drawable` TOML manifest)
  - [X] `DrawableManifest` serde struktury (`MaterialDef`, `EntityDef::MESH/COLLISION`, `TextureInfo`, `MaterialParams`)
  - [X] `DrawableManifestLoader` — Bevy `AssetLoader` pro příponu `.drawable` (TOML 1.1)
  - [X] `DrawableManifestRegistry` — mapa `model_name → Handle<DrawableManifest>`, plněná z `NativeAssetsPlugin`
  - [X] `TextureRegistry` — globální cache sdílených DDS textur (`source = "shared"`)
  - [X] `DrawableMaterial` = `ExtendedMaterial<StandardMaterial, DrawableExtension>` s weather parametry
  - [X] `drawable_extension.wgsl` — fragment shader: vertex color masky (R=layer, G=dirt, B=wet, A=palette), sníh, špína, vlhkost, 1D paleta LUT
  - [X] `SceneReadyId` observer (`On<SceneInstanceReady>`) + `hook_drawable_scenes` polling systém
  - [X] Vertex color sanitizace (fill `[0,0,0,0]` pokud mesh nemá `ATTRIBUTE_COLOR`)
  - [X] Material swap: GLTF `StandardMaterial` → `DrawableMaterial` podle `[entities]`/`[materials]` v manifestu
  - [X] COL_ uzly: Schování (`Visibility::Hidden`) — Phase 5 přidá Avian colliders
  - [X] `NativeAssetsPlugin` rozšířen o scan `assets/models/*.drawable`

---

### Phase 5 — FPS Core Systems

**Filosofie Phase 5:** Rust Core implementuje fyzikální engine, datové kontrakty a autoritativní simulaci. Lua Resources definují vše herně specifické — zbraně, munici, hitboxy, herní módy. Žádná konkrétní zbraň nebo herní pravidlo se nesmí hardcodovat do Rustu.

---

#### 5.1 — Weapon & Ammo Registry

Datové schéma zbraní, munice a doplňků definované v Lua, uložené v Rust Bevy Resources.

**Rust side:**
- [ ] `WeaponDef` struct — kompletní definice zbraně (viz schéma níže)
- [ ] `AmmoDef` struct — balistická data náboje (viz schéma níže)
- [ ] `AttachmentDef` struct — modifikátory doplňků
- [ ] `MaterialDef` struct — vlastnosti materiálu pro penetraci
- [ ] `WeaponRegistry(HashMap<String, WeaponDef>)` Bevy Resource
- [ ] `AmmoRegistry(HashMap<String, AmmoDef>)` Bevy Resource
- [ ] `AttachmentRegistry(HashMap<String, AttachmentDef>)` Bevy Resource
- [ ] `MaterialRegistry(HashMap<String, MaterialDef>)` Bevy Resource
- [ ] Lua API: `Weapon.Register(id, def)`, `Ammo.Register(id, def)`, `Attachment.Register(id, def)`, `Material.Register(id, def)`
- [ ] Lua API: `Weapon.Get(id)` → readonly tabulka, `Ammo.Get(id)` → readonly tabulka

**WeaponDef schéma (Lua resource definuje):**
```lua
Weapon.Register('ak47', {
    display_name        = 'AK-47',
    category            = 'rifle',      -- pistol|smg|rifle|shotgun|lmg|sniper|launcher|melee

    -- Náboj a hlaveň
    caliber             = '7.62x39',
    default_ammo        = '7.62x39_fmj',
    barrel_length_mm    = 415,          -- ovlivňuje úsťovou rychlost přes AmmoDef.velocity_per_mm
    twist_rate_inches   = 9.45,         -- stabilizace (1 otáčka za N palců) — vliv na těžké střely

    -- Střelba
    fire_modes          = { 'semi', 'full' },
    default_fire_mode   = 'full',
    rpm                 = 600,
    fire_from_open_bolt = false,        -- open-bolt zbraně (SMG) mají jiný delay

    -- Zásobník
    mag_capacity        = 30,
    reload_empty_sec    = 3.2,          -- bez náboje v komoře (bolt lock open)
    reload_tactical_sec = 2.6,          -- náboj v komoře zůstává

    -- Přesnost — spread ve stupních
    spread = {
        base         = 0.15,
        moving       = 0.45,
        sprinting    = 1.20,
        crouch       = 0.08,
        prone        = 0.04,
        ads          = 0.05,
        ads_moving   = 0.18,
        per_shot     = 0.06,           -- nárůst spreadu za výstřel (tepelné roztahování)
        recovery_rps = 3.5,            -- recovery stupeň/sekundu po přestání střelby
    },

    -- Recoil pattern — seznam {x, y} offsetů (stupeň/výstřel), aplikuje se postupně
    recoil_pattern = {
        {x =  0.00, y = 0.28}, {x =  0.05, y = 0.26}, {x = -0.08, y = 0.25},
        {x =  0.06, y = 0.24}, {x = -0.04, y = 0.23}, {x =  0.09, y = 0.22},
        -- pattern se po vyčerpání opakuje od indexu recoil_loop_from
    },
    recoil_loop_from    = 4,            -- od tohoto indexu se pattern opakuje (plateau fáze)
    recoil_recovery_dps = 8.0,          -- stupeň/sekundu recovery po uvolnění spouště

    -- Mířidla
    ads_fov_mult        = 0.75,         -- FOV multiplikátor při ADS
    ads_time_sec        = 0.22,         -- čas přechodu do/z ADS

    -- Fyzické vlastnosti
    mass_kg             = 3.47,         -- ovlivňuje pohybový postih
    length_folded_mm    = 645,
    length_extended_mm  = 875,

    -- Doplňky
    slots = {
        optic        = true,
        muzzle       = true,            -- závit: definuje kompatibilní doplňky
        barrel       = false,           -- AK nemá swappable barrel
        underbarrel  = true,
        stock        = true,
        magazine     = true,
        pistol_grip  = false,
    },
    muzzle_thread       = 'M14x1LH',

    -- Brokovnicové shotguny
    pellet_count        = 1,            -- > 1 pro buckshot (každý pellet má vlastní balistiku)
    choke               = 'none',       -- none|improved_cylinder|modified|full
})
```

**AmmoDef schéma:**
```lua
Ammo.Register('7.62x39_fmj', {
    display_name            = '7.62×39mm FMJ',
    caliber                 = '7.62x39',

    -- Střela
    bullet_mass_g           = 8.0,
    bullet_diameter_mm      = 7.92,

    -- Balistika (referenční hodnoty)
    muzzle_velocity_mps     = 715,      -- při reference_barrel_mm
    reference_barrel_mm     = 415,
    velocity_per_mm         = 0.55,     -- Δm/s na mm hlavně nad/pod referenci
                                        -- (záporné = kratší hlaveň ztrácí rychlost)
    -- Odpor vzduchu
    ballistic_model         = 'G7',     -- G1|G7|custom
    ballistic_coeff         = 0.255,    -- G7 BC — vyšší = méně odporu
    -- nebo zjednodušený model:
    -- drag_coefficient     = 0.47,

    -- Poškození
    base_damage             = 42.0,     -- při dopadu s plnou energií (muzzle velocity)
    damage_velocity_ref_mps = 715,      -- rychlost při které je base_damage platné
    -- Damage se škáluje: dmg = base_damage * (impact_vel / damage_velocity_ref)^1.5

    -- Penetrace
    penetration_class       = 3,        -- 1=FMJ nízký, 2=FMJ std, 3=FMJ těžký, 4=AP, 5=AP+, 6=API
    armor_penetration       = 0.45,     -- frakce poškození procházející skrz brnění (0=žádná, 1=full)
    penetration_energy_j    = 2100,     -- kinetická energie [J] potřebná k průniku materiálem
    after_penetration_mult  = 0.65,     -- multiplikátor velocity po průniku (zbytek energie)

    -- Efekty střely
    fragmentation_vel_mps   = 600,      -- pod touto rychlostí střela nefragmentuje
    fragmentation_factor    = 0.0,      -- 0=žádná expanze (FMJ), 1=plná expanze (hollow point)
    wound_mult              = 1.0,      -- násobič wound kanálu

    -- Speciální
    tracer                  = false,
    incendiary              = false,
    explosive               = false,
    subsonic                = false,    -- nemá sonic crack, jiný zvukový podpis při tlumení
    bleed_chance            = 0.0,      -- 0..1 šance na krvácení (hollow point > 0)
    bleed_dps               = 0.0,      -- HP/s ztracené krvácením

    -- Zvuk
    crack_range_m           = 400,      -- nadzvukový crack slyšitelný do N metrů od trajektorie
    thump_range_m           = 1500,     -- úsťový tlak slyšitelný do N metrů od ústí
})

-- Příklady dalších typů
Ammo.Register('7.62x39_hp', {
    -- ... stejná balistika jako FMJ, ale:
    fragmentation_vel_mps   = 400,
    fragmentation_factor    = 0.85,    -- hollow point se plně expanduje
    wound_mult              = 1.6,
    armor_penetration       = 0.10,    -- špatná penetrace pancíře
    bleed_chance            = 0.65,
    bleed_dps               = 3.5,
})

Ammo.Register('7.62x39_ap', {
    penetration_class       = 5,
    armor_penetration       = 0.85,
    penetration_energy_j    = 3800,
    base_damage             = 36.0,    -- AP náboje mívají nižší wound damage
})

Ammo.Register('7.62x39_subsonic', {
    muzzle_velocity_mps     = 295,     -- pod 343 m/s (rychlost zvuku)
    subsonic                = true,
    crack_range_m           = 0,       -- žádný sonic crack
    base_damage             = 28.0,    -- méně kinetické energie
    bullet_mass_g           = 12.5,    -- těžší střela kompenzuje nízkou rychlost
})
```

**AttachmentDef schéma:**
```lua
Attachment.Register('suppressor_ak_pbs4', {
    display_name            = 'PBS-4 Suppressor',
    slot                    = 'muzzle',
    compatible_threads      = { 'M14x1LH' },

    -- Balistické modifikátory
    velocity_mult           = 0.96,    -- ztráta rychlosti díky prodloužení cesty plynu
    barrel_length_delta_mm  = 215,     -- prodloužení efektivní délky hlavně

    -- Zvukové modifikátory
    db_reduction            = 28,      -- snížení hladiny akustického tlaku [dB]
    changes_sonic_signature = true,    -- mění směrovost zvuku

    -- Handling
    ads_time_delta_sec      = 0.04,    -- zpomalení ADS kvůli hmotnosti
    recoil_mult             = 0.90,    -- tlumič snižuje zpětný ráz
    mass_kg                 = 0.55,

    -- Vizuál
    model                   = 'suppressor_pbs4',
})

Attachment.Register('acog_ta31', {
    slot                    = 'optic',
    display_name            = 'ACOG TA31 4×32',
    ads_fov_mult_override   = 0.25,    -- přepíše ads_fov_mult zbraně
    ads_time_delta_sec      = 0.06,    -- těžší optika zpomaluje ADS
    zero_range_m            = 300,     -- nastavení nulového bodu
    model                   = 'optic_acog_ta31',
})
```

**MaterialDef schéma (pro penetraci):**
```lua
Material.Register('steel_3mm', {
    display_name        = 'Ocel 3mm',
    thickness_mm        = 3.0,
    hardness_brinell    = 200,
    required_energy_j   = 900,         -- minimální kinetická energie J pro průnik
    velocity_retention  = 0.58,        -- frakce velocity zachovaná po průniku (0..1)
    ricochet_angle_deg  = 15,          -- pod tímto úhlem dopadu se střela odráží
})

Material.Register('wood_25mm',  { thickness_mm=25, required_energy_j=120, velocity_retention=0.85 })
Material.Register('concrete_100mm', { thickness_mm=100, required_energy_j=4000, velocity_retention=0.10 })
Material.Register('glass_5mm',  { thickness_mm=5, required_energy_j=80, velocity_retention=0.92 })
```

---

#### 5.2 — Balistický engine

Fyzikálně přesná simulace letu střely na serveru. Klient dostane výsledky přes eventi.

**Rust side:**
- [ ] `BallisticsPlugin` — registruje systémy a resources
- [ ] `BulletProjectile` component — fyzikální stav střely v letu:
  - `position: Vec3`, `velocity: Vec3` (m/s), `remaining_energy_j: f32`
  - `ammo_id: String`, `source_entity: Entity`, `team: u8`
  - `spawn_tick: u32` (pro lag compensation)
- [ ] `ProjectileSimulator` systém (FixedUpdate) — integruje pozici, aplikuje drag a gravitaci:
  - Drag model: `F_drag = 0.5 * rho * Cd * A * v²` (nebo G1/G7 BC výpočet)
  - Gravity: `v.y -= 9.81 * dt`
  - Energie: `E = 0.5 * mass_kg * v²` — aktualizuje se každý tick
- [ ] `MuzzleVelocityCalc` utility: `effective_muzzle_vel(ammo, barrel_mm, attachments) -> f32`
  - Vzorec: `v = ammo.muzzle_velocity + (barrel_mm - ammo.reference_barrel_mm) * ammo.velocity_per_mm`
  - Doplněk na ústí (tlumič) přidá svůj `velocity_mult` a `barrel_length_delta_mm`
- [ ] Hitscan vs. projectile rozhodnutí:
  - `v_muzzle > 700 m/s` → hitscan raycast s lag compensací (instantní, serveru autoritativní)
  - `v_muzzle ≤ 700 m/s` nebo `explosive=true` → skutečný `BulletProjectile` entity
- [ ] `PenetrationResolver` — při průniku materiálu vypočte výstupní velocity ze `MaterialDef`
- [ ] `RicochetResolver` — odraz střely od tvrdého povrchu pod plochým úhlem
- [ ] Lua event `onBulletImpact`: `{shooter, position, normal, material, velocity_mps, penetrated}`
- [ ] Lua event `onBulletFlyby`: `{listener_entity, distance_m, velocity_mps}` — sonic crack pro blízké hráče

---

#### 5.3 — Hitbox systém a server-side hit detection

Autoritativní detekce zásahu na serveru s lag compensací.

**Rust side:**
- [ ] `HitboxDef` struct — definice hitboxů pro model (kapsle na kostech)
- [ ] `HitboxRegistry(HashMap<String, HitboxDef>)` Bevy Resource
- [ ] `PlayerHitbox` component — reference na `HitboxDef` podle aktivního modelu hráče
- [ ] `PositionHistory` component — ring buffer posledních N ticků pozic/rotací hráče (lag compensation)
- [ ] `LagCompensator` systém — při hit check "přetočí" pozice ostatních hráčů na tick kdy byl výstřel vyslán
- [ ] `HitResolver` systém — pro každý dopadlý raycast / projectile:
  1. Rewind pozice (lag comp)
  2. Test kapsle všech hitboxů
  3. Vrátí zasaženou kost + frakci depth (pro průnik)
  4. Vyvolá `DamageEvent` s hitzone
- [ ] Lua API: `Hitbox.Register(model_id, def)`

**HitboxDef schéma:**
```lua
Hitbox.Register('player_default', {
    -- Každá kost: {mult, armor_bypass, capsule = {radius, half_height, offset_y}}
    bones = {
        head      = { mult = 4.0, armor_bypass = 0.5,  capsule = {r=0.12, hh=0.10, oy=1.75} },
        neck      = { mult = 2.5, armor_bypass = 0.8,  capsule = {r=0.07, hh=0.06, oy=1.55} },
        chest     = { mult = 1.0, armor_bypass = 0.0,  capsule = {r=0.22, hh=0.18, oy=1.30} },
        stomach   = { mult = 0.9, armor_bypass = 0.1,  capsule = {r=0.18, hh=0.12, oy=1.05} },
        pelvis    = { mult = 0.8, armor_bypass = 0.2,  capsule = {r=0.18, hh=0.10, oy=0.90} },
        upper_arm = { mult = 0.7, armor_bypass = 1.0,  capsule = {r=0.07, hh=0.14, oy=1.35} },
        lower_arm = { mult = 0.6, armor_bypass = 1.0,  capsule = {r=0.06, hh=0.12, oy=1.10} },
        upper_leg = { mult = 0.75,armor_bypass = 1.0,  capsule = {r=0.09, hh=0.18, oy=0.60} },
        lower_leg = { mult = 0.65,armor_bypass = 1.0,  capsule = {r=0.07, hh=0.18, oy=0.28} },
    },
    -- Zóna chráněná přilbou / vestou (pro armor_bypass override)
    armor_zones = {
        helmet = { bones = {'head','neck'} },
        vest   = { bones = {'chest','stomach','pelvis'} },
    },
})
```

---

#### 5.4 — Weapon State & Equipment per hráč

**Rust side:**
- [ ] `WeaponSlots` component — 4 sloty: `[primary, secondary, melee, throwable]` — každý `Option<EquippedWeapon>`
- [ ] `EquippedWeapon` struct: `weapon_id, ammo_in_mag, ammo_type_id, attachments: HashMap<slot, id>`
- [ ] `AmmoReserve` component: `HashMap<caliber_id, u32>` — zásoby munice v kapsách
- [ ] `ActiveSlot` component: `u8` — aktuálně držená zbraň (0–3)
- [ ] `ReloadState` component — FSM: `Idle | Reloading { elapsed, duration, reload_type }`
- [ ] `FireState` component — `Ready | Firing { shots_fired } | Cooling { remaining_sec }`
- [ ] `WeaponSwapState` component — `Ready | Swapping { elapsed, duration }`
- [ ] `reload_system` (FixedUpdate) — tickuje `ReloadState`, na dokončení přeplní zásobník
- [ ] `fire_system` (FixedUpdate) — čte PRIMARY_FIRE z `LastPlayerInputs`, kontroluje `FireState` + `ReloadState`, spawne střelu / provede hitscan
- [ ] Lua API — čtení/mutace equipment:
  - `Weapon.GetEquipped(player_id)` → `{weapon_id, ammo_in_mag, ammo_type, attachments}`
  - `Weapon.SetEquipped(player_id, slot, weapon_id, ammo_type?)`
  - `Weapon.GetAmmoReserve(player_id, caliber)` → `integer`
  - `Weapon.SetAmmoReserve(player_id, caliber, count)`
  - `Weapon.GetActiveSlot(player_id)` → `0..3`
  - `Weapon.ForceReload(player_id)`
- [ ] Nové `PlayerInput` bity: `RELOAD` (bit 2), `WEAPON_SLOT_1..4` (bity 13–16), `ADS` (bit 12)

---

#### 5.5 — Armor & rozšířený damage pipeline

**Rust side:**
- [ ] `ArmorComponent` component:
  ```rust
  pub struct ArmorComponent {
      pub helmet:   Option<ArmorPiece>,  // { class: u8, durability: f32, max_durability: f32 }
      pub vest:     Option<ArmorPiece>,
  }
  ```
- [ ] `ArmorClass` enum: `I | II | IIIa | III | IV` (NIJ standard) — mapuje na `absorption_table`
- [ ] `absorption_table` — 2D lookup: `[armor_class][penetration_class] → frakce pohlceného poškození`
- [ ] `ArmorDurabilitySystem` — snižuje `durability` při každém zásahu (degradace pancíře)
- [ ] Rozšířený damage pipeline (nahrazuje prostý `health.current -= damage`):
  1. `impact_velocity` z `BulletProjectile` nebo hitscan (vzdálenost→velocity přes drag model)
  2. `kinetic_energy_j = 0.5 * mass_kg * v²`
  3. Penetrace armoru: porovnej `penetration_energy_j` vs `armor.durability` pro daný armor_class
  4. Pokud nepenetrovalo: `damage *= (1.0 - armor_absorption)` + degraduj brnění
  5. Pokud penetrovalo: plné poškození + degraduj brnění (`after_penetration_mult`)
  6. Aplikuj hitbox multiplikátor (`hitzone_mult`)
  7. Aplikuj `wound_mult` z AmmoDef
  8. `health.current -= final_damage`
- [ ] Status efekty komponent: `BleedEffect { dps, remaining_sec }`, `BurnEffect { dps, remaining_sec }`
- [ ] `status_effect_system` (FixedUpdate) — tickuje efekty, aplikuje poškození z krvácení/hoření
- [ ] Lua event `onPlayerDamage` (bohatší než současný `onPlayerHit`):
  ```json
  {
    "attacker": "12345", "victim": "67890",
    "weapon": "ak47", "ammo": "7.62x39_ap",
    "hitzone": "chest", "final_damage": 38.4,
    "raw_damage": 42.0, "armor_absorbed": 3.6,
    "penetrated_armor": true, "distance_m": 45.2,
    "impact_velocity_mps": 698.3, "headshot": false,
    "through_wall": false
  }
  ```
- [ ] Lua API: `Player.GetArmor(player_id)` → `{helmet, vest}`, `Player.SetArmor(player_id, slot, class, durability)`

---

#### 5.6 — Rozšířená fyzika hráče

**Rust side — nové `PlayerInput` bity:**
- `LEAN_LEFT` (bit 8), `LEAN_RIGHT` (bit 9)
- `PRONE` (bit 10)
- `VAULT` (bit 11)
- `ADS` (bit 12)

**Nové komponenty:**
- [ ] `PlayerStance` component: `Standing | Crouching | Prone | Vaulting`
- [ ] `LeanState` component: `None | Left(f32) | Right(f32)` — frakce náklonu (0..1 lerp)
- [ ] `StaminaComponent` component: `current: f32, max: f32` (sprint/skok čerpá, regeneruje)
- [ ] `AdsState` component: `Idle | Entering { progress } | Ads | Leaving { progress }` + `fov_mult: f32`
- [ ] `VaultState` component — FSM pro mantle přes překážku

**Nové systémy:**
- [ ] `stance_system` (FixedUpdate) — přechody Standing↔Crouching↔Prone, kolizní výška capsule
- [ ] `lean_system` (FixedUpdate) — aplikuje horizontální offset + roll kamery, zachovává hitbox
- [ ] `stamina_system` (FixedUpdate) — čerpá při sprint/jump, regeneruje jinak
- [ ] `ads_system` (FixedUpdate) — lerp FOV multiplier (posílán klientovi jako `onAdsStateChange` event)
- [ ] `vault_system` (FixedUpdate) — detekce nízké překážky (raycast dopředu + nahoru), arc pohyb

**Kolizní výška dle stance:**
| Stance | Capsule height | Camera offset Y |
|--------|---------------|-----------------|
| Standing | 1.80 m | 1.65 m |
| Crouching | 1.10 m | 0.95 m |
| Prone | 0.40 m | 0.30 m |

**Vliv staminy na balistiku:**
- `stamina < 30%` → spread `+0.3°`, ADS sway `×1.5`
- `stamina == 0` → nelze sprintovat, spread `+0.8°`

---

#### 5.7 — Recoil, Spread & ADS na serveru

- [ ] `RecoilAccumulator` component: `offset: Vec2, shot_index: usize`
- [ ] `recoil_system` (FixedUpdate):
  - Při výstřelu: přidej `recoil_pattern[shot_index % loop_from + wrapping]` do `offset`
  - Každý tick bez střelby: `offset → ZERO` rychlostí `recoil_recovery_dps * dt`
  - Recoil offset se přidává k `PlayerInput.look` před simulací pohybu
- [ ] `spread_calculator` utility: vrátí aktuální spread zbraně dle stance + ADS + pohybu + staminy
- [ ] Server posílá klientovi `onRecoilUpdate` event → klient animuje vizuální zpětný ráz (odděleno od autoritativního)
- [ ] Shotgun pellet fan: pro `pellet_count > 1` server spawne N hitscanů s equal-angle rozptylem v choke kuželi

---

#### 5.8 — Audio Events Bridge

Rust emituje zvukové události do LocalEventBus; Lua resource `core/audio` rozhodne, kteří hráči slyší co.

- [ ] `onGunfire` event (server):
  ```json
  {
    "shooter": "12345", "position": {x,y,z}, "direction": {x,y,z},
    "weapon": "ak47", "ammo": "7.62x39_fmj", "suppressed": false,
    "muzzle_velocity_mps": 715, "crack_range_m": 400, "thump_range_m": 1500
  }
  ```
- [ ] `onFootstep` event (server):
  ```json
  { "entity": "12345", "position": {x,y,z}, "surface": "concrete", "stance": "standing", "speed": 4.5 }
  ```
- [ ] `onExplosion` event: `{ position, radius_m, sound_range_m, visual_radius_m }`
- [ ] `onReload` event: `{ player, weapon, type }` — `type = "empty"|"tactical"`
- [ ] `FootstepDetector` systém (FixedUpdate) — porovná pohyb hráčů, emituje footstep event každých N metrů
- [ ] Lua resource `core/audio` zodpovídá za proximitu výpočet a `TriggerClientEvent` příslušným hráčům

---

#### 5.9 — Kill Feed & Hit Confirmation

- [ ] `onHitConfirm` unicast k útočníkovi po každém zásahu:
  ```json
  { "victim": "67890", "hitzone": "head", "damage": 150.0, "kill": true, "headshot": true }
  ```
- [ ] `onPlayerKill` broadcast (nebo Lua resource posílá kill feed):
  ```json
  {
    "killer": "12345", "victim": "67890",
    "weapon": "ak47", "ammo": "7.62x39_fmj",
    "headshot": true, "distance_m": 87.3, "through_wall": false
  }
  ```
- [ ] `KillStreak` component per hráč: `current_streak: u32, best_streak: u32`
- [ ] `kill_streak_system` — inkrementuje při kill, resetuje při smrti
- [ ] Lua event `onKillStreak`: `{ player, streak }` — každých 5 killů nebo na milnících

---

#### 5.10 — Spawn System

Spawn logic zůstává plně v Lua resources; Rust poskytuje primitivy.

- [ ] `SpawnPoint` component: `team: Option<u8>, active: bool`
- [ ] `Spawn.Register(id, pos, rot, opts?)` Lua API — spawne `SpawnPoint` entitu do ECS
- [ ] `Spawn.GetFree(team?)` Lua API → `{id, pos, rot}` nebo `nil` — vrátí spawn point s nejmenší nepřátelskou hrozbou (vzdálenosti hráčů)
- [ ] `Spawn.SetActive(id, bool)` Lua API — aktivace/deaktivace spawn pointu
- [ ] `Spawn.GetAll()` Lua API → tabulka všech spawn pointů
- [ ] `RespawnTimer` component per hráč: `remaining_sec: f32` (tickuje v FixedUpdate, emituje `onRespawnReady`)
- [ ] Lua event `onRespawnReady`: `{ player }` — Lua resource zavolá respawn logiku
- [ ] Server Lua resource `core/spawns` → implementuje respawn delay, výběr bodu, team balance

---

#### 5.11 — Round State & Game Mode Foundation

Rust poskytuje jen časovač a eventy; herní logika je v Lua.

- [ ] `RoundState` Bevy Resource: `enum { WarmUp, Active { elapsed_sec }, PostRound { winner_team } }`
- [ ] `round_timer_system` (FixedUpdate) — tickuje `RoundState::Active.elapsed_sec`
- [ ] Lua API: `Round.GetState()` → `{ phase, elapsed, time_limit }`
- [ ] Lua API: `Round.SetTimeLimit(sec)`, `Round.End(winner_team?)`
- [ ] Lua event `onRoundStart`, `onRoundEnd { winner_team, scores }`, `onRoundTick { elapsed, remaining }`
- [ ] `TeamAssignment` component per hráč: `team: u8` (0 = unassigned)
- [ ] Lua API: `Player.GetTeam(player_id)` → `u8`, `Player.SetTeam(player_id, team)`
- [ ] `ScoreBoard` Bevy Resource: `HashMap<u64, PlayerScore>` (`kills, deaths, assists, score`)
- [ ] Lua API: `Score.Add(player_id, kills?, deaths?, assists?, score?)`, `Score.Get(player_id)`, `Score.GetAll()`

---

#### Phase 5 — Nová Lua API (přehled)

| Namespace | Funkce | Strana | Popis |
|-----------|--------|--------|-------|
| `Weapon.Register(id, def)` | server | Zaregistruje WeaponDef do registry |
| `Weapon.Get(id)` | both | Vrátí readonly def nebo nil |
| `Weapon.GetEquipped(pid)` | server | Equipped weapon state hráče |
| `Weapon.SetEquipped(pid, slot, wid)` | server | Vybaví hráče zbraní |
| `Weapon.GetAmmoReserve(pid, cal)` | server | Počet nábojů daného kalibru |
| `Weapon.SetAmmoReserve(pid, cal, n)` | server | Nastaví zásobu munice |
| `Weapon.ForceReload(pid)` | server | Vynutí přebíjení |
| `Ammo.Register(id, def)` | server | Zaregistruje AmmoDef |
| `Attachment.Register(id, def)` | server | Zaregistruje AttachmentDef |
| `Material.Register(id, def)` | server | Zaregistruje MaterialDef |
| `Hitbox.Register(model, def)` | server | Zaregistruje hitbox definition |
| `Player.GetArmor(pid)` | server | Stav brnění hráče |
| `Player.SetArmor(pid, slot, cls, dur)` | server | Nastaví brnění |
| `Player.GetTeam(pid)` | both | Team assignment |
| `Player.SetTeam(pid, team)` | server | Přiřadí hráče do týmu |
| `Player.GetStamina(pid)` | server | Aktuální stamina |
| `Player.GetStance(pid)` | server | standing\|crouching\|prone |
| `Spawn.Register(id, pos, rot, opts)` | server | Zaregistruje spawn point |
| `Spawn.GetFree(team?)` | server | Vrátí volný spawn point |
| `Spawn.SetActive(id, bool)` | server | Aktivace spawn pointu |
| `Round.GetState()` | both | Stav kola |
| `Round.SetTimeLimit(sec)` | server | Nastaví délku kola |
| `Round.End(winner?)` | server | Ukončí kolo |
| `Score.Add(pid, opts)` | server | Přidá body hráči |
| `Score.Get(pid)` | both | Skóre jednoho hráče |
| `Score.GetAll()` | both | Celý scoreboard |

---

## Project Layout

```text
/Cargo.toml                      workspace root, sjednocené [workspace.dependencies] (+ serde_json)
/blender_plugin/                Blender authoring tooling pro Apparatus Drawable workflow
  bevy_toolkit.py                 ADS exporter (`.glb` + `.toml`), material/texture metadata, collision entity metadata, vertex mask painting
/core_shared/                    sdílené typy mezi serverem a klientem
  src/lib.rs                       SharedPlugin, LuaEvent (Bevy Message), LuaEventRegistry
/core_resources/                 VFS + manifest + Lua sandbox (Phase 1–3)
  src/types.rs                     ResourceId, Side
  src/manifest.rs                  Manifest, ResourceKind, parse_manifest
  src/vfs.rs                       Vfs (Bevy resource), walkdir scanner, ScanReport, scan_stream_models()
  src/watcher.rs                   notify watcher + debounce, ResourcesDirty (Bevy Message)
  src/resolver.rs                  resolve_load_order (Kahn's, ResolveError)
  src/sandbox.rs                   LuaSandbox, LocalEvent, LocalEventBus, RaycastBridge,
                                     LuaEventOut/LuaEventDirection, JSON helpers
  src/plugin.rs                    ResourcesPlugin, SandboxRegistry (NonSend), dispatch_local_events
  src/cmd_queue.rs                 LuaCommand (incl. SpawnNetworkedObject), CommandQueue,
                                     LuaWorldState, LocalObjectMarker, NetworkedObjectMarker,
                                     PendingDamageEvent, process_lua_commands
  src/model_registry.rs            ModelRegistry, ModelEntry, ModelCommand, ModelCommandQueue,
                                     process_model_commands
/core_net/                       Phase 2–3 — networking, digest, handshake, Lua RPC bridge, sim
  src/protocol.rs                  ServerHello, ClientReady, LuaEventMessage (wire types)
  src/digest.rs                    FileDigest, ResourceDigest, compute_resource_digest (blake3)
  src/digest_cache.rs              ResourceDigestCache + DigestPlugin (server-side)
  src/net_plugin.rs                ProtocolPlugin (channels), ServerNetPlugin, ClientNetPlugin
  src/handshake.rs                 ServerHandshakePlugin + ClientHandshakePlugin (download)
  src/lua_rpc.rs                   ServerLuaRpcPlugin + ClientLuaRpcPlugin (unicast drain/dispatch)
  src/sim.rs                       ServerSimPlugin, Health, WeaponConfig, WeaponCooldown,
                                     LastPlayerInputs, collect_last_inputs, process_combat,
                                     spawn_player_on_connect, emit_player_disconnect,
                                     attach_replication_to_networked_object
/host_server/                    dedicated headless server
  src/main.rs                      MinimalPlugins + Tokio + ServerNetPlugin + DigestPlugin + ...
  src/http_server.rs               Axum HTTP file server (`/resources/<id>/<path>`)
  src/config.rs                    ServerConfig (server.toml: identity/gameplay/net/auth/...)
/server.toml                     repo-tracked default server config (edit jako šablonu)
/host_client/                    herní klient
  src/main.rs                      DefaultPlugins + ClientNetPlugin + ClientHandshakePlugin + ...
  src/config.rs                    ClientConfig (player/network/graphics/quality/audio/ui/input/paths/...)
  src/gameplay.rs                  ClientGameplayPlugin, update_raycast_bridge, collect_and_send_input,
                                     publish_input_state_to_lua (`input:state` local bus event),
                                     3D player visual attach + 1st/3rd person camera follow
  src/native_assets.rs             NativeAssetsPlugin — scan assets/fonts/*.{ttf,otf}, assets/models/*.{glb,gltf,drawable}
  src/drawable/                    Apparatus Drawable System (ADS)
    manifest.rs                      DrawableManifest (serde), MaterialDef, EntityDef, CollisionShape
    loader.rs                        DrawableManifestLoader (AssetLoader, ext=.drawable)
    registry.rs                      DrawableManifestRegistry, TextureRegistry (shared DDS cache)
    material.rs                      DrawableExtension + DrawableMaterial (ExtendedMaterial<StandardMaterial, DrawableExtension>)
    hook.rs                          observe_scene_ready, attach_drawable_intent, hook_drawable_scenes
    mod.rs                           DrawablePlugin
  assets/shaders/drawable_extension.wgsl   Fragment shader: vertex color masky, sníh, špína, vlhkost, paleta LUT
/cache/resources/                lokální cache klienta (download během handshake; gitignored)
/resources/                      game content (Lua + assets) — *server-side autoritativní*
  core/init/                       bootstrap resource (root, no deps)
  example/hello/                   demo závislého resource (depend na core/init)
  example/moving_square/           demo per-player moving actors (`sq:pos` -> `SetTransform`) + `shared/input.lua`
    stream/blacksmith.glb            model pro lokalni spawn (`World.SpawnLocalObject('blacksmith', ...)`)
```

---

## Lua Sandbox Runtime API

Každý resource má vlastní izolovanou `mlua::Lua` instanci. Tyto globály jsou
dostupné ve všech `shared_scripts` / `server_scripts` / `client_scripts`:

| Symbol                                               | Strana      | Význam                                                                                               |
| ---------------------------------------------------- | ----------- | ----------------------------------------------------------------------------------------------------- |
| `RESOURCE_ID`                                      | both        | Kanonická cesta resource (`"core/init"`)                                                           |
| `SIDE`                                             | both        | `"server"` nebo `"client"`                                                                        |
| `IS_SERVER` / `IS_CLIENT`                        | both        | Pohodlný shortcut pro `assert(IS_SERVER)`                                                          |
| `print(...)`                                       | both        | Bevy log info, prefix `[lua:RESOURCE_ID]`                                                           |
| `log_debug(s)` / `log_info(s)` / `log_warn(s)` | both        | Strukturovaný log s explicitní úrovní                                                             |
| `RegisterEvent(name, handler)`                     | both        | Uloží Lua callback. Volaný při `TriggerServerEvent` / `TriggerClientEvent` / `TriggerEvent` |
| `TriggerServerEvent(name, payload?)`               | client only | Pošle `LuaEventMessage` serveru (JSON payload). Volání na serveru je runtime error               |
| `TriggerClientEvent(name, target, payload?)`       | server only | Unicast pokud `target` je `u64` player_id; broadcast pokud `nil`/`false`                      |
| `TriggerEvent(name, payload?)`                     | both        | Cross-sandbox bus uvnitř jednoho procesu — funkční od Phase 3.8                                   |
| `World.SpawnLocalObject(model, pos, rot)`          | both        | Spawne lokální (non-replikovanou) entitu → vrátí `handle` (u64)                                |
| `World.SpawnNetworkedObject(model, pos, rot)`      | server only | Spawne replikovanou entitu (lightyear) → vrátí `handle` (u64)                                    |
| `World.DeleteObject(handle)`                       | both        | Despawne entitu podle handle                                                                          |
| `World.SetTransform(handle, pos, rot)`             | both        | Nastaví pozici + rotaci (Euler XYZ°), zachová scale                                               |
| `World.SetPosition(handle, pos)`                   | both        | Nastaví jen pozici, zachová rotaci a scale                                                           |
| `World.SetRotation(handle, rot)`                   | both        | Nastaví jen rotaci (Euler XYZ°), zachová pozici a scale                                           |
| `World.SetScale(handle, scale)`                    | both        | Nastaví scale — číslo (uniform) nebo `{x,y,z}`                                                   |
| `World.SetModel(handle, model)`                    | both        | Změní jméno modelu entity (Phase 4: swap meshe)                                                    |
| `World.PlayAnimation(handle, name, loop?, speed?)` | both        | Spustí animaci; `loop` default `true`, `speed` default `1.0`                                    |
| `World.StopAnimation(handle)`                      | both        | Zastaví animaci entity                                                                               |
| `World.IsValid(handle)`                            | both        | `true` pokud handle mapuje na živou ECS entitu                                                    |
| `World.IsAlive(handle)`                            | both        | `true` pokud entita existuje a health > 0 (nebo nemá Health komponentu)                         |
| `World.GetHealth(handle)`                          | both        | Vrátí current health nebo `nil` (entita nemá Health komponentu)                                  |
| `World.GetModel(handle)`                           | both        | Vrátí jméno modelu nebo `nil`                                                                       |
| `World.GetPosition(handle)`                        | both        | Vrátí `{x, y, z}` nebo `nil`                                                                      |
| `World.GetRotation(handle)`                        | both        | Vrátí `{x, y, z}` Euler° nebo `nil`                                                               |
| `World.GetQuaternion(handle)`                      | both        | Vrátí `{x, y, z, w}` kvaternion nebo `nil` (přesné rotace)                                    |
| `World.GetScale(handle)`                           | both        | Vrátí `{x, y, z}` nebo `nil`                                                                      |
| `World.GetTransform(handle)`                       | both        | Vrátí `{pos, rot, scale}` najednou nebo `nil`                                                     |
| `World.GetAnimation(handle)`                       | both        | Vrátí název aktuální animace nebo `nil`                                                            |
| `World.GetAnimationSpeed(handle)`                  | both        | Vrátí rychlost animace (default `1.0`)                                                              |
| `World.ApplyDamage(target, amount, source?)`       | server only | Enqueue damage intent do `CommandQueue`                                                             |
| `Engine.RequestModel(name)`                        | both        | Inkrementuje ref_count modelu v `ModelRegistry`                                                     |
| `Engine.HasModelLoaded(name)`                      | both        | Vrátí `true` pokud je model v registry s `ref_count > 0`                                        |
| `Engine.SetModelAsNoLongerNeeded(name)`            | both        | Dekrementuje ref_count modelu                                                                         |
| `Raycast.GetGroundPosition()`                      | client only | Vrátí `{x, y, z}` world-space pozici myši (Y=0 rovina); na serveru vrací `{0,0,0}`            |

Client-only local event bridge (bez síťové replikace):

- `input:state` — payload `{ move = {x, y}, keys = {...} }`, emitován každý frame v `host_client` do `LocalEventBus`; Lua resources ho čtou přes `RegisterEvent("input:state", handler)`.

`payload` je libovolná Lua hodnota (nil, string, number, table) — automaticky serializována jako JSON.
Handler dostane `(payload, sender)`, kde `sender` je `u64` player_id (nebo `nil` pro lokální eventy / server-side handlery).

Pozn.: `TriggerClientEvent(..., target, ...)` podporuje `target` jako integer/number i string (`"123"`) pro bezpečný routing velkých player_id bez ztráty přesnosti v Lua.

**Stdlib povolen:** `string`, `table`, `math`, `utf8`, `coroutine`.
**Stdlib zakázán:** `io`, `os`, `package`, `require`, `debug`, `dofile`, `load`, `loadfile`, `loadstring`.

### Sandbox Isolation — důležité pravidlo

- Každý resource = vlastní Lua VM. **Globální hodnoty (např. `Core = {...}`) se nepropagují mezi resources.**
- Cross-resource API výhradně přes event bus (`TriggerEvent` / `RegisterEvent`), nikdy přes shared globals.
- Manifest parser běží v ještě omezenější VM (jen `string`/`table`/`math`) — manifest je deklarativní DSL, ne runtime.
- `mlua::Lua` je `!Send` ⇒ `SandboxRegistry` je Bevy `NonSend` resource (drží na main threadu). Až budeme v Phase 3 spouštět Lua handlery z paralelních systémů, přepneme `mlua` na `send` feature a obtočíme `Mutex`em.

---

## Client Config — `client.toml` (per-user)

`host_client` na prvním spuštění **vygeneruje** `client.toml` v platform-specific
config dir (přes [`dirs`](https://crates.io/crates/dirs) crate):

| OS      | Cesta                                                   |
| ------- | ------------------------------------------------------- |
| Windows | `%APPDATA%\bevy_game\client.toml`                     |
| Linux   | `~/.config/bevy_game/client.toml`                     |
| macOS   | `~/Library/Application Support/bevy_game/client.toml` |

Resource cache je samostatně v `cache_dir` (Win: `%LOCALAPPDATA%\bevy_game\cache\resources`).
Override cesty: první positional CLI argument, nebo `BEVY_GAME_CLIENT_CONFIG` env variable.

Sekce ([`host_client::config::ClientConfig`](host_client/src/config.rs), `deny_unknown_fields`):

| Sekce                  | Co řídí                                                                               |
| ---------------------- | ---------------------------------------------------------------------------------------- |
| `[player]`           | `name`, `saved_client_id` (0 = generovat), `avatar`                                |
| `[network]`          | `default_server`, `local_bind`, `download_concurrency`, `strict_https`, timeouty |
| `[graphics]`         | `backend` (auto/vulkan/dx12/metal/gl/browser_webgpu), window mode, resolution, vsync   |
| `[graphics.quality]` | `preset`, shadow/texture/AA/SSAO/SSR/volumetric/water/foliage, view distance, LOD bias |
| `[audio]`            | Master + 5 kanálů, output device, spatial audio, mute on focus lost                    |
| `[ui]`               | Jazyk, UI scale, font, HUD opacity, FPS/ping/minimap toggles, crosshair, subtitles       |
| `[input]`            | Mouse sensitivity (ADS i hip), invert Y, raw input, gamepad deadzone, vibration          |
| `[input.keys]`       | 39 keybindings (movement, combat, weapon slots 1–9, UI, screenshot, …)                 |
| `[input.mouse]`      | `attack_primary`, `attack_secondary`, `middle_click`                               |
| `[paths]`            | Cache / screenshot / savegame / mod / log dir overridy                                   |
| `[server_history]`   | Recent / favorite servery, last used username                                            |
| `[advanced]`         | log level/filter, debug/profile overlaye, dev console, GPU validation, preload toggle    |

Phase 2 wire-up: graphics → `RenderPlugin` + `WgpuSettings` + `WindowPlugin`,
`advanced.log_*` → `LogPlugin`, `network.*` → `ClientNetConfig`,
paths.cache_dir → `ClientHandshakeConfig`. Klientsky `AssetPlugin` ma
`unapproved_path_mode = Allow`, protoze stream modely se nacitaji z
absolutnich cest v local cache (`cache/resources/...`). Ostatní pole jsou dostupná
jako `ClientConfigResource` pro Phase 3+ konzumenty (audio mixer, action
mapper, …).

## Server Config — `server.toml`

`host_server` při startu hledá config v pořadí:

1. první positional CLI argument (`host_server.exe my.toml`),
2. `<dir(.exe)>/server.toml` — typický distribuční layout (vedle binárky),
3. `<cwd>/server.toml` — fallback pro `cargo run` z projekt rootu.

Stejná logika platí pro `[resources].root` (typicky `"resources"`) a
`[net].private_key_path`: relativní cesty se nejdřív zkoušejí vedle `.exe`,
když tam neexistují, použijí se relativně k CWD. Absolutní cesty zůstávají
beze změny. Tím **distribuce** s rozložením `host_server.exe + server.toml + resources/ + secrets/netcode.key` v jedné složce funguje out-of-the-box,
zatímco dev `cargo run` z projekt rootu pokračuje fungovat (binárka leží
v `target/debug/`, takže fallback najde projekt-rootový `resources/`).

Sekce (vše má defaulty):

| Sekce           | Co řídí                                                                      |
| --------------- | ------------------------------------------------------------------------------- |
| `[server]`    | Display name, MOTD, tagy, ikona, public listing flag                            |
| `[gameplay]`  | `max_players`, `queue_max`, root gamemode, `idle_kick_sec`                |
| `[net]`       | UDP/HTTP bind, public URL, tickrate,`protocol_id`, klíč, connection timeout |
| `[resources]` | VFS root,`hot_reload`, watcher debounce                                       |
| `[handshake]` | Phase 2/3 timeoutu pro digest delivery a klientův ready signal                 |
| `[auth]`      | `mode = "open" / "token" / "whitelist"` (Phase 4 enforce)                     |
| `[logging]`   | tracing level + per-modul filter                                                |
| `[admin]`     | Phase 4: bind + bearer token pro admin API                                      |
| `[database]`  | Phase 4: sqlx connection string + pool size + migrations                        |
| `[dev]`       | Debug toggles (`auto_acknowledge_clients`, `print_digest_on_startup`, ...)  |

Strukturu definuje [`host_server::config::ServerConfig`](host_server/src/config.rs) — přidat
volbu = přidat pole + popsat v `server.toml`. Neznámá pole odmítáme (`deny_unknown_fields`),
takže překlepy pukají hned při startu.

---

## Cargo Commands

```powershell
# Standardní dev běh — server a klient v samostatných shellech.
# Klient po startu přečte digest, stáhne soubory do cache/resources/
# a teprve pak spustí Lua sandboxy.
cargo run -p host_server
cargo run -p host_client

# Dev s dynamic linkingem (rychlejší inkrementální rebuild)
cargo run -p host_server --features dynamic_linking
cargo run -p host_client --features dynamic_linking

# Release
cargo run -p host_server --release
cargo run -p host_client --release

# Headless server build pro Docker/CI (bez render stacku)
cargo build -p host_server --release

# Validace celého workspace
cargo check --workspace
```

### Síťové porty (default)

| Proto    | Port | Endpoint                                |
| -------- | ---- | --------------------------------------- |
| UDP      | 5000 | lightyear netcode (game traffic)        |
| TCP/HTTP | 8081 | Axum file server (`/resources/<...>`) |

Klient se defaultně connectuje na `127.0.0.1:5000` a `http://127.0.0.1:8081`.
Pro LAN multiplayer prohoď `ClientNetConfig::server` a `ServerHandshakeConfig::http_base_url`.

> Pozn. k Windows: při `cargo run` se `bevy_dylib.dll` najde automaticky. Při přímém spouštění `.exe` přidej `target\debug\deps` do `PATH`.

---

## Workflow Notes

- **Po každé změně, splnění roadmapy nebo rozšíření aktualizuj CLAUDE.md** (sekce Roadmap, Project Layout, Lua Sandbox Runtime API, Cargo Commands).
- **Nová funkce = nový resource**, nikoliv úprava Rust core. Rust core upravujeme jen pokud to vyžaduje nový bridge / nový typ events / nový síťový primitiv.
- Filesystem watcher hot-reloaduje při změně souboru pod `/resources/` automaticky (debounce 150 ms). Pro vypnutí watcheru: `ResourcesPlugin::new(...).with_watch(false)`.
