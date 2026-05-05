//! Lightyear integrace — Phase 2.
//!
//! Tři Bevy pluginy:
//!
//! * [`ProtocolPlugin`] — sdílený, registruje messages + channels v lightyearu.
//!   Musí být přidaný **až po** `ClientPlugins` / `ServerPlugins`, jinak
//!   lightyear neví, kde si registrace vyzvednout.
//! * [`ServerNetPlugin`] — server-side. Spawne entitu se `NetcodeServer` +
//!   UDP IO a posílá `ServerHello` všem nově connected klientům (`Add<Connected>`
//!   observer). Pro Phase 2 ještě neposíláme reálné digesty (to propojí
//!   handshake systém v `host_server`).
//! * [`ClientNetPlugin`] — client-side. Spawne klienta s `NetcodeClient` +
//!   UDP IO a po connectu hlásí log. Reálná logika "přijmi digest, stáhni
//!   soubory, pošli ClientReady" je v `host_client` (závisí na HTTP downloader).
//!
//! Authentikace: zatím `Authentication::Manual` s `Key::default()` /
//! `protocol_id = 0` — vhodné pro LAN dev, **nikoliv** pro produkci.
//! Phase 4 sem přijde proper token-based auth.

use std::net::SocketAddr;
use std::time::Duration;

use bevy::prelude::*;
use lightyear::prelude::client::{
    Client, ClientPlugins, NetcodeClient, NetcodeConfig as ClientNetcodeConfig,
};
use lightyear::prelude::server::{
    NetcodeConfig as ServerNetcodeConfig, NetcodeServer, ServerPlugins, ServerUdpIo, Start,
};
// `Authentication`, `UdpIo`, `LocalAddr`, `PeerAddr`, `Link`, `Connect`,
// `Connected` a channel/message ext traity přijdou z top-level prelude
// (re-exporty z lightyear_netcode/prelude, lightyear_udp/prelude atd.).
use lightyear::prelude::*;

use crate::protocol::{ClientReady, LuaEventMessage, ServerHello};

/// Tickrate pro lightyear (musí ladit s herní simulací).
pub const FIXED_TIMESTEP_HZ: f64 = 60.0;

/// Default protocol_id pro netcode auth — dev-only sentinel.
/// Produkce by měla brát z env / config souboru, ať server odmítne klienty
/// s nesprávným buildem.
pub const NETCODE_PROTOCOL_ID: u64 = 0x6265_7679_5f67_616d; // ascii "bevy_gam"

/// Default server bind addr — UDP 5000 na všech rozhraních.
pub const DEFAULT_SERVER_BIND: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
    5000,
);

/// Default client local addr — `0.0.0.0:0` ⇒ kernel přidělí ephemeral port.
pub const DEFAULT_CLIENT_LOCAL: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
    0,
);

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

/// Reliable, ordered kanál pro handshake (`ServerHello` / `ClientReady`).
/// Ztráta nebo přeřazení by uvedla klienta do nekonzistentního stavu.
pub struct HandshakeChannel;

/// Reliable, ordered kanál pro Lua RPC (`LuaEventMessage`).
/// Phase 3 si můžeme dovolit přidat unreliable variant pro vysokofrekvenční
/// eventy (movement, particles), ale Phase 2 default je reliable.
pub struct LuaRpcChannel;

// ---------------------------------------------------------------------------
// ProtocolPlugin — sdílený
// ---------------------------------------------------------------------------

/// Registruje messages + channels v lightyearu. Musí být na obou stranách
/// **identický**, jinak deserializace selže.
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Messages.
        app.register_message::<ServerHello>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<ClientReady>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<LuaEventMessage>()
            .add_direction(NetworkDirection::Bidirectional);

        // Channels.
        app.add_channel::<HandshakeChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        app.add_channel::<LuaRpcChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}

// ---------------------------------------------------------------------------
// ServerNetPlugin
// ---------------------------------------------------------------------------

/// Konfigurace server-side networkingu. Hodnoty obvykle vkládá `host_server`
/// po načtení `server.toml` před přidáním pluginu.
#[derive(Resource, Clone, Debug)]
pub struct ServerNetConfig {
    pub bind: SocketAddr,
    pub tick_duration: Duration,
    /// Netcode protocol ID. Klient s rozdílným ID se nepřipojí.
    pub protocol_id: u64,
    /// 32-bytový privátní klíč netcode tokenů. Default = all-zeros (insecure,
    /// dev only); produkce by měla mít unikátní klíč ze `private_key_path`.
    pub private_key: [u8; 32],
    /// Po jak dlouhé nečinnosti se klient odpojí (sec).
    pub client_timeout_sec: i32,
}

