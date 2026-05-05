//! `client.toml` — uživatelský config klienta.
//!
//! **Lokace:**
//! * Windows: `%APPDATA%\bevy_game\client.toml`
//! * Linux:   `~/.config/bevy_game/client.toml`
//! * macOS:   `~/Library/Application Support/bevy_game/client.toml`
//!
//! Pokud na startu config neexistuje, klient vygeneruje výchozí soubor
//! v této lokaci a okamžitě ho zase načte. Override cesty přes:
//! * první positional CLI argument (`host_client.exe path\\custom.toml`),
//! * env variable `BEVY_GAME_CLIENT_CONFIG`.
//!
//! Schéma odpovídá [`ClientConfig`]. Neznámá pole klient odmítá
//! (`deny_unknown_fields`) — překlepy se ohlásí na startu.
//!
//! Phase 2 wire-up:
//! * `graphics.backend` ↦ `RenderPlugin` / `WgpuSettings`
//! * `graphics.window_mode`, `resolution`, `present_mode` ↦ `WindowPlugin`
//! * `advanced.log_level`, `log_filter` ↦ `LogPlugin`
//! * `network.default_server`, `client_id` ↦ `ClientNetConfig`
//! * `paths.cache_dir` ↦ `ClientHandshakeConfig` + `ResourcesPlugin` root
//!
//! Sekce [`GraphicsQuality`], [`AudioConfig`], [`UiConfig`], [`InputConfig`]
//! drží hodnoty pro **Phase 3+ konzumenty** (shadow systémy, audio mixer,
//! action mapper). Žijí v Bevy resource registry, takže gameplay code je
//! načte bez čekání na další refaktor.

use std::path::{Path, PathBuf};

use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

const APP_DIR_NAME: &str = "bevy_game";
const CONFIG_FILE_NAME: &str = "client.toml";
const ENV_OVERRIDE: &str = "BEVY_GAME_CLIENT_CONFIG";

/// Hlavička, která se vepíše na začátek auto-generovaného `client.toml`.
/// Ne-funkční — jen ukazuje uživateli, že soubor je editovatelný.
const HEADER_COMMENT: &str = "\
# =============================================================================
# bevy_game client configuration
# =============================================================================
#
# Tento soubor klient vygeneroval na prvním spuštění. Můžete ho editovat
# a restartovat klienta, změny se aplikují po startu.
#
# Lokace:
#   Windows: %APPDATA%\\bevy_game\\client.toml
#   Linux:   ~/.config/bevy_game/client.toml
#   macOS:   ~/Library/Application Support/bevy_game/client.toml
#
# Override:
#   host_client.exe \"C:/path/to/custom.toml\"     # první CLI argument
#   $env:BEVY_GAME_CLIENT_CONFIG = \"...\"          # env variable
#
# Pro reset na defaulty stačí soubor smazat a klient si ho znovu vygeneruje.
# Schéma a popis polí: viz CLAUDE.md / host_client/src/config.rs.
# =============================================================================

";

// ---------------------------------------------------------------------------
// Top-level
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientConfig {
    pub player: PlayerConfig,
    pub network: NetworkConfig,
    pub graphics: GraphicsConfig,
    pub audio: AudioConfig,
    pub ui: UiConfig,
    pub input: InputConfig,
    pub paths: PathsConfig,
    pub server_history: ServerHistoryConfig,
    pub advanced: AdvancedConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            player: PlayerConfig::default(),
            network: NetworkConfig::default(),
            graphics: GraphicsConfig::default(),
            audio: AudioConfig::default(),
            ui: UiConfig::default(),
            input: InputConfig::default(),
            paths: PathsConfig::default(),
            server_history: ServerHistoryConfig::default(),
            advanced: AdvancedConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// [player]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlayerConfig {
    /// Display name, který klient pošle serveru při připojení.
    pub name: String,
    /// Stabilní client_id pro netcode auth. 0 = generovat náhodně při startu.
    pub saved_client_id: u64,
    /// Volitelná cesta k avatar PNG (Phase 3+ — UI / lobby).
    pub avatar: Option<PathBuf>,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            name: "Player".to_string(),
            saved_client_id: 0,
            avatar: None,
        }
    }
}

