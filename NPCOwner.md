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
