# NPC Ownership — Phase B: Client-Side Simulation

## Cíl

Přesunout fyzikální simulaci NPC ze serveru na vlastnícího klienta, aby NPC reálně
reagovala na terén, kolize a překážky v herním světě. Server zůstává autoritativní —
přijímá pozice od ownera a replikuje je ostatním.

---

## Aktuální stav (Phase A — hotovo)

- `NpcOwner(Option<u64>)` — replikovaný komponent, říká kdo NPC vlastní
- `assign_npc_owners` — server každé 2 s přiřadí nejbližšího hráče do 200 m
- `tick_npc_agents` — server simuluje pohyb (waypoints, bez fyziky), freeze při `None`
- Klient zná svého ownera, ale simulaci NEBĚŽÍ — jen přijímá `NetTransform`

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

Doporučení: **varianta A** pro rychlou implementaci, varianta B pro optimalizaci later.

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

1. `NpcTransformUpdate` message + registrace směru Client→Server
2. `receive_npc_transform_updates` na serveru (s owner validací)
3. Replikace `NpcAgent` na klienta (`register_component`)
4. `tick_owned_npc_agents` na klientovi (kopie server logiky + send message)
5. Server: přestat simulovat NPC s aktivním ownerem (nebo přidat fallback timer)
6. Terrain snapping v klientské simulaci (raycast + Y korekce)
7. (Volitelné) NavMesh pathfinding místo přímočarého waypoint pohybu

---

## Závislosti / Předpoklady

- NavMesh (Phase 3.6) — pro správný pathfinding kolem překážek
- `NpcAgent` musí být `Serialize + Deserialize` (přidat derive)
- `NpcMoveGoal` musí být `Serialize + Deserialize` (přidat derive)
- lightyear channel pro Client→Server unreliable messages

---

## Soubory ke změně

| Soubor | Změna |
|--------|-------|
| `core_net/src/protocol.rs` | `NpcTransformUpdate` message |
| `core_net/src/net_plugin.rs` | registrace channel + message |
| `core_net/src/sim.rs` | `receive_npc_transform_updates` |
| `core_resources/src/cmd_queue.rs` | `NpcAgent` + `NpcMoveGoal` Serialize, server freeze pro owned |
| `host_client/src/npc_sim.rs` | nový soubor — `tick_owned_npc_agents` |
| `host_client/src/main.rs` | registrace `NpcSimPlugin` |
