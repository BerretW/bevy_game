//! `core_net` — síťový protokol mezi serverem a klientem.
//!
//! Phase 2 zodpovědnosti tohoto crate:
//! 1. Definovat wire-format zprávy (handshake, manifest digest, Lua RPC).
//! 2. Spočítat file digest (blake3) pro každý resource v manifestu.
//!
//! Lightyear plugin a HTTP file server jsou postaveny vrstvou výš
//! (host_server, host_client) — tady jen typy a čisté funkce, žádný I/O
//! kromě deterministického `compute_resource_digest` čtení souborů.
//!
//! Wire types jsou plain `serde` struct — lightyear je doručuje přes
//! `MessageReceiver<M>` / `MessageSender<M>` komponenty na connection
//! entitě, takže žádný Bevy buffered-event derive nepotřebují.

pub mod auth;
pub mod digest;
pub mod digest_cache;
pub mod handshake;
pub mod lua_rpc;
pub mod net_plugin;
pub mod protocol;
pub mod sim;

pub use digest::{
    compute_resource_digest, normalize_rel_path, DigestError, FileDigest, ResourceDigest,
    ResourceDigestSet,
};
pub use digest_cache::{DigestPlugin, ResourceDigestCache};
pub use auth::{ClientAuthPlugin, ServerAuthConfig, ServerAuthPlugin};
pub use handshake::{
    ClientHandshakeComplete, ClientHandshakeConfig, ClientHandshakePlugin, ClientHandshakeState,
    HandshakeStatus, ServerHandshakeConfig, ServerHandshakePlugin,
};
pub use lua_rpc::{ClientLuaRpcPlugin, ServerLuaRpcPlugin};
pub use core_shared::Health;
pub use sim::{
    collect_last_inputs, LastPlayerInputs, ServerSimPlugin, WeaponConfig, WeaponType,
    PLAYER_MOVE_SPEED,
};
pub use net_plugin::{
    ClientNetConfig, ClientNetPlugin, ConnectClient, HandshakeChannel, InputChannel, LuaRpcChannel,
    ProtocolPlugin, ServerNetConfig, ServerNetPlugin, FIXED_TIMESTEP_HZ, NETCODE_PROTOCOL_ID,
};
pub use protocol::{
    player_action, ClientReady, LuaEventMessage, PlayerInput, ServerHello, PROTOCOL_VERSION,
};
