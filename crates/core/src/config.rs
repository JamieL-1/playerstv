use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{self, GameProfile};

/// Output resolution. `Native` keeps the captured window size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    Native,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "2160p")]
    P2160,
    /// Exactly twice the window size with nearest-neighbour scaling: every
    /// game pixel becomes a 2×2 block, so even 4:2:0 video keeps full colour
    /// detail and YouTube serves it at its higher 4K bitrate.
    #[serde(rename = "2x")]
    Double,
}

impl Resolution {
    pub const ALL: [Resolution; 6] = [
        Resolution::Native,
        Resolution::Double,
        Resolution::P720,
        Resolution::P1080,
        Resolution::P1440,
        Resolution::P2160,
    ];

    /// Fixed output height, or `None` for sizes derived from the window.
    pub fn height(self) -> Option<u32> {
        match self {
            Resolution::Native | Resolution::Double => None,
            Resolution::P720 => Some(720),
            Resolution::P1080 => Some(1080),
            Resolution::P1440 => Some(1440),
            Resolution::P2160 => Some(2160),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Resolution::Native => "Native (window size)",
            Resolution::Double => "2× pixel-perfect (sharpest, bigger files)",
            Resolution::P720 => "720p",
            Resolution::P1080 => "1080p",
            Resolution::P1440 => "1440p",
            Resolution::P2160 => "4K (2160p)",
        }
    }
}

/// Visual quality, best last. Maps to an NVENC constant-quality value and an
/// automatic bitrate cap, so there's no "lower number is better" trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Low,
    Medium,
    High,
    VeryHigh,
    Max,
}

impl Quality {
    pub const ALL: [Quality; 5] = [Quality::Low, Quality::Medium, Quality::High, Quality::VeryHigh, Quality::Max];

    pub fn label(self) -> &'static str {
        match self {
            Quality::Low => "Low (small files)",
            Quality::Medium => "Medium",
            Quality::High => "High",
            Quality::VeryHigh => "Very high",
            Quality::Max => "Maximum (big files)",
        }
    }

    /// Short form for summaries, e.g. "Very high".
    pub fn short_name(self) -> &'static str {
        ["Low", "Medium", "High", "Very high", "Maximum"][self as usize]
    }

    /// NVENC `-cq` for this level. AV1 uses a 0–63 scale, H.264/HEVC 0–51.
    pub fn cq(self, codec: Codec) -> u32 {
        let i = self as usize;
        match codec {
            Codec::H264 | Codec::Hevc => [28, 25, 22, 19, 16][i],
            Codec::Av1 => [40, 36, 32, 28, 24][i],
        }
    }

    /// Peak bits per pixel per frame for the automatic bitrate cap.
    pub fn cap_bits_per_pixel(self) -> f64 {
        [0.05, 0.08, 0.12, 0.18, 0.28][self as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub const ALL: [Codec; 3] = [Codec::H264, Codec::Hevc, Codec::Av1];

    pub fn nvenc_encoder(self) -> &'static str {
        match self {
            Codec::H264 => "h264_nvenc",
            Codec::Hevc => "hevc_nvenc",
            Codec::Av1 => "av1_nvenc",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264 (most compatible)",
            Codec::Hevc => "HEVC / H.265 (smaller files)",
            Codec::Av1 => "AV1 (smallest, RTX 40+)",
        }
    }

    /// Largest frame width/height NVENC accepts for this codec.
    pub fn max_dimension(self) -> u32 {
        match self {
            Codec::H264 => 4096,
            Codec::Hevc | Codec::Av1 => 8192,
        }
    }
}

pub const FPS_CHOICES: [u32; 4] = [30, 60, 90, 120];

// Older configs had `cq` and `max_bitrate_kbps` here (replaced by `quality`
// and `bitrate_cap_mbps`) and `full_chroma` (4:4:4, removed: the app's
// player can't play it). Unknown keys are skipped on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    pub resolution: Resolution,
    pub fps: u32,
    pub codec: Codec,
    pub quality: Quality,
    /// Peak bitrate cap; `None` picks one from quality, size and frame rate.
    pub bitrate_cap_mbps: Option<u32>,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            resolution: Resolution::Native,
            fps: 60,
            codec: Codec::H264,
            quality: Quality::High,
            bitrate_cap_mbps: None,
        }
    }
}