impl Default for ServerNetConfig {
    fn default() -> Self {
        Self {
            bind: DEFAULT_SERVER_BIND,
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
            protocol_id: NETCODE_PROTOCOL_ID,
            private_key: [0u8; 32],
            client_timeout_sec: 15,
        }
    }
}

/// Plugin, který přidává:
/// 1. lightyear `ServerPlugins` (s tick_duration z config),
/// 2. [`ProtocolPlugin`] (pořadí důležité — až po `ServerPlugins`),
/// 3. systém, který na startu spawne server entitu.
///
/// Lifecycle observers (Add<Connected>) řeší samostatný handshake systém
/// v `host_server` — ten ví, kdy je `ResourceDigestCache` ready.
pub struct ServerNetPlugin;

impl Plugin for ServerNetPlugin {
    fn build(&self, app: &mut App) {
        let tick_duration = app
            .world()
            .get_resource::<ServerNetConfig>()
            .map(|c| c.tick_duration)
            .unwrap_or_else(|| Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ));

        app.init_resource::<ServerNetConfig>();
        app.add_plugins(ServerPlugins { tick_duration });
        app.add_plugins(ProtocolPlugin);
        app.add_systems(Startup, spawn_server);
        app.add_observer(log_client_connected);
    }
}

fn spawn_server(mut commands: Commands, config: Res<ServerNetConfig>) {
    let netcode = ServerNetcodeConfig::default()
        .with_protocol_id(config.protocol_id)
        .with_key(config.private_key)
        .with_client_timeout_secs(config.client_timeout_sec);

    let entity = commands
        .spawn((
            NetcodeServer::new(netcode),
            LocalAddr(config.bind),
            ServerUdpIo::default(),
        ))
        .id();
    commands.trigger(Start { entity });
    info!(
        "[core_net::server] netcode server spawned on {} (protocol_id=0x{:016x}, timeout={}s)",
        config.bind, config.protocol_id, config.client_timeout_sec
    );
}

fn log_client_connected(trigger: On<Add, Connected>, peers: Query<&PeerAddr>) {
    let entity = trigger.entity;
    let peer = peers
        .get(entity)
        .map(|p| p.0.to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    info!(
        "[core_net::server] client connected on entity {:?} from {}",
        entity, peer
    );
}

// ---------------------------------------------------------------------------
// ClientNetPlugin
// ---------------------------------------------------------------------------

#[derive(Resource, Clone, Debug)]
pub struct ClientNetConfig {
    pub server: SocketAddr,
    pub local: SocketAddr,
    pub client_id: u64,
    pub tick_duration: Duration,
    /// Musí ladit s `ServerNetConfig::protocol_id`.
    pub protocol_id: u64,
    /// Musí ladit s `ServerNetConfig::private_key`.
    pub private_key: [u8; 32],
}

impl Default for ClientNetConfig {
    fn default() -> Self {
        Self {
            server: SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                5000,
            ),
            local: DEFAULT_CLIENT_LOCAL,
            client_id: 0,
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
            protocol_id: NETCODE_PROTOCOL_ID,
            private_key: [0u8; 32],
        }
    }
}

/// Plugin přidá `ClientPlugins`, `ProtocolPlugin` a Startup systém, který
/// otevře connection. Phase 2 čte handshake message v navazujícím systému
/// v `host_client`.
pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        let tick_duration = app
            .world()
            .get_resource::<ClientNetConfig>()
            .map(|c| c.tick_duration)
            .unwrap_or_else(|| Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ));

        app.init_resource::<ClientNetConfig>();
        app.add_plugins(ClientPlugins { tick_duration });
        app.add_plugins(ProtocolPlugin);
        app.add_systems(Startup, spawn_client);
        app.add_observer(log_server_connected);
    }
}

fn spawn_client(
    mut commands: Commands,
    config: Res<ClientNetConfig>,
) -> std::result::Result<(), BevyError> {
    let auth = Authentication::Manual {
        server_addr: config.server,
        client_id: config.client_id,
        private_key: config.private_key,
        protocol_id: config.protocol_id,
    };

    let entity = commands
        .spawn((
            Client::default(),
            LocalAddr(config.local),
            PeerAddr(config.server),
            Link::new(None),
            NetcodeClient::new(auth, ClientNetcodeConfig::default())?,
            UdpIo::default(),
        ))
        .id();
    commands.trigger(Connect { entity });
    info!(
        "[core_net::client] connecting to {} (local {})",
        config.server, config.local
    );
    Ok(())
}

fn log_server_connected(_trigger: On<Add, Connected>) {
    info!("[core_net::client] connection established to server");
}
