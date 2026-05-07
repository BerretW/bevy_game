

# Apparatus Drawable System (ADS) – Technical Design Document

## 1. Úvod a Filosofie

**Apparatus Drawable** (dále jen "Drawable") je základní stavební blok pro reprezentaci herních objektů (assets) v enginu. Namísto tradičního přístupu (načtení prostého 3D modelu a jeho ručního nastavování v editoru enginu) je Drawable **data-driven kontejner**, který sdružuje:

1. **Vizuální data:** Hierarchii meshů a vertex data (obsahující masky).
2. **Fyzikální data:** Zjednodušenou geometrii a parametry (hmotnost, tření, statický/dynamický stav) pro fyzikální engine (např. Rapier/Avian).
3. **Materiálovou logiku:** Mapování jmen materiálů na konkrétní "Uber-Shadery" s definovanými parametry (včetně počasí) a způsobem získávání textur (sdílené vs. přibalené).

Tento systém umožňuje oddělit práci grafiků/designerů v Blenderu od herní logiky v Rustu a zajišťuje masivní úsporu VRAM pomocí globálních registrů textur.

---

## 2. Anatomie Drawable

Z hlediska distribuce se jeden Drawable skládá ze dvou souborů stejného jména:

1. `[asset_name].glb` – **Binární kontejner (Tělo).** Obsahuje surovou geometrii, hierarchii uzlů (nodes), Vertex Colors (použité jako masky materiálu) a případně vložené (embedded) obrazové soubory.
2. `[asset_name].toml` – **Manifest (Duše).** Definuje sémantiku uzlů uvnitř GLB a mapuje materiály na shadery.

---

## 3. Specifikace Manifestu (`.toml`)

Manifest se parsuje pomocí `serde` a má tři hlavní sekce: metadata, materiály a entity.

### 3.1. Ukázková struktura (Příklad: `barrel.toml`)

```toml
asset_name = "barrel"
version = "1.1"

[materials]
"Metal_Rust_Mat" = {
    template = "standard_pbr",
    textures = {
        albedo = { name = "barrel_01_d", source = "embedded" },
        mrao = { name = "rust_generic_mrao", source = "shared" },
        normal = { name = "rust_generic_n", source = "shared" },
        palette = { name = "default_lut", source = "shared" },
        snow = { name = "snow_01_d", source = "shared" }
    },
    params = {
        tint = [1.0, 1.0, 1.0, 1.0],
        tiling = 1.0,
        porosity = 0.0,
        wetness = 0.0,
        snow_level = 0.0,
        dirt_level = 0.5
    }
}

[entities]
"Barrel_Vis" = {
    type = "MESH",
    cast_shadows = true
}

"COL_Barrel" = {
    type = "COLLISION",
    shape = "CYLINDER",
    mass = 20.0,
    is_static = false,
    friction = 0.6,
    restitution = 0.2,
    tags = ["explosive", "prop"]
}
```

### 3.2. Rust Serde Struktury

Zde je návrh struktur pro deserializaci v Rustu:

```rust
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Debug, Clone)]
pub struct DrawableManifest {
    pub asset_name: String,
    pub version: String,
    pub materials: HashMap<String, MaterialDef>,
    pub entities: HashMap<String, EntityDef>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MaterialDef {
    /// Název wgsl šablony (např. "standard_pbr", "layered_env")
    pub template: String,
    /// Sloty textur (klíče: "albedo", "normal", atd.)
    pub textures: HashMap<String, TextureInfo>,
    /// Hodnoty pro Uniform buffer shaderu
    #[serde(default)]
    pub params: MaterialParams,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TextureInfo {
    pub name: String,
    pub source: TextureSource,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextureSource {
    Shared,
    Embedded,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct MaterialParams {
    pub tint: Option<[f32; 4]>,
    pub tiling: Option<f32>,
    pub l0_tiling: Option<f32>,
    pub l1_tiling: Option<f32>,
    pub porosity: Option<f32>,
    pub wetness: Option<f32>,
    pub snow_level: Option<f32>,
    pub dirt_level: Option<f32>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")] // Očekává v TOML klíč "type = 'MESH'" apod.
pub enum EntityDef {
    MESH {
        #[serde(default)]
        cast_shadows: bool,
    },
    COLLISION {
        shape: CollisionShape,
        #[serde(default)]
        mass: f32,
        #[serde(default)]
        is_static: bool,
        #[serde(default)]
        friction: f32,
        #[serde(default)]
        restitution: f32,
        #[serde(default)]
        tags: Vec<String>,
    },
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "UPPERCASE")]
pub enum CollisionShape {
    BOX,
    SPHERE,
    CAPSULE,
    CYLINDER,
    CONVEX,
    MESH,
}
```