/// What the non-mic audio track records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSourceKind {
    /// Only the game process (no Discord, music, browser…).
    Game,
    /// Everything playing on the default output device.
    Desktop,
}

impl AudioSourceKind {
    pub const ALL: [AudioSourceKind; 2] = [AudioSourceKind::Game, AudioSourceKind::Desktop];

    /// Track title in saved files.
    pub fn track_title(self) -> &'static str {
        match self {
            AudioSourceKind::Game => "Game",
            AudioSourceKind::Desktop => "Desktop",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioSourceKind::Game => "Only the game",
            AudioSourceKind::Desktop => "All desktop audio (music, Discord, …)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Record game/desktop audio at all.
    pub game_audio: bool,
    pub source: AudioSourceKind,
    pub mic: bool,
    /// Friendly name of the capture device; `None` = Windows default mic.
    pub mic_device: Option<String>,
    /// Volume of each source in the mixed track, in percent. The separate
    /// per-source tracks are always recorded untouched.
    pub game_volume: u32,
    pub mic_volume: u32,
    /// With both sources on, add a first track mixing them, so players and
    /// YouTube (which only play the first track) hear everything.
    pub mixed_track: bool,
    pub bitrate_kbps: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            game_audio: true,
            source: AudioSourceKind::Game,
            mic: true,
            mic_device: None,
            game_volume: 100,
            mic_volume: 100,
            mixed_track: true,
            bitrate_kbps: 160,
        }
    }
}

/// A per-game bitrate cap: automatic, or a fixed peak. Written as `"auto"`
/// or a number, so "not overridden" can simply be a missing key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitrateCap {
    Auto,
    Mbps(u32),
}

impl BitrateCap {
    /// The [`VideoConfig::bitrate_cap_mbps`] this stands for.
    pub fn mbps(self) -> Option<u32> {
        match self {
            BitrateCap::Auto => None,
            BitrateCap::Mbps(m) => Some(m),
        }
    }
}

impl Serialize for BitrateCap {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            BitrateCap::Auto => s.serialize_str("auto"),
            BitrateCap::Mbps(m) => s.serialize_u32(*m),
        }
    }
}

impl<'de> Deserialize<'de> for BitrateCap {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Mbps(u32),
            Text(String),
        }
        match Repr::deserialize(d)? {
            Repr::Mbps(m) => Ok(BitrateCap::Mbps(m)),
            Repr::Text(t) if t == "auto" => Ok(BitrateCap::Auto),
            Repr::Text(t) => {
                Err(serde::de::Error::custom(format!("bitrate cap must be \"auto\" or a number, not {t:?}")))
            }
        }
    }
}

/// Per-game replacements for [`VideoConfig`] fields; `None` follows the
/// global setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec: Option<Codec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Quality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate_cap: Option<BitrateCap>,
}

impl VideoOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn apply(&self, v: &mut VideoConfig) {
        v.resolution = self.resolution.unwrap_or(v.resolution);
        v.fps = self.fps.unwrap_or(v.fps);
        v.codec = self.codec.unwrap_or(v.codec);
        v.quality = self.quality.unwrap_or(v.quality);
        if let Some(cap) = self.bitrate_cap {
            v.bitrate_cap_mbps = cap.mbps();
        }
    }
}

/// Per-game replacements for [`AudioConfig`] fields. The mic device and
/// audio bitrate stay global: they describe the PC, not the game.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_audio: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<AudioSourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_volume: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_volume: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mixed_track: Option<bool>,
}

impl AudioOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn apply(&self, a: &mut AudioConfig) {
        a.game_audio = self.game_audio.unwrap_or(a.game_audio);
        a.source = self.source.unwrap_or(a.source);
        a.mic = self.mic.unwrap_or(a.mic);
        a.game_volume = self.game_volume.unwrap_or(a.game_volume);
        a.mic_volume = self.mic_volume.unwrap_or(a.mic_volume);
        a.mixed_track = self.mixed_track.unwrap_or(a.mixed_track);
    }
}

/// A game's own recording settings. Anything left unset uses the global
/// setting, so changing a global still reaches every game that didn't
/// override it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_full_session: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_seconds: Option<u32>,
    #[serde(skip_serializing_if = "VideoOverrides::is_empty")]
    pub video: VideoOverrides,
    #[serde(skip_serializing_if = "AudioOverrides::is_empty")]
    pub audio: AudioOverrides,
}