// ---------------------------------------------------------------------------
// [network]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Default server, ke kterému se klient připojí, pokud není zadán jinak.
    pub default_server: String,
    /// Local bind addr klienta. `0.0.0.0:0` = kernel zvolí port.
    pub local_bind: String,
    /// Max počet konkurentních HTTP downloadů během handshake.
    pub download_concurrency: u32,
    /// Klient odmítne připojit se na non-HTTPS server (Phase 4 produkce).
    pub strict_https: bool,
    /// Po jaké době klient přestane čekat na ServerHello a disconnectne (sec).
    pub handshake_timeout_sec: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            default_server: "127.0.0.1:5000".to_string(),
            local_bind: "0.0.0.0:0".to_string(),
            download_concurrency: 4,
            strict_https: false,
            handshake_timeout_sec: 30,
        }
    }
}

// ---------------------------------------------------------------------------
// [graphics]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphicsConfig {
    /// Render backend.
    pub backend: GraphicsBackend,
    /// Mód okna.
    pub window_mode: WindowModeConfig,
    /// Index monitoru pro fullscreen módy (0 = primární).
    pub monitor_index: u32,
    /// Šířka okna v pixelech (windowed mód) nebo target resolution.
    pub resolution_width: u32,
    /// Výška okna v pixelech.
    pub resolution_height: u32,
    /// Present mode (vsync politika).
    pub present_mode: PresentModeConfig,
    /// FPS cap pro `unlocked` present mód. 0 = bez limitu.
    pub fps_cap: u32,
    /// HDR rendering. Vyžaduje HDR-capable display.
    pub hdr: bool,
    /// GPU power preference, kterou wgpu bere v potaz.
    pub gpu_priority: GpuPriority,
    /// Window title — užitečné pro "v okně mám 4 klienty zároveň".
    pub window_title: String,

    /// Detail / kvalita renderu.
    pub quality: GraphicsQuality,
}

