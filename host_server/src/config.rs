//! `server.toml` config — strukturovaný popis všech runtime nastavení serveru.
//!
//! Filosofie:
//! * **Vše má rozumný default**, takže `cargo run -p host_server` bez configu
//!   běží out of the box.
//! * Každá sekce mapuje na jeden Bevy plugin / resource v `main.rs`.
//! * Sekce, které ještě nejsou plně zapojené (Phase 4 DB, admin API, atd.),
//!   schválně necháváme v configu — slouží jako roadmap a nezavádí breaking
//!   změny, až je doplníme.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// Default jméno config souboru. Server ho hledá nejprve vedle `.exe`,
/// teprve potom v CWD (viz [`resolve_path_relative_to_exe`]).
pub const DEFAULT_CONFIG_FILE: &str = "server.toml";


/// Vrátí absolutní cestu pro daný (typicky relativní) path s fallback mechanismem:
///
/// 1. Když je `rel` **absolutní**, vrátíme ho beze změny.
/// 2. Zkusíme `<dir(.exe)>/<rel>` — pokud existuje, vrátíme absolutní cestu.
///    Tím distribuce s rozložením `.exe + resources/ + server.toml` funguje
///    bez ohledu na CWD, ze kterého admin server spustí.
/// 3. Zkusíme `<cwd>/<rel>` — pokud existuje (pro `cargo run` z projektu rootu).
/// 4. Pokud je `rel == "resources"`, zkusíme fallback na `<project_root>/resources`
///    (pro `cargo run` z build directory).
/// 5. Když ani to neexistuje, vrátíme `rel` tak jak je. OS pak interpretuje
///    relativně k CWD.
pub fn resolve_path_relative_to_exe(rel: &Path) -> PathBuf {
    if rel.is_absolute() {
        return rel.to_path_buf();
    }

    // 1. Vedle exe
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join(rel);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // 2. V CWD
    let cwd_candidate = std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join(rel));
    if let Some(candidate) = cwd_candidate {
        if candidate.exists() {
            return candidate;
        }
    }

    // 3. Pro "resources" — zkusíme fallback na ../resources
    // (využitečné když je server zkompilován v target/debug)
    if rel == Path::new("resources") {
        if let Ok(exe) = std::env::current_exe() {
            // Try: <exe_dir>/../resources
            if let Some(exe_dir) = exe.parent() {
                if let Some(parent) = exe_dir.parent() {
                    let candidate = parent.join("resources");
                    if candidate.exists() {
                        return candidate;
                    }
                }
            }
        }
    }

    // 4. Fallback: vrátíme path tak jak je
    rel.to_path_buf()
}