impl GameOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Same ranges as the global settings.
    pub fn sanitize(&mut self) {
        self.buffer_seconds = self.buffer_seconds.map(clamp_buffer_seconds);
        self.video.fps = self.video.fps.map(clamp_fps);
        if let Some(BitrateCap::Mbps(m)) = &mut self.video.bitrate_cap {
            *m = clamp_bitrate_cap(*m);
        }
        self.audio.game_volume = self.audio.game_volume.map(|v| v.min(MAX_VOLUME_PERCENT));
        self.audio.mic_volume = self.audio.mic_volume.map(|v| v.min(MAX_VOLUME_PERCENT));
    }
}

fn clamp_buffer_seconds(s: u32) -> u32 {
    s.clamp(MIN_BUFFER_SECONDS, MAX_BUFFER_SECONDS)
}

fn clamp_fps(fps: u32) -> u32 {
    fps.clamp(10, 120)
}

fn clamp_bitrate_cap(mbps: u32) -> u32 {
    mbps.clamp(*BITRATE_CAP_MBPS.start(), *BITRATE_CAP_MBPS.end())
}

/// Base colour scheme of the window (Discord-style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BaseTheme {
    #[default]
    Dark,
    Ash,
    Onyx,
    Light,
}

/// A two-colour background gradient (Nitro-style themes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gradient {
    /// Preset id, or "custom".
    pub name: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: BaseTheme,
    /// Accent colour as `#rrggbb`.
    pub accent: String,
    /// Background gradient; `None` = the plain base theme.
    pub gradient: Option<Gradient>,
}

pub const DEFAULT_ACCENT: &str = "#5865f2";

impl Default for UiConfig {
    fn default() -> Self {
        Self { theme: BaseTheme::Dark, accent: DEFAULT_ACCENT.into(), gradient: None }
    }
}

/// `#rrggbb` (any case), the only colour format the UI writes.
pub fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

impl UiConfig {
    /// Replace anything a hand-edited config got wrong with defaults, so the
    /// UI never has to cope with a malformed colour.
    pub fn sanitize(&mut self) {
        if !is_hex_colour(&self.accent) {
            self.accent = DEFAULT_ACCENT.into();
        }
        self.accent.make_ascii_lowercase();
        if let Some(g) = &mut self.gradient {
            if !is_hex_colour(&g.from) || !is_hex_colour(&g.to) {
                self.gradient = None;
            } else {
                g.from.make_ascii_lowercase();
                g.to.make_ascii_lowercase();
            }
        }
    }
}

pub const MAX_VOLUME_PERCENT: u32 = 400;
pub const MAX_STORAGE_LIMIT_GB: u32 = 20_000;
// Shared with the settings UI so its sliders accept every value a config can
// hold (a narrower slider would silently clamp the value just by drawing).
pub const STORAGE_LIMIT_GB: std::ops::RangeInclusive<u32> = 1..=MAX_STORAGE_LIMIT_GB;
pub const BITRATE_CAP_MBPS: std::ops::RangeInclusive<u32> = 2..=400;
pub const AUDIO_BITRATE_KBPS: std::ops::RangeInclusive<u32> = 64..=320;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Seconds of gameplay kept in the replay buffer / saved per clip.
    pub buffer_seconds: u32,
    /// Global hotkey, e.g. "Alt+F10".
    pub clip_hotkey: String,
    /// Global hotkey to toggle full-session recording.
    pub session_hotkey: String,
    /// Where clips and sessions are written.
    pub output_dir: PathBuf,
    /// Record the whole session automatically whenever a game is detected.
    pub record_full_session: bool,
    /// Launch playerstv when Windows starts.
    pub start_with_windows: bool,
    /// Show a toast when a clip is saved.
    pub notifications: bool,
    /// Keep saved clips + sessions under this many GB by deleting the oldest.
    /// `None` = no limit.
    pub storage_limit_gb: Option<u32>,
    pub video: VideoConfig,
    pub audio: AudioConfig,
    pub games: Vec<GameProfile>,
    /// Look of the playerstv window. Only the UI reads this; the tray ignores it.
    pub ui: UiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            buffer_seconds: 30,
            clip_hotkey: "Alt+F10".into(),
            session_hotkey: "Alt+F9".into(),
            output_dir: default_output_dir(),
            record_full_session: false,
            start_with_windows: false,
            notifications: true,
            storage_limit_gb: None,
            video: VideoConfig::default(),
            audio: AudioConfig::default(),
            games: profile::builtins(),
            ui: UiConfig::default(),
        }
    }
}