---

## 4. Architektura Spawnování (Scene Hooking)

V Bevy nelze GLB nahrát a okamžitě upravit. `AssetServer` vrací Handle a scéna se spawnuje asynchronně. Proces zpracování (Scene Hooking) probíhá ve třech fázích.

### Fáze 1: Požadavek na Spawn

Herní logika (např. Lua skript přes `CommandQueue`) si vyžádá spawn objektu.

```rust
// Lua API zavolá: World.SpawnObject("barrel", {x=0, y=0, z=0})

// V Rustu (process_lua_commands):
commands.spawn((
    SceneRoot(asset_server.load("models/barrel.glb#Scene0")),
    Transform::from_xyz(0.0, 0.0, 0.0),
    DrawableSpawnIntent {
        manifest_handle: asset_server.load("models/barrel.toml"),
    },
));
```

*(Poznámka: K načtení TOML souborů pomocí `AssetServer` je potřeba implementovat jednoduchý `AssetLoader` pro typ `DrawableManifest`.)*

### Fáze 2: Zpracování uzlů scény (Hooking)

Vytvoříme systém, který naslouchá na události `SceneInstanceReady` (nebo iteruje Entity, které mají `SceneInstance` a `DrawableSpawnIntent`, ale ještě nebyly zpracovány).

Když je scéna (GLB) připravena a `.toml` manifest je načten:

1. **Iterace Potomků:** Systém projde všechny děti entity s `SceneInstance`.
2. **Identifikace uzlů:** U každého dítěte získá jeho komponentu `Name`. Porovná ji s klíči v sekci `[entities]` v `DrawableManifest`.

### Fáze 3: Aplikace definic

Na základě definice v TOML se entita (uzel z GLB) radikálně upraví.

#### 3.A. Vizuální uzly (`type = "MESH"`)

Pokud uzel odpovídá `EntityDef::MESH`:

1. Ponecháme mu komponenty `Transform` a `Mesh3d`.
2. Z `MeshMaterial3d` přečteme původní (placeholder) materiál z GLTF a získáme jeho jméno.
3. Najdeme toto jméno v sekci `[materials]` v Manifestu.
4. **Získání Textur:** Systém projde sloty (albedo, normal...).
   * Pokud `source == "shared"`: Vyžádá Handle z globálního `TextureRegistry` (viz sekce 6).
   * Pokud `source == "embedded"`: Najde texturu podle jména přímo uvnitř načtené struktury Bevy `Gltf`.
5. **Aplikace Shaderu:** Na základě hodnoty `template` se vytvoří instance příslušného Custom Shaderu (např. `StandardPbrMaterial` nebo `LayeredEnvMaterial`), naplní se Handles textur a parametry.
6. Původní `MeshMaterial3d<StandardMaterial>` se nahradí za `MeshMaterial3d<CustomMaterial>`.

#### 3.B. Fyzikální uzly (`type = "COLLISION"`)

Pokud uzel odpovídá `EntityDef::COLLISION`:

1. **Odstranění Vizuálu:** Odstraníme komponenty `Mesh3d` a `MeshMaterial3d` (nebo přidáme `Visibility::Hidden`). Fyzika nesmí být vidět.
2. **Generování Collideru:** Podle `shape` vygenerujeme Collider z fyzikálního enginu (Avian).
   * Pro tvary jako `BOX` nebo `SPHERE` spočítáme rozměry z (nyní odstraněného) Bounding Boxu mesh geometrie.
   * Pro `CONVEX` získáme pozice vertexů z meshe a vytvoříme `Collider::convex_hull(vertices)`.
3. **Nastavení RigidBody:**
   * Vložíme `RigidBody::Static` (pokud `is_static == true`), jinak `RigidBody::Dynamic`.
   * Nastavíme hmotnost a fyzikální materiál (tření, odrazivost).

---

