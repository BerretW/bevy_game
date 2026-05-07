# Lua Cookbook — Nativní API

Kompletní přehled všech funkcí a konstant, které Rust core vystavuje do každého Lua sandboxu.
Každý resource má vlastní izolovanou VM — globály se mezi resources nesdílejí, komunikace probíhá výhradně přes event bus.

---

## Obsah

1. [Globální konstanty](#1-globální-konstanty)
2. [Logging](#2-logging)
3. [Event system](#3-event-system)
4. [World — entity](#4-world--entity)
   - [Spawn / Despawn](#41-spawn--despawn)
   - [Transform settery](#42-transform-settery)
   - [Transform gettery](#43-transform-gettery)
   - [Model](#44-model)
   - [Animace](#45-animace)
   - [Stav entity](#46-stav-entity)
   - [Combat](#47-combat)
5. [Player](#5-player)
6. [Engine](#6-engine)
7. [Raycast](#7-raycast)
8. [Database](#8-database)
9. [GUI — vykreslování](#9-gui--vykreslování)
   - [Primitivy](#91-primitivy)
   - [Pokročilé tvary](#92-pokročilé-tvary)
   - [Obrázky (sprites)](#93-obrázky-sprites)
   - [Vstup myši](#94-vstup-myši)
   - [UI framework](#95-ui-framework)
10. [Threading](#10-threading)
11. [Vestavěné události](#11-vestavěné-události)
12. [Vzory a recepty](#12-vzory-a-recepty)

---

## 1. Globální konstanty

Dostupné v každém skriptu (shared, server, client).

| Konstanta | Typ | Popis |
|-----------|-----|-------|
| `RESOURCE_ID` | `string` | Kanonická cesta resource od `/resources/` rootu, např. `"example/moving_square"` |
| `SIDE` | `string` | `"server"` nebo `"client"` |
| `IS_SERVER` | `bool` | Zkratka pro `SIDE == "server"` |
| `IS_CLIENT` | `bool` | Zkratka pro `SIDE == "client"` |

```lua
-- Ochrana skriptu před spuštěním na špatné straně
assert(IS_SERVER, 'tento skript musí běžet jen na serveru')

-- Podmíněná logika
if IS_CLIENT then
    -- klientská část
end

log_info(string.format('[%s] načteno na straně %s', RESOURCE_ID, SIDE))
```

---

## 2. Logging

### `print(...)`
Vypíše všechny argumenty do Bevy logu na úrovni `INFO`. Automaticky přidá prefix `[lua:RESOURCE_ID]`.

```lua
print('ahoj svete')                        -- [lua:core/init] "ahoj svete"
print('hráč:', player_id, 'HP:', hp)       -- více hodnot oddělených tabulátorem
```

### `log_debug(msg: string)`
### `log_info(msg: string)`
### `log_warn(msg: string)`

Explicitní úroveň logu. Preferuj tyto funkce před `print` pro produkční kód.

```lua
log_debug('tick: zpracovávám vstup')       -- viditelné jen při DEBUG log levelu
log_info('hráč se připojil')
log_warn('neznámý item: ' .. item_id)
```

---

## 3. Event system

Tříúrovňový systém: lokální sandbox bus → cross-sandbox bus → síťový RPC.

### `RegisterEvent(name, handler)`

Zaregistruje Lua callback pro pojmenovanou událost. Jeden název může mít více handlerů — všechny se zavolají.

**Handler signature:** `function(payload, sender)`
- `payload` — libovolná Lua hodnota (nil, string, number, table); JSON ze sítě se automaticky dekóduje na tabulku
- `sender` — `u64` player_id kdo poslal (nebo `nil` pro lokální eventy)

```lua
RegisterEvent('onPlayerHit', function(data, sender)
    log_info(string.format('hráč %s zasažen za %s dmg', data.victim, data.damage))
end)

-- Payload může být nil
RegisterEvent('serverShutdown', function()
    log_warn('server se vypíná!')
end)
```

---

### `TriggerEvent(name, payload?)`

Pošle událost **v rámci jednoho procesu** do všech sandboxů (server→všechny server resources, nebo client→všechny client resources). Nesíťuje se.

```lua
-- Publikuj stav pro ostatní resources ve stejném procesu
TriggerEvent('inventory:changed', { player = player_id, item = 'sword', count = 1 })

-- Přijmi ve stejném nebo jiném resource (musí běžet na stejné straně)
RegisterEvent('inventory:changed', function(data)
    log_info('inventář změněn pro hráče ' .. data.player)
end)
```

---

### `TriggerServerEvent(name, payload?)` *(client only)*

Pošle událost ze klienta na server přes lightyear. Na serveru je volání runtime chybou.

```lua
assert(IS_CLIENT)

-- Odeslat serveru žádost o respawn
TriggerServerEvent('player:requestRespawn', { reason = 'fell' })

-- Jednoduchý ping
TriggerServerEvent('ping', 'hello')
```

---

### `TriggerClientEvent(name, target, payload?)` *(server only)*

Pošle událost ze serveru na klienta nebo všem klientům.

- `target = nil` nebo `false` → **broadcast** všem připojeným klientům
- `target = player_id` (číslo nebo string) → **unicast** konkrétnímu klientovi

> **Pozor na přesnost:** Lua čísla jsou `f64`. Pro velká `player_id` (u64) předávej ID jako `string`, aby nedošlo ke ztrátě přesnosti.

```lua
assert(IS_SERVER)

-- Unicast — pošli jen danému hráči
TriggerClientEvent('ui:showMessage', player_id, { text = 'Vítej!', duration = 3 })

-- Broadcast — pošli všem
TriggerClientEvent('world:announcement', nil, { text = 'Hra začíná za 10 sekund' })

-- Bezpečný routing s string ID (pro velká u64)
local safe_id = tostring(player_id)
TriggerClientEvent('player:sync', safe_id, snapshot)
```

---

## 4. World — entity

Všechny funkce pro práci s objekty ve světě. Objekty jsou identifikovány neprůhledným `handle` (`u64`), který dostaneš při spawnu.

> **Cache model:** Gettery čtou z `EntityStateCache`, která se aktualizuje každý frame v PostUpdate. Settery jsou záměry — zpracují se v PostUpdate *dalšího* framu, cache pak odráží nový stav.

### 4.1 Spawn / Despawn

#### `World.SpawnLocalObject(model, pos, rot)` → `handle`

Spawne entitu **bez síťové replikace** (viditelnou jen lokálně — na serveru nebo na klientu, podle toho kde se zavolá). Typické použití: dekorativní objekty, UI prvky, debug vizualizace.

- `model` — string, název modelu (musí existovat v `stream/` složce resource nebo v `assets/models/` klienta)
- `pos` — `{x, y, z}` nebo `{[1], [2], [3]}`
- `rot` — `{x, y, z}` Euler úhly **ve stupních**

```lua
-- Spawn truhly na dané pozici
local chest = World.SpawnLocalObject(
    'chest',
    { x = 10.0, y = 0.0, z = -5.0 },
    { x = 0.0, y = 45.0, z = 0.0 }   -- otočená o 45° kolem Y
)
```

---

#### `World.SpawnNetworkedObject(model, pos, rot)` → `handle` *(server only)*

Spawne entitu replikovanou přes lightyear na všechny klienty. Vhodné pro herní objekty, NPC, interaktivní prvky.

```lua
assert(IS_SERVER)

local barrel = World.SpawnNetworkedObject(
    'barrel_explosive',
    { x = 0.0, y = 0.0, z = 10.0 },
    { x = 0.0, y = 0.0, z = 0.0 }
)
```

---

#### `World.DeleteObject(handle)`

Despawne entitu. Handle se stane neplatným; `World.IsValid(handle)` od dalšího framu vrátí `false`.

```lua
World.DeleteObject(chest)
```

---

### 4.2 Transform settery

Všechny settery jsou záměry — ECS se aktualizuje v PostUpdate po jejich enqueue.

#### `World.SetTransform(handle, pos, rot)`
#### `World.SetPosition(handle, pos)`
#### `World.SetRotation(handle, rot)`
#### `World.SetScale(handle, scale)`

```lua
World.SetTransform(obj, { x=5, y=0, z=3 }, { x=0, y=90, z=0 })
World.SetPosition(npc, { x=100, y=0, z=50 })
World.SetRotation(door, { x=0, y=90, z=0 })
World.SetScale(obj, 2.0)                            -- uniformní
World.SetScale(obj, { x=1.0, y=2.0, z=1.0 })       -- neuniformní
```

---

### 4.3 Transform gettery

Vrací stav z cache aktualizované na konci minulého framu. Vrací `nil` pokud handle neexistuje.

#### `World.GetPosition(handle)` → `{x, y, z}` | `nil`
#### `World.GetRotation(handle)` → `{x, y, z}` | `nil`  *(Euler stupně)*
#### `World.GetQuaternion(handle)` → `{x, y, z, w}` | `nil`
#### `World.GetScale(handle)` → `{x, y, z}` | `nil`
#### `World.GetTransform(handle)` → `{pos, rot, scale}` | `nil`

```lua
-- GetTransform je preferovaný — jeden lock místo tří
local tf = World.GetTransform(obj)
if tf then
    log_info(string.format('pos=(%.1f,%.1f,%.1f) rot.y=%.1f',
        tf.pos.x, tf.pos.y, tf.pos.z, tf.rot.y))
end
```

---

### 4.4 Model

#### `World.SetModel(handle, model_name)`
#### `World.GetModel(handle)` → `string` | `nil`

```lua
if hp < 50 then
    World.SetModel(vehicle, 'car_damaged')
end
local model = World.GetModel(obj)
```

---

### 4.5 Animace

#### `World.PlayAnimation(handle, name, looping?, speed?)`
- `looping` — `bool`, default `true`
- `speed` — `number`, default `1.0`

#### `World.StopAnimation(handle)`
#### `World.GetAnimation(handle)` → `string` | `nil`
#### `World.GetAnimationSpeed(handle)` → `number`

```lua
World.PlayAnimation(npc, 'walk')
World.PlayAnimation(npc, 'death', false)         -- jednorázová
World.PlayAnimation(npc, 'run', true, 0.75)      -- zpomalená

if World.GetAnimation(npc) == 'walk' then
    World.PlayAnimation(npc, 'run')
end
```

---

### 4.6 Stav entity

#### `World.IsValid(handle)` → `bool`
#### `World.IsAlive(handle)` → `bool`
#### `World.GetHealth(handle)` → `number` | `nil`

```lua
if World.IsAlive(target) then
    local hp = World.GetHealth(target)
    log_info('HP: ' .. (hp or '?'))
end
```

---

### 4.7 Combat

#### `World.ApplyDamage(target_handle, amount, source_handle?)` *(server only)*

```lua
assert(IS_SERVER)
if World.IsAlive(target) then
    World.ApplyDamage(target, 25.0, attacker)
end
```

---

## 5. Player

Přístup ke statistikám a inventáři hráčů.

> Všechna `player_id` lze předávat jako `number` i `string`.

### `Player.GetStat(player_id, stat_name)` → `number` | `nil`
### `Player.GetStats(player_id)` → `table` | `nil`
### `Player.SetStat(player_id, name, value)` *(server only)*

```lua
local xp = Player.GetStat(player_id, 'xp')
Player.SetStat(player_id, 'xp', 1000)
```

---

### `Player.GetHealth(player_id)` → `number` | `nil`

```lua
local hp = Player.GetHealth(player_id)
```

---

### `Player.GetInventory(player_id)` → `table` | `nil`
### `Player.GetItemCount(player_id, item_id)` → `integer`
### `Player.GiveItem(player_id, item_id, count)` *(server only)*
### `Player.TakeItem(player_id, item_id, count)` *(server only)*

```lua
local arrows = Player.GetItemCount(player_id, 'arrow')
Player.GiveItem(player_id, 'gold_coin', 50)
Player.TakeItem(player_id, 'ammo_9mm', 30)
```

---

### `Player.GetLocalStats()` → `{hp: number}` *(client only)*

Vrátí snapshot HP lokálního hráče aktualizovaný serverem. Bezpečné volat v draw threadu.

```lua
assert(IS_CLIENT)
CreateThread(function()
    while true do
        local stats = Player.GetLocalStats()
        -- vykresli HP bar
        Wait(0)
    end
end)
```

---

## 6. Engine

### `Engine.RequestModel(name)`
### `Engine.HasModelLoaded(name)` → `bool`
### `Engine.SetModelAsNoLongerNeeded(name)`

Ref-counted registry modelů. Nativní modely z `assets/models/` jsou dostupné pod jménem souboru bez přípony (např. `"player"` pro `player.glb`).

```lua
Engine.RequestModel('player')
Engine.SetModelAsNoLongerNeeded('player')
```

---

### `Engine.SetCursorLocked(locked: bool)` *(client only)*

Zapne/vypne kurzor myši. `true` = hra zachycuje myš (FPS mód), `false` = viditelný kurzor (menu).

```lua
assert(IS_CLIENT)
Engine.SetCursorLocked(false)   -- odemkni kurzor pro menu
Engine.SetCursorLocked(true)    -- zamkni zpět do hry
```

---

### `Engine.Disconnect()` *(client only)*

Odpojí klienta od serveru a vrátí ho do lobby.

```lua
assert(IS_CLIENT)
Engine.Disconnect()
```

---

### `Engine.Quit()` *(client only)*

Ukončí aplikaci.

```lua
assert(IS_CLIENT)
Engine.Quit()
```

---

## 7. Raycast

### `Raycast.GetGroundPosition()` → `{x, y, z}` *(client only)*

Vrátí world-space pozici myši promítnutou na rovinu `Y = 0`.
Na serveru vrací vždy `{0, 0, 0}`.

```lua
assert(IS_CLIENT)

RegisterEvent('input:state', function(input)
    local ground = Raycast.GetGroundPosition()
    local dx = ground.x - player_pos.x
    local dz = ground.z - player_pos.z
    local yaw = math.atan(dx, dz) * (180.0 / math.pi)
    TriggerServerEvent('player:lookAt', { yaw = yaw })
end)
```

---

## 8. Database

Asynchronní SQL API. Callback se zavolá až po dokončení dotazu — nesmíš blokovat ECS loop.

> Dostupné jen na serveru pokud je nakonfigurované `[database]` v `server.toml`.

### `Database.execute(sql, params, callback)`

INSERT / UPDATE / DELETE. Callback dostane počet ovlivněných řádků.

```lua
assert(IS_SERVER)
Database.execute(
    'INSERT INTO kills (killer, victim) VALUES (?, ?)',
    { tostring(killer_id), tostring(victim_id) },
    function(rows) log_info('vloženo: ' .. rows) end
)
```

---

### `Database.query(sql, params, callback)`

SELECT. Callback dostane tabulku řádků.

```lua
assert(IS_SERVER)
Database.query(
    'SELECT xp FROM players WHERE id = ?',
    { tostring(player_id) },
    function(rows)
        if rows and rows[1] then
            Player.SetStat(player_id, 'xp', rows[1].xp)
        end
    end
)
```

---

### `Database.isConnected()` → `bool`

```lua
if not Database.isConnected() then return end
```

---

## 9. GUI — vykreslování

Immediate-mode API. Dostupné **jen na klientovi**. Všechny souřadnice jsou normalizované `0.0–1.0` (origin vlevo nahoře). Barvy jsou `0–255` RGBA integers.

> Volej z draw threadu (viz [Threading](#10-threading)) — `Wait(0)` každý frame zajistí plynulé vykreslování.

---

### 9.1 Primitivy

#### `Gui.DrawRect(x, y, w, h, r, g, b, a)`

Vyplněný obdélník. `x, y` = střed; `w, h` = rozměry.

```lua
Gui.DrawRect(0.5, 0.5, 0.3, 0.1, 30, 30, 30, 200)
```

---

#### `Gui.DrawText(text, x, y, scale, r, g, b, a, font_id?)`

Text s anchoringem vlevo nahoře. `scale = 1.0` ≈ 24 px.
`font_id` — volitelný string, ID fontu z `assets/fonts/` (název souboru bez přípony, např. `"SephoraHayden"`).

```lua
Gui.DrawText('Hello World', 0.1, 0.05, 1.0, 255, 255, 255, 255)
Gui.DrawText('Score: 100',  0.1, 0.10, 0.8, 255, 220, 100, 255, 'SephoraHayden')
```

---

#### `Gui.DrawLine(x1, y1, x2, y2, r, g, b, a)`

```lua
-- Oddělovač
Gui.DrawLine(0.1, 0.5, 0.9, 0.5, 255, 255, 255, 80)
```

---

#### `Gui.DrawCircle(x, y, radius, r, g, b, a)`

Obrys kruhu (24 segmentů). Pro vyplněný kruh viz `DrawDisc`.

```lua
Gui.DrawCircle(0.5, 0.5, 0.05, 255, 255, 255, 180)
```

---

#### `Gui.DrawDisc(x, y, radius, r, g, b, a)`

Vyplněný anti-aliased kruh (GPU textura).

```lua
Gui.DrawDisc(0.5, 0.5, 0.03, 255, 100, 100, 220)
```

---

### 9.2 Pokročilé tvary

#### `Gui.DrawRoundedRect(x, y, w, h, radius, r, g, b, a)`

Obdélník se zaoblenými rohy.

```lua
Gui.DrawRoundedRect(0.5, 0.5, 0.3, 0.12, 0.015, 40, 40, 60, 220)
```

---

#### `Gui.DrawBorder(x, y, w, h, thickness, r, g, b, a)`

Obrys obdélníku bez výplně.

```lua
Gui.DrawBorder(0.5, 0.5, 0.3, 0.12, 0.002, 255, 255, 255, 120)
```

---

#### `Gui.DrawShadow(x, y, w, h, spread, r, g, b, a)`

Vrstvený drop-shadow. **Volej před** vykreslením prvku, ke kterému stín patří.

```lua
Gui.DrawShadow(0.5, 0.5, 0.3, 0.12, 0.012, 0, 0, 0, 160)
Gui.DrawRoundedRect(0.5, 0.5, 0.3, 0.12, 0.015, 40, 40, 60, 220)
```

---

### 9.3 Obrázky (sprites)

#### `Gui.DrawSprite(id, x, y, w, h, r?, g?, b?, a?, opts?)`

Vykreslí obrázek registrovaný v manifestu (`images { {id='...', path='...'} }`).

**opts** (volitelná tabulka):
| Klíč | Typ | Výchozí | Popis |
|------|-----|---------|-------|
| `fit` | `string` | `"stretch"` | `"stretch"` \| `"fit"` (letterbox) \| `"fill"` (crop ke středu) |
| `uv` | `{u0,v0,u1,v1}` | celý obrázek | UV ořez v normalizovaných souřadnicích (0–1) |
| `flip_x` | `bool` | `false` | Horizontální zrcadlení |
| `flip_y` | `bool` | `false` | Vertikální zrcadlení |

```lua
-- Celoplošné pozadí (obrázek vyplní celou obrazovku)
Gui.DrawSprite('esc_bg', 0.5, 0.5, 1.0, 1.0, 255, 255, 255, 180, { fit = 'fill' })

-- Logo zachovávající poměr stran
Gui.DrawSprite('logo', 0.5, 0.1, 0.4, 0.12, 255, 255, 255, 235, { fit = 'fit' })

-- Ikona z sprite sheetu 2×2 (UV crop)
Gui.DrawSprite('icons', 0.5, 0.5, 0.04, 0.04, 255, 255, 255, 200, {
    uv = { 0.0, 0.0, 0.5, 0.5 }   -- levý horní čtverec
})

-- Zrcadlení
Gui.DrawSprite('arrow', 0.8, 0.5, 0.03, 0.03, 255, 255, 255, 255, { flip_x = true })
```

---

### 9.4 Vstup myši

#### `Gui.GetCursorPos()` → `{x, y}`

Pozice kurzoru v normalizovaných souřadnicích (0–1).

#### `Gui.IsMouseOver(x, y, w, h)` → `bool`

`true` pokud je kurzor v obdélníku (střed + rozměry).

#### `Gui.IsMouseDown(btn?)` → `bool`

`true` pokud je tlačítko myši stisknuto. `btn` = `"left"` (výchozí) | `"right"` | `"middle"`.

#### `Gui.IsMouseClicked(btn?)` → `bool`

`true` pokud bylo tlačítko myši právě puštěno (click = down→up v tomto framu).

```lua
local cx, cy = 0.5, 0.5
local bw, bh = 0.2, 0.06

local hov = Gui.IsMouseOver(cx, cy, bw, bh)
local clk = Gui.IsMouseClicked()

local bg = hov and {60, 80, 120, 230} or {40, 50, 80, 200}
Gui.DrawRoundedRect(cx, cy, bw, bh, 0.01, bg[1], bg[2], bg[3], bg[4])
Gui.DrawText('Klikni', cx - bw*0.5 + 0.01, cy - 0.01, 0.85, 255, 255, 255, 255)

if clk then
    log_info('tlačítko stisknuto')
end
```

---

### 9.5 UI framework

#### `UI.Window(opts)` → window objekt

Vytvoří spravované okno s tlačítky, labely a separátory. Vrácený objekt volej metodami pro sestavení obsahu, pak každý frame voláním `:Render()`.

**opts:**
| Klíč | Typ | Popis |
|------|-----|-------|
| `title` | `string` | Titulek v headeru |
| `width` | `number` | Šířka okna (0–1) |
| `x`, `y` | `number` | Střed okna (0–1), default 0.5, 0.5 |

**Metody:**

| Metoda | Popis |
|--------|-------|
| `:Button(label, callback, opts?)` | Tlačítko; `opts = {style="danger"\|"accent"\|"normal"}` |
| `:Label(text, opts?)` | Textový řádek; `opts = {dim=bool}` |
| `:Sep()` | Horizontální oddělovač |
| `:Open()` / `:Close()` | Programové otevření/zavření |
| `:Toggle()` | Přepne otevřeno/zavřeno |
| `:IsOpen()` → `bool` | Stav okna |
| `:Render()` | Vykreslí okno — volej každý frame |

```lua
assert(IS_CLIENT)

local menu = UI.Window({ title = 'Nastavení', width = 0.30 })
menu:Button('Pokračovat', function() menu:Close() end)
menu:Sep()
menu:Button('Odpojit', function() Engine.Disconnect() end, { style = 'danger' })
menu:Button('Konec', function() Engine.Quit() end, { style = 'danger' })

RegisterEvent('input:state', function(payload)
    if payload and payload.keys_just and payload.keys_just.options_menu then
        menu:Toggle()
        Engine.SetCursorLocked(not menu:IsOpen())
    end
end)

CreateThread(function()
    while true do
        menu:Render()
        Wait(0)
    end
end)
```

#### `UI.Theme()` → `table`

Vrátí aktuální theme tabulku (barvy, rozměry panelu). Jen pro čtení.

```lua
local T = UI.Theme()
-- T.bg, T.text, T.btn, T.btn_hover, T.btn_active, T.btn_danger, T.btn_accent
-- T.header_h, T.btn_h, T.btn_gap, T.pad_x, T.pad_y, T.radius, T.border_w
-- T.shadow_size, T.shadow_col, T.border, T.sep, T.text_dim
-- T.fade_in, T.fade_out
```

---

## 10. Threading

### `CreateThread(fn)`

Spustí funkci jako Lua coroutinu. Coroutina se okamžitě nezačne provádět — první tick nastane v příštím `PreUpdate` framu.

### `Wait(ms)`

Pozastaví aktuální coroutinu na `ms` milisekund. `Wait(0)` = pokračuj v příštím framu.

> `Wait` volej **jen z coroutiny** spuštěné přes `CreateThread`. Volání v hlavním kódu způsobí chybu.

```lua
-- Draw loop — vykresluje každý frame
CreateThread(function()
    while true do
        Gui.DrawRect(0.5, 0.5, 0.2, 0.05, 255, 0, 0, 180)
        Wait(0)   -- příští frame
    end
end)

-- Časovač — odpočítává 5 sekund
CreateThread(function()
    for i = 5, 1, -1 do
        log_info('odpočet: ' .. i)
        Wait(1000)
    end
    log_info('čas vypršel!')
end)

-- Polling s intervalem
CreateThread(function()
    while true do
        local hp = Player.GetLocalStats().hp
        if hp < 25 then
            -- zobraz varování nízkého zdraví
        end
        Wait(200)   -- kontroluj každých 200 ms
    end
end)
```

---

## 11. Vestavěné události

Rust core emituje tyto události automaticky. Přihlás se přes `RegisterEvent`.

### Server-side události

| Název | Kdy | Payload |
|-------|-----|---------|
| `playerConnecting` | Klient se připojil | `{id: string, entity: string}` |
| `playerDropped` | Klient se odpojil | `{id: string, reason: string}` |
| `onPlayerPosition` | Každý FixedUpdate tick | `{players: [{id, x, z}]}` |
| `onPlayerHit` | Hráč zasažen | `{attacker, victim, damage, weapon, position}` |
| `onPlayerDeath` | Hráč zemřel | `{victim, killer, cause}` |

```lua
RegisterEvent('playerConnecting', function(data)
    local pid = data.id
    log_info('hráč ' .. pid .. ' se připojil')
    Player.SetStat(pid, 'xp', 0)
    TriggerClientEvent('ui:welcome', pid, { message = 'Vítej!' })
end)

RegisterEvent('onPlayerHit', function(data)
    log_info(string.format(
        'hráč %s zasažen hráčem %s za %.1f dmg',
        data.victim, data.attacker, data.damage))
end)
```

---

### Client-side události

#### `input:state`

Emitováno každý frame. Payload:

```lua
{
    move = { x = number, y = number },  -- -1..1, camera-relative
    keys = {
        jump     = bool,
        crouch   = bool,
        sprint   = bool,
        fire     = bool,
        reload   = bool,
        interact = bool,
    },
    keys_just = {
        options_menu = bool,  -- true jen v framu kdy bylo stisknuto Escape
    }
}
```

```lua
assert(IS_CLIENT)

RegisterEvent('input:state', function(input)
    if not input then return end

    -- ESC menu toggle
    if input.keys_just and input.keys_just.options_menu then
        -- otevři/zavři menu
    end

    -- Pohyb
    local speed = input.keys.sprint and 0.15 or 0.08
    if player_obj then
        local pos = World.GetPosition(player_obj)
        if pos then
            World.SetPosition(player_obj, {
                x = pos.x + input.move.x * speed,
                y = pos.y,
                z = pos.z + input.move.y * speed,
            })
        end
    end
end)
```

---

## 12. Vzory a recepty

### Inicializace resource (robustní pattern)

```lua
-- server/main.lua
RegisterEvent('sq:ready', function(_, sender)
    TriggerClientEvent('sq:init', sender, { map = 'downtown', time = 'noon' })
end)

-- client/main.lua
RegisterEvent('sq:init', function(data)
    log_info('inicializuji mapu: ' .. data.map)
end)

TriggerServerEvent('sq:ready')
```

---

### ESC menu s UI.Window

```lua
assert(IS_CLIENT)

local menu = UI.Window({ title = 'PAUZA', width = 0.28, x = 0.5, y = 0.5 })
menu:Button('Pokračovat', function()
    menu:Close()
    Engine.SetCursorLocked(true)
end)
menu:Sep()
menu:Button('Odpojit', function() Engine.Disconnect() end, { style = 'danger' })
menu:Button('Ukončit hru', function() Engine.Quit() end, { style = 'danger' })

RegisterEvent('input:state', function(payload)
    if payload and payload.keys_just and payload.keys_just.options_menu then
        menu:Toggle()
        Engine.SetCursorLocked(not menu:IsOpen())
    end
end)

CreateThread(function()
    while true do
        menu:Render()
        Wait(0)
    end
end)
```

---

### HUD s draw threadem

```lua
assert(IS_CLIENT)

CreateThread(function()
    while true do
        local stats = Player.GetLocalStats()
        local hp = stats.hp or 100

        -- HP bar (vlevo dole)
        local bw, bh = 0.20, 0.018
        local bx, by = 0.13, 0.93
        Gui.DrawRoundedRect(bx, by, bw, bh, 0.005, 20, 20, 20, 180)
        Gui.DrawRoundedRect(bx - bw*0.5*(1-(hp/100)), by, bw*(hp/100), bh, 0.005,
            math.floor(200*(1-hp/100)), math.floor(200*(hp/100)), 30, 220)
        Gui.DrawText(string.format('HP: %d', hp), bx - bw*0.5 + 0.006, by - 0.007, 0.7,
            255, 255, 255, 220, 'SephoraHayden')

        Wait(0)
    end
end)
```

---

### Persist dat při odpojení

```lua
assert(IS_SERVER)

RegisterEvent('playerDropped', function(data)
    local pid = data.id
    if not Database.isConnected() then return end
    local stats = Player.GetStats(pid)
    if not stats then return end
    Database.execute(
        'INSERT INTO player_state (id, xp) VALUES (?,?) '
        .. 'ON CONFLICT(id) DO UPDATE SET xp=excluded.xp',
        { pid, stats.xp or 0 },
        function(_) end
    )
end)

RegisterEvent('playerConnecting', function(data)
    local pid = data.id
    Database.query('SELECT xp FROM player_state WHERE id = ?', { pid },
        function(rows)
            if rows and rows[1] then
                Player.SetStat(pid, 'xp', rows[1].xp)
            end
        end
    )
end)
```

---

### Cross-resource komunikace

```lua
-- resource A: core/inventory
RegisterEvent('inventory:give', function(data)
    Player.GiveItem(data.player, data.item, data.count)
    TriggerEvent('inventory:changed', data)
end)

-- resource B: example/shop
local function buy(player_id, item_id, price)
    if Player.GetItemCount(player_id, 'gold') < price then
        TriggerClientEvent('ui:error', player_id, { text = 'Nedostatek zlatých' })
        return
    end
    TriggerEvent('inventory:give', { player = player_id, item = 'gold',  count = -price })
    TriggerEvent('inventory:give', { player = player_id, item = item_id, count = 1 })
end
```

---

## Omezení sandboxu

| Povoleno | Zakázáno |
|----------|----------|
| `string`, `table`, `math`, `utf8`, `coroutine` | `io`, `os`, `package` |
| `print`, `log_*` | `require`, `dofile`, `loadfile` |
| Všechna `World.*`, `Player.*`, `Engine.*`, `Gui.*`, `UI.*` API | `load`, `loadstring`, `debug` |
| `RegisterEvent`, `TriggerEvent`, `TriggerServerEvent`, `TriggerClientEvent` | Přímý přístup k filesystému |
| `CreateThread`, `Wait` | Globální sdílení mezi resources |
| `Database.*` (server + DB configured) | |

Každý resource = vlastní izolovaná VM. Globál definovaný v resource A není viditelný v resource B. Veškerá cross-resource komunikace probíhá přes `TriggerEvent` / `RegisterEvent`.
