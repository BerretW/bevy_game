# NPC Ownership — Phase B: Client-Side Simulation

## Cíl

Přesunout fyzikální simulaci NPC ze serveru na vlastnícího klienta, aby NPC reálně
reagovala na terén, kolize a překážky v herním světě. Server zůstává autoritativní —
přijímá pozice od ownera a replikuje je ostatním.

## Vybraný přístup

Pro cílový směr ala REDM a stovky NPC nevolíme plnou replikaci interního `NpcAgent` stavu.
Volíme hybridní model:

- Server řídí vysokou vrstvu: populaci, spawn/despawn, scenario body, schedules, relationship groups, high-level goal a ownership lease.
- Owning klient řídí nízkou vrstvu: navmesh path following, terrain snap, obstacle avoidance, lokální kolize, animaci a krátkodobé steering korekce.
- Ostatní klienti dostávají jen výsledný transform a kompaktní replikační stav mozku NPC, ne interní runtime timery.
- NPC běží v AI LOD vrstvách: full simulation blízko hráče, reduced simulation ve střední vzdálenosti, background/sleep mimo zájem.

Tohle je důležité pro škálování:

- Stovky NPC neutáhneme, pokud budeme všem replikovat `rng_state`, `current_path`, `waypoint_index`, lokální steering timery a podobný runtime šum.
- Dynamiku ala REDM dostaneme přes scenario/task systém a ownership handoff, ne přes jeden globální server tick s plnou fyzikou pro každé NPC.
- Navmesh corridor a krátkodobé avoidance mají zůstat lokální na ownerovi; server má validovat výsledky, ne deterministicky přepočítávat každý krok.

---

## Aktuální stav

- `NpcOwner(Option<u64>)` — replikovaný komponent, říká kdo NPC vlastní
- `NpcOwnershipLease` — server-side lease metadata pro ownership handoff
- `assign_npc_owners` — server každé 2 s přiřadí ownera s acquire/release radiusem a handoff cooldownem
- `tick_npc_agents` — server simuluje pohyb (waypoints, bez fyziky), freeze při `None`
- Klient zná svého ownera, ale simulaci NEBĚŽÍ — jen přijímá `NetTransform`
- `SpawnNetworkedNpc` teď vkládá `NpcOwner::default()`, takže ownership systém pokrývá i nově spawnované NPC

### Hotovo z ownership handoff foundation

- hysteresis přes `NPC_OWNERSHIP_RADIUS` a `NPC_OWNERSHIP_RELEASE_RADIUS`
- handoff cooldown přes `NPC_OWNERSHIP_HANDOFF_COOLDOWN`
- přepnutí ownera jen pokud nový kandidát překoná stávajícího o minimální rezervu
- `NpcTransformUpdate` Client→Server cesta je zavedená a server validuje `NpcOwner` před aplikací transformu
- klient bootstrapuje lokální `NpcAgent` jen pro owned NPC, takže remote NPC se lokálně nesimulují
- client-owned NPC ignorují replicated `NetTransform` writeback na owning klientu a místo toho posílají svůj transform serveru
- server drží `NpcLastClientUpdate` a po `NPC_CLIENT_UPDATE_TIMEOUT_SECS` automaticky fallbackne zpět na server simulaci, pokud owner umlkne
- owning klient má základní terrain snap pro NPC přes raycast dolů, takže client-owned NPC drží terénní Y místo čistého plovoucího transformu
- nový owner bootstrapuje lokální `NpcAgent` přímo z aktuálního `ReplicatedNpcBrain`, takže handoff nezačíná mezikrokem v `Idle`
- coarse steering continuity je teď přenášená přes `ReplicatedNpcSteering` / rozšířený `NpcTransformUpdate` (`home`, `wander_target`, `wander_timer`, `orbit_angle`, `patrol_to_target`, `current_path`, `waypoint_index`, `map_id`, `last_nav_target`)
- chase/follow continuity je nově rozšířená i o lehkou entity-target steering cache (`entity_target_position`, `entity_target_velocity`, `formation_offset`), takže handoff lépe drží pursuit lead a escort/follow offset bez replikace plného avoidance runtime