impl Default for GraphicsConfig {
    fn default() -> Self {
        Self {
            backend: GraphicsBackend::Auto,
            window_mode: WindowModeConfig::Windowed,
            monitor_index: 0,
            resolution_width: 1280,
            resolution_height: 720,
            present_mode: PresentModeConfig::Auto,
            fps_cap: 0,
            hdr: false,
            gpu_priority: GpuPriority::HighPerformance,
            window_title: "bevy_game".to_string(),
            quality: GraphicsQuality::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GraphicsBackend {
    /// Operační systém zvolí best-fit (DX12 na Windows, Vulkan na Linux, Metal na macOS).
    #[default]
    Auto,
    Vulkan,
    Dx12,
    Metal,
    /// OpenGL ES — fallback pro starý hardware. Často horší než Vulkan/DX12.
    Gl,
    /// Browser WebGPU (jen WASM target).
    BrowserWebgpu,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowModeConfig {
    #[default]
    Windowed,
    BorderlessFullscreen,
    Fullscreen,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentModeConfig {
    /// Driver/OS si vybere podle vsync nastavení.
    #[default]
    Auto,
    /// Vsync ON, double buffering — žádný tearing, max latency.
    Fifo,
    /// Vsync ON, triple buffering — žádný tearing, nižší latency než Fifo.
    Mailbox,
    /// Vsync OFF — možný tearing, nejnižší latency.
    Immediate,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuPriority {
    #[default]
    HighPerformance,
    LowPower,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphicsQuality {
    /// Preset hint pro UI ("low" / "medium" / "high" / "ultra" / "custom").
    /// Phase 3 UI gameplay si může nabídnout "Apply low preset" tlačítko,
    /// které přepíše ostatní pole na low-preset hodnoty.
    pub preset: QualityPreset,

    pub shadow_quality: QualityLevel,
    /// Maximální vzdálenost stínů (metry).
    pub shadow_distance: f32,
    pub texture_quality: QualityLevel,
    /// Anisotropic filtering: 1, 2, 4, 8, 16.
    pub anisotropic_filter: u32,
    pub antialiasing: AntialiasingMode,

    /// Post-process efekty.
    pub bloom: bool,
    pub motion_blur: bool,
    pub chromatic_aberration: bool,
    pub film_grain: bool,
    pub vignette: bool,
    pub depth_of_field: bool,

    pub ssao: bool,
    pub ssr: bool,
    pub volumetric_lighting: QualityLevel,
    pub particles_quality: QualityLevel,
    pub foliage_density: f32,
    pub water_quality: QualityLevel,

    /// View distance multiplier (1.0 = default).
    pub view_distance: f32,
    /// LOD bias: -1.0 = více detailů blíž, +1.0 = LOD swapne dřív (rychlejší).
    pub lod_bias: f32,
    /// Hard cap nejdetailnějšího LOD (0 = nejvyšší, 4 = nejnižší). Default 0.
    pub lod_max_level: u32,
}

impl Default for GraphicsQuality {
    fn default() -> Self {
        Self {
            preset: QualityPreset::High,
            shadow_quality: QualityLevel::High,
            shadow_distance: 80.0,
            texture_quality: QualityLevel::High,
            anisotropic_filter: 8,
            antialiasing: AntialiasingMode::Taa,
            bloom: true,
            motion_blur: false,
            chromatic_aberration: false,
            film_grain: false,
            vignette: true,
            depth_of_field: false,
            ssao: true,
            ssr: false,
            volumetric_lighting: QualityLevel::Medium,
            particles_quality: QualityLevel::High,
            foliage_density: 1.0,
            water_quality: QualityLevel::High,
            view_distance: 1.0,
            lod_bias: 0.0,
            lod_max_level: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QualityPreset {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
    Custom,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QualityLevel {
    Off,
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AntialiasingMode {
    Off,
    Fxaa,
    Smaa,
    #[default]
    Taa,
    Msaa2,
    Msaa4,
    Msaa8,
}

// ---------------------------------------------------------------------------
// [audio]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    /// Master volume (0.0 – 1.0).
    pub master_volume: f32,
    pub music_volume: f32,
    pub sfx_volume: f32,
    pub voice_volume: f32,
    pub ambient_volume: f32,
    pub ui_volume: f32,
    /// Output device hint. `None` ⇒ system default.
    pub output_device: Option<String>,
    /// Spatial audio (HRTF, Phase 3+).
    pub spatial_audio: bool,
    /// Mute všech zvuků v pozadí (alt-tab).
    pub mute_on_focus_lost: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            music_volume: 0.6,
            sfx_volume: 1.0,
            voice_volume: 1.0,
            ambient_volume: 0.8,
            ui_volume: 0.7,
            output_device: None,
            spatial_audio: true,
            mute_on_focus_lost: true,
        }
    }
}

// ---------------------------------------------------------------------------
// [ui]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Jazyk lokalizace ("en", "cs", "de", ...). `auto` = OS default.
    pub language: String,
    /// UI scale (1.0 = 100 %). Pro 4K monitory bývá 1.5–2.0.
    pub ui_scale: f32,
    /// Velikost fontu v pixelech (před UI scale).
    pub font_size: f32,
    pub hud_opacity: f32,
    pub show_fps: bool,
    pub show_ping: bool,
    pub show_minimap: bool,
    pub show_subtitles: bool,
    pub subtitle_size: f32,
    pub chat_size: f32,
    pub crosshair_enabled: bool,
    pub crosshair_color: [u8; 4], // RGBA
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "auto".to_string(),
            ui_scale: 1.0,
            font_size: 16.0,
            hud_opacity: 0.9,
            show_fps: false,
            show_ping: false,
            show_minimap: true,
            show_subtitles: true,
            subtitle_size: 18.0,
            chat_size: 14.0,
            crosshair_enabled: true,
            crosshair_color: [255, 255, 255, 200],
        }
    }
}

// ---------------------------------------------------------------------------
// [input]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct InputConfig {
    pub mouse_sensitivity: f32,
    pub mouse_sensitivity_aim: f32,
    pub mouse_smoothing: f32,
    pub invert_y: bool,
    pub raw_mouse_input: bool,
    pub mouse_acceleration: bool,
    pub gamepad_enabled: bool,
    pub gamepad_deadzone: f32,
    pub gamepad_vibration: bool,
    pub keys: Keybindings,
    pub mouse: MouseBindings,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 1.0,
            mouse_sensitivity_aim: 0.7,
            mouse_smoothing: 0.0,
            invert_y: false,
            raw_mouse_input: true,
            mouse_acceleration: false,
            gamepad_enabled: true,
            gamepad_deadzone: 0.15,
            gamepad_vibration: true,
            keys: Keybindings::default(),
            mouse: MouseBindings::default(),
        }
    }
}

/// Mapování akcí na klávesy. Variantní jména `KeyCode` (např. `"KeyW"`,
/// `"Space"`, `"ArrowLeft"`, `"F12"`) odpovídají variantám
/// [`bevy::input::keyboard::KeyCode`] — TOML serde derive je serializuje
/// právě takhle.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keybindings {
    // Pohyb
    pub move_forward: KeyCode,
    pub move_backward: KeyCode,
    pub move_left: KeyCode,
    pub move_right: KeyCode,
    pub jump: KeyCode,
    pub crouch: KeyCode,
    pub prone: KeyCode,
    pub sprint: KeyCode,
    pub walk: KeyCode,

    // Souboj / útoky
    pub reload: KeyCode,
    pub melee: KeyCode,
    pub throw_grenade: KeyCode,
    pub sheathe_weapon: KeyCode,
    pub use_item: KeyCode,
    pub aim: KeyCode,

    // Sloty zbraní 1–9
    pub weapon_1: KeyCode,
    pub weapon_2: KeyCode,
    pub weapon_3: KeyCode,
    pub weapon_4: KeyCode,
    pub weapon_5: KeyCode,
    pub weapon_6: KeyCode,
    pub weapon_7: KeyCode,
    pub weapon_8: KeyCode,
    pub weapon_9: KeyCode,

    // Interakce / UI
    pub interact: KeyCode,
    pub inventory: KeyCode,
    pub map: KeyCode,
    pub character: KeyCode,
    pub journal: KeyCode,
    pub chat: KeyCode,
    pub voice_chat: KeyCode,
    pub scoreboard: KeyCode,
    pub options_menu: KeyCode,

    // Kamera
    pub toggle_camera: KeyCode,
    pub zoom_toggle: KeyCode,

    // Misc
    pub toggle_hud: KeyCode,
    pub screenshot: KeyCode,
    pub push_to_talk: KeyCode,
    pub developer_console: KeyCode,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            move_forward: KeyCode::KeyW,
            move_backward: KeyCode::KeyS,
            move_left: KeyCode::KeyA,
            move_right: KeyCode::KeyD,
            jump: KeyCode::Space,
            crouch: KeyCode::ControlLeft,
            prone: KeyCode::KeyZ,
            sprint: KeyCode::ShiftLeft,
            walk: KeyCode::AltLeft,

            reload: KeyCode::KeyR,
            melee: KeyCode::KeyF,
            throw_grenade: KeyCode::KeyG,
            sheathe_weapon: KeyCode::KeyX,
            use_item: KeyCode::KeyQ,
            aim: KeyCode::KeyT,

            weapon_1: KeyCode::Digit1,
            weapon_2: KeyCode::Digit2,
            weapon_3: KeyCode::Digit3,
            weapon_4: KeyCode::Digit4,
            weapon_5: KeyCode::Digit5,
            weapon_6: KeyCode::Digit6,
            weapon_7: KeyCode::Digit7,
            weapon_8: KeyCode::Digit8,
            weapon_9: KeyCode::Digit9,

            interact: KeyCode::KeyE,
            inventory: KeyCode::KeyI,
            map: KeyCode::KeyM,
            character: KeyCode::KeyC,
            journal: KeyCode::KeyJ,
            chat: KeyCode::Enter,
            voice_chat: KeyCode::KeyV,
            scoreboard: KeyCode::Tab,
            options_menu: KeyCode::Escape,

            toggle_camera: KeyCode::F5,
            zoom_toggle: KeyCode::KeyZ,

            toggle_hud: KeyCode::KeyH,
            screenshot: KeyCode::F12,
            push_to_talk: KeyCode::KeyB,
            developer_console: KeyCode::Backquote,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MouseBindings {
    pub attack_primary: MouseButton,
    pub attack_secondary: MouseButton,
    pub middle_click: MouseButton,
}

impl Default for MouseBindings {
    fn default() -> Self {
        Self {
            attack_primary: MouseButton::Left,
            attack_secondary: MouseButton::Right,
            middle_click: MouseButton::Middle,
        }
    }
}

// ---------------------------------------------------------------------------
// [paths]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PathsConfig {
    /// Cache root (resource download). `None` ⇒ použijeme platform cache dir
    /// (`%LOCALAPPDATA%\bevy_game\cache\resources` na Windows).
    pub cache_dir: Option<PathBuf>,
    /// Adresář pro screenshoty (F12). `None` ⇒ `<config>/screenshots`.
    pub screenshot_dir: Option<PathBuf>,
    /// Save game directory. `None` ⇒ `<config>/saves`.
    pub savegame_dir: Option<PathBuf>,
    /// User-installed mods. `None` ⇒ `<config>/mods`.
    pub mod_dir: Option<PathBuf>,
    /// Logy klienta. `None` ⇒ `<config>/logs`.
    pub log_dir: Option<PathBuf>,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            cache_dir: None,
            screenshot_dir: None,
            savegame_dir: None,
            mod_dir: None,
            log_dir: None,
        }
    }
}

