

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

---

## 8. Skeleton & Animace

### 8.1. Prefix Konvence Uzlů (ADS Bone Standard)

Všechny kosti/uzly v Blenderu pojmenovávejte podle prefixu, aby engine mohl automaticky rozlišit jejich roli:

| Prefix | Typ | Popis |
|--------|-----|-------|
| `DEF_` | Deformační kost | Deformuje mesh (skinning). Nejpočetnější skupina. |
| `IK_` | IK target / Pole | Cíl IK solveru, engine si ho zaregistruje jako `AdsNodeKind::IkTarget`. |
| `SOC_` | Socket | Bod pro attachment jiných objektů (zbraně, příslušenství, efekty). Viditelnost = Hidden. |
| `MEC_` | Mechanická kost | Speciální kost pro procedurální mechaniky (kola, klouby…). |
| *(bez prefixu)* | Standard | Organizační uzly, root bones, pivot helpers. |

Uzly jsou klasifikovány při hookování scény funkcí `classify_ads_node_name()` v `core_drawable/src/hook.rs` a dostávají komponentu `AdsNodeKind`.

---

### 8.2. Socket Systém

Sockety (`SOC_*`) slouží jako pojmenované přichytávací body. Engine je sbírá do komponenty `AdsSocketMap` na root entitě modelu.

```
AdsSocketMap {
    "SOC_R_Hand_Weapon" -> Entity(42)
    "SOC_Spine_Backpack" -> Entity(43)
    …
}
```

#### Použití z Lua

```lua
-- Přichyť zbraň k pravé ruce postavy
World.Attach(weapon_handle, "SOC_Origin", player_handle, "SOC_R_Hand_Weapon")

-- Odepni zbraň (zachová world-space transform)
World.Detach(weapon_handle)

-- Přečti world-space pozici socketu (např. pro spawn projectilu)
local muzzle = World.GetSocketTransform(weapon_handle, "SOC_Muzzle")
if muzzle then
    print(muzzle.pos.x, muzzle.pos.y, muzzle.pos.z)
    -- muzzle.rot je kvaternion {x, y, z, w}
end
```

**Mechanika `World.Attach`:**
Engine vypočítá takový `Transform` child entity, aby se socket child entity (`child_socket`) překryl se socketem parent entity (`parent_socket`). Child se stane potomkem parent root entity (`ChildOf`).

---

### 8.3. Formát ADM v3 (binární animace)

ADM (*Apparatus Drawable Mesh*) je binární formát `.adm` vhodný pro exporty z Blenderu. Verze **3** přidává nepovinnou animační sekci za sekcí textur a k trackům ukládá i masky částí těla.

#### Hlavička souboru

```
[4b]  Magic: "ADM\0"
[4b]  Version: u32 (1 = bez animací, 2 = animace bez track flags, 3 = animace s track flags)
[4b]  mesh_count: u32
[4b]  node_count: u32
[4b]  has_embedded_textures: u32 (0 nebo 1)
```

#### Animační sekce (v2/v3)

```
[4b]  clip_count: u32
for každý clip:
  [str]  name           (u16 délka + utf-8 bajty)
  [4b]   duration: f32  (sekundy)
  [4b]   track_count: u32
  for každý track:
    [str]  node_name    (jméno Blender objektu / uzlu)
        [4b]   flags        (bitmask části těla pro filtering)
    [4b]   key_count: u32
    for každý keyframe:
      [4b]  time: f32       (sekundy od začátku)
      [12b] pos: vec3 f32   (Bevy souřadnice)
      [16b] rot: quat xyzw  (Bevy kvaternion)
      [12b] scale: vec3 f32
```

> **Poznámka souřadnic:** Exporter automaticky konvertuje Blender Z-up systém na Bevy Y-up pomocí matice `_C @ mat @ _C_INV` při každém keyframu.

#### Track flagy

Engine filtruje animaci na straně runtime podle bitmasky. To umožňuje z jednoho clipu přehrát jen horní polovinu těla, jen pravou ruku nebo jen spodní tělo bez přepisu framů.

- `1` = všechno / bez filtru
- `14` = pravá horní končetina
- `112` = levá horní končetina
- `8064` = spodní část těla
- `24576` = horní část těla / torso