### Ještě chybí

- snapshot / resume sync při skutečném client-side handoffu
- avoidance cache a další jemná obstacle steering metadata zatím stále nejsou součástí handoff snapshotu
- AI LOD foundation je nově v runtime přes `NpcAiLodState` + distance thresholds (`full`, `reduced`, `background`), ale zatím bez density budgetů per tile/zone a bez scheduler napojení
- ownership runtime teď navíc používá první per-player density budgety (`full_budget_per_player`, `reduced_budget_per_player`): přebytek v `Full` se demotuje do `Reduced` a přebytek nad celkový aktivní budget padá do `Background`
- budgety už nejsou čistě per-player: `assign_npc_owners` teď navíc aplikuje i coarse zone budget (`zone_size`, `full_budget_per_zone`, `reduced_budget_per_zone`), takže přetížené zóny padají do `Reduced/Background` i když je nejbližší hráč stále v dosahu
- owned klient teď drží i lehkou obstacle avoidance cache (`avoidance_offset`, `avoidance_timer`) z forward raycastu; cache se snapshotuje přes `ReplicatedNpcSteering` / `NpcTransformUpdate`, takže handoff nemusí po obstacle kontaktu restartovat z čisté corridor follow trajektorie
- obstacle avoidance a kvalitnější slope/terrain locomotion pro owned NPC

---

## Phase B — Návrh

### 1. Nový network message: `NpcTransformUpdate`

```rust
// core_net/src/protocol.rs
#[derive(Message, Clone, Serialize, Deserialize)]
pub struct NpcTransformUpdate {
    pub handle: u64,
    pub translation: Vec3,
    pub rotation: Quat,
}
```

Směr: **Client → Server** (pouze owning client posílá)

Registrace v `net_plugin.rs`:
```rust
app.add_channel::<NpcTransformUpdate>(ChannelSettings {
    mode: ChannelMode::UnorderedUnreliable, // pozice = unreliable, stačí nejnovější
    ..default()
});
```

---

### 2. Server: přijímá a aplikuje update

```rust
// core_net/src/sim.rs nebo nový npc_authority.rs
fn receive_npc_transform_updates(
    mut reader: EventReader<MessageEvent<NpcTransformUpdate>>,
    world_state: Res<LuaWorldState>,
    mut npcs: Query<(&mut Transform, &mut NetTransform, &NpcOwner)>,
) {
    for event in reader.read() {
        let sender_id = event.context.client_id;
        let msg = &event.message;

        if let Some(entity) = world_state.entity_for(msg.handle) {
            if let Ok((mut tf, mut net_tf, owner)) = npcs.get_mut(entity) {
                // Autoritativnost: přijmeme jen od skutečného ownera
                if owner.0 != Some(sender_id) {
                    continue;
                }
                tf.translation = msg.translation;
                tf.rotation = msg.rotation;
                net_tf.translation = msg.translation;
                net_tf.rotation = msg.rotation;
            }
        }
    }
}
```

Server pak přestane volat `tick_npc_agents` pro NPC s ownerem — nebo ho ponechá
jako fallback (server simulace jako záloha, pokud klient přestane posílat).

---

### 3. Klient: detekuje owned NPCs a spouští lokální simulaci

```rust
// host_client/src/npc_sim.rs (nový soubor)

fn tick_owned_npc_agents(
    time: Res<Time<Fixed>>,
    local_id: Option<Res<LocalClientId>>,
    world_state: Res<LuaWorldState>,
    mut npcs: Query<(
        &EntityHandle,
        &mut Transform,
        &NpcOwner,
        &mut NpcAgent,      // musí být replikován nebo rekonstruován z Lua state
    )>,
    spatial: SpatialQuery,         // Avian — terén, kolize
    mut writer: MessageSender<NpcTransformUpdate>,
) {
    let Some(my_id) = local_id.map(|r| r.0) else { return };

    for (handle, mut tf, owner, mut agent) in &mut npcs {
        if owner.0 != Some(my_id) { continue; }

        // Pohybová logika (kopie tick_npc_agents ze serveru)
        simulate_npc_movement(&mut tf, &mut agent, &spatial, time.delta_secs());

        // Pošli pozici serveru
        writer.send(NpcTransformUpdate {
            handle: handle.0,
            translation: tf.translation,
            rotation: tf.rotation,
        });
    }
}
```