// ---------------------------------------------------------------------------
// [server_history]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerHistoryConfig {
    pub last_used_username: Option<String>,
    pub recent_servers: Vec<ServerEntry>,
    pub favorites: Vec<ServerEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerEntry {
    pub address: String,
    pub label: Option<String>,
    /// ISO 8601 timestamp posledního připojení (Phase 3+ vyplní automaticky).
    pub last_connected: Option<String>,
}

// ---------------------------------------------------------------------------
// [advanced]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdvancedConfig {
    /// Tracing log level (trace / debug / info / warn / error).
    pub log_level: LogLevel,
    /// Per-modul log filter (RUST_LOG syntax).
    pub log_filter: String,
    /// F3-overlay s FPS / draw calls / chunk count.
    pub show_debug_overlay: bool,
    /// Render collision meshes (developer asset audit).
    pub show_collision_meshes: bool,
    /// Profile overlay (Tracy-style timeline).
    pub profile_overlay: bool,
    /// Network statistics overlay (RTT, packet loss).
    pub network_stats_overlay: bool,
    /// Developer console (~ key by default).
    pub developer_console_enabled: bool,
    /// Předgenerovat sandboxy ihned po startu, místo lazy.
    pub preload_resources: bool,
    /// wgpu validation layer — pomalejší, ale chytá GPU bugy.
    pub gpu_validation: bool,
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            log_level: LogLevel::Info,
            log_filter: "wgpu=warn,naga=warn".to_string(),
            show_debug_overlay: false,
            show_collision_meshes: false,
            profile_overlay: false,
            network_stats_overlay: false,
            developer_console_enabled: cfg!(debug_assertions),
            preload_resources: true,
            gpu_validation: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
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
// Bevy resource wrappers
// ---------------------------------------------------------------------------

