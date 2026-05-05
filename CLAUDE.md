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

| Oblast      | Technologie                                              |
| ----------- | -------------------------------------------------------- |
| Engine      | Bevy Engine (latest stable), headless-first pro server   |
| Networking  | `lightyear` — prediction, rollback, replication       |
| Scripting   | `mlua` via `bevy_mod_scripting`                      |
| Database    | `sqlx` — PostgreSQL (prod) / SQLite (dev)             |
| Shadery     | WGSL, hot-reload přes Bevy AssetServer                  |
| WebUI / NUI | Dioxus / local HTML nebo `bevy_egui`; Axum (admin API) |

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

### Phase 2 — Network Handshake & File Sync

- [ ] Navázat `lightyear` spojení
- [ ] Implementovat handshake sekvenci pro stažení assetů/skriptů (Server → Client)
- [ ] Implementovat Rust-to-Lua RPC (`TriggerServerEvent`)

### Phase 3 — Universal ECS API (Rust → Lua Bridge)

- [ ] Vystavit komponenty `Stats`, `Transform`, `Inventory` do Lua
- [ ] Vytvořit generický Action/Intent systém (Lua žádá akci, Rust Server validuje)

### Phase 4 — WebUI, DB & QOL

- [ ] Integrovat `sqlx` a namapovat Lua Database exporty
- [ ] Umožnit Lua resources registrovat vlastní WGSL shadery a aplikovat je na materiály
- [ ] Implementovat NUI (CEF/WebView nebo WebUI přes Axum) pro HTML/JS player rozhraní

---

## Project Layout

```text
/Cargo.toml                      workspace root, sjednocené [workspace.dependencies]
/core_shared/                    sdílené typy mezi serverem a klientem
  src/lib.rs                       SharedPlugin, LuaEvent, LuaEventRegistry
/core_resources/                 VFS + manifest + Lua sandbox (Phase 1)
  src/types.rs                     ResourceId, Side
  src/manifest.rs                  Manifest, ResourceKind, parse_manifest
  src/vfs.rs                       Vfs (Bevy resource), walkdir scanner, ScanReport
  src/watcher.rs                   notify watcher + debounce, ResourcesDirty event
  src/resolver.rs                  resolve_load_order (Kahn's, ResolveError)
  src/sandbox.rs                   LuaSandbox (per-resource izolovaná mlua VM)
  src/plugin.rs                    ResourcesPlugin, SandboxRegistry (NonSend)
/host_server/                    dedicated headless server
  src/main.rs                      MinimalPlugins + LogPlugin + Tokio runtime + ServerCorePlugin
/host_client/                    herní klient
  src/main.rs                      DefaultPlugins (winit + render) + ClientCorePlugin
/resources/                      game content (Lua + assets)
  core/init/                       bootstrap resource (root, no deps)
  example/hello/                   demo závislého resource (depend na core/init)
```

---

## Lua Sandbox Runtime API

Každý resource má vlastní izolovanou `mlua::Lua` instanci. Tyto globály jsou
dostupné ve všech `shared_scripts` / `server_scripts` / `client_scripts`:

| Symbol                                         | Typ      | Význam                                       |
| ---------------------------------------------- | -------- | -------------------------------------------- |
| `RESOURCE_ID`                                  | string   | Kanonická cesta resource (`"core/init"`)     |
| `SIDE`                                         | string   | `"server"` nebo `"client"`                   |
| `IS_SERVER` / `IS_CLIENT`                      | boolean  | Pohodlný shortcut pro `assert(IS_SERVER)`    |
| `print(...)`                                   | function | Bevy log info, prefix `[lua:RESOURCE_ID]`    |
| `log_debug(s)` / `log_info(s)` / `log_warn(s)` | function | Strukturovaný log s explicitní úrovní        |
| `TriggerEvent(name, ...)`                      | function | **Phase 3 stub** — bude broadcast na bus     |
| `RegisterEvent(name, handler)`                 | function | **Phase 3 stub** — registrace handleru       |

**Stdlib povolen:** `string`, `table`, `math`, `utf8`, `coroutine`.
**Stdlib zakázán:** `io`, `os`, `package`, `require`, `debug`, `dofile`, `load`, `loadfile`, `loadstring`.

### Sandbox Isolation — důležité pravidlo

- Každý resource = vlastní Lua VM. **Globální hodnoty (např. `Core = {...}`) se nepropagují mezi resources.**
- Cross-resource API výhradně přes event bus (`TriggerEvent` / `RegisterEvent`), nikdy přes shared globals.
- Manifest parser běží v ještě omezenější VM (jen `string`/`table`/`math`) — manifest je deklarativní DSL, ne runtime.
- `mlua::Lua` je `!Send` ⇒ `SandboxRegistry` je Bevy `NonSend` resource (drží na main threadu). Až budeme v Phase 3 spouštět Lua handlery z paralelních systémů, přepneme `mlua` na `send` feature a obtočíme `Mutex`em.

---

## Cargo Commands

```powershell
# Standardní dev běh
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

> Pozn. k Windows: při `cargo run` se `bevy_dylib.dll` najde automaticky. Při přímém spouštění `.exe` přidej `target\debug\deps` do `PATH`.

---

## Workflow Notes

- **Po každé změně, splnění roadmapy nebo rozšíření aktualizuj CLAUDE.md** (sekce Roadmap, Project Layout, Lua Sandbox Runtime API, Cargo Commands).
- **Nová funkce = nový resource**, nikoliv úprava Rust core. Rust core upravujeme jen pokud to vyžaduje nový bridge / nový typ events / nový síťový primitiv.
- Filesystem watcher hot-reloaduje při změně souboru pod `/resources/` automaticky (debounce 150 ms). Pro vypnutí watcheru: `ResourcesPlugin::new(...).with_watch(false)`.
