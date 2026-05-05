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

| Oblast      | Technologie                                                                       |
| ----------- | --------------------------------------------------------------------------------- |
| Engine      | Bevy Engine 0.18, headless-first pro server                                       |
| Networking  | `lightyear` 0.26 — UDP netcode, channels, replication (Phase 3+)                  |
| File sync   | `axum` HTTP server (host_server) + `reqwest::blocking` downloader (host_client)   |
| Scripting   | `mlua` via `bevy_mod_scripting`                                                   |
| Database    | `sqlx` — PostgreSQL (prod) / SQLite (dev)                                         |
| Shadery     | WGSL, hot-reload přes Bevy AssetServer                                            |
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

- [x] Cargo workspace: `core_shared`, `core_resources`, `host_server`, `host_client`
- [x] `SharedPlugin` (event bus prep), `ServerCorePlugin` (headless + Tokio runtime), `ClientCorePlugin` (DefaultPlugins)
- [x] VFS scanner sledující `/resources/` (`walkdir` + `notify` watcher s 150 ms debounce)
- [x] `manifest.lua` DSL parser v izolovaném mlua VM (omezený stdlib: `string`/`table`/`math`)
- [x] Dependency resolver (Kahn's topological sort, detekce cyklů / missing / self-deps)
- [x] Per-resource izolovaný Lua sandbox (vlastní `mlua::Lua` na resource)

### Phase 2 — Network Handshake & File Sync ✅

- [x] `core_net` crate (wire protocol, file digest, lightyear plugin scaffolding)
- [x] Lightyear 0.26 UDP netcode connection (server listen + client connect)
- [x] Axum HTTP file server na `host_server` (`/resources/<id>/<path>`)
- [x] `ServerHello` digest → klient `reqwest::blocking` download → `ClientReady` handshake
- [x] Klient přepnutý na cache mode (`cache/resources/`); `ResourcesPlugin` watcher hot-reloaduje stažené soubory
- [x] Lua RPC bridge: `TriggerServerEvent`, `TriggerClientEvent`, `RegisterEvent` přes lightyear `LuaEventMessage` kanál

### Phase 3 — Universal ECS Bridge & World Streaming

**Hlavní filozofie:** Rust (Core) = slepý sval (rendering, kolize, netcode, paměť). Lua (Resources) = mozek (pravidla, logika, inventář, ukládání). Vše komunikuje asynchronně přes Intents a Events.

#### 3.1 — Gameplay Foundations ✅

- [x] `PlayerInput` struct (move_dir, look, actions bitfield, client_tick)
- [x] `InputChannel` — sequenced unreliable, 60 Hz tick stream
- [x] `ReplicationSender` attach observer (`Add<LinkOf>`) na transport entitu
- [x] Server-side player spawn observer (`spawn_player_on_connect` při `Add<Connected>`)
- [x] `NetTransform`, `NetVelocity`, `PlayerMarker` s lightyear prediction
- [x] `apply_inputs_to_velocity` + `integrate_velocity` v `FixedUpdate` (`sim.rs`)
- [x] Klientský renderer replikovaných hráčů (Sprite per `PlayerMarker`) + WASD input collection (`gameplay.rs`)

#### 3.2 — Command Queue & Bezpečný Lua Bridge ✅

- [x] `LuaCommand` enum: `SpawnLocalObject`, `DespawnEntity`, `SetTransform`, `ApplyDamage`
- [x] Sdílený `CommandQueue` buffer (`Arc<Mutex<Vec<LuaCommand>>>`) — Lua closures zachytí Arc klon
- [x] Bevy systém `process_lua_commands` (PostUpdate) — bezpečně aplikuje příkazy na ECS svět
- [x] `LuaWorldState` Bevy Resource — mapuje Lua handles (`u64`) na Bevy `Entity`
- [x] `LocalObjectMarker` Component — marker pro lokální (non-lightyear) objekty (Phase 3.4 přidá mesh)
- [x] `PendingDamageEvent` Message — Phase 3.3 combat systémy se přihlásí přes `MessageReader`
- [x] Lua API: `World.SpawnLocalObject(model, pos, rot)` → handle, `World.DeleteObject(handle)`, `World.SetTransform(handle, pos, rot)`, `World.ApplyDamage(target, amount, source?)` (server only)

#### 3.3 — Data-Driven Combat & Akce

- [ ] `WeaponConfig` Bevy resource (fire_rate, range, type) — Lua dodá konfiguraci, Rust ji připne na entitu hráče
- [ ] Rust-side raycast exekuce: čte `actions` bitfield + `WeaponConfig`, provádí fyzikální raycast bez Lua latence
- [ ] Vizuální efekty (jiskry, muzzle flash) a cooldown management na Rust vrstvě
- [ ] Event `onPlayerHit` zpět do Lua event busu (kdo, koho, hit position, zbraň)
- [ ] Server autoritativní validace v Lua: munice, brnění, buffy → `ApplyDamage` intent → `onPlayerDeath`

#### 3.4 — Global Model Registry

- [ ] Rozšíření VFS Scanneru: projití složky `stream/` v každém resource automaticky
- [ ] Slovník `model_name → absolute_path` (konflikty: logovat + first-wins)
- [ ] Reference counting — Rust drží model v RAM/VRAM jen pokud `refcount > 0`
- [ ] Lua API: `Engine.RequestModel(name)` (async load), `Engine.HasModelLoaded(name) -> bool`, `Engine.SetModelAsNoLongerNeeded(name)`

#### 3.5 — YMAP World Objects

- [ ] YMAP JSON formát: `[{"model": "prop_name", "pos": [x,y,z], "rot": [x,y,z]}]`
- [ ] `World.SpawnNetworkedObject(model, pos, rot)` — replikovaná entita (server autoritativní)
- [ ] Mapper/Spooner tool — Lua in-game editor využívající lokální entity + Raycast API
- [ ] Oddělit práci Modellera (`stream/prop.glb` + `prop.col`) od Mappera (YMAP instance JSON)

#### 3.6 — YMAP Streaming & Culling

- [ ] AABB bounding box per-YMAP (dopočítaný při načtení JSON)
- [ ] Klientský streaming: Load/Unload radius → async YMAP load → GPU Instancing (jeden draw call per model)
- [ ] Serverový culling: fyzika YMAPu se načte jen pokud je v AABB živý hráč nebo AI
- [ ] Server Physics Instancing: `.col` soubory, jeden `Shape` per model type → N instancí (šetří RAM)

#### 3.7 — Raycasting API

- [ ] `Raycast.GetMouseHitPosition() -> {x, y, z}` Lua bridge
- [ ] Klientská implementace: screen-to-world ray → Bevy/Rapier raycast → výsledek do Lua callback

#### 3.8 — Zbývající ECS Bridge položky

- [ ] Cross-sandbox `TriggerEvent` bus (uvnitř jednoho procesu) — nahradit Phase 2 no-op stub
- [ ] Per-target unicast `TriggerClientEvent(name, player_id, ...)` — Phase 2 broadcastuje všem
- [ ] Strukturovaný payload (LuaTable ↔ MessagePack) místo string-only
- [ ] Vystavit `Stats`, `Inventory` komponenty do Lua přes bridge
- [ ] `sender` player_id v handlerech (`dispatch_incoming` zatím předává `nil`)

### Phase 4 — WebUI, DB & QOL

- [ ] Integrovat `sqlx` a namapovat Lua Database exporty
- [ ] Umožnit Lua resources registrovat vlastní WGSL shadery a aplikovat je na materiály
- [ ] Implementovat NUI (CEF/WebView nebo WebUI přes Axum) pro HTML/JS player rozhraní

---

## Project Layout

```text
/Cargo.toml                      workspace root, sjednocené [workspace.dependencies]
/core_shared/                    sdílené typy mezi serverem a klientem
  src/lib.rs                       SharedPlugin, LuaEvent (Bevy Message), LuaEventRegistry
/core_resources/                 VFS + manifest + Lua sandbox (Phase 1)
  src/types.rs                     ResourceId, Side
  src/manifest.rs                  Manifest, ResourceKind, parse_manifest
  src/vfs.rs                       Vfs (Bevy resource), walkdir scanner, ScanReport
  src/watcher.rs                   notify watcher + debounce, ResourcesDirty (Bevy Message)
  src/resolver.rs                  resolve_load_order (Kahn's, ResolveError)
  src/sandbox.rs                   LuaSandbox + LuaEventOut/LuaEventDirection (RPC bridge state)
  src/plugin.rs                    ResourcesPlugin, SandboxRegistry (NonSend)
/core_net/                       Phase 2 — networking, digest, handshake, Lua RPC bridge
  src/protocol.rs                  ServerHello, ClientReady, LuaEventMessage (wire types)
  src/digest.rs                    FileDigest, ResourceDigest, compute_resource_digest (blake3)
  src/digest_cache.rs              ResourceDigestCache + DigestPlugin (server-side)
  src/net_plugin.rs                ProtocolPlugin (channels), ServerNetPlugin, ClientNetPlugin
  src/handshake.rs                 ServerHandshakePlugin + ClientHandshakePlugin (download)
  src/lua_rpc.rs                   ServerLuaRpcPlugin + ClientLuaRpcPlugin (drain/dispatch)
/host_server/                    dedicated headless server
  src/main.rs                      MinimalPlugins + Tokio + ServerNetPlugin + DigestPlugin + ...
  src/http_server.rs               Axum HTTP file server (`/resources/<id>/<path>`)
  src/config.rs                    ServerConfig (server.toml: identity/gameplay/net/auth/...)
/server.toml                     repo-tracked default server config (edit jako šablonu)
/host_client/                    herní klient
  src/main.rs                      DefaultPlugins + ClientNetPlugin + ClientHandshakePlugin + ...
  src/config.rs                    ClientConfig (player/network/graphics/quality/audio/ui/input/paths/...)
/cache/resources/                lokální cache klienta (download během handshake; gitignored)
/resources/                      game content (Lua + assets) — *server-side autoritativní*
  core/init/                       bootstrap resource (root, no deps)
  example/hello/                   demo závislého resource (depend na core/init)
```

---

## Lua Sandbox Runtime API

Každý resource má vlastní izolovanou `mlua::Lua` instanci. Tyto globály jsou
dostupné ve všech `shared_scripts` / `server_scripts` / `client_scripts`:

| Symbol                                                | Strana       | Význam                                                                              |
| ----------------------------------------------------- | ------------ | ----------------------------------------------------------------------------------- |
| `RESOURCE_ID`                                         | both         | Kanonická cesta resource (`"core/init"`)                                            |
| `SIDE`                                                | both         | `"server"` nebo `"client"`                                                          |
| `IS_SERVER` / `IS_CLIENT`                             | both         | Pohodlný shortcut pro `assert(IS_SERVER)`                                           |
| `print(...)`                                          | both         | Bevy log info, prefix `[lua:RESOURCE_ID]`                                           |
| `log_debug(s)` / `log_info(s)` / `log_warn(s)`        | both         | Strukturovaný log s explicitní úrovní                                               |
| `RegisterEvent(name, handler)`                        | both         | Uloží Lua callback. Volaný při `TriggerServerEvent` / `TriggerClientEvent`          |
| `TriggerServerEvent(name, payload?)`                  | client only  | Pošle `LuaEventMessage` serveru. Volání na serveru je runtime error                 |
| `TriggerClientEvent(name, target?, payload?)`         | server only  | Pošle klientům (Phase 2 = broadcast, `target` se zatím ignoruje)                    |
| `TriggerEvent(name, ...)`                             | both         | **Phase 3 stub** — cross-sandbox bus uvnitř jednoho procesu                         |

`payload` je v Phase 2 čistý string. Volaný handler dostane `(payload, sender)`,
kde `sender` je `Option<u64>` (Phase 2 vždy `nil` — Phase 3 doplní player_id).
Strukturovaná data (LuaTable ↔ JSON/MessagePack) přidáme v Phase 3.

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

| OS      | Cesta                                                |
| ------- | ---------------------------------------------------- |
| Windows | `%APPDATA%\bevy_game\client.toml`                    |
| Linux   | `~/.config/bevy_game/client.toml`                    |
| macOS   | `~/Library/Application Support/bevy_game/client.toml`|

Resource cache je samostatně v `cache_dir` (Win: `%LOCALAPPDATA%\bevy_game\cache\resources`).
Override cesty: první positional CLI argument, nebo `BEVY_GAME_CLIENT_CONFIG` env variable.

Sekce ([`host_client::config::ClientConfig`](host_client/src/config.rs), `deny_unknown_fields`):

| Sekce               | Co řídí                                                                                  |
| ------------------- | ---------------------------------------------------------------------------------------- |
| `[player]`          | `name`, `saved_client_id` (0 = generovat), `avatar`                                      |
| `[network]`         | `default_server`, `local_bind`, `download_concurrency`, `strict_https`, timeouty         |
| `[graphics]`        | `backend` (auto/vulkan/dx12/metal/gl/browser_webgpu), window mode, resolution, vsync     |
| `[graphics.quality]`| `preset`, shadow/texture/AA/SSAO/SSR/volumetric/water/foliage, view distance, LOD bias   |
| `[audio]`           | Master + 5 kanálů, output device, spatial audio, mute on focus lost                      |
| `[ui]`              | Jazyk, UI scale, font, HUD opacity, FPS/ping/minimap toggles, crosshair, subtitles       |
| `[input]`           | Mouse sensitivity (ADS i hip), invert Y, raw input, gamepad deadzone, vibration          |
| `[input.keys]`      | 39 keybindings (movement, combat, weapon slots 1–9, UI, screenshot, …)                   |
| `[input.mouse]`     | `attack_primary`, `attack_secondary`, `middle_click`                                     |
| `[paths]`           | Cache / screenshot / savegame / mod / log dir overridy                                   |
| `[server_history]`  | Recent / favorite servery, last used username                                            |
| `[advanced]`        | log level/filter, debug/profile overlaye, dev console, GPU validation, preload toggle    |

Phase 2 wire-up: graphics → `RenderPlugin` + `WgpuSettings` + `WindowPlugin`,
`advanced.log_*` → `LogPlugin`, `network.*` → `ClientNetConfig`,
paths.cache_dir → `ClientHandshakeConfig`. Ostatní pole jsou dostupná
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
beze změny. Tím **distribuce** s rozložením `host_server.exe + server.toml +
resources/ + secrets/netcode.key` v jedné složce funguje out-of-the-box,
zatímco dev `cargo run` z projekt rootu pokračuje fungovat (binárka leží
v `target/debug/`, takže fallback najde projekt-rootový `resources/`).

Sekce (vše má defaulty):

| Sekce         | Co řídí                                                                            |
| ------------- | ---------------------------------------------------------------------------------- |
| `[server]`    | Display name, MOTD, tagy, ikona, public listing flag                               |
| `[gameplay]`  | `max_players`, `queue_max`, root gamemode, `idle_kick_sec`                         |
| `[net]`       | UDP/HTTP bind, public URL, tickrate, `protocol_id`, klíč, connection timeout       |
| `[resources]` | VFS root, `hot_reload`, watcher debounce                                           |
| `[handshake]` | Phase 2/3 timeoutu pro digest delivery a klientův ready signal                     |
| `[auth]`      | `mode = "open" / "token" / "whitelist"` (Phase 4 enforce)                          |
| `[logging]`   | tracing level + per-modul filter                                                   |
| `[admin]`     | Phase 4: bind + bearer token pro admin API                                         |
| `[database]`  | Phase 4: sqlx connection string + pool size + migrations                           |
| `[dev]`       | Debug toggles (`auto_acknowledge_clients`, `print_digest_on_startup`, ...)         |

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

| Proto      | Port  | Endpoint                               |
| ---------- | ----- | -------------------------------------- |
| UDP        | 5000  | lightyear netcode (game traffic)       |
| TCP/HTTP   | 8081  | Axum file server (`/resources/<...>`)  |

Klient se defaultně connectuje na `127.0.0.1:5000` a `http://127.0.0.1:8081`.
Pro LAN multiplayer prohoď `ClientNetConfig::server` a `ServerHandshakeConfig::http_base_url`.

> Pozn. k Windows: při `cargo run` se `bevy_dylib.dll` najde automaticky. Při přímém spouštění `.exe` přidej `target\debug\deps` do `PATH`.

---

## Workflow Notes

- **Po každé změně, splnění roadmapy nebo rozšíření aktualizuj CLAUDE.md** (sekce Roadmap, Project Layout, Lua Sandbox Runtime API, Cargo Commands).
- **Nová funkce = nový resource**, nikoliv úprava Rust core. Rust core upravujeme jen pokud to vyžaduje nový bridge / nový typ events / nový síťový primitiv.
- Filesystem watcher hot-reloaduje při změně souboru pod `/resources/` automaticky (debounce 150 ms). Pro vypnutí watcheru: `ResourcesPlugin::new(...).with_watch(false)`.