/// Resource s plnou parsed konfigurací — gameplay code si může číst všechno
/// (např. `cfg.input.keys.jump`).
#[derive(Resource, Debug, Clone)]
pub struct ClientConfigResource(pub ClientConfig);

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not determine user config directory (set BEVY_GAME_CLIENT_CONFIG to override)")]
    NoConfigDir,
    #[error("io error at {path:?}: {source}")]
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
    #[error("toml serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl ClientConfig {
    /// Resolve cestu ke configu v pořadí: CLI arg #1 → env → platform default.
    pub fn resolve_path() -> Result<PathBuf, ConfigError> {
        if let Some(arg) = std::env::args().nth(1) {
            return Ok(PathBuf::from(arg));
        }
        if let Ok(env_path) = std::env::var(ENV_OVERRIDE) {
            if !env_path.is_empty() {
                return Ok(PathBuf::from(env_path));
            }
        }
        let dir = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
        Ok(dir.join(APP_DIR_NAME).join(CONFIG_FILE_NAME))
    }

    /// Load existing config, nebo vygeneruj defaultní soubor a vrať defaulty.
    /// Když config existuje a parsuje, ale chybí nějaká pole, doplníme defaulty
    /// (`#[serde(default)]`).
    pub fn load_or_create(path: &Path) -> Self {
        match Self::load(path) {
            Ok(cfg) => {
                eprintln!("[host_client] loaded config from {}", path.display());
                cfg
            }
            Err(ConfigError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                let default = Self::default();
                match default.save(path) {
                    Ok(()) => eprintln!(
                        "[host_client] generated fresh config at {} — edit and restart to apply",
                        path.display()
                    ),
                    Err(e) => eprintln!(
                        "[host_client] WARNING: failed to write default config to {}: {} \
                         (running with compiled defaults; settings will not persist)",
                        path.display(),
                        e
                    ),
                }
                default
            }
            Err(e) => {
                eprintln!(
                    "[host_client] config load failed ({e}) — running with defaults. \
                     Fix or delete {} to regenerate.",
                    path.display()
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

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let body = toml::to_string_pretty(self)?;
        let mut full = String::with_capacity(HEADER_COMMENT.len() + body.len());
        full.push_str(HEADER_COMMENT);
        full.push_str(&body);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        std::fs::write(path, full.as_bytes()).map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

impl PathsConfig {
    /// Resolve cache dir: explicit override → platform `cache_dir/bevy_game/cache/resources`.
    pub fn resolve_cache_dir(&self) -> PathBuf {
        if let Some(p) = &self.cache_dir {
            return p.clone();
        }
        dirs::cache_dir()
            .map(|d| d.join(APP_DIR_NAME).join("cache").join("resources"))
            .unwrap_or_else(|| PathBuf::from("cache/resources"))
    }
}
