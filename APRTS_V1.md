Zde je kompletní, aktualizovaná **Specifikace APRTS_V1** s doplněným odstavcem o škálovatelnosti lag kompenzace v sekci 18.1. Tento přístup plně odpovídá modernímu programování v Unity s využitím Job Systemu a Burst Compileru pro maximální optimalizaci síťového kódu na straně serveru.

---

# Specifikace APRTS_V1: Technická a herní dokumentace frameworku

Tento dokument představuje ucelenou technickou specifikaci a koncept architektury pod označením **APRTS_V1** (*Advanced Procedural Runtime Tracking & Scripting - Verze 1*). Cílem této specifikace je definovat standardy pro modulární, plně data-driven multiplayerový framework v prostředí Unity 6+, který funguje na principu autoritativního dedikovaného serveru bez nutnosti práce v Unity Editoru ze strany koncových vývojářů.

---

## 1. Globální architektura systému

Specifikace **APRTS_V1** rozděluje běhové prostředí na dva samostatné subsystémy sdílející stejnou fyzikální vrstvu (Nvidia PhysX):

### 1.1. Unity Dedicated Server (Linux Headless)
Serverová část frameworku běží jako **nativní Unity Dedicated Server (Linux Headless Build)**, nikoliv jako samostatná konzolová aplikace mimo engine. Tento přístup je zvolen kvůli zachování stoprocentní shody fyzikálního světa (PhysX), kolizních vrstev a výpočetních algoritmů s klientskou aplikací.
*   **Optimalizace:** Sestavení je kompletně ořezáno o vykreslovací (rendering pipeline) a audio vrstvy (Render Stripping).
*   **Běhové prostředí:** Server plně využívá moderní runtime **CoreCLR (.NET)** integrovaný v Unity 6+, což zajišťuje nativní JIT kompilaci, vysoký výkon zpracování vláken a moderní garbage collection [1.1.2, 1.2.3].
*   **Zpracování 3D Assetů:** Server asynchronně parsuje `.glb` soubory pouze pro extrakci kolizních sítí (Mesh Colliders) [1.1.2]. Vizuální geometrie, LOD a materiály jsou při načítání na serveru kompletně zahozeny pro úsporu RAM.

### 1.2. Unity Client Runtime (Windows/Mac/Linux)
Klientská aplikace, která asynchronně stahuje, dešifruje přímo do paměti a vykresluje 3D modely a textury, spouští klientské skripty v izolovaném sandboxu a renderuje uživatelské rozhraní [1.1.2].

---

## 2. Datové modely a formáty objektů (Asset & Entity Schemas)

Veškeré objekty, konfigurace, mapové segmenty, nastavení serveru a databázové definice jsou popsány v jednotném JSON standardu.

### 2.1. Manifest Zdroje (`resource.json`)
Každý nezávislý Resource (modul) musí obsahovat tento soubor v kořenovém adresáři.

```json
{
  "name": "cyber_apartments",
  "version": "1.0.0",
  "author": "CyberDev Team",
  "server_scripts": [
    "server/main.js",
    "server/db_logger.js"
  ],
  "client_scripts": [
    "client/main.lua",
    "client/interaction.lua"
  ],
  "ui_page": "client/ui/index.html",
  "stream": [
    "stream/apartment_building.glb",
    "stream/heavy_door.glb",
    "stream/tuning_parts.glb"
  ]
}
```

### 2.2. Konfigurace Vozidla & Handling (`handling.json`)
Definuje fyzikální, výkonnostní a vizuální parametry vozidel.

```json
{
  "handling_id": "quadra_vtech_01",
  "mass": 1420.0,
  "drag_coefficient": 0.29,
  "center_of_mass_offset": [0.0, -0.15, 0.08],
  "engine": {
    "max_rpm": 8200,
    "idle_rpm": 900,
    "torque_curve": [
      [1000, 280],
      [3000, 520],
      [5500, 680],
      [8000, 410]
    ],
    "gears": 6,
    "transmission_type": "rwd"
  },
  "suspension": {
    "force": 42000.0,
    "damping": 3100.0,
    "travel": 0.15,
    "front_rear_bias": 0.48
  },
  "tires": {
    "grip_limit_forward": 1.28,
    "grip_limit_sideways": 1.18,
    "drift_bias": 0.35
  },
  "visual": {
    "emissive_neon_color": "#00FF66FF",
    "emissive_neon_intensity": 3.0,
    "sockets": [
      { "name": "spoiler_socket", "local_pos": [0.0, 0.95, -1.82] },
      { "name": "exhaust_socket", "local_pos": [-0.45, 0.25, -2.01] }
    ]
  }
}
```

### 2.3. Konfigurace Interaktivních Dveří (`door_entity.json`)
Definuje chování, limity a počáteční stav dveří ve 3D světě.

```json
{
  "door_id": "apt_lobby_gate",
  "model": "stream/heavy_door.glb",
  "door_type": "sliding",
  "physics": {
    "axis": [1.0, 0.0, 0.0],
    "movement_limit": 1.8,
    "speed": 2.2,
    "auto_close_delay": 4.0,
    "collision_layer": "Default"
  },
  "states": {
    "is_locked": true,
    "is_open": false
  }
}
```

### 2.4. Konfigurace Vizuálních Promptů (`prompt_theme.json`)
Definuje data-driven vizuální styly pro holografické a 3D nápovědy.