---

### 4. Replikace `NpcAgent` na klienta

Aby klient věděl, kam NPC míří, musí znát `NpcAgent.goal`. Možnosti:

**A) Replikovat `NpcAgent`** — jednodušší, ale posílá zbytečná data (rng_state, timery).
```rust
app.register_component::<NpcAgent>(); // core_net/net_plugin.rs
```

**B) Replikovat jen `NpcMoveGoal`** — čistší, nový komponent `ReplicatedNpcGoal(NpcMoveGoal)`.
Klient drží vlastní `NpcAgent` lokálně, server mu jen pošle goal update při změně.

Vybraný směr: **varianta B**.

Pro stovky NPC je vhodnější rozdělit stav takto:

- `ReplicatedNpcBrain` nebo `ReplicatedNpcGoal`: high-level intent, scenario, target, stance, combat state, seed.
- Lokální `NpcAgent`: current_path, waypoint index, wander/orbit timery, steering, avoidance, terrain/contact cache.
- `NpcOwner`: ownership lease s hysteresis a handoff cooldownem.

Varianta A může posloužit jen jako krátký bootstrap pro debug, ale není to cílová architektura.

---

### 4.1. Scenario/Task systém místo čistého waypoint AI

Inspirace REDM znamená, že NPC nemají být jen "běž na bod" entity. Server by měl držet scénářovou vrstvu:

- `NpcScenarioId`: např. guard_post, saloon_idle, shopkeep_counter, town_walk_loop.
- `NpcTask`: idle, wander_zone, patrol_route, use_scenario_point, chase_target, flee, investigate, combat.
- `NpcSchedule`: denní/noční změny chování, occupancy scenario bodů, spawn budget per zone.

Owner klient pak z těchto tasků vyrábí konkrétní lokální pohyb:

- scenario point → lokální anchor + facing + anim set
- patrol_route → navmesh corridor mezi route body
- chase_target → periodický repath + avoidance
- wander_zone → výběr reachable targetů na navmeshi místo přímé čáry

Tohle je pružnější než replikovat celý behavior tree nebo interní steering stav.

Aktuální stav:

- `scenario_id` už není jen metadata v `ReplicatedNpcBrain`
- `NpcScenarioRegistry` drží runtime scénářové definice registrované z Lua resources přes `World.NpcRegisterScenario(id, def)`
- `apply_replicated_npc_brain()` mergeuje scenario default params do aktuálního brain kontraktu a při `UseScenarioPoint` převádí scénář na efektivní task/target ještě před překladem do `NpcMoveGoal`
- `NpcScenarioDef` teď umí i `active_from_hour`, `active_until_hour`, `max_occupants` a `lod_priority`
- `NpcScenarioTime` + Lua API `World.NpcSetScenarioTime(hour)` dávají jednoduchý testovací scheduler clock
- `sync_npc_scenario_runtime()` počítá runtime stav scénáře per NPC: `active`, `occupancy_granted`, `occupancy_slot`, `lod_priority`
- `sync_npc_brains_to_agents()` fallbackne na `Idle`, pokud je scénář neaktivní nebo přeobsazený
- `assign_npc_owners()` používá scenario priority + task type + brain archetype při trimování `Full/Reduced/Background` budgetů
- `NpcScenarioClockConfig` + `advance_npc_scenario_time()` posouvají serverovou denní dobu automaticky bez ručního Lua bootstrapu
- `run_npc_population_director()` umí auto-assignnout volná NPC do scénářů s `auto_assign=true`, `required_tags`, `preferred_brain_kind` a `assignment_radius`
- auto-assigned NPC drží `NpcPopulationAssignment`; při neaktivním scénáři nebo odchodu mimo release radius se scénář zase uvolní
- globální chování už není zadrátované jen v Rust defaults: Lua může runtime ladit přes `World.NpcConfigureScenarioClock(opts)`, `World.NpcConfigurePopulationDirector(opts)` a `World.NpcConfigureAiLod(opts)`

