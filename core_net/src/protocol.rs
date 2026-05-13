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
    /// True = server vyžaduje přihlášení uživatelským jménem a heslem
    /// před vstupem do hry. Klient zobrazí přihlašovací prompt.
    pub auth_required: bool,
}

// ---------------------------------------------------------------------------
// Auth messages (Phase 4 — password-based account system)
// ---------------------------------------------------------------------------

/// **Server → Client** — posláno po úspěšném handshaku (ClientReady),
/// pokud `ServerHello.auth_required == true`.
/// Signalizuje klientu, že musí zaslat přihlašovací údaje.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthChallenge;

/// **Client → Server** — přihlašovací nebo registrační údaje.
/// Posílá se přes šifrovaný lightyear kanál — heslo je v plaintextu,
/// server ho hashuje (blake3) před uložením nebo porovnáváním.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCredentials {
    /// 0 = přihlášení, 1 = registrace.
    pub action: u8,
    pub username: String,
    /// Plaintext heslo — bezpečné díky šifrovanému lightyear spojení.
    pub password: String,
}

/// **Server → Client** — výsledek pokusu o přihlášení/registraci.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub success: bool,
    /// Permanentní account ID (např. "user:abc123") — prázdný při neúspěchu.
    pub account_id: String,
    /// Chybová zpráva — prázdná při úspěchu.
    pub error: String,
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

// ---------------------------------------------------------------------------
// Phase 3 — Player input & action intents
// ---------------------------------------------------------------------------

/// **Klient → Server** každý FixedUpdate (~60 Hz).
///
/// Server validuje a aplikuje na autoritativní entitu hráče. Klient si
/// stejnou simulaci pouští lokálně (client-side prediction); když se
/// odpovědi rozcházejí, lightyear `prediction` udělá rollback + replay.
///
/// `client_tick` je **client tick**, kdy byl input zachycen — server
/// ho použije pro lag-compensation hit-testu (rewind cílů na ten tick).
/// 32-bit stačí na ~24 dní souvislého běhu při 60 Hz.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Normalized 2D move vektor (typicky WASD → x,y in [-1.0, 1.0]).
    pub move_dir: [f32; 2],
    /// Look direction (yaw, pitch ve stupních).
    pub look: [f32; 2],
    /// Bitfield aktivních akcí — viz [`PlayerAction`] konstanty.
    pub actions: u32,
    pub client_tick: u32,
    /// Klientská fyzikální pozice (FiveM-style client-trusted).
    /// Server ji přímo zapíše do NetTransform — žádná server-side simulace pohybu.
    pub position: [f32; 3],
}

/// **Server → Client** ~10 Hz — snapshot stavu lokálního hráče.
/// Klient drží `LocalPlayerStats` resource, Lua čte přes `Player.GetLocalStats()`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerStatsUpdate {
    pub hp: f32,
    pub max_hp: f32,
}

/// **Client → Server** — transform update od owning klienta pro NPC.
/// Server přijímá pouze zprávy od aktuálního `NpcOwner` dané entity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NpcTransformUpdate {
    pub handle: u64,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}

/// Konstanty pro `PlayerInput::actions` bitfield. Phase 3+ scripty
/// (Lua weapon definice) si je můžou číst přes Bevy resource registry.
pub mod player_action {
    pub const PRIMARY_FIRE: u32 = 1 << 0;
    pub const SECONDARY_FIRE: u32 = 1 << 1;
    pub const RELOAD: u32 = 1 << 2;
    pub const JUMP: u32 = 1 << 3;
    pub const CROUCH: u32 = 1 << 4;
    pub const SPRINT: u32 = 1 << 5;
    pub const INTERACT: u32 = 1 << 6;
    pub const USE_ITEM: u32 = 1 << 7;
}

// ---------------------------------------------------------------------------
// Phase 5 — Tile-based world streaming (server authority)
// ---------------------------------------------------------------------------

/// **Server → Client** — server-authoritative tile streaming command.
/// Sent periodically (~1 Hz) to validate/correct client's loaded tiles.
///
/// Action:
/// - `"load"` — client should load this tile (idempotent)
/// - `"unload"` — client should unload this tile (idempotent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileStreamingCommand {
    pub tile_id: String,
    pub action: String, // "load" or "unload"
}