```json
{
  "theme_id": "cyber_neon_red",
  "style": {
    "font_path": "assets/fonts/cyberpunk_reg.ttf",
    "primary_color": "#FF0055FF",
    "secondary_color": "#00FFFFFF",
    "background_color": "#120108DD",
    "border_width": 1.8,
    "glow_intensity": 2.5,
    "scale_curve": "sine_wave",
    "scale_frequency": 1.2
  }
}
```

### 2.5. Konfigurace Segmentu Světa (`map_manifest.json`)
Definuje statické rozmístění 3D meshů (chunků) a dynamických entit v rámci mapového segmentu.

```json
{
  "segment_id": "district_01_north",
  "world_grid_coordinates": [12, 45],
  "chunks": [
    {
      "model": "stream/road_segment_12_45.glb",
      "position": [1200.0, 0.0, 4500.0],
      "rotation": [0.0, 0.0, 0.0]
    }
  ],
  "entities": [
    {
      "type": "interactive_door",
      "config_file": "cyber_apartments/door_entity.json",
      "position": [1205.4, 1.2, 4510.5],
      "rotation": [0.0, 90.0, 0.0]
    }
  ]
}
```

### 2.6. Globální Konfigurace Webového Serveru (`server_http_config.json`)
Určuje parametry pro integrovaný webový a WebSocket server.

```json
{
  "http_server": {
    "enabled": true,
    "listen_address": "0.0.0.0",
    "port": 30120,
    "enable_ssl": false,
    "ssl_certificate_path": "certs/live_cert.pfx",
    "api_rate_limit_per_minute": 120,
    "cors_allowed_origins": [
      "https://my-cyber-project.com",
      "http://localhost:3000"
    ]
  }
}
```

### 2.7. Konfigurace Zbraně (`weapon_config.json`)
Popisuje mechanické, střelné a vizuální chování zbraní.

```json
{
  "weapon_id": "cyber_rifle_lex",
  "model": "stream/weapons/lex_rifle.glb",
  "weapon_class": "rifle",
  "damage": 38.0,
  "fire_rate": 650,
  "max_ammo": 30,
  "range": 120.0,
  "recoil": {
    "vertical_kick": [1.2, 1.8],
    "horizontal_kick": [-0.5, 0.5],
    "settle_speed": 4.5,
    "camera_shake_intensity": 1.2
  },
  "dispersion": {
    "base_spread": 0.02,
    "max_spread": 0.15,
    "bloom_per_shot": 0.015,
    "recover_speed": 0.25
  },
  "attachments": {
    "scope_socket": [0.0, 0.12, -0.15],
    "muzzle_socket": [0.0, 0.02, 0.65],
    "mag_socket": [0.0, -0.18, 0.12]
  }
}
```

### 2.8. Konfigurace Modulárního Oblečení (`clothing_item.json`)
Definuje propojení oblečení s kostrou a optimalizační masky.

```json
{
  "clothing_id": "cyber_leather_jacket_05",
  "model": "stream/clothing/leather_jacket_05.glb",
  "slot": "torso",
  "skeleton_standard": "unity_humanoid",
  "blendshapes_map": {
    "clipping_shield": 100.0
  },
  "hidden_body_parts": [
    "chest",
    "upper_arms"
  ]
}
```

### 2.9. Konfigurace Kustomizace Humanoida (`character_customization.json`)
Popisuje stav parametrů vzhledu postavy odesílaný po síti.

```json
{
  "skin": {
    "primary_color": "#D3B79C",
    "roughness": 0.6,
    "cyberware_visibility": 1.0
  },
  "hair": {
    "model": "stream/hair/cyber_mohawk_02.glb",
    "primary_color": "#FF00FFFF",
    "secondary_color": "#00FFFFFF"
  },
  "face_sliders": {
    "nose_width": 0.45,
    "nose_bridge_height": -0.2,
    "jaw_width": 0.15,
    "cheekbone_height": 0.35,
    "eye_scale": 1.1
  },
  "eye_customization": {
    "iris_color": "#00FFDDFF",
    "iris_glow_intensity": 1.5,
    "cybernetic_lens_active": true
  }
}
```

---

## 3. Síťový kód (Netcode) a synchronizace stavu

