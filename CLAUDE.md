# Project Context: FiveM-Style Modular Multiplayer Framework

---

## Vision

Genre-agnostic, high-performance multiplayer game framework v Rustu (Bevy Engine).
Architektura napodobuje FiveM/CitizenFX:

- **Rust Core** = "Host Shell" — ECS, networking, physics, rendering, DB pooly
- **Lua Resources** = veškerá herní logika, obsah a UI (hot-reloadable)

Podporuje libovolný žánr (FPS, Extraction Shooter, Survival, Turn-based RPG) pouhým swapem Lua resources — bez rekompilace Rust core.

---

## Tech Stack

| Oblast       | Technologie                                              |
|--------------|----------------------------------------------------------|
| Engine       | Bevy Engine (latest stable), headless-first pro server   |
| Networking   | `lightyear` — prediction, rollback, replication          |
| Scripting    | `mlua` via `bevy_mod_scripting`                          |
| Database     | `sqlx` — PostgreSQL (prod) / SQLite (dev)                |
| Shadery      | WGSL, hot-reload přes Bevy AssetServer                   |
| WebUI / NUI  | Dioxus / local HTML nebo `bevy_egui`; Axum (admin API)   |

---

## Core Architecture: Resource Paradigm

```
/resources/[category]/[resource_name]/
```

| Vrstva           | Odpovědnost                                                         |
|------------------|---------------------------------------------------------------------|
| Rust Core        | `lightyear`, WGPU, SQLx pool, Lua Sandbox API (`mlua`)              |
| Resource Manifest| `manifest.lua` — metadata, závislosti, seznam souborů               |
| Lua Layer        | Herní logika — stats, inventory, items, game loop                   |

### Pravidlo: Nová funkce = nový Resource

> **NIKDY neupravuj Rust core**, pokud to není nezbytně nutné. Vždy vytvoř nový Resource.

#### Příklad `manifest.lua`

```lua
-- /resources/survival/metabolism/manifest.lua
resource_type 'script'
author      'Developer'
version     '1.0.0'

dependencies {
    'core_inventory',
    'core_stats',
}

client_scripts  { 'client/ui_hunger.lua' }
server_scripts  { 'server/decay_loop.lua' }
shared_scripts  { 'shared/config.lua' }

files {
    'assets/ui_icons.png',
    'shaders/screen_blur_low_health.wgsl',
}
```

---

## Coding Standards & Guardrails

### 1. Rust / Lua Boundary

| Pravidlo       | Detail                                                                          |
|----------------|---------------------------------------------------------------------------------|
| Entity IDs     | Lua komunikuje se světem výhradně přes Bevy Entity ID (integer / opaque userdata) |
| Data Access    | Žádné raw Rust pointery do Lua. Používej safe bridge: `SetComponent(entity, "Hunger", 50)` |
| Events         | Globální Event Bus. Rust překládá network eventy na Lua hooky: `TriggerEvent("onPlayerJoin", player_id)` |

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

### Phase 1 — Shell & VFS

- [ ] Vytvořit `Shared`, `Client` a `Server` Bevy Pluginy
- [ ] Implementovat VFS sledující složku `/resources/`
- [ ] Parsovat `manifest.lua`, řešit závislosti, spouštět izolované Lua prostředí

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
