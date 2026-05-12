
# Technická Specifikace: ADS Animation Decoupling & Scriptable Loading

## 1. Architektura dat (ADM v6 + ADS_ANIM)

Místo jednoho souboru budeme používat dva typy assetů:

- **`.adm` (Geometry Asset):** Obsahuje mesh, uzel kostry (rest pose), skinning weights a materiály. Neobsahuje klipy.
- **`.ads_anim` (Animation Set):** Samostatný binární soubor obsahující pouze animační data (klipy, notifies, blend spacy). Uložiště: `assets/animations/`.
- **Párování:** Runtime provádí "Late Binding" – spojí track z animace s kostí modelu na základě shody názvu (např. `DEF_spine_01`).

## 2. Blender Toolkit: Automatizace a Export (Python)

### A. Batch Mixamo & Animation Importer

Vytvořte operátor `BEVY_OT_BatchMixamoImport`:

1. Uživatel vybere složku s FBX soubory.
2. Skript naimportuje model, převede kosti na `DEF_` prefixy a automaticky vyčistí redundantní kosti.
3. Proiteruje ostatní FBX, extrahuje animace a uloží je do rigu jako pojmenované Actions.

### B. IK Bone Automation

Vytvořte operátor `BEVY_OT_CreateIkBone`:

1. Animátor vybere kost (např. `DEF_hand_r`).
2. Skript vytvoří `IK_hand_r`, nastaví ji jako non-deform, unparented a přidá k původní kosti `Inverse Kinematics` constraint.
3. Exporter automaticky označí kosti s prefixem `IK_` jako `AdsNodeKind::IkTarget`.

### C. Nový Exporter: "Animation Library"

Upravte `adm_export.py` tak, aby umožňoval:

1. **Export Modelu:** Zapíše pouze geometrii do `.adm`.
2. **Export Animací:** Zapíše vybrané Actions do `.ads_anim`.
   - **Důležité:** Transformace se ukládají jako **delta vůči rest-pose**, aby animace postavy s delšíma nohama fungovala i na postavě s kratšíma nohama (Rotation-only retargeting).

## 3. Lua API pro modery a gameplay

Implementujte následující funkce do Lua sandboxu:

### Správa assetů

- `Engine.RequestAnimSet(path: string)` – asynchronně načte knihovnu animací z `assets/animations/`.
- `Engine.HasAnimSetLoaded(path: string) -> bool` – vrací stav načtení.

### Aplikace na entitu

- `World.ApplyAnimSet(handle: u64, path: string)` – "nasadí" sadu animací na konkrétní entitu. Od tohoto momentu entita reaguje na klipy z této knihovny.
- `World.PlayAnimation(handle: u64, clip_name: string, loop?: bool, speed?: f32, blend_time?: f32)` – vyhledá klip v aktuálně aplikovaných sadách a spustí jej.

*Příklad použití v Lua:*

```lua
-- Načtení sady (např. v inicializaci resource)
Engine.RequestAnimSet("animations/human_combat")

-- Aplikace na spawnovaného peda
local ped = World.SpawnNetworkedObject("models/civilian_01.adm", pos, rot)
if Engine.HasAnimSetLoaded("animations/human_combat") then
    World.ApplyAnimSet(ped, "animations/human_combat")
    World.PlayAnimation(ped, "idle_aggressive", true)
end
```

## 4. Runtime Implementace (Rust - core_drawable)

### A. Asset Loader pro `.ads_anim`

- Implementujte `AdsAnimLoader`, který parsuje binární tracky a klipy.
- Vytvořte `AnimationSet` asset, který lze sdílet mezi více entitami.

### B. Late Binding System

- Upravte `apply_adm_animations`, aby při přehrávání animace nehledal data uvnitř `AdmScene`, ale v komponentě `AttachedAnimSets`, která drží reference na načtené `.ads_anim` soubory.

### C. Pokročilé funkce

1. **Blend Spaces:** Evaluace IDW vah pro míchání více klipů z knihovny.
2. **Root Motion:** Extrakce pohybu z `DEF_root` tracku v `.ads_anim` a aplikace na Bevy `Transform`.
3. **IK Solver:** Pokud entita má aktivní IK target (`IK_` kost), runtime spustí Two-Bone IK solver pro korekci postoje (např. nohy na schodech).

## 5. Akceptační kritéria

- Moderská workflow: Uživatel vloží nový `.ads_anim` do složky a pomocí jednoduchého Lua skriptu jej aplikuje na existující postavu ve hře bez rekompilace čehokoliv.
- Blender: Export animace bez meshe proběhne bez chyb.
- Paměť: Deset různých modelů postav sdílejících jednu knihovnu animací zabírá v paměti pouze jednu instanci animačních dat.

---

### Proč je to ideální pro modery:

Modeři nebudou muset řešit tvůj exportní formát pro modely, pokud budou chtít jen přidat nové tance nebo bojové pohyby. Stačí jim tvůj Blender Toolkit, kde vyberou "Export Animation Set", a jejich soubor pak kdokoli na serveru načte přes Lua. **Tohle je přesně ta "FiveM intuitivnost", kterou stavíme.**