/// Default config path s exe-adjacent fallbackem (viz [`resolve_path_relative_to_exe`]).
pub fn resolve_default_config_path() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_candidate = cwd.join(DEFAULT_CONFIG_FILE);
        if cwd_candidate.exists() {
            return cwd_candidate;
        }
    }

    resolve_path_relative_to_exe(Path::new(DEFAULT_CONFIG_FILE))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub server: ServerInfo,
    pub gameplay: GameplayConfig,
    pub net: NetConfig,
    pub resources: ResourcesConfig,
    pub handshake: HandshakeConfig,
    pub auth: AuthConfig,
    pub logging: LoggingConfig,
    pub admin: AdminConfig,
    pub database: DatabaseConfig,
    pub dev: DevConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            server: ServerInfo::default(),
            gameplay: GameplayConfig::default(),
            net: NetConfig::default(),
            resources: ResourcesConfig::default(),
            handshake: HandshakeConfig::default(),
            auth: AuthConfig::default(),
            logging: LoggingConfig::default(),
            admin: AdminConfig::default(),
            database: DatabaseConfig::default(),
            dev: DevConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerInfo {
    /// Display name v server browseru.
    pub name: String,
    /// Volný popis / MOTD.
    pub description: String,
    /// Tagy pro filtrování v server browseru (žánr, region, mód).
    pub tags: Vec<String>,
    /// Volitelná ikonka (PNG, doporučeně 64×64 nebo 128×128).
    pub icon: Option<PathBuf>,
    /// Public listing v master serveru (Phase 4).
    pub public: bool,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            name: "bevy_game dedicated server".to_string(),
            description: "A FiveM-style modular Bevy multiplayer server.".to_string(),
            tags: vec!["dev".to_string()],
            icon: None,
            public: false,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GameplayConfig {
    /// Hard limit počtu připojených hráčů.
    pub max_players: u32,
    /// Velikost čekací fronty nad rámec `max_players`. 0 = fronta vypnutá.
    pub queue_max: u32,
    /// Volitelný gamemode resource ID, který server hostí jako root.
    /// Phase 3+ klient si může v `ServerHello` zkontrolovat, že má
    /// správný build.
    pub gamemode: Option<String>,
    /// Po jaké době nečinnosti se klient odpojí (sec).
    pub idle_kick_sec: u64,
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            max_players: 32,
            queue_max: 0,
            gamemode: None,
            idle_kick_sec: 600,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetConfig {
    /// Bind addr lightyear UDP socketu.
    pub udp_bind: String,
    /// Bind addr Axum HTTP file serveru.
    pub http_bind: String,
    /// Public URL HTTP serveru — to, co klient dostane v `ServerHello`
    /// a používá jako base pro stahování souborů. Defaultně shoduje s
    /// `http_bind`, ale pro NAT / reverse proxy nastav ručně.
    pub http_public_url: String,
    /// Tickrate ECS simulace (Hz). Musí ladit s klientovou simulací.
    pub tick_hz: f64,
    /// Netcode protocol ID. Klient/server s rozdílným ID se nespárují.
    /// Default ASCII "bevy_gam".
    pub protocol_id: u64,
    /// Cesta k 32-bytovému netcode privátnímu klíči. `None` ⇒ all-zero
    /// klíč (insecure, dev only).
    pub private_key_path: Option<PathBuf>,
    /// Netcode-level connection timeout: po jak dlouho bez keepalive
    /// server klienta dropne (sec). Toto je nízkoúrovňový timeout —
    /// vyšší úroveň (idle_kick_sec) řeší gameplay logika zvlášť.
    pub connection_timeout_sec: i32,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            udp_bind: "0.0.0.0:5000".to_string(),
            http_bind: "0.0.0.0:8081".to_string(),
            http_public_url: "http://127.0.0.1:8081".to_string(),
            tick_hz: 60.0,
            protocol_id: 0x6265_7679_5f67_616d, // "bevy_gam"
            private_key_path: None,
            connection_timeout_sec: 15,
        }
    }
}

impl NetConfig {
    pub fn udp_bind_addr(&self) -> Result<SocketAddr, ConfigError> {
        self.udp_bind
            .parse()
            .map_err(|e: std::net::AddrParseError| ConfigError::AddrParse {
                field: "net.udp_bind",
                value: self.udp_bind.clone(),
                source: e,
            })
    }

    pub fn http_bind_addr(&self) -> Result<SocketAddr, ConfigError> {
        self.http_bind
            .parse()
            .map_err(|e: std::net::AddrParseError| ConfigError::AddrParse {
                field: "net.http_bind",
                value: self.http_bind.clone(),
                source: e,
            })
    }