pub const MIN_BUFFER_SECONDS: u32 = 5;
pub const MAX_BUFFER_SECONDS: u32 = 600;

impl Config {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Self::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.sanitize();
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        // Write-then-rename so a watcher never sees a half-written file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Clamp values a hand-edited config might get wrong.
    pub fn sanitize(&mut self) {
        self.buffer_seconds = clamp_buffer_seconds(self.buffer_seconds);
        self.video.fps = clamp_fps(self.video.fps);
        self.video.bitrate_cap_mbps = self.video.bitrate_cap_mbps.map(clamp_bitrate_cap);
        self.audio.bitrate_kbps = self.audio.bitrate_kbps.clamp(*AUDIO_BITRATE_KBPS.start(), *AUDIO_BITRATE_KBPS.end());
        self.audio.game_volume = self.audio.game_volume.min(MAX_VOLUME_PERCENT);
        self.audio.mic_volume = self.audio.mic_volume.min(MAX_VOLUME_PERCENT);
        profile::merge_builtins(&mut self.games);
        for g in &mut self.games {
            g.overrides.sanitize();
        }
        self.ui.sanitize();
        self.storage_limit_gb =
            self.storage_limit_gb.map(|g| g.clamp(*STORAGE_LIMIT_GB.start(), *STORAGE_LIMIT_GB.end()));
    }

    pub fn storage_limit_bytes(&self) -> Option<u64> {
        self.storage_limit_gb.map(|g| g as u64 * crate::storage::BYTES_PER_GB)
    }

    /// True if a change from `old` to `self` requires restarting the encoder.
    /// Game list changes don't count: toggling another game mustn't restart
    /// (and split) the recording of the one being played. Compare the
    /// [`Self::for_game`] views to include that game's own overrides.
    pub fn capture_changed(&self, old: &Config) -> bool {
        self.video != old.video || self.audio != old.audio
    }

    /// The settings a game actually records with: its overrides on top of
    /// the global settings. Unknown ids get the global settings.
    pub fn for_game(&self, id: &str) -> Config {
        let mut cfg = self.clone();
        let Some(o) = self.games.iter().find(|g| g.id == id).map(|g| &g.overrides) else { return cfg };
        cfg.record_full_session = o.record_full_session.unwrap_or(cfg.record_full_session);
        cfg.buffer_seconds = o.buffer_seconds.map_or(cfg.buffer_seconds, clamp_buffer_seconds);
        o.video.apply(&mut cfg.video);
        o.audio.apply(&mut cfg.audio);
        // Overrides from a config that skipped sanitize() still get clamped.
        cfg.video.fps = clamp_fps(cfg.video.fps);
        cfg.video.bitrate_cap_mbps = cfg.video.bitrate_cap_mbps.map(clamp_bitrate_cap);
        cfg.audio.game_volume = cfg.audio.game_volume.min(MAX_VOLUME_PERCENT);
        cfg.audio.mic_volume = cfg.audio.mic_volume.min(MAX_VOLUME_PERCENT);
        cfg
    }

    pub fn game_enabled(&self, id: &str) -> bool {
        self.games.iter().any(|g| g.id == id && g.enabled)
    }

    /// Turns a game's auto-capture on or off. Returns false if no such game.
    pub fn set_game_enabled(&mut self, id: &str, enabled: bool) -> bool {
        match self.games.iter_mut().find(|g| g.id == id) {
            Some(g) => {
                g.enabled = enabled;
                true
            }
            None => false,
        }
    }
}

