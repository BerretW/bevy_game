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

use core_shared::{NetTransform, NetVelocity, PlayerMarker};

use crate::protocol::{
    AuthChallenge, AuthCredentials, AuthResult,
    ClientReady, LuaEventMessage, PlayerInput, PlayerStatsUpdate, ServerHello,
};

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

/// Phase 3 — sequenced unreliable kanál pro `PlayerInput` (60 Hz tick stream).
/// Zastaralé pakety se na příjmu zahodí (latest wins), nikdy se neretransmituje
/// → konzistentní latency. Vhodné pro tick-rate inputy, ne pro reliable RPC.
pub struct InputChannel;

/// Phase 4 — sequenced unreliable kanál pro `PlayerStatsUpdate` (~10 Hz, server→klient).
/// Vždy zajímá jen poslední stav; ztráta paketu je přijatelná.
pub struct StatsChannel;

/// Phase 4 — ordered reliable kanál pro autentizaci (AuthChallenge / AuthCredentials / AuthResult).
/// Ztráta nebo přeřazení by znemožnila přihlášení.
pub struct AuthChannel;

// ---------------------------------------------------------------------------
// ProtocolPlugin — sdílený
// ---------------------------------------------------------------------------

/// Registruje messages + channels v lightyearu. Musí být na obou stranách
/// **identický**, jinak deserializace selže.
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        // -----------------------------------------------------------------
        // Messages
        // -----------------------------------------------------------------
        app.register_message::<ServerHello>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<ClientReady>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<LuaEventMessage>()
            .add_direction(NetworkDirection::Bidirectional);
        // Phase 3 — tick-rate input messages.
        app.register_message::<PlayerInput>()
            .add_direction(NetworkDirection::ClientToServer);

        // Phase 4 — stats update (server → client).
        app.register_message::<PlayerStatsUpdate>()
            .add_direction(NetworkDirection::ServerToClient);

        // -----------------------------------------------------------------
        // Channels
        // -----------------------------------------------------------------
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

        // Inputs — sequenced unreliable: zastaralé pakety dropujeme,
        // čerstvý tick state nahradí starší.
        app.add_channel::<InputChannel>(ChannelSettings {
            mode: ChannelMode::SequencedUnreliable,
            ..default()
        })
        .add_direction(NetworkDirection::ClientToServer);

        // Stats — sequenced unreliable server→klient; vždy zajímá jen poslední snapshot.
        app.add_channel::<StatsChannel>(ChannelSettings {
            mode: ChannelMode::SequencedUnreliable,
            ..default()
        })
        .add_direction(NetworkDirection::ServerToClient);

        // Auth — ordered reliable, bidirectional.
        app.add_channel::<AuthChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        // Auth messages
        app.register_message::<AuthChallenge>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<AuthCredentials>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<AuthResult>()
            .add_direction(NetworkDirection::ServerToClient);

        // -----------------------------------------------------------------
        // Replicated components (Phase 3)
        // -----------------------------------------------------------------
        // `add_prediction()` zapne client-side prediction + rollback pro
        // komponentu — server posílá authoritativní stav, klient predikuje
        // lokálně, lightyear porovnává a rollbacka když se rozcházejí.
        // Interpolation pro remote entity přidáme později (vyžaduje `Ease`
        // impl nebo custom lerp fn pro `NetTransform`).
        app.register_component::<NetTransform>().add_prediction();
        app.register_component::<NetVelocity>().add_prediction();
        // add_prediction zkopíruje PlayerMarker i na Predicted entitu,
        // aby sprite attachment query v gameplay.rs správně cílila na
        // renderovanou (Predicted) entitu, ne jen na Confirmed datovou entitu.
        app.register_component::<PlayerMarker>().add_prediction();

        // Phase 5 — replikace Lua entity handles a jmen modelů.
        // EntityHandle (u64) je "network ID" — stejné číslo na serveru i klientovi.
        // ModelName řekne klientovi, jaký model má načíst pro replicated entity.
        // Žádná prediction není potřeba — tato data se po spawnu nemění.
        app.register_component::<core_resources::EntityHandle>();
        app.register_component::<core_resources::ModelName>();
        app.register_component::<core_resources::NpcPedMarker>();
        app.register_component::<core_resources::NpcOwner>();
        app.register_component::<core_resources::PedProfileOverride>();
        app.register_component::<core_resources::DummyObjectMarker>();
        app.register_component::<core_resources::ColliderObjectMarker>();
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

/// Zpráva pro spuštění spojení klienta se serverem.
/// Lobby ji vyšle po kliknutí na "Connect"; `ClientNetPlugin` ji zachytí
/// a teprve tehdy spawne klientskou entitu a zahájí handshake.
#[derive(Message, Clone)]
pub struct ConnectClient;

/// Plugin přidá `ClientPlugins`, `ProtocolPlugin` a systém, který na příchozí
/// `ConnectClient` zprávu spawne entitu klienta a zahájí connection.
/// Samotné spojení se nespouští automaticky — lobby vyšle `ConnectClient`.
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
        app.add_message::<ConnectClient>();
        app.add_systems(Update, spawn_client_on_connect);
        app.add_observer(log_server_connected);
    }
}

fn spawn_client_on_connect(
    mut reader: MessageReader<ConnectClient>,
    mut commands: Commands,
    config: Res<ClientNetConfig>,
) -> std::result::Result<(), BevyError> {
    for _ in reader.read() {
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
                ReplicationReceiver::default(),
            ))
            .id();
        commands.trigger(Connect { entity });
        info!(
            "[core_net::client] connecting to {} (local {})",
            config.server, config.local
        );
    }
    Ok(())
}

fn log_server_connected(_trigger: On<Add, Connected>) {
    info!("[core_net::client] connection established to server");
}