Masku předávej přes `World.PlayAnimation(handle, name, looping?, speed?, blend_time?, flags?)`.

#### Atributy skinningu v mesh datech

ADM mesh může volitelně obsahovat skinning data:

```
ATTR_JOINT_INDICES (bit 7): Vec<[u32; 4]>  -- 4 indexy kostí na vertex
ATTR_JOINT_WEIGHTS (bit 8): Vec<[f32; 4]>  -- 4 váhy na vertex
```

---

### 8.4. Export animací z Blenderu

Toolkit dnes exportuje animace přímo z armatury: při nalezení `ARMATURE` objektu zapíše i bone uzly a klipy z NLA / aktivní `Action`. Každý bone track dostane automaticky `flags` podle názvu kosti, aby engine mohl filtrovat jen požadovanou část těla.
Pokud má mesh vertex groups navázané na kostry stejné armatury, exporter zároveň zapíše i `ATTR_JOINT_INDICES` a `ATTR_JOINT_WEIGHTS`; runtime z nich při načtení vytvoří Bevy `SkinnedMesh` a inverse bind poses.

**Workflow:**
1. Vyber armaturu a deformované meshe.
2. Pokud chceš více clipů, dej každou `Action` do samostatného NLA stripu.
3. Spusť **Apparatus Drawable Toolkit → Export ADM (Bones + Clips)**.
4. Exporter zapíše skeleton uzly, bone tracky, jejich masky a všechny nalezené clipy do jednoho `.adm` souboru.

**Konstantní tracky jsou automaticky vynechány** a clip se pojmenuje podle názvu NLA stripu nebo `Action`.

---

### 8.5. Runtime Přehrávání Animací

Engine přehrává ADM animace v systému `apply_adm_animations` (`core_drawable/src/adm.rs`), který běží každý frame v `PostUpdate`.

#### Selektor clipu

Lua funkce `World.PlayAnimation` přijímá selektor animace:

| Formát selektoru | Výsledek |
|-----------------|---------|
| `"0"` nebo `"clip:0"` nebo `"anim:0"` | Clip na indexu 0 |
| `"scene"` | Clip pojmenovaný `"scene"` |
| *(libovolné jméno)* | Clip s daným názvem |

```lua
-- Přehrání ADM animace — jménem
World.PlayAnimation(obj_handle, "scene")

-- Přehrání ADM animace — indexem
World.PlayAnimation(obj_handle, "clip:0", true, 1.0)

-- Zastavení
World.StopAnimation(obj_handle)
```

#### Podpisové tvary `World.PlayAnimation`

```lua
World.PlayAnimation(handle, name)
World.PlayAnimation(handle, name, blend_time)          -- číslo jako 3. arg = blend_time
World.PlayAnimation(handle, name, looping, speed, blend_time)
```

#### Vnitřní logika

Každá root entita ADM modelu nese komponentu `AdmAnimationPlayback`, která sleduje aktuálně přehrávaný clip a herní čas uvnitř klipu. Systém na každý frame:

1. Přečte `AnimationState.current` (jméno/selektor).
2. Vyhledá clip v `AdmScene.animations` pomocí `resolve_clip_index`.
3. Pokud se clip změnil, resetuje čas na 0.
4. Posune čas o `delta_secs * speed`; při `looping=true` použije `rem_euclid(duration)`.
5. Pro každý track v clipu vyhledá entitu v `AdmNodeEntityMap` a samplujem transform pomocí **lineární interpolace** (pos/scale) a **slerp** (rotation).

#### GLTF animace

Pro GLB/GLTF modely engine používá Bevy `AnimationPlayer` a `AnimationGraph`. Systém `apply_lua_animation_state` v `host_client/src/gameplay.rs` mapuje Lua `AnimationState` na Bevy animační graf:

- Selektor `"0"` nebo `"clip:0"` = GLTF animace na indexu 0.
- Engine cachuje vytvořené `AnimationGraph` handly v `LuaAnimationGraphCache`, aby je nepřevytvářel každý frame.

---

### 8.6. LOD Skeletal Pruning

Při LOD úrovni ≥ 2 (vzdálené objekty) engine automaticky **skrývá detailní DEF_ kosti**, které jsou pro vzdálené modely nevýznamné:

