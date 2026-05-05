//! Wire-format zpráv pro Phase 2 handshake.
//!
//! Tyto typy jsou plain `serde` struct + Bevy `Event` derive; samotnou
//! lightyear integraci (`Channel`, `Message` traity) udělá nadřízený crate
//! tak, že tyto typy obalí. Záměr: kdyby se v budoucnu lightyear vyměnil
//! (např. za `quinn` přímý), wire-types zůstávají.

use serde::{Deserialize, Serialize};

use crate::digest::ResourceDigest;

/// Verze našeho aplikačního protokolu. Inkrementujeme při breaking changi
/// `ServerHello` / `ClientReady` / `LuaEventMessage`. Klient i server kontrolují
/// rovnost; rozdíl ⇒ disconnect s logem `protocol mismatch`.
pub const PROTOCOL_VERSION: u32 = 1;

/// **Server → Client**, posláno jako úplně první aplikační zpráva po
/// navázání lightyear spojení.
///
/// Klient po přijetí:
/// 1. zkontroluje `protocol_version`,
/// 2. spočítá si lokální digest cache,
/// 3. stáhne přes HTTP soubory, které chybí nebo se liší,
/// 4. postaví VFS + sandboxy,
/// 5. odpoví [`ClientReady`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerHello {
    pub protocol_version: u32,
    /// Base URL HTTP file serveru, kde leží soubory resources.
    /// Příklad: `"http://10.0.0.5:8081"`. Klient pak fetchuje
    /// `<base>/resources/<resource_id>/<rel_path>`.
    pub http_base_url: String,
    /// Kompletní digest všech resources, které server hostí.
    /// Klient si je všechny musí dotáhnout, jinak nedostane povolení
    /// vstoupit do hry.
    pub manifests: Vec<ResourceDigest>,
}

/// **Client → Server**, posláno až po stáhnutí všech resources a postavení
/// klientských sandboxů. Server po této zprávě uvolní klienta do "in-game"
/// stavu (Phase 3+ začne posílat replikaci entit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientReady {
    pub protocol_version: u32,
}

/// Obousměrný Lua RPC mostík.
///
/// * Klient → Server: vznikne voláním `TriggerServerEvent(name, ...)` v Lua.
///   `target` je vždy `None` (server ví, od kterého klienta zpráva přišla
///   z transport vrstvy).
/// * Server → Klient: vznikne voláním `TriggerClientEvent(name, target, ...)`
///   v Lua. `target = Some(player_id)` = unicast, `None` = broadcast všem.
///
/// `payload` je sériová binární data, formát si zvolí konkrétní pár
/// resources (typicky MessagePack, ale Phase 2 je agnostická a posílá
/// `Vec<u8>` "as is").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaEventMessage {
    pub name: String,
    pub target: Option<u64>,
    pub payload: Vec<u8>,
}