---

### 4.2. AI LOD pro stovky NPC

Bez LOD vrstvami nepůjde hustá populace rozumně škálovat.

- LOD0 Full: owner klient, plná lokální simulace, navmesh, avoidance, animace, terrain snap.
- LOD1 Reduced: owner klient nebo server, pomalejší tick, bez jemného avoidance, zjednodušené corridor following.
- LOD2 Background: server-only schedule/scenario state, žádná plná fyzika, jen coarse transform nebo úplný sleep.

Přepínání LOD má řídit server podle vzdálenosti, relevance a density budgetu per tile/zone.

---

### 4.3. Ownership handoff

Ownership nesmí každé dvě sekundy skákat mezi klienty bez pravidel. Potřebujeme:

- hysteresis radius pro převzetí a odevzdání
- handoff cooldown
- snapshot brain state při převodu ownera
- fallback na server simulaci, pokud owner umlkne

To je důležité hlavně pro chase, escort a městské populace kolem více hráčů.

---

### 5. Server: zastavit server-side simulaci pro owned NPC

```rust
// tick_npc_agents — upravit podmínku freeze
if let Some(o) = owner {
    // Zmrazit: buď nikdo není nablízku, nebo owner je klient (klient simuluje)
    // Ponecháme server simulaci jen jako fallback po N sekundách ticha od klienta.
    if o.0.is_none() { continue; }
    // TODO: přidat `NpcLastClientUpdate(Instant)` a fallback na server sim po 5 s
}
```

---

### 6. Terrain-aware pohyb na klientovi

Klient má přístup k Avian `SpatialQuery` a terénu. Rozšíření `simulate_npc_movement`:

```rust
// Raycast dolů pro ground snapping (stejný princip jako terrain_snap_kinematic)
if let Some(hit) = spatial.cast_ray(pos + Vec3::Y * 0.5, Dir3::NEG_Y, 2.0, ...) {
    tf.translation.y = hit.point.y;
}

// Obstacle avoidance: raycast dopředu, pokud hit → obejít (steering)
// NavMesh pathfinding: použít NavMeshSurfaceCache (až bude navmesh hotový)
```

---

## Pořadí implementace

1. [X] `NpcTransformUpdate` message + registrace směru Client→Server
2. [X] `receive_npc_transform_updates` na serveru (s owner validací)
3. [X] bootstrap lokálního `NpcAgent` jen pro owned NPC na klientovi
4. [X] první client-owned NPC loop: owned klient simuluje lokální `NpcAgent` a posílá transform serveru
5. [X] server přestane simulovat NPC s aktivním ownerem (zatím bez fallback timeru)
6. [X] Terrain snapping v klientské simulaci (raycast + Y korekce)
7. [X] goal replication je aktuálně řešená přes `ReplicatedNpcBrain`
8. [ ] Přidat scenario/task vrstvu a AI LOD budgety nad současný brain kontrakt
9. [X] Rozšířit ownership handoff o fallback timer po tichu ownera a napojit ho na client-authoritative NPC transform updates

---

## Závislosti / Předpoklady

- NavMesh (Phase 3.6) — pro správný pathfinding kolem překážek
- Replikovaný high-level brain goal musí být `Serialize + Deserialize`
- Lokální `NpcAgent` nemusí být plně replikovaný
- lightyear channel pro Client→Server unreliable messages

---

## Delivery plán do cíle: populace pro města i divočinu

**Cíl:** NPC AI systém schopný dlouhodobě obsluhovat hustou populaci ve městech i rozptýlenou populaci v divočině bez toho, aby Rust core držel herně specifické chování natvrdo.

### Co už máme jako základ