    pub fn tick_duration(&self) -> Duration {
        if self.tick_hz <= 0.0 {
            // Fallback, ať nedělíme nulou ani záporným hz.
            Duration::from_secs_f64(1.0 / 60.0)
        } else {
            Duration::from_secs_f64(1.0 / self.tick_hz)
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourcesConfig {
    /// VFS root, který server skenuje.
    pub root: PathBuf,
    /// Hot-reload watcher zapnutý.
    pub hot_reload: bool,
    /// Debounce window pro filesystem eventy (ms).
    pub watcher_debounce_ms: u64,
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("resources"),
            hot_reload: cfg!(debug_assertions),
            watcher_debounce_ms: 150,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HandshakeConfig {
    /// Maximální čas mezi `ServerHello` a `ClientReady` před disconnectem.
    pub ready_timeout_sec: u64,
    /// Timeout pro samotný lightyear connect (Phase 3).
    pub connect_timeout_sec: u64,
    /// Maximum bytes per single resource file allowed in digest. Phase 3+.
    pub max_file_size_mb: u32,
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        Self {
            ready_timeout_sec: 30,
            connect_timeout_sec: 10,
            max_file_size_mb: 64,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Authentikace klientů. Pro Phase 2 = `"open"` (LAN dev).
    /// Phase 4 přinese `"token"` a `"whitelist"`.
    pub mode: AuthMode,
    /// Cesta k whitelist souboru (jen když `mode = "whitelist"`).
    pub whitelist_path: Option<PathBuf>,
    /// Vyžadovat přihlášení uživatelským jménem a heslem.
    /// Vyžaduje `[database].url` a resource `core/auth`.
    /// `false` = kdokoli se připojí bez hesla (LAN dev).
    pub require_auth: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::Open,
            whitelist_path: None,
            require_auth: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    Open,
    Token,
    Whitelist,
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    /// Globální tracing level: trace / debug / info / warn / error.
    pub level: LogLevel,
    /// Per-modul filter v tracing-subscriber syntaxi (skládá se přes `level`).
    pub filter: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            filter: "wgpu=warn,naga=warn".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn to_bevy(self) -> bevy::log::Level {
        match self {
            LogLevel::Trace => bevy::log::Level::TRACE,
            LogLevel::Debug => bevy::log::Level::DEBUG,
            LogLevel::Info => bevy::log::Level::INFO,
            LogLevel::Warn => bevy::log::Level::WARN,
            LogLevel::Error => bevy::log::Level::ERROR,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminConfig {
    /// Bind addr admin / metrics HTTP API. `None` ⇒ admin endpoint vypnutý.
    pub bind: Option<String>,
    /// Bearer token požadovaný pro admin requesty.
    pub token: Option<String>,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            bind: None,
            token: None,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    /// SQL backend connection string. `None` ⇒ server běží bez DB.
    pub url: Option<String>,
    /// Velikost connection poolu.
    pub pool_size: u32,
    /// Cesta ke složce s migracemi.
    pub migrations_path: Option<PathBuf>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: None,
            pool_size: 8,
            migrations_path: None,
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DevConfig {
    /// Když true, server odpovídá `ClientReady` rovnou bez čekání na klienta.
    /// Užitečné pro headless integration testy. Pozor — bypassuje validaci.
    pub auto_acknowledge_clients: bool,
    /// Když true, na startu se zaloguje kompletní digest všech resources
    /// (užitečné pro porovnání mezi server / client buildy).
    pub print_digest_on_startup: bool,
    /// Maximální počet frame v profileru, pak server skončí. 0 = neomezený.
    pub profile_first_n_frames: u64,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            auto_acknowledge_clients: false,
            print_digest_on_startup: false,
            profile_first_n_frames: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error reading {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("toml parse error in {path:?}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid socket addr in {field}: {value:?} ({source})")]
    AddrParse {
        field: &'static str,
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("private key file {path:?} must be exactly 32 bytes (got {actual})")]
    KeyLength { path: PathBuf, actual: usize },
}

impl ServerConfig {
    /// Načte config ze souboru. Pokud soubor neexistuje, vrátí default
    /// s informativním logem (server pojede s defaulty).
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Ok(cfg) => {
                eprintln!("[host_server] loaded config from {}", path.display());
                cfg
            }
            Err(ConfigError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                eprintln!(
                    "[host_server] {} not found — using compiled defaults. \
                     Drop a copy of server.toml from the repo root to override.",
                    path.display()
                );
                Self::default()
            }
            Err(e) => {
                eprintln!(
                    "[host_server] config load failed ({e}) — falling back to defaults"
                );
                Self::default()
            }
        }
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let s = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        toml::from_str(&s).map_err(|e| ConfigError::Toml {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Načte 32-bytový privátní netcode klíč ze souboru, pokud je nakonfigurovaný.
    /// Pokud cesta není uvedená, vrací `None` (použije se all-zero default).
    /// Relativní cesty resolvujeme vůči `.exe` (viz [`resolve_path_relative_to_exe`]),
    /// aby distribuce s `secrets/netcode.key` vedle `.exe` šla rozjet.
    pub fn load_private_key(&self) -> Result<Option<[u8; 32]>, ConfigError> {
        let Some(path) = &self.net.private_key_path else {
            return Ok(None);
        };
        let path = resolve_path_relative_to_exe(path);
        let bytes = std::fs::read(&path).map_err(|e| ConfigError::Io {
            path: path.clone(),
            source: e,
        })?;
        if bytes.len() != 32 {
            return Err(ConfigError::KeyLength {
                path,
                actual: bytes.len(),
            });
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Some(key))
    }
}