fn default_output_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Videos")
        .join("playerstv")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_default() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let cfg: Config = toml::from_str("buffer_seconds = 45\n[video]\nfps = 120\n").unwrap();
        assert_eq!(cfg.buffer_seconds, 45);
        assert_eq!(cfg.video.fps, 120);
        assert_eq!(cfg.video.codec, Codec::H264);
        assert_eq!(cfg.games.len(), 3);
    }

    #[test]
    fn toggling_games() {
        let mut cfg = Config::default();
        assert!(cfg.game_enabled(profile::CS2));
        assert!(cfg.set_game_enabled(profile::CS2, false));
        assert!(!cfg.game_enabled(profile::CS2));
        assert!(!cfg.set_game_enabled("nope", false));
        // Toggling a game never restarts the encoder for the current one.
        assert!(!cfg.capture_changed(&Config::default()));
        // The toggle survives a save/load round trip.
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert!(!back.game_enabled(profile::CS2));
    }

    #[test]
    fn old_cq_settings_are_dropped() {
        // The old "Quality (CQ)" slider dragged to the max meant cq = 35: worst.
        let cfg: Config = toml::from_str("[video]\nfps = 120\ncq = 35\nmax_bitrate_kbps = 20000\n").unwrap();
        assert_eq!(cfg.video.quality, Quality::High);
        assert_eq!(cfg.video.bitrate_cap_mbps, None);
        assert_eq!(cfg.video.fps, 120);
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(!text.contains("cq"));
    }

    #[test]
    fn ui_theme_defaults_and_sanitizing() {
        // Old configs have no [ui] section.
        let cfg: Config = toml::from_str("buffer_seconds = 30\n").unwrap();
        assert_eq!(cfg.ui, UiConfig::default());
        let mut cfg: Config = toml::from_str(
            "[ui]\ntheme = \"onyx\"\naccent = \"#FF00AA\"\n[ui.gradient]\nname = \"sunset\"\nfrom = \"#ff7a59\"\nto = \"#8a2be2\"\n",
        )
        .unwrap();
        cfg.sanitize();
        assert_eq!(cfg.ui.theme, BaseTheme::Onyx);
        assert_eq!(cfg.ui.accent, "#ff00aa");
        assert_eq!(cfg.ui.gradient.as_ref().unwrap().to, "#8a2be2");
        // Bad colours fall back instead of reaching the UI.
        cfg.ui.accent = "red".into();
        cfg.ui.gradient.as_mut().unwrap().from = "#12345".into();
        cfg.sanitize();
        assert_eq!(cfg.ui.accent, DEFAULT_ACCENT);
        assert_eq!(cfg.ui.gradient, None);
        // Round trip.
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.ui, cfg.ui);
    }

    #[test]
    fn old_full_chroma_setting_still_loads() {
        // 4:4:4 was removed (the app's player can't play it); configs that
        // had it switched on must still load, and drop it on save.
        let cfg: Config = toml::from_str("[video]\ncodec = \"hevc\"\nfull_chroma = true\n").unwrap();
        assert_eq!(cfg.video.codec, Codec::Hevc);
        assert!(!toml::to_string_pretty(&cfg).unwrap().contains("full_chroma"));
    }

    #[test]
    fn quality_maps_best_last() {
        for codec in Codec::ALL {
            let cqs: Vec<u32> = Quality::ALL.iter().map(|q| q.cq(codec)).collect();
            assert!(cqs.windows(2).all(|w| w[0] > w[1]), "{codec:?}: {cqs:?}");
        }
        let cfg: Config = toml::from_str("[video]\nquality = \"very_high\"\nresolution = \"2x\"\n").unwrap();
        assert_eq!(cfg.video.quality, Quality::VeryHigh);
        assert_eq!(cfg.video.resolution, Resolution::Double);
    }

    #[test]
    fn older_config_gets_new_audio_defaults() {
        // A config written before `source`/volumes/mixed_track existed.
        let cfg: Config = toml::from_str("[audio]\ngame_audio = true\nmic = true\nbitrate_kbps = 160\n").unwrap();
        assert_eq!(cfg.audio.source, AudioSourceKind::Game);
        assert_eq!(cfg.audio.game_volume, 100);
        assert!(cfg.audio.mixed_track);
        let cfg: Config = toml::from_str("[audio]\nsource = \"desktop\"\n").unwrap();
        assert_eq!(cfg.audio.source, AudioSourceKind::Desktop);
    }

    #[test]
    fn sanitize_clamps() {
        let mut cfg = Config { buffer_seconds: 1, ..Default::default() };
        cfg.video.fps = 500;
        cfg.games.clear();
        cfg.sanitize();
        assert_eq!(cfg.buffer_seconds, MIN_BUFFER_SECONDS);
        assert_eq!(cfg.video.fps, 120);
        assert_eq!(cfg.games.len(), 3);
    }

    fn game_mut<'a>(cfg: &'a mut Config, id: &str) -> &'a mut GameOverrides {
        &mut cfg.games.iter_mut().find(|g| g.id == id).unwrap().overrides
    }

    #[test]
    fn configs_without_overrides_stay_clean() {
        let cfg: Config = toml::from_str("buffer_seconds = 30\n").unwrap();
        assert!(cfg.games.iter().all(|g| g.overrides.is_empty()));
        assert!(!toml::to_string_pretty(&cfg).unwrap().contains("overrides"));
    }

    #[test]
    fn overrides_round_trip() {
        let mut cfg = Config::default();
        let o = game_mut(&mut cfg, profile::LEAGUE);
        o.record_full_session = Some(true);
        o.video.fps = Some(120);
        o.video.bitrate_cap = Some(BitrateCap::Mbps(80));
        o.audio.source = Some(AudioSourceKind::Desktop);
        game_mut(&mut cfg, profile::RUNELITE).video.bitrate_cap = Some(BitrateCap::Auto);
        let text = toml::to_string_pretty(&cfg).unwrap();
        let mut back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, cfg, "{text}");
        // Survives the load-time clean-up too (built-ins are rebuilt there).
        back.sanitize();
        assert_eq!(back, cfg);
        // Only what's set is written.
        assert!(text.contains("[games.overrides.audio]\nsource = \"desktop\"\n\n[[games]]"), "{text}");
    }

    #[test]
    fn bitrate_cap_formats() {
        let o: VideoOverrides = toml::from_str("bitrate_cap = \"auto\"").unwrap();
        assert_eq!(o.bitrate_cap, Some(BitrateCap::Auto));
        let o: VideoOverrides = toml::from_str("bitrate_cap = 50").unwrap();
        assert_eq!(o.bitrate_cap, Some(BitrateCap::Mbps(50)));
        assert!(toml::from_str::<VideoOverrides>("bitrate_cap = \"fast\"").is_err());
    }

    #[test]
    fn for_game_layers_overrides_on_globals() {
        let mut cfg = Config { buffer_seconds: 45, ..Default::default() };
        cfg.video.bitrate_cap_mbps = Some(40);
        let o = game_mut(&mut cfg, profile::LEAGUE);
        o.record_full_session = Some(true);
        o.video.fps = Some(120);
        o.video.bitrate_cap = Some(BitrateCap::Auto);
        o.audio.mic = Some(false);

        let league = cfg.for_game(profile::LEAGUE);
        assert!(league.record_full_session);
        assert_eq!(league.buffer_seconds, 45, "not overridden: global");
        assert_eq!(league.video.fps, 120);
        assert_eq!(league.video.codec, cfg.video.codec);
        assert_eq!(league.video.bitrate_cap_mbps, None, "auto overrides a fixed global cap");
        assert!(!league.audio.mic);
        assert_eq!(league.audio.game_volume, cfg.audio.game_volume);

        assert_eq!(cfg.for_game(profile::RUNELITE), cfg);
        assert_eq!(cfg.for_game("nope"), cfg);
    }

    #[test]
    fn overrides_are_clamped() {
        let mut cfg = Config::default();
        let o = game_mut(&mut cfg, profile::CS2);
        o.buffer_seconds = Some(1);
        o.video.fps = Some(500);
        o.video.bitrate_cap = Some(BitrateCap::Mbps(9_999));
        o.audio.mic_volume = Some(9_999);
        // Applied even before sanitize()…
        let cs2 = cfg.for_game(profile::CS2);
        assert_eq!((cs2.buffer_seconds, cs2.video.fps), (MIN_BUFFER_SECONDS, 120));
        // …and stored clamped after it.
        cfg.sanitize();
        let o = &game_mut(&mut cfg, profile::CS2);
        assert_eq!(o.video.bitrate_cap, Some(BitrateCap::Mbps(*BITRATE_CAP_MBPS.end())));
        assert_eq!(o.audio.mic_volume, Some(MAX_VOLUME_PERCENT));
    }

    #[test]
    fn only_the_active_games_overrides_restart_it() {
        let old = Config::default();
        let mut new = old.clone();
        game_mut(&mut new, profile::CS2).video.fps = Some(120);
        let changed = |id| new.for_game(id).capture_changed(&old.for_game(id));
        assert!(changed(profile::CS2));
        assert!(!changed(profile::LEAGUE), "another game's settings don't split this recording");
    }
}