- runtime brain registry + fallback `core/human`
- high-level replikační kontrakt `ReplicatedNpcBrain`
- ownership handoff s hysteresis, cooldownem a fallbackem po tichu ownera
- client-owned locomotion loop + server fallback
- coarse steering continuity snapshot
- lehký obstacle avoidance cache snapshot
- AI LOD foundation (`Full` / `Reduced` / `Background`) + per-player a coarse per-zone budgety
- model viewer NPC brain debug režim

To už stačí jako základ lokomotion/ownership vrstvy. Do cílového stavu ale ještě chybí nadstavba pro populaci, obsah a provozní škálování.

### Fáze A — Dokončit lokomotion core

Smysl: stabilní low-level pohyb, který neunaví klienta ani server a neshazuje chase/escort při handoffu.

- dotáhnout obstacle avoidance z lehkého sidestep impulse na robustnější steering vrstvu s krátkodobou pamětí hitů
- oddělit `Full` a `Reduced` locomotion profile: reduced bez jemných bočních korekcí, delší repath interval, méně časté target refresh
- doplnit terrain/slope heuristiky pro NPC mimo schody: měkký ground snap, jednoduché slope limity, prevence jitteru na hranách colliderů
- sjednotit pohybový krok mezi client runtime a model viewer debug helperem, aby viewer nelhal o chování

Exit kritéria:

- chase/follow/escort handoff nepůsobí reset corridoru ani po obstacle kontaktu
- reduced NPC zůstávají pohybově stabilní bez jemného avoidance
- viewer a in-game debug dávají podobné chování pro stejné brain/task vstupy

### Fáze B — Přesunout LOD z coarse gridu na skutečnou relevanci světa

Smysl: hustá města a rozlehlá wilderness nebudou škálovat dobře jen podle nejbližšího hráče a hrubého world-gridu.

- napojit `NpcAiLodConfig` a ownership na skutečné tile/zone relevance z map streaming vrstvy
- zavést budgety per tile/zone a ne jen per hráč / coarse grid bucket
- odlišit LOD budgety pro typy populace: městská civilní populace, guards, fauna, ambient critters, vehicles
- přidat pravidla priorit: quest/scenario/combat NPC mají přednost před ambient populací

Exit kritéria:

- přetížené centrum města degraduje ambient populaci dřív než důležité scenario/combat NPC
- wilderness tile poblíž hráče drží lokálně relevantní zvířata/NPC bez zbytečného oživování vzdálených zón
- server umí vysvětlit proč NPC spadlo do `Reduced` nebo `Background`

### Fáze C — Scenario/Task vrstva pro obsah

Smysl: bez scenario vrstvy budou NPC jen lokálně pobíhat, ale nebudou tvořit věrohodný svět.

- rozšířit `NpcTask` / `NpcScenarioId` kontrakt o skutečné scénářové šablony: guard post, vendor counter, tavern idle, street patrol, camp idle, herd graze, predator stalk, flee to cover
- definovat occupancy body a route data v resources, ne v Rust core
- doplnit jednoduchý scheduler: den/noc, počasí, alarm/combat override, occupancy release/acquire
- přidat scenario interpreter nad současný brain kontrakt místo toho, aby každé resource ručně skládalo pouze low-level go-to příkazy

Exit kritéria:

- město jde postavit jako sada scenario points + schedules bez hardcoded Rust logiky
- wilderness fauna používá stejný systém, jen jiné brain/task/scenario kombinace
- při hot-reload resources lze měnit scénáře bez rebuildu Rustu

### Fáze D — Population director pro spawn/despawn a budgety

Smysl: samotný brain systém nestačí; potřebujeme vrstvu, která rozhoduje kdo vůbec ve světě existuje.

- zavést server-side population director nad tile/zone indexem
- director bude držet cílové budgety per zone: town civilian density, guards, traffic, wildlife, predators, camp NPC, random events
- spawn bude driven scénářem a relevancí, ne jen trvalým seznamem všech NPC ve světě
- background NPC přejdou do lightweight representation: scenario/schedule stav bez plné ECS fyziky a bez detailní lokomotion simulace

Exit kritéria:

- server neudržuje plně aktivní stovky až tisíce NPC najednou; detailní ECS dostanou jen relevantní jednotky
- městská zóna i wilderness zóna mají samostatné budgety a spawn pravidla
- při návratu hráče do zóny se populace obnoví z deterministického scenario/schedule stavu, ne náhodně bez continuity

### Fáze E — Reakční AI a combat vrstva

Smysl: populace musí umět reagovat na hráče, hrozby a ruch světa.

- doplnit perception bridge: sight/hearing/alert propagation, minimálně jako server-side high-level eventy
- přidat task přechody `investigate`, `flee`, `combat`, `return_to_scenario`, `call_for_help`
- guards a hostile NPC musí umět přecházet mezi scenario a combat režimem bez rozpadu ownership/LOD modelu
- fauna potřebuje základní predator/prey loop a panic propagation pro herd/chase situace

Exit kritéria:

- guard populace ve městě reaguje na incident a po odeznění se vrací do scenario layer
- wilderness fauna umí flee / regroup / stalk podle typu brainu
- combat/investigate NPC mají prioritu v LOD a director je nedemotuje příliš agresivně

### Fáze F — Debug, telemetrie, authoring workflow

Smysl: bez nástrojů nepůjde systém ladit ani authorovat ve větším měřítku.

- viewer overlay a in-game debug panel pro `NpcAiLodState`, ownera, scenario id, current task, avoidance cache, budget reason
- log nebo event tracing pro handoff, LOD změny, scenario transitions a population director decisions
- authoring workflow pro zone/scenario body/route body v resources nebo map datech
- profilovací režim: počet full/reduced/background NPC per zone, per player a per archetype

Exit kritéria:

- z debug panelu je vidět proč konkrétní NPC existuje, kdo ho simuluje a proč je v daném LOD
- content autor umí přidat městskou i wilderness populaci bez změn Rust core

### Fáze G — Zátěžové ověření a produkční hranice

Smysl: cílový systém musí mít ověřené limity, ne jen architektonický záměr.

- připravit benchmark mapy: malé město, velké město, lesní zóna, mixed frontier oblast
- ověřit minimálně tři profily: 1 hráč / 4 hráči / 16 hráčů v jedné oblasti
- měřit: počet full/reduced/background NPC, fixed tick cost, handoff churn, nav/repath cost, network traffic na `NpcTransformUpdate`
- podle výsledků doladit LOD prahy, budgety a throttling cadence

Exit kritéria:

- máme známé provozní budgety pro města i divočinu
- víme, kde je limit client-owned modelu a kdy je nutné agresivněji přepínat na background representation

## Praktické pořadí implementace odteď

1. Dotáhnout locomotion core a avoidance tak, aby chase/follow/escort byly stabilní i v reduced režimu.
2. Přepnout LOD budgety z coarse gridu na map/tile/zone relevance a přidat prioritizaci archetypů.
3. Založit scenario/schedule vrstvu nad současným brain kontraktem.
4. Postavit population director pro spawn/despawn a lightweight background representation.
5. Dodat reaction/combat AI přechody nad scenario vrstvu.
6. Přidat debug/telemetrii a benchmark scénáře.

Pokud se budeme držet tohoto pořadí, dostaneme se od dnešního „ownership + locomotion foundation“ k systému, který skutečně obslouží města i divočinu bez rozbití škálování nebo authoringu.

---

## Soubory ke změně

| Soubor | Změna |
|--------|-------|
| `core_net/src/protocol.rs` | `NpcTransformUpdate` message |
| `core_net/src/net_plugin.rs` | registrace channel + message |
| `core_net/src/sim.rs` | `receive_npc_transform_updates` |
| `core_resources/src/cmd_queue.rs` | lokální `NpcAgent`, server freeze pro owned, shared movement helpers |
| `host_client/src/npc_sim.rs` | nový soubor — `tick_owned_npc_agents` |
| `host_client/src/main.rs` | registrace `NpcSimPlugin` |
| nový shared AI modul | `ReplicatedNpcGoal` / `ReplicatedNpcBrain`, scenario/task kontrakt |
