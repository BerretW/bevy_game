
# Apparatus Drawable System (ADS) – Technical Design Document

## 1. Úvod a Filosofie

**Apparatus Drawable** (dále jen "Drawable") je základní stavební blok pro reprezentaci herních objektů (assets) v enginu. Namísto tradičního přístupu (načtení prostého 3D modelu) je Drawable **data-driven kontejner**, který sdružuje:

1. **Vizuální data:** Hierarchii meshů a vertex data (obsahující masky).
2. **Fyzikální data:** Zjednodušenou geometrii a parametry (hmotnost, tření, statický/dynamický stav) pro fyzikální engine (např. Rapier/Avian).
3. **Materiálovou logiku:** Mapování jmen materiálů na konkrétní "Uber-Shadery" s definovanými parametry (včetně počasí) a způsobem získávání textur (sdílené vs. přibalené).

Tento systém umožňuje oddělit práci grafiků/designerů v Blenderu od herní logiky v Rustu a zajišťuje masivní úsporu VRAM pomocí globálních registrů textur.

---

## 2. Anatomie Drawable

Z hlediska distribuce se jeden Drawable skládá ze dvou souborů stejného jména:

1. `[asset_name].glb` – **Binární kontejner (Tělo).** Obsahuje surovou geometrii, hierarchii uzlů (nodes) a případně vložené (embedded) obrazové soubory.
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
        albedo = { name = "barrel_01_d.dds", source = "embedded" },
        mrao = { name = "rust_generic_mrao.dds", source = "shared" },
        normal = { name = "rust_generic_n.dds", source = "shared" },
        palette = { name = "default_lut.dds", source = "shared" },
        snow = { name = "snow_01_d.dds", source = "shared" }
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

Zde je návrh struktur pro deserializaci:

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

// Zjednodušený přístup pro parametry (lze řešit i přes untyped serde_json::Value pro větší flexibilitu)
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
2. Z `MeshMaterial3d` přečteme původní (placeholder) materiál z GLTF a získáme jeho jméno (vyžaduje přístup k Gltf assetu nebo iteraci `Assets<StandardMaterial>`).
3. Najdeme toto jméno v sekci `[materials]` v Manifestu.
4. **Získání Textur:** Systém projde sloty (albedo, normal...).
   * Pokud `source == "shared"`: Vyžádá Handle z globálního `TextureRegistry`.
   * Pokud `source == "embedded"`: Najde texturu podle jména přímo uvnitř načtené struktury Bevy `Gltf` (`gltf.named_textures.get(&tex_info.name)`).
5. **Aplikace Shaderu:** Na základě hodnoty `template` se vytvoří instance příslušného Custom Shaderu (např. `StandardPbrMaterial` nebo `LayeredEnvMaterial`), naplní se Handles textur a parametry.
6. Původní `MeshMaterial3d<StandardMaterial>` se nahradí za `MeshMaterial3d<StandardPbrMaterial>`.

#### 3.B. Fyzikální uzly (`type = "COLLISION"`)

Pokud uzel odpovídá `EntityDef::COLLISION`:

1. **Odstranění Vizuálu:** Odstraníme komponenty `Mesh3d` a `MeshMaterial3d`, případně nastavíme `Visibility::Hidden`. Fyzika nesmí být vidět.
2. **Generování Collideru:** Podle `shape` vygenerujeme Collider.
   * Pro tvary jako `BOX` nebo `SPHERE` můžeme číst rozměry z (nyní odstraněného) meshe, nebo (lépe) je vypočítat z Bounding Boxu mesh geometrie.
   * Pro `CONVEX` získáme pozice vertexů z meshe a vytvoříme `Collider::convex_hull(vertices)`.
3. **Nastavení RigidBody:**
   * Vložíme `RigidBody::Static` (pokud `is_static == true`), jinak `RigidBody::Dynamic`.
   * Nastavíme hmotnost: `ColliderMassProperties::Mass(def.mass)`.
   * Nastavíme materiál: `Friction::new(def.friction)` a `Restitution::new(def.restitution)`.
4. *(Volitelně)* Pokud má kontejnerový kořen také mít RigidBody, zvážíme spojení colliderů na kořenovou entitu (Collider hierarchies).

---

## 5. Globální Texture Registry (VRAM optimalizace)

Aby se zabránilo vícenásobnému načítání stejných `.dds` textur, engine udržuje centrální resource.

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
      
        // Předpoklad: Všechny sdílené textury jsou uloženy ve stream/textures/
        let path = format!("stream/textures/{}.dds", name); 
        let handle: Handle<Image> = asset_server.load(path);
        self.loaded_textures.insert(name.to_string(), handle.clone());
        handle
    }
}
```

---

## 6. Integrace Počasí a Masek

1. **Vertex Colors (Masky):** GLB obsahuje Vertex Colors. Bevy je naimportuje do atributu `Mesh::ATTRIBUTE_COLOR`. Náš WGSL shader tyto hodnoty v `FragmentInput` (`in.color`) automaticky uvidí a využije je jako mísící faktory (R, G, B, A).
2. **Globální Počasí:** Pro minimalizaci aktualizací se v enginu založí `GlobalEnvironmentBuffer` resource, který se do shaderu posílá jako BindGroup. To umožní okamžitou změnu počasí (deště/sněhu) pro všechny Apparatus Drawables najednou, bez nutnosti měnit Uniformy jednotlivých materiálů.

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
6. [ ] **Physics Swapper:** Logika ve Scene Hookingu, která najde `COL_` nody, skryje je a vytvoří na nich Collidery z Avian/Rapier na základě geometrie meshe.