Nízkoúrovňová síťová vrstva specifikace **APRTS_V1** je postavena na optimalizované knihovně **LiteNetLib** (spolehlivý/nespolehlivý přenos nad UDP v čistém C# bez alokací).

### 3.1. Synchronizace kontinuálního stavu (State Synchronization)
*   **Lokální hráč:** Využívá **Client-Side Prediction (CSP)** s lokální simulací pohybové fyziky a následnou **Server-side Reconciliation** (rekonciliací). Pokud se stav na serveru liší od lokální predikce klienta o více než definovanou toleranci, klient plynule opraví svou pozici (Rollback & Replay).
*   **Ostatní entity (Vzdálení hráči a vozidla):** Využívají **Entity Interpolation** s konfigurovatelným bufferem (standardně 100 ms) pro vyhlazení kolísání sítě (jitter).
*   **Komprese dat:** Pozice a rotace jsou před odesláním komprimovány pomocí delta komprese a kvantizace (quantization) desetinných čísel na 16-bitové integery relativně k herním chunkům.

### 3.2. Prostorové dělení sítě (Interest Management / Network Culling)
Server implementuje **Grid-based 3D Spatial Partitioning** (Area of Interest - AoI).
*   Herní svět je rozdělen do virtuálních 3D buněk o rozměru 128m × 128m.
*   Klient odebírá aktualizace pouze o entitách, které se nacházejí v jeho domovské buňce a sousedních buňkách (mřížka 3 × 3). Při přechodu entity přes hranici buňky server automaticky odešle klientům síťový příkaz k odregistrování (Unspawn) nebo registraci (Spawn) daného objektu.

### JS (JavaScript) Implementace (Event Routing):
```javascript
// SERVER-SIDE: Příjem eventu a síťové vysílání
Server.addEventHandler("aprts:server:requestSpawn", (player, vehicleModel) => {
    const coords = player.getCoords();
    const veh = World.createVehicle(vehicleModel, coords, player.getRotation());
    
    // Odeslání potvrzení zpět klientovi
    Server.triggerClientEvent(player, "aprts:client:spawnCompleted", veh.getId());
});

// CLIENT-SIDE: Odeslání požadavku na server a registrace callbacku
Client.triggerServerEvent("aprts:server:requestSpawn", "quadra_vtech_01");

Client.addEventHandler("aprts:client:spawnCompleted", (netId) => {
    Client.triggerCEFEvent("hud:notification", `Vozidlo vygenerováno s ID: ${netId}`);
});
```

### Lua Implementace (Event Routing):
```lua
-- SERVER-SIDE: Příjem eventu a síťové vysílání
AddEventHandler("aprts:server:requestSpawn", function(player, vehicleModel)
    local coords = player.GetCoords()
    local veh = CreateVehicle(vehicleModel, coords, player.GetRotation())
    
    -- Odeslání potvrzení zpět klientovi
    TriggerClientEvent(player, "aprts:client:spawnCompleted", veh.GetId())
end)

-- CLIENT-SIDE: Odeslání požadavku na server a registrace callbacku
TriggerServerEvent("aprts:server:requestSpawn", "quadra_vtech_01")

AddEventHandler("aprts:client:spawnCompleted", function(netId)
    TriggerCEFEvent("hud:notification", "Vozidlo vygenerováno s ID: " .. tostring(netId))
end)
```

---

## 4. Zabezpečení a ochrana duševního vlastnictví (Asset Protection & Sandbox)

### 4.1. Ochrana assetů (Anti-Theft)
Pro zamezení zcizení 3D modelů a textur z klientské cache je implementována šifrovací vrstva:
*   **Formát kontejneru:** Všechny streamované soubory jsou zabaleny do chráněného binárního archivního formátu `.aprts`.
*   **Šifrování:** Archivy jsou šifrovány algoritmem **AES-256-GCM**. Symetrický dešifrovací klíč je generován unikátně pro každou herní relaci (session) během handshake fáze připojení k serveru a rotován v reálném čase.
*   **Runtime dešifrování:** Klient dešifruje data **výhradně v operační paměti (RAM)** přímo do paměťového streamu knihovny `glTFast` / `KtxUnity` [1.1.2]. Dešifrovaná raw data (`.glb`, `.ktx2`) se nikdy nezapisují na pevný disk klienta.

### 4.2. Klientský Sandbox
Klientské skripty (JS/Lua) běží v přísně izolovaném prostředí:
*   **I/O Operace:** Jsou kompletně zakázány. Skripty nemají přístup k lokálnímu souborovému systému hráče.
*   **Síťová komunikace:** Skripty nemají možnost vytvářet raw sokety. HTTP požadavky jsou povoleny výhradně přes vestavěné zabezpečené API rozhraní `fetch()`, které podporuje pouze explicitně povolené domény (Whitelisting v konfiguraci serveru) a všechny dotazy jsou proxyovány přes server.

---

## 5. CDN a distribuce assetů (Asset Streaming Pipeline)

Stahování velkých 3D modelů nesmí omezovat šířku pásma (bandwidth) herního UDP protokolu.

### 5.1. Distribuce souborů
*   Pro vývojové a testovací účely obsahuje dedikovaný server vestavěný odlehčený HTTP server (Kestrel), který dokáže lokálně distribuovat soubory z aktivních resources.
*   V produkčním režimu vygeneruje autoritativní herní server během připojení hráče časově omezené **podepsané URL (Signed URLs)** pro externí **CDN (např. Cloudflare, AWS CloudFront)**. Klient následně stahuje veškerá herní data paralelně z CDN přes protokol HTTPS.

### 5.2. Chunk-based loading (Streamování světa)
*   Klient v reálném čase monitoruje vzdálenost hráče od středů mapových segmentů (chunků).
*   **Aktivní rádius (Loading):** Chunks se začnou asynchronně načítat na pozadí, jakmile je vzdálenost hráče menší než 200 metrů.
*   **Neaktivní rádius (Unloading):** Chunks jsou uvolněny z paměti a odstraněny z PhysX scény, pokud vzdálenost překročí 300 metrů (hystereze 100 metrů zabraňuje neustálému načítání a uvolňování na hranici segmentů).
*   Zpracování probíhá inkrementálně na vedlejších vláknech procesoru (Time-Slicing), aby se předešlo propadům snímkové frekvence (framerate drops) na klientovi [1.2.7].

---

## 6. Systém Entit a Prostorového Vyhledávání (Spatial Queries)

Rychlé prostorové dotazování za využití nativní Octree/PhysX struktury v C# jádru Unity [1.2.6].

### JS (JavaScript) Implementace:
```javascript
// CLIENT-SIDE: Vyhledání nejbližšího vozidla v okolí hráče
const searchRadius = 15.0;
const playerCoords = player.getCoords();

const closestVeh = World.getClosestVehicle(playerCoords, searchRadius);
if (closestVeh !== 0) {
    const isVeh = World.isEntityVehicle(closestVeh);
    if (isVeh) {
        Client.triggerServerEvent("aprts:server:interactVehicle", closestVeh);
    }
}
```

### Lua Implementace:
```lua
-- CLIENT-SIDE: Vyhledání nejbližšího vozidla v okolí hráče
local searchRadius = 15.0
local playerCoords = player.GetCoords()

local closestVeh = GetClosestVehicle(playerCoords, searchRadius)
if closestVeh ~= 0 then
    local isVeh = IsEntityVehicle(closestVeh)
    if isVeh then
        TriggerServerEvent("aprts:server:interactVehicle", closestVeh)
    end
end
```

---

## 7. Interaktivní Dveře (Interactive Door System)

Umožňuje plynulé ovládání a synchronizaci fyzických dveří.

### JS (JavaScript) Implementace:
```javascript
// CLIENT-SIDE: Ovládání dveří přes uživatelský skript
const door = World.getClosestDoor(player.getCoords(), 5.0);

if (World.isEntityDoor(door)) {
    const currentLocked = door.isLocked();
    door.setLocked(!currentLocked);
}
```

### Lua Implementace:
```lua
-- CLIENT-SIDE: Ovládání dveří přes uživatelský skript
local door = GetClosestDoor(player.GetCoords(), 5.0)

if IsEntityDoor(door) then
    local currentLocked = IsDoorLocked(door)
    LockDoor(door, not currentLocked)
end
```

---

## 8. Uživatelské rozhraní (Hybridní UI & Input Routing)

### 8.1. Výkon: Hybridní rozhraní
Vzhledem k vysoké procesorové a paměťové režii Chromium Embedded Frameworku (CEF) implementuje specifikace **APRTS_V1** hybridní přístup k UI:
1.  **CEF (Heavy-duty UI):** Je využíváno výhradně pro komplexní, statické panely, které nejsou citlivé na milisekundovou odezvu (např. herní inventář, tablet, obchody, interaktivní hackerské terminály).
2.  **Unity UI Toolkit / Canvas (Performant UI):** Běží nativně v enginu pro rychlé akční prvky s minimální režií (HUD, rychloměr, nitkový kříž, vizuální efekty poškození).

### 8.2. Input Routing (Směrování vstupu)
Nativní C# třída `InputManager` na klientovi spravuje zaměření vstupu (focus) mezi herním světem a CEF oknem:
*   `GAMEPLAY_MODE`: Myš je uzamčena a skryta. Všechny klávesy a pohyby myši jsou předávány pohybovému kontroleru postavy. CEF ignoruje veškeré kliky (Pass-through).
*   `UI_FOCUS_MODE`: Myš je odemčena a zobrazena. Veškeré vstupy myši a klávesnice zachytává CEF. Herní akce jsou blokovány, s výjimkou omezených pohybových kláves (WASD) pro chůzi při otevřeném HUDu, pokud je to v konfiguraci povoleno.

#### JS (JavaScript) Implementace (Přepínání Focusu):
```javascript
// CLIENT-SIDE: Přepnutí vstupu na CEF rozhraní (JS)
Client.addEventHandler("hud:openInventory", () => {
    // Parametry: enableCursor, blockGameplayInputs
    UI.setInputFocus(true, true);
    Client.triggerCEFEvent("inventory:show", true);
});
```

#### Lua Implementace (Přepínání Focusu):
```lua
-- CLIENT-SIDE: Přepnutí vstupu na CEF rozhraní (Lua)
AddEventHandler("hud:openInventory", function()
    -- Parametry: enableCursor, blockGameplayInputs
    SetInputFocus(true, true)
    TriggerCEFEvent("inventory:show", true)
end)
```

---

## 9. CEF Raycasting & World Interaction

Převod 2D pixelových souřadnic z webového rozhraní CEF na 3D bod v Unity scéně.

### JS (JavaScript) Implementace:
```javascript
// CLIENT-SIDE: Kliknutí myši v CEF odeslané do hry
Client.addCEFEventHandler("ui:onClick", (pixelX, pixelY) => {
    const maxRaycastDistance = 100.0;
    const result = Camera.screenToWorld(pixelX, pixelY, maxRaycastDistance);

    if (result.hit) {
        Client.log(`Klik na 3D bod: [${result.coords[0]}, ${result.coords[1]}, ${result.coords[2]}]`);
        if (result.entity !== 0) {
            Client.triggerServerEvent("aprts:server:entityClicked", result.entity);
        }
    }
});
```

### Lua Implementace:
```lua
-- CLIENT-SIDE: Kliknutí myši v CEF odeslané do hry
AddCEFEventHandler("ui:onClick", function(pixelX, pixelY)
    local maxRaycastDistance = 100.0
    local hit, coords, entity = ScreenToWorld(pixelX, pixelY, maxRaycastDistance)

    if hit then
        print(string.format("Klik na 3D bod: [%f, %f, %f]", coords.x, coords.y, coords.z))
        if entity ~= 0 then
            TriggerServerEvent("aprts:server:entityClicked", entity)
        end
    end
end)
```

---

## 10. Nativní Prompt Engine (Holografické nápovědy)

Zajišťuje vykreslování optimalizovaných interaktivních prvků nad entitami.

### JS (JavaScript) Implementace:
```javascript
// CLIENT-SIDE: Registrace a napojení promptu na dům
const prompt = UI.registerPrompt("F", "Vstoupit do bytu", "cyber_neon_red");
UI.setPromptRequiredHoldTime(prompt, 1000);

const apartmentCoords = [1205.4, 1.2, 4510.5];
UI.attachPromptToCoords(prompt, apartmentCoords, 3.5);

UI.onPromptCompleted(prompt, () => {
    Client.triggerServerEvent("aprts:server:enterApartment");
});
```

### Lua Implementace:
```lua
-- CLIENT-SIDE: Registrace and napojení promptu na dům
local prompt = RegisterPrompt("F", "Vstoupit do bytu", "cyber_neon_red")
SetPromptRequiredHoldTime(prompt, 1000)

local apartmentCoords = Vector3(1205.4, 1.2, 4510.5)
AttachPromptToCoords(prompt, apartmentCoords, 3.5)

OnPromptCompleted(prompt, function()
    TriggerServerEvent("aprts:server:enterApartment")
end)
```

---

## 11. Dynamický Animační Engine (PlayableGraph / Masking)

Načítání a runtime modifikace animací bez závislosti na předem připravených stavových strojích.

### JS (JavaScript) Implementace:
```javascript
// CLIENT-SIDE: Dynamická kombinace chůze a animace kouření
Client.registerAnimationDict("ambient_cyber");

player.playAnim("ambient_cyber", "walk_relaxed", 1.0, true, "legs_only");
player.playAnim("ambient_cyber", "smoke_cigarette", 0.3, true, "right_arm");
```

### Lua Implementace:
```lua
-- CLIENT-SIDE: Dynamická kombinace chůze a animace kouření
RegisterAnimationDict("ambient_cyber")

PlayAnim(playerPed, "ambient_cyber", "walk_relaxed", 1.0, true, "legs_only")
PlayAnim(playerPed, "ambient_cyber", "smoke_cigarette", 0.3, true, "right_arm")
```

---

## 12. Abstraktní Databázová Vrstva (Unified DBAL)

Zajišťuje jednotné datové operace. Konkrétní driver (SQLite, MySQL, PostgreSQL) řeší serverové C# jádro na základě globálního nastavení, vývojář pracuje se shodným API [1.1.2].

### 12.1. Connection Pooling
C# jádro serveru implementuje nativní asynchronní pool připojení (např. *MySqlConnectionPool* nebo *NpgsqlConnectionPool* pro PostgreSQL), který je řízen na dedikovaných vláknech mimo herní tick. Každá asynchronní operace si vyžádá připojení z poolu a po dokončení transakce ho okamžitě uvolní.

### 12.2. Migrace a schémata (Resource-driven Migrations)
Každá Resource složka může obsahovat podsložku `migrations/` se sekvenčně číslovanými `.sql` skripty (např. `0001_init.sql`, `0002_add_inventory.sql`).
*   Při spuštění Resource C# jádro automaticky porovná přítomné soubory s interní systémovou tabulkou `aprts_migrations`.
*   Chybějící skripty jsou automaticky spuštěny v rámci jedné databázové transakce a tabulka je aktualizována. Správa schématu je tak plně automatizovaná.

### JS (JavaScript) Implementace:
```javascript
// SERVER-SIDE: Bezpečné asynchronní uložení dat
Server.addEventHandler("aprts:server:saveUserData", async (player) => {
    const license = player.getLicense();
    const money = player.getMoney();
    
    const query = "UPDATE users SET money = ? WHERE license = ?";
    const params = [money, license];
    
    try {
        const rowsAffected = await Database.executeNonQuery(query, params);
        if (rowsAffected > 0) {
            Server.log(`Data uložena pro licenci: ${license}`);
        }
    } catch (e) {
        Server.logError(`Chyba zápisu do DB: ${e.message}`);
    }
});
```

### Lua Implementace:
```lua
-- SERVER-SIDE: Bezpečné asynchronní uložení dat
AddEventHandler("aprts:server:saveUserData", function(player)
    local license = player.GetLicense()
    local money = player.GetMoney()
    
    local query = "UPDATE users SET money = ? WHERE license = ?"
    local params = { money, license }
    
    Database.ExecuteNonQuery(query, params, function(rowsAffected)
        if rowsAffected > 0 then
            print("Data uložena pro licenci: " .. license)
        else
            print("Varování: Žádný řádek nebyl v databázi upraven.")
        end
    end)
end)
```

---

## 13. Prostorové 3D Audio

Umožňuje dynamické přehrávání a útlum prostorových zvuků na základě 3D souřadnic.

### JS (JavaScript) Implementace:
```javascript
// CLIENT-SIDE: 3D lokální ambientní zvuk neonového bzučení
const soundPos = [1205.4, 3.5, 4510.5];
const soundId = Audio.play3DAudioAtCoords("sfx/neon_buzz.ogg", soundPos, {
    volume: 0.5,
    maxDistance: 8.0,
    loop: true
});
```

### Lua Implementace:
```lua
-- CLIENT-SIDE: 3D lokální ambientní zvuk neonového bzučení
local soundPos = Vector3(1205.4, 3.5, 4510.5)
local soundOptions = {
    volume = 0.5,
    maxDistance = 8.0,
    loop = true
}

local soundId = Play3DAudioAtCoords("sfx/neon_buzz.ogg", soundPos, soundOptions)
```

---

## 14. NFS UG2-style Vehicle Tuning

Umožňuje detailní vizuální a strukturální úpravy vozidel v reálném čase.

### JS (JavaScript) Implementace:
```javascript
// CLIENT-SIDE: Kompletní úprava laku, spoileru a neonů v garáži
const veh = World.getClosestVehicle(player.getCoords(), 5.0);

if (veh !== 0) {
    veh.setPaintStyle("#00FF66", "#330033", 0.9, 0.1);
    veh.installTuningPart("spoiler_socket", "tuning_spoiler_shogun.glb");
    veh.setNeonState(true, "#00FF66", 4.0);
}
```

### Lua Implementace:
```lua
-- CLIENT-SIDE: Kompletní úprava laku, spoileru a neonů v garáži
local veh = GetClosestVehicle(player.GetCoords(), 5.0)

if veh ~= 0 then
    SetVehiclePaintStyle(veh, "#00FF66", "#330033", 0.9, 0.1)
    InstallVehicleTuningPart(veh, "spoiler_socket", "tuning_spoiler_shogun.glb")
    SetVehicleNeonState(veh, true, "#00FF66", 4.0)
end
```

---

## 15. Integrovaný Webový a API Server (HTTP & WebSockets)

Zpracování HTTP a WebSocket požadavků probíhá na samostatných systémových vláknech mimo herní tickrate.

### JS (JavaScript) Implementace:
```javascript
// SERVER-SIDE: Registrace API a WebSocket trasy pro online mapu (JS)
const WEB_KEY = "secure_token_9876";

Http.onGet("/api/v1/players", (request, response) => {
    const list = Server.getPlayers().map(p => ({
        id: p.getId(),
        name: p.getName(),
        coords: p.getCoords()
    }));
    response.setStatus(200);
    response.setContentType("application/json");
    response.send(JSON.stringify(list));
});

Http.onWebSocket("/ws/v1/live_map", (socket) => {
    const intervalId = setInterval(() => {
        if (!socket.isOpen()) {
            clearInterval(intervalId);
            return;
        }
        const data = Server.getPlayers().map(p => ({
            id: p.getId(),
            x: p.getCoords()[0],
            z: p.getCoords()[2]
        }));
        socket.send(JSON.stringify(data));
    }, 100);

    socket.onClose(() => {
        clearInterval(intervalId);
    });
});
```

### Lua Implementace:
```lua
-- SERVER-SIDE: Registrace API a WebSocket trasy pro online mapu (Lua)
local WEB_KEY = "secure_token_9876"

Http.OnGet("/api/v1/players", function(request, response)
    local list = {}
    local activePlayers = GetPlayers()
    for _, p in ipairs(activePlayers) do
        table.insert(list, {
            id = p.GetId(),
            name = p.GetName(),
            coords = p.GetCoords()
        })
    end
    response.SetStatus(200)
    response.SetContentType("application/json")
    response.Send(JsonEncode(list))
end)

Http.OnWebSocket("/ws/v1/live_map", function(socket)
    local runLoop = true
    SetTimeout(100, function(loopFunc)
        if not socket.IsOpen() then
            runLoop = false
            return
        end
        local data = {}
        local activePlayers = GetPlayers()
        for _, p in ipairs(activePlayers) do
            local coords = p.GetCoords()
            table.insert(data, {
                id = p.GetId(),
                x = coords.x,
                z = coords.z
            })
        end
        socket.Send(JsonEncode(data))
        if runLoop then
            SetTimeout(100, loopFunc)
        end
    end)

    socket.OnClose(function()
        runLoop = false
    end)
end)
```

---

## 16. Diagnostika, Debugging a Hot-Reload

### 16.1. Izolace chyb (Exception Handling)
Každý spuštěný skript (JS/Lua) v rámci konkrétního Resource běží ve svém vlastním chráněném kontextu (try-catch na úrovni hostitelského interpretu).
*   Pokud dojde k neošetřené výjimce, **zhroutí se pouze kontext daného Resource**.
*   Ostatní běžící Resources, herní klient i dedikovaný server pokračují bez přerušení v chodu. Chyba je okamžitě zachycena a zapsána do diagnostického logu s úplným stack trace.

### 16.2. Vzdálené ladění (Debugging)
*   **JavaScript (V8):** Běhové prostředí podporuje protokol **V8 Inspector**. Spuštěním serveru či klienta s parametrem `--debug-port=9229` lze k běžícímu skriptovacímu kontextu připojit standardní nástroje jako *Chrome DevTools* nebo *VS Code Debugger*.
*   **Lua (MoonSharp):** Podporuje integraci se vzdáleným VS Code Lua debuggerem přes dedikovaný TCP port.

### 16.3. Hot-Reloading a migrace stavových proměnných
Podpora pro kompletní reload skriptů za běhu bez nutnosti odpojovat hráče ze serveru.
*   **Mechanismus:** Při požadavku na reload vyvolá framework událost `onResourcePreReload`. Skript může v této fázi serializovat své klíčové proměnné (např. aktivní herní relace, rozpracované úkoly) do JSON řetězce a uložit je do dočasného úložiště v C# paměti.
*   Po kompilaci a spuštění nové verze skriptu se vyvolá událost `onResourcePostReload`, která data z C# paměti přečte a obnoví stav.

#### JS (JavaScript) Implementace (Hot-Reload):
```javascript
// SERVER-SIDE: Ukázka serializace a obnovy stavu během reloadu (JS)
let activeQuests = { player_12: "quest_hacker_lvl1" };

Server.on("onResourcePreReload", () => {
    Server.setTempData("activeQuestsState", JSON.stringify(activeQuests));
});

Server.on("onResourcePostReload", () => {
    const rawData = Server.getTempData("activeQuestsState");
    if (rawData) {
        activeQuests = JSON.parse(rawData);
        Server.log("Stav úkolů byl po hot-reloadu úspěšně obnoven.");
    }
});
```

#### Lua Implementace (Hot-Reload):
```lua
-- SERVER-SIDE: Ukázka serializace a obnovy stavu během reloadu (Lua)
local activeQuests = { player_12 = "quest_hacker_lvl1" }

AddEventHandler("onResourcePreReload", function()
    SetTempData("activeQuestsState", JsonEncode(activeQuests))
end)

AddEventHandler("onResourcePostReload", function()
    local rawData = GetTempData("activeQuestsState")
    if rawData then
        activeQuests = JsonDecode(rawData)
        print("Stav úkolů byl po hot-reloadu úspěšně obnoven.")
    end
end)
```

---

## 17. Vývojářský a Modding Pipeline

### 17.1. Pravidla pro 3D grafiky (Blender)
*   **Formát:** Export do `.glb` s povolenou kompresí KTX 2.0 (Basis Universal) [1.1.2].
*   **Kostra:** Všechny postavy musí dodržovat standardní pojmenování a strukturu Unity Humanoid Rig pro funkčnost dynamického maskování animací.
*   **Materiálové kanály:** Vertexové barvy (`COLOR_0`) slouží jako masky pro detailní vrstvení shaderů (R = Poškození, G = Špína, B = Mokrý povrch).
*   **Pivoty:** Interaktivní objekty (např. dveře) must mít pivot (střed otáčení) přesně umístěn na ose rotace/pantů.

### 17.2. Životní cyklus Resource na Klientovi
Když administrátor spustí nebo zastaví Resource, C# jádro frameworku provede automatický úklid paměti:
1.  **Stop:** Ukončí příslušný JS/Lua kontext (sandbox) [1.1.8, 1.2.1].
2.  **Zničení WebView:** CEF instance spojená s daným Resource je uvolněna z paměti.
3.  **Uvolnění Assetů:** Všechny dynamicky načtené GLB modely, zvuky a textury jsou bezpečně odstraněny z RAM a VRAM, čímž se předchází zaplnění paměti (memory leakům).

---

## 18. Systém zbraní a střelby (Weapon & Recoil Engine)

Tento modul definuje chování zbraní, aplikaci poškození, zpětný ráz (recoil), rozptyl střel (dispersion) a instalaci doplňků na zbraňové sockety.

### 18.1. Logika střelby a balistika
*   Zbraně jsou reprezentovány samostatnými GLB entitami asynchronně uchycenými na socket pravé ruky humanoida (`hand_r_socket`).
*   Výpočet zásahů probíhá autoritativně na serveru přes matematický paprsek. Pro zajištění škálovatelnosti při vysokém počtu hráčů server neprovádí fyzickou transformaci PhysX těles v herní scéně. Lag kompenzace využívá Job System a Burst Compiler k matematickému vyhodnocení průsečíku paprsku se zjednodušenou historií hitboxů (složených z max. 5 kapslí na humanoidní entitu), přičemž výpočet je prostorově omezen pouze na Area of Interest (AoI) střelce.
*   **Zpětný ráz (Recoil):** Po každém výstřelu se na klientovi aplikuje vertikální a horizontální ráz, který posune úhel kamery (Camera Kick). Tento ráz se plynule vrací do původní polohy rychlostí `settle_speed`.
*   **Rozptyl střel (Dispersion/Bloom):** Opakovaný výstřel lineárně zvětšuje rozptyl střel o hodnotu `bloom_per_shot` až do limitu `max_spread`. Pokud hráč přestane střílet, rozptyl se obnoví rychlostí `recover_speed`.

### JS (JavaScript) Implementace (Zbraně & Doplňky):
```javascript
// CLIENT-SIDE: Vybavení zbraně a montáž kolimátoru (JS)
const playerPed = Player.getPed();

// Vybavení zbraně a nastavení munice (Server-Authoritative)
Client.triggerServerEvent("weapons:equip", "cyber_rifle_lex", 90);

Client.addEventHandler("weapons:equipped", (weaponEntityId) => {
    const weapon = World.getEntity(weaponEntityId);
    
    if (World.isEntityWeapon(weapon)) {
        // Instalace zaměřovače na příslušný socket zbraně
        weapon.attachAttachment("scope_socket", "scope_red_dot.glb");
        // Instalace tlumiče na hlaveň
        weapon.attachAttachment("muzzle_socket", "suppressor_light.glb");
    }
});
```

### Lua Implementace (Zbraně & Doplňky):
```lua
-- CLIENT-SIDE: Vybavení zbraně a montáž kolimátoru (Lua)
local playerPed = GetPlayerPed()

-- Vybavení zbraně a nastavení munice (Server-Authoritative)
TriggerServerEvent("weapons:equip", "cyber_rifle_lex", 90)

AddEventHandler("weapons:equipped", function(weaponEntityId)
    local weapon = GetEntity(weaponEntityId)
    
    if IsEntityWeapon(weapon) then
        -- Instalace zaměřovače na příslušný socket zbraně
        AttachWeaponAttachment(weapon, "scope_socket", "scope_red_dot.glb")
        -- Instalace tlumiče na hlaveň
        AttachWeaponAttachment(weapon, "muzzle_socket", "suppressor_light.glb")
    end
end)
```

---

## 19. Systém modulárního oblečení (Modular Clothing & Bone Sharing)

Pro zamezení prolínání textur těla humanoida a oblečení (clipping) implementuje specifikace **APRTS_V1** modulární architekturu humanoidů s technologií sdílení kostry (Bone Sharing).

### 19.1. Odstranění clippingu a Bone Sharing
*   **Modulární model humanoida:** Výchozí postava postrádá jedno celistvé tělo. Je složena z nezávislých částí (hlava, torzo, ruce, nohy, chodidla).
*   **Skrývání částí těla (Culling):** Při načtení oblečení, například bundy, JSON konfigurace definuje, které části těla se mají zneaktivnit (např. `chest`, `upper_arms`). Tím se kompletně zamezí pronikání kůže skrz 3D model bundy.
*   **Sdílení kostry (Bone Sharing):** Oblečení je importováno jako GLB s vlastním `SkinnedMeshRendererem`. Na klientské úrovni C# jádro vyhledá kosti hlavního Animátoru postavy a přiřadí je přímo do `SkinnedMeshRendereru` oblečení [1.1.2]. Oblečení se tak ohýbá a animuje přesně podle kostí humanoida bez nutnosti počítat duplicitní kostry.

### JS (JavaScript) Implementace (Modular Clothing):
```javascript
// CLIENT-SIDE: Oblečení kožené bundy s maskováním těla (JS)
const playerPed = Player.getPed();

// Požadavek na server pro změnu oblečení
Client.triggerServerEvent("clothing:equip", "cyber_leather_jacket_05");

Client.addEventHandler("clothing:applied", (clothingSlot, glbModel, hiddenParts) => {
    // Klientská aplikace oblečení
    // 1. Skryje definované části těla humanoida pro zamezení clippingu
    hiddenParts.forEach(partName => {
        playerPed.setBodyPartActive(partName, false);
    });

    // 2. Načte GLB model oblečení a sdílí kosti s humanoidem
    playerPed.equipModularClothing(clothingSlot, glbModel);
});
```

### Lua Implementace (Modular Clothing):
```lua
-- CLIENT-SIDE: Oblečení kožené bundy s maskováním těla (Lua)
local playerPed = GetPlayerPed()

-- Požadavek na server pro změnu oblečení
TriggerServerEvent("clothing:equip", "cyber_leather_jacket_05")

AddEventHandler("clothing:applied", function(clothingSlot, glbModel, hiddenParts)
    -- Klientská aplikace oblečení
    -- 1. Skryje definované části těla humanoida pro zamezení clippingu
    for _, partName in ipairs(hiddenParts) do
        SetPedBodyPartActive(playerPed, partName, false)
    end

    -- 2. Načte GLB model oblečení a sdílí kosti s humanoidem
    EquipPedModularClothing(playerPed, clothingSlot, glbModel)
end)
```

---

## 20. Kustomizace humanoida a obličejových Blendshapů (Morph Targets)

Tento modul definuje detailní úpravu vzhledu postavy (vlasy, barvy, proporce obličeje) a podporu obličejových blendshapů (morph targetů) v reálném čase.

### 20.1. Kustomizace proporcí (Face Sliders)
*   Změna šířky nosu, výšky čelistí či měřítka očí se provádí manipulací s **BlendShapy (Morph Targety)** integrovanými přímo v základním GLB modelu hlavy postavy.
*   Knihovna `glTFast` naimportuje geometrii včetně morph targetů, které jsou mapovány na `SkinnedMeshRenderer` hlavy [1.1.2]. C# jádro umožňuje plynule měnit váhu (weight) jednotlivých blendshapů v rozsahu `0.0` až `100.0`.
*   **Materiálová kustomizace:** Barva očí, tón pleti a barva vlasů jsou korigovány dynamickou změnou instancí materiálů (Shader Properties). Vlasy podporují dvoubarevný gradient (primární a sekundární barva) s nastavitelnou ostrostí přechodu.

### 20.2. Obličejové Blendshapy a Lip-Sync
*   Model hlavy obsahuje 52 standardizovaných blendshapů (odpovídajících Apple ARKit Face Tracking standardu), což zajišťuje univerzální kompatibilitu s externími animacemi a sledováním obličeje.
*   **Lip-Sync:** Systém podporuje automatické otevírání úst a tvorbu hlásek (Phonemes) na základě audio streamu. Hlasový chat nebo přehrávané nahrávky NPC v reálném čase analyzují frekvence zvuku a převádějí je na váhy blendshapů `jawOpen`, `mouthPucker` a `mouthFunnel`.

### JS (JavaScript) Implementace (Customization & Expressions):
```javascript
// CLIENT-SIDE: Změna parametrů obličeje a spuštění úsměvu (JS)
const playerPed = Player.getPed();

// 1. Nastavení proporcí obličeje (Sliders)
playerPed.setFaceSlider("nose_width", 0.85);
playerPed.setFaceSlider("eye_scale", 1.2);

// 2. Nastavení barev (Shader Properties)
playerPed.setSkinColor("#D3B79C");
playerPed.setEyeIrisCustomization("#00FFDD", 1.5);

// 3. Spuštění obličejového výrazu (Blendshape kombinace)
playerPed.setFaceExpression("smile", 0.8, 300);
```

### Lua Implementace (Customization & Expressions):
```lua
-- CLIENT-SIDE: Změna parametrů obličeje a spuštění úsměvu (Lua)
local playerPed = GetPlayerPed()

-- 1. Nastavení proporcí obličeje (Sliders)
SetPedFaceSlider(playerPed, "nose_width", 0.85)
SetPedFaceSlider(playerPed, "eye_scale", 1.2)

-- 2. Nastavení barev (Shader Properties)
SetPedSkinColor(playerPed, "#D3B79C")
SetPedEyeIrisCustomization(playerPed, "#00FFDD", 1.5)

-- 3. Spuštění obličejového výrazu (Blendshape kombinace)
SetPedFaceExpression(playerPed, "smile", 0.8, 300)
```

---

Tento dokument ve verzi **specifikace APRTS_V1** definuje závazné technologické standardy, datové struktury a skriptovací protokoly pro moderní, vysoce výkonný a otevřený multiplayerový ekosystém postavený na platformě Unity.