**Skryté skupiny DEF_ kostí na LOD2+:**
- Prsty rukou (`finger`, `thumb`, `index`, `middle`, `ring`, `pinky`)
- Obličejové kosti (`jaw`, `lip`, `cheek`, `brow`, `eyelid`, `tongue`, `nose`, `ear`, `face`, `moustache`, `beard`)

Kosti se **nemaží** — pouze dostanou `Visibility::Hidden`. Při přechodu zpět na LOD0/LOD1 se automaticky obnoví na `Visibility::Inherited`.

Systém `apply_skeletal_pruning` běží v `PostUpdate` po `update_lod_visibility`.

---

### 8.7. Blender Workflow pro Animovaný ADM Asset

```
1. Vymodeluj a oretopologizuj postavu / objekt
2. Vytvoř armature, pojmenuj kosti dle prefixové konvence:
     DEF_spine_01, DEF_thigh_l, IK_foot_l, SOC_R_Hand_Weapon, …
3. Nastav skinning (Parent → With Automatic Weights nebo ručně)
4. Vytvoř akce v Action Editoru nebo importuj Mixamo animace přes toolkitem
5. Pokud chceš více clipů, ulož každou akci do vlastního NLA stripu
6. Apparatus Drawable Toolkit → Export ADM (Bones + Clips)
7. Výsledek: [model].adm (v3, obsahuje skeleton, skinning data, clipy i track flags)
8. V Lua: World.PlayAnimation(handle, "idle") nebo World.PlayAnimation(handle, "clip:0", true, 1.0)
```

**Doporučení pro více clipů:**
Použij NLA editor — každá NLA akce na separátním pásmu. Toolkit pak při exportu vloží každý strip jako samostatný clip do jednoho `.adm` souboru.

---

## 9. Návod: Mixamo Asset → ADM

Mixamo (mixamo.com) nabízí postavy a animace ve formátu FBX nebo GLB. Jejich kosterní soustava neodpovídá ADS prefix konvenci, takže je potřeba několik kroků přípravy v Blenderu.

### 9.1. Stažení z Mixamo

1. Zvol postavu a animaci na [mixamo.com](https://www.mixamo.com).
2. Stáhni **FBX for Unity** (formát FBX, bez skin → spolu s mesh; nebo T-Pose pro základní model).
3. Pro každou animaci zvlášť: vyber animaci → **Download** → FBX for Unity (tentokrát lze stáhnout i bez skin, pokud model máš).

> Doporučený formát: **FBX Binary** (ne Collada). Mixamo FBX je kompatibilní s Blender 4.x importem.

---

### 9.2. Import do Blenderu

1. Otevři nový Blender projekt.
2. **File → Import → FBX (.fbx)** → vyber stažený soubor.
3. Nastavení importu:
   - ✅ `Automatic Bone Orientation`
   - ✅ `Force Connect Children` (volitelné, záleží na modelu)
4. Po importu budeš mít `Armature` objekt + `Body` mesh.

---

### 9.3. Retopologie kostry do ADS konvence

Mixamo kosti se jmenují např. `mixamorig:Hips`, `mixamorig:Spine`, `mixamorig:RightHand`. Je potřeba je přejmenovat.

**Možnosti:**

**A) Toolkit operátor (doporučeno)**

V Blenderu spusť **Appartus Drawable Toolkit → Auto Rename Mixamo Rig**. Operátor přejmenuje kosti na `DEF_` konvenci a aktualizuje i odpovídající vertex groups.

**B) Skript přejmenování**

V Blender Python konzoli spusť:

```python
import bpy, re

arm = bpy.context.object  # vyber armature v Object Mode
for bone in arm.data.bones:
    name = bone.name
    # Odstraň mixamorig: prefix
    name = re.sub(r'^mixamorig:', '', name)
    # Přidej DEF_ prefix (všechny Mixamo kosti deformují mesh)
    bone.name = 'DEF_' + name

print("Přejmenováno:", len(arm.data.bones), "kostí")
```

**C) Ruční přejmenování** — vhodné jen pro malý počet kostí.

**Sockety přidej ručně:** V `Edit Mode` armatury přidej nové kosti (Add → Bone) na požadovaná místa (pravá ruka, záda, hlava) a pojmenuj je `SOC_R_Hand_Weapon`, `SOC_Spine_Backpack` atd. Nastav jim `Deform = OFF` v Bone Properties.

