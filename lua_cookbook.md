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
6. [Engine — model registry](#6-engine--model-registry)
7. [Raycast](#7-raycast)
8. [Database](#8-database)
9. [Vestavěné události](#9-vestavěné-události)
10. [Vzory a recepty](#10-vzory-a-recepty)

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

- `model` — string, název modelu (musí existovat v `stream/` složce resource)
- `pos` — `{x, y, z}` nebo `{[1], [2], [3]}`
- `rot` — `{x, y, z}` Euler úhly **ve stupních**

```lua
-- Spawn truhly na dané pozici
local chest = World.SpawnLocalObject(
    'chest',
    { x = 10.0, y = 0.0, z = -5.0 },
    { x = 0.0, y = 45.0, z = 0.0 }   -- otočená o 45° kolem Y
)

-- Spawn na pozici hráče (zjistit přes GetPosition nebo z eventu)
local torch = World.SpawnLocalObject('torch', {x=0,y=0,z=0}, {x=0,y=0,z=0})
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

log_info('sud spawnut, handle=' .. barrel)
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

Nastaví pozici a rotaci najednou. Zachová stávající scale.

```lua
World.SetTransform(
    obj,
    { x = 5.0, y = 0.0, z = 3.0 },
    { x = 0.0, y = 90.0, z = 0.0 }
)
```

---

#### `World.SetPosition(handle, pos)`

Nastaví jen pozici. Rotace a scale zůstanou beze změny.

```lua
-- Teleport objektu
World.SetPosition(npc, { x = 100.0, y = 0.0, z = 50.0 })

-- Pohyb každý tick (v event handleru)
RegisterEvent('input:state', function(input)
    local pos = World.GetPosition(player_obj)
    if pos then
        World.SetPosition(player_obj, {
            x = pos.x + input.move.x * 0.1,
            y = pos.y,
            z = pos.z + input.move.y * 0.1,
        })
    end
end)
```

---

#### `World.SetRotation(handle, rot)`

Nastaví jen rotaci jako Euler XYZ ve stupních. Pozice a scale zůstanou beze změny.

```lua
-- Otočení dveří o 90°
World.SetRotation(door, { x = 0.0, y = 90.0, z = 0.0 })

-- Postupná rotace
local angle = 0.0
RegisterEvent('onTick', function()
    angle = (angle + 1.0) % 360.0
    World.SetRotation(windmill, { x = 0.0, y = 0.0, z = angle })
end)
```

---

#### `World.SetScale(handle, scale)`

Nastaví scale. Přijímá číslo (uniform) nebo tabulku `{x, y, z}`.

```lua
World.SetScale(obj, 2.0)                        -- uniformní zvětšení 2×
World.SetScale(obj, { x = 1.0, y = 2.0, z = 1.0 }) -- jen na výšku
World.SetScale(obj, 0.5)                        -- zmenšení na polovinu
```

---

### 4.3 Transform gettery

Vrací stav z cache aktualizované na konci minulého framu. Vrací `nil` pokud handle neexistuje.

#### `World.GetPosition(handle)` → `{x, y, z}` | `nil`

```lua
local pos = World.GetPosition(obj)
if pos then
    log_info(string.format('pozice: %.2f, %.2f, %.2f', pos.x, pos.y, pos.z))
end
```

---

#### `World.GetRotation(handle)` → `{x, y, z}` | `nil`

Vrátí rotaci jako Euler XYZ ve stupních — stejný formát jako `SetRotation`.

```lua
local rot = World.GetRotation(obj)
if rot then
    log_info('yaw: ' .. rot.y)
end
```

---

#### `World.GetQuaternion(handle)` → `{x, y, z, w}` | `nil`

Vrátí rotaci jako kvaternion. Použij pro přesné interpolace nebo výpočty bez gimbal locku.

```lua
local q = World.GetQuaternion(obj)
if q then
    -- slerp mezi dvěma rotacemi (implementováno v Lua)
    local t = 0.5
    local interp_w = q.w * (1 - t) + target_q.w * t
    -- ...
end
```

---

#### `World.GetScale(handle)` → `{x, y, z}` | `nil`

```lua
local s = World.GetScale(obj)
if s then
    log_debug(string.format('scale: %s %s %s', s.x, s.y, s.z))
end
```

---

#### `World.GetTransform(handle)` → `{pos, rot, scale}` | `nil`

Vrátí celý transform najednou — jeden lock místo tří. Preferuj před opakovanými gettery.

```lua
local tf = World.GetTransform(obj)
if tf then
    -- tf.pos  = {x, y, z}
    -- tf.rot  = {x, y, z}  (Euler stupně)
    -- tf.scale = {x, y, z}
    log_info(string.format(
        'pos=(%.1f,%.1f,%.1f) rot.y=%.1f',
        tf.pos.x, tf.pos.y, tf.pos.z, tf.rot.y
    ))
end
```

---

### 4.4 Model

#### `World.SetModel(handle, model_name)`

Změní jméno modelu entity. V Phase 4 to vyvolá i swap meshe (GPU unload/load). Prozatím ukládá název pro čtení přes `GetModel`.

```lua
-- Swap modelu po poškození
if hp < 50 then
    World.SetModel(vehicle, 'car_damaged')
else
    World.SetModel(vehicle, 'car_normal')
end
```

---

#### `World.GetModel(handle)` → `string` | `nil`

```lua
local model = World.GetModel(obj)
log_info('model entity: ' .. (model or 'neznámý'))
```

---

### 4.5 Animace

Animační stav se ukládá na entitu. Phase 4 propojí s Bevy `AnimationPlayer`.

#### `World.PlayAnimation(handle, name, looping?, speed?)`

- `looping` — `bool`, default `true`
- `speed` — `number`, default `1.0` (0.5 = poloviční rychlost, 2.0 = dvojitá)

```lua
-- Základní spuštění animace (looping)
World.PlayAnimation(npc, 'walk')

-- Jednorázová animace (např. death)
World.PlayAnimation(npc, 'death', false)

-- Zpomalená animace
World.PlayAnimation(npc, 'run', true, 0.75)

-- Animace útoku v poloviční rychlosti, neopakující se
World.PlayAnimation(weapon_obj, 'attack', false, 0.5)
```

---

#### `World.StopAnimation(handle)`

Zastaví aktuální animaci. `GetAnimation` bude od dalšího framu vracet `nil`.

```lua
World.StopAnimation(npc)
```

---

#### `World.GetAnimation(handle)` → `string` | `nil`

Vrátí název aktuálně přehrávané animace nebo `nil` pokud žádná neběží.

```lua
local anim = World.GetAnimation(npc)
if anim == 'walk' then
    World.PlayAnimation(npc, 'run')
end
```

---

#### `World.GetAnimationSpeed(handle)` → `number`

Vrátí aktuální rychlost animace. Pokud entita nemá nastavenu animaci, vrátí `1.0`.

```lua
local spd = World.GetAnimationSpeed(npc)
log_debug('rychlost animace: ' .. spd)
```

---

### 4.6 Stav entity

#### `World.IsValid(handle)` → `bool`

`true` pokud handle mapuje na existující entitu v ECS. Po `DeleteObject` vrátí `false` od dalšího framu.

```lua
if not World.IsValid(obj) then
    log_warn('entita již neexistuje, přeskakuji')
    return
end
```

---

#### `World.IsAlive(handle)` → `bool`

`true` pokud entita existuje **a zároveň** má health > 0 (nebo nemá `Health` komponentu vůbec).
`false` pokud handle neexistuje nebo zdraví ≤ 0.

```lua
-- Útok jen na živé cíle
if World.IsAlive(target) then
    World.ApplyDamage(target, 25.0)
end
```

---

#### `World.GetHealth(handle)` → `number` | `nil`

Vrátí aktuální health. `nil` pokud entita nemá `Health` komponentu (většina non-player objektů).

```lua
local hp = World.GetHealth(npc)
if hp then
    local pct = hp / 100.0
    log_info(string.format('HP: %.0f%%', pct * 100))
end
```

---

### 4.7 Combat

#### `World.ApplyDamage(target_handle, amount, source_handle?)` *(server only)*

Enqueue damage záměr. Server combat systémy ho zpracují v dalším FixedUpdate ticku.

- `target_handle` — handle cíle
- `amount` — poškození (`f32`)
- `source_handle` — volitelně handle útočníka (pro kill-feed apod.)

```lua
assert(IS_SERVER)

-- Výbuch poškodí všechny entity v dosahu
RegisterEvent('explosion:trigger', function(data)
    for _, handle in ipairs(nearby_entities) do
        if World.IsAlive(handle) then
            World.ApplyDamage(handle, data.damage, data.source)
        end
    end
end)
```

---

## 5. Player

Přístup ke statistikám a inventáři hráčů. Čtení je synchronní (z `PlayerStatsCache`), zápis je záměr přes `CommandQueue`.

> Všechny `player_id` lze předávat jako `number` i jako `string` (pro bezpečný routing velkých u64).

### `Player.GetStat(player_id, stat_name)` → `number` | `nil`

```lua
local xp = Player.GetStat(player_id, 'xp')
if xp then
    log_info('XP hráče: ' .. xp)
end
```

---

### `Player.GetStats(player_id)` → `table` | `nil`

Vrátí celou tabulku statistik hráče.

```lua
local stats = Player.GetStats(player_id)
if stats then
    for name, value in pairs(stats) do
        log_info(name .. ' = ' .. value)
    end
end
```

---

### `Player.SetStat(player_id, name, value)` *(server only)*

```lua
assert(IS_SERVER)
Player.SetStat(player_id, 'xp', 1000)
Player.SetStat(player_id, 'level', 5)
```

---

### `Player.GetHealth(player_id)` → `number` | `nil`

Vrátí aktuální HP hráče ze snapshotu.

```lua
local hp = Player.GetHealth(player_id)
log_info('HP: ' .. (hp or 'neznámé'))
```

---

### `Player.GetInventory(player_id)` → `table` | `nil`

Vrátí inventář jako tabulku `{item_id = count}`.

```lua
local inv = Player.GetInventory(player_id)
if inv then
    log_info('mečů: ' .. (inv['sword'] or 0))
end
```

---

### `Player.GetItemCount(player_id, item_id)` → `integer`

Vrátí počet kusů daného itemu. Nikdy nevrátí `nil` — pokud hráč item nemá, vrátí `0`.

```lua
local arrows = Player.GetItemCount(player_id, 'arrow')
if arrows < 10 then
    log_warn('málo šípů!')
end
```

---

### `Player.GiveItem(player_id, item_id, count)` *(server only)*

Přidá (`count > 0`) nebo odebere (`count < 0`) itemy. Počet neklesne pod 0.

```lua
assert(IS_SERVER)
Player.GiveItem(player_id, 'gold_coin', 50)    -- dej 50 zlatých
Player.GiveItem(player_id, 'health_potion', -1) -- odeber 1 lektvar
```

---

### `Player.TakeItem(player_id, item_id, count)` *(server only)*

Alias pro `GiveItem` se záporným počtem. Čitelnější pro odebírání.

```lua
assert(IS_SERVER)
Player.TakeItem(player_id, 'ammo_9mm', 30)
```

---

## 6. Engine — model registry

Ref-counted registr modelů. Říkáš Rustu, které modely chceš mít načtené v paměti.

### `Engine.RequestModel(name)`

Inkrementuje ref-count modelu. Volej před tím, než ho budeš potřebovat.

```lua
Engine.RequestModel('blacksmith')
Engine.RequestModel('barrel_explosive')
```

---

### `Engine.HasModelLoaded(name)` → `bool`

Vrátí `true` pokud je model registrován s `ref_count > 0`.

> **Phase 3 stub:** Prozatím vždy vrátí `false`. Phase 4 přidá skutečný async load z GPU.

```lua
-- Phase 4+ pattern: čekej na load
Engine.RequestModel('heavy_tank')
-- Engine.HasModelLoaded vrátí false dokud není model na GPU
```

---

### `Engine.SetModelAsNoLongerNeeded(name)`

Dekrementuje ref-count. Při dosažení 0 může být model uvolněn z paměti.

```lua
-- Po despawnu objektu model nepotřebujeme
World.DeleteObject(tank)
Engine.SetModelAsNoLongerNeeded('heavy_tank')
```

---

## 7. Raycast

### `Raycast.GetGroundPosition()` → `{x, y, z}`

Vrátí world-space pozici myši promítnutou na rovinu `Y = 0`.
Na serveru vrací vždy `{0, 0, 0}` (raycast není k dispozici bez rendereru).

> Klientský gameplay systém aktualizuje tuto hodnotu každý frame z camera forward ray.

```lua
assert(IS_CLIENT)

RegisterEvent('input:state', function(input)
    local ground = Raycast.GetGroundPosition()
    -- Otočení hráče směrem k pozici myši
    local dx = ground.x - player_pos.x
    local dz = ground.z - player_pos.z
    local yaw = math.atan(dx, dz) * (180.0 / math.pi)
    TriggerServerEvent('player:lookAt', { yaw = yaw })
end)
```

---

## 8. Database

Asynchronní SQL API. Callback se zavolá až po dokončení dotazu — nesmíš blokovat ECS loop.

> Dostupné jen na serveru pokud je nakonfigurované `[database]` v `server.toml`. Bez DB vrátí volání runtime chybu.

### `Database.execute(sql, params, callback)`

INSERT / UPDATE / DELETE. Callback dostane počet ovlivněných řádků (`integer`).

```lua
assert(IS_SERVER)

Database.execute(
    'INSERT INTO kills (killer, victim, weapon) VALUES (?, ?, ?)',
    { tostring(killer_id), tostring(victim_id), 'rifle' },
    function(rows_affected)
        log_info('záznam vložen, rows=' .. rows_affected)
    end
)
```

---

### `Database.query(sql, params, callback)`

SELECT. Callback dostane tabulku řádků (`table of tables`).

```lua
assert(IS_SERVER)

Database.query(
    'SELECT item_id, count FROM inventory WHERE player_id = ?',
    { tostring(player_id) },
    function(rows)
        if not rows then return end
        for _, row in ipairs(rows) do
            log_info(row.item_id .. ': ' .. row.count)
        end
    end
)
```

---

### `Database.isConnected()` → `bool`

```lua
if not Database.isConnected() then
    log_warn('DB nedostupná, přeskakuji persistenci')
    return
end
```

---

## 9. Vestavěné události

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
    -- data.id je string (bezpečné u64)
    local pid = data.id
    log_info('hráč ' .. pid .. ' se připojil')
    Player.SetStat(pid, 'xp', 0)
    Player.GiveItem(pid, 'starter_pistol', 1)
    TriggerClientEvent('ui:welcome', pid, { message = 'Vítej!' })
end)

RegisterEvent('playerDropped', function(data)
    log_info('hráč ' .. data.id .. ' odešel: ' .. data.reason)
    -- uložit stav do DB
end)

RegisterEvent('onPlayerHit', function(data)
    local hp = Player.GetHealth(data.victim)
    log_info(string.format(
        'hráč %s zasažen hráčem %s za %.1f dmg (zbývá %.1f HP)',
        data.victim, data.attacker, data.damage, hp or 0
    ))
end)

RegisterEvent('onPlayerDeath', function(data)
    TriggerClientEvent('ui:deathScreen', data.victim, {
        killer = data.killer,
        cause  = data.cause,
    })
    -- respawn za 5 sekund by musel být implementován přes coroutine/timer resource
end)
```

---

### Client-side události

| Název | Kdy | Payload |
|-------|-----|---------|
| `input:state` | Každý frame | `{move: {x, y}, keys: {jump, crouch, sprint, fire, reload, interact}}` |

```lua
assert(IS_CLIENT)

local player_obj = nil

RegisterEvent('input:state', function(input)
    if not player_obj then return end

    -- Pohyb lokálního objektu podle vstupu
    local pos = World.GetPosition(player_obj)
    if not pos then return end

    local speed = input.keys.sprint and 0.15 or 0.08
    World.SetPosition(player_obj, {
        x = pos.x + input.move.x * speed,
        y = pos.y,
        z = pos.z + input.move.y * speed,
    })

    -- Animace podle pohybu
    local moving = math.abs(input.move.x) + math.abs(input.move.y) > 0.1
    if moving then
        local anim = input.keys.sprint and 'run' or 'walk'
        if World.GetAnimation(player_obj) ~= anim then
            World.PlayAnimation(player_obj, anim)
        end
    else
        if World.GetAnimation(player_obj) ~= nil then
            World.PlayAnimation(player_obj, 'idle')
        end
    end
end)
```

---

## 10. Vzory a recepty

### Inicializace resource (robustní pattern)

Sandboxe vznikají po síťovém handshake — klient může minout `playerConnecting`. Použij request/response handshake:

```lua
-- server/main.lua
RegisterEvent('sq:ready', function(_, sender)
    -- Klient žádá o inicializační data
    TriggerClientEvent('sq:init', sender, {
        map   = 'downtown',
        time  = 'noon',
    })
end)

-- client/main.lua
RegisterEvent('sq:init', function(data)
    log_info('inicializuji mapu: ' .. data.map)
    -- spawni objekty, nastav UI atd.
end)

-- Odeslat hned po načtení — server odpoví sq:init
TriggerServerEvent('sq:ready')
```

---

### Ochrana server-only kódu

```lua
-- Varianta 1: assert při načtení skriptu
assert(IS_SERVER, 'combat.lua smí běžet jen na serveru')

-- Varianta 2: guard uvnitř handleru
RegisterEvent('player:cheat', function(data, sender)
    if not IS_SERVER then return end
    -- ...
end)
```

---

### Sledování stavu entit v tabulce

```lua
-- server/npc_manager.lua
local npcs = {}   -- { [handle] = { type, spawn_pos } }

local function spawn_npc(model, pos)
    local h = World.SpawnNetworkedObject(model, pos, {x=0,y=0,z=0})
    npcs[h] = { type = model, spawn_pos = pos }
    World.PlayAnimation(h, 'idle')
    return h
end

local function tick_npcs()
    for h, info in pairs(npcs) do
        if not World.IsAlive(h) then
            -- respawn po smrti
            World.DeleteObject(h)
            npcs[h] = nil
            spawn_npc(info.type, info.spawn_pos)
        end
    end
end

RegisterEvent('onPlayerPosition', function()
    tick_npcs()
end)

-- Inicializace
spawn_npc('zombie', {x=20, y=0, z=10})
spawn_npc('zombie', {x=25, y=0, z=15})
```

---

### Cross-resource komunikace

Resources nesdílejí globály — vše přes event bus:

```lua
-- resource A (core/inventory) — server/api.lua
RegisterEvent('inventory:give', function(data, _)
    Player.GiveItem(data.player, data.item, data.count)
    TriggerEvent('inventory:changed', {
        player = data.player,
        item   = data.item,
        delta  = data.count,
    })
end)

-- resource B (example/shop) — server/shop.lua
local function buy_item(player_id, item_id, price)
    local gold = Player.GetItemCount(player_id, 'gold')
    if gold < price then
        TriggerClientEvent('ui:error', player_id, { text = 'Nedostatek zlatých' })
        return
    end
    -- Zaplatit
    TriggerEvent('inventory:give', { player = player_id, item = 'gold', count = -price })
    -- Dostat item
    TriggerEvent('inventory:give', { player = player_id, item = item_id, count = 1 })
end

RegisterEvent('shop:buy', function(data, sender)
    buy_item(tostring(sender), data.item, data.price)
end)
```

---

### Persist dat při odpojení

```lua
-- server/persistence.lua
assert(IS_SERVER)

RegisterEvent('playerDropped', function(data)
    local pid = data.id
    local stats = Player.GetStats(pid)
    local inv   = Player.GetInventory(pid)
    if not stats or not Database.isConnected() then return end

    Database.execute(
        'INSERT INTO player_state (player_id, xp, level) VALUES (?, ?, ?) '
        .. 'ON CONFLICT(player_id) DO UPDATE SET xp=excluded.xp, level=excluded.level',
        { pid, stats.xp or 0, stats.level or 1 },
        function(_) end
    )
end)

RegisterEvent('playerConnecting', function(data)
    local pid = data.id
    Database.query(
        'SELECT xp, level FROM player_state WHERE player_id = ?',
        { pid },
        function(rows)
            if rows and rows[1] then
                Player.SetStat(pid, 'xp',   rows[1].xp)
                Player.SetStat(pid, 'level', rows[1].level)
            end
        end
    )
end)
```

---

### Animace podle zdraví

```lua
RegisterEvent('onPlayerHit', function(data)
    local hp = Player.GetHealth(data.victim) or 0
    -- Informovat klienta o stavu (klient nemá přímý přístup k HP)
    TriggerClientEvent('player:hpUpdate', data.victim, { hp = hp, max = 100 })
end)

-- client/ui.lua
RegisterEvent('player:hpUpdate', function(data)
    local pct = data.hp / data.max
    if pct < 0.25 then
        -- Vizuální efekt nízkého zdraví
        TriggerEvent('vfx:lowHealth', { intensity = 1.0 - pct })
    end
end)
```

---

## Omezení sandboxu

| Povoleno | Zakázáno |
|----------|----------|
| `string`, `table`, `math`, `utf8`, `coroutine` | `io`, `os`, `package` |
| `print`, `log_*` | `require`, `dofile`, `loadfile` |
| Všechna `World.*`, `Player.*`, `Engine.*` API | `load`, `loadstring`, `debug` |
| `RegisterEvent`, `TriggerEvent`, `TriggerServerEvent`, `TriggerClientEvent` | Přímý přístup k filesystému |
| `Database.*` (server + DB configured) | Globální sdílení mezi resources |

Každý resource = vlastní izolovaná VM. Globál definovaný v resource A není viditelný v resource B. Veškerá cross-resource komunikace probíhá přes `TriggerEvent` / `RegisterEvent`.