## 5. Pipeline pro Vertex Colors (Masky materiálu)

Zásadním prvkem vizuálu jsou Vertex Colors, které slouží jako data (nikoliv barvy) pro míchání vrstev v našich Uber-Shaderech. Blender plugin tato data ukládá během exportu.

### 5.1. Logika mapování kanálů

Ve WGSL shaderu (`in.color`) reprezentují kanály následující vlastnosti:

* `R (Red)`: Faktor pro prolínání vrstev (`l0_albedo` vs `l1_albedo`).
* `G (Green)`: Maska pro zobrazení krve nebo špíny (reaguje na globální proměnnou prostředí).
* `B (Blue)`: Maska pro tvorbu kaluží a zadržování vlhkosti.
* `A (Alpha)`: UV souřadnice (U) pro vzorkování palety barev (Tinting).

### 5.2. Zpracování v Bevy (Sanitizace)

Standardní glTF načte barvy do atributu `Mesh::ATTRIBUTE_COLOR` jako `vec4<f32>`. Ne všechny modely (zvláště ty stažené z internetu, nikoliv exportované přes náš plugin) ale tento atribut mají.

**Ošetření chybějících Vertex Colors:**
Aby nedošlo k havárii shaderu nebo neočekávanému vizuálu (např. celý model krvavý kvůli chybějícím datům), musí proces Scene Hookingu u `type = "MESH"` zkontrolovat existenci tohoto atributu.

```rust
// Při aplikaci nového materiálu na MESH entitu:
if let Some(mut mesh) = meshes.get_mut(mesh_handle) {
    if !mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR) {
        // Model nemá Vertex Colors. Naplníme neutrálními daty:
        // R=0 (Základní vrstva), G=0 (Čisto), B=0 (Sucho), A=0 (Default paleta)
        let num_vertices = mesh.count_vertices();
        let default_colors = vec![[0.0, 0.0, 0.0, 0.0]; num_vertices];
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, default_colors);
    }
}
```

---

## 6. Globální Texture Registry (VRAM optimalizace)

Aby se zabránilo vícenásobnému načítání stejných `.dds` textur do paměti GPU, engine udržuje centrální resource pro všechny `source = "shared"` textury.

```rust
#[derive(Resource, Default)]
pub struct TextureRegistry {
    loaded_textures: HashMap<String, Handle<Image>>,
}

impl TextureRegistry {
    pub fn request_texture(&mut self, name: &str, asset_server: &AssetServer) -> Handle<Image> {
        if let Some(handle) = self.loaded_textures.get(name) {
            return handle.clone();
        }
      
        // Předpoklad: Všechny sdílené textury jsou ve stream/textures/
        let path = format!("stream/textures/{}.dds", name); 
        let handle: Handle<Image> = asset_server.load(path);
        self.loaded_textures.insert(name.to_string(), handle.clone());
        handle
    }
}
```

---

## 7. Shrnutí Workflow pro Implementaci (Checklist)

1. [ ] **TOML Asset Loader:** Napsat `AssetLoader` pro `DrawableManifest`, aby mohl načítat `.toml` soubory pomocí Bevy `AssetServer`.
2. [ ] **Texture Registry:** Vytvořit `TextureRegistry` Resource.
3. [ ] **Custom Materials:**
    * Implementovat `StandardPbrMaterial` (`standard_pbr.wgsl`).
    * Implementovat `LayeredEnvMaterial` (`layered_env.wgsl`).
    * Zaregistrovat je pomocí `MaterialPlugin`.
4. [ ] **Scene Hooking System:** Napsat systém, který reaguje na spawnované GLTF scény s `DrawableSpawnIntent`.
5. [ ] **Material Swapper:** Logika ve Scene Hookingu, která parsne GLTF nody a vymění materiály za Custom Materials, včetně správného resolvementu textur (Shared vs. Embedded).
6. [ ] **Vertex Colors Sanitization:** Doplnit logiku, která při výměně materiálu zkontroluje a případně doplní atribut `Mesh::ATTRIBUTE_COLOR` nulovými hodnotami.
7. [ ] **Physics Swapper:** Logika ve Scene Hookingu, která najde fyzikální nody (`COL_`), skryje jejich vizuál a vytvoří na nich Collidery z Avian/Rapier na základě geometrie původního meshe a tagu `shape` z TOMLu.