---

### 9.4. Příprava animací z Mixamo

Každá Mixamo animace je samostatný FBX. Pro každou animaci:

1. **File → Import → FBX** nebo rovnou **Import Mixamo Animations** v toolkitu.
2. Toolkit automaticky najde importovanou armaturu, přejmenuje ji na ADS konvenci a přenese action do cílové armatury.
3. Pro více animací importuj více FBX souborů najednou a nech je přidat jako NLA stripy na stejný rig.
4. Pokud importuješ ručně, přesuň akce do NLA editoru a udržuj stejný naming jako exportované clipy.

---

### 9.5. Přizpůsobení materiálů

Mixamo model má obvykle základní `Lambert` nebo `Phong` materiál bez PBR textur.

1. V `Material Properties` každého materiálu:
   - Přejmenuj materiál na logické jméno (např. `Body_Mat`, `Hair_Mat`).
   - Přiřaď albedo texturu (pokud Mixamo postava texturu obsahuje).
2. Vytvoř `.drawable` TOML manifest vedle `.adm` souboru — viz sekce 3.1.
3. Nastav `template = "standard_pbr"` a dodej potřebné textury.

Pokud Mixamo postava nemá vlastní textury, použij placeholder nebo sdílené textury z registru.

---

### 9.6. Export ADM

1. Vyber armaturu a všechny mesh objekty postavy (**A** pro výběr všeho viditelného).
2. V případě více clipů dej každou akci do vlastního NLA stripu.
3. **Apparatus Drawable Toolkit panel → Export ADM (Bones + Clips)** → vyber výstupní cestu.
4. Výsledek: `postava.adm` (v3, skeleton + více clipů + track flags v jednom souboru).

Pokud chceš přesto exportovat po jednotlivých klipech, nech aktivní jen jednu `Action` nebo jeden NLA strip a export opakuj.

---

### 9.7. Integrace do hry (Lua)

```lua
-- server/init.lua nebo resource server script

RegisterEvent("onPlayerSpawn", function(player_id)
    -- Spawn základního modelu postavy
    local handle = World.SpawnNetworkedObject("models/postava.adm", {0, 0, 0}, {0, 0, 0})

    -- Spusť idle animaci
    World.PlayAnimation(handle, "scene", true, 1.0)
end)
```

Pro systém přepínání animací (idle/walk/run) je potřeba mít buď:
- **Jeden ADM s více clipy** (export po jednom a pojmenovat soubory zvlášť, nebo rozšíření exporteru).
- **Více ADM souborů** — spawn jiného modelu při změně stavu (jednodušší, ale méně efektivní).

```lua
-- Příklad přepínání stavů (client script)
local current_anim = nil

RegisterEvent("input:state", function(state)
    local moving = state.move.x ~= 0 or state.move.y ~= 0

    local anim = moving and "walk" or "idle"
    if anim ~= current_anim then
        current_anim = anim
        -- Předpokládá jeden ADM soubor pojmenovaný podle animace
        TriggerServerEvent("player:setAnim", anim)
    end
end)
```

---

### 9.8. Časté problémy s Mixamo

| Problém | Příčina | Řešení |
|---------|---------|--------|
| Model po importu je otočený o 90° | FBX Z-up vs Blender Z-up | Při importu FBX nastav `Forward = -Z`, `Up = Y`; nebo po importu Ctrl+A → Apply All Transforms |
| Kosti mají nulové váhy | Skin weights se neimportovaly | Vyber mesh → Properties → Vertex Groups → zkontroluj skupiny; re-skin přes `Armature → With Automatic Weights` |
| Animace se přehrává jen na jednom framu | Action není přiřazena správné armatuře | V Action Editoru zkontroluj že aktivní armatura má správnou Action |
| Sockety nejsou v `AdsSocketMap` | Socket kost nemá prefix `SOC_` | Přejmenuj kost na `SOC_NazevSocketu` v Blender Edit Mode |
| Přejmenované kosti rozbily skin weights | Vertex Groups mají starý název | Spusť přejmenování ve skriptu výše — Blender automaticky přejmenuje i Vertex Groups pokud je Bone propojený s Mesh modifierem |
