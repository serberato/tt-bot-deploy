use serde::{Deserialize, Serialize};

use crate::services::Service;

/// Check whether a string is a recognised gender alias.
pub fn is_valid_gender(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "male" | "m" | "man" | "female" | "f" | "woman" | "neutral" | "n" | "nb"
    )
}

/// Parse a gender string into a TeamTalk UserGender.
/// Accepts: male/m/man, female/f/woman, neutral/n/nb (and anything else defaults to Neutral).
pub fn parse_gender(s: &str) -> ::teamtalk::types::UserGender {
    match s.to_lowercase().as_str() {
        "male" | "m" | "man" => ::teamtalk::types::UserGender::Male,
        "female" | "f" | "woman" => ::teamtalk::types::UserGender::Female,
        _ => ::teamtalk::types::UserGender::Neutral,
    }
}

/// How the bot decides who may run admin-gated commands.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AdminMode {
    /// No gating; every user may run every command (opt-out / legacy behavior).
    Everyone,
    /// Only TeamTalk server admins (account marked admin on the server).
    TtRights,
    /// Only usernames in the `admins` list.
    List,
    /// A TeamTalk server admin OR a username in the `admins` list.
    #[default]
    Both,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum PlayMode {
    #[default]
    Queue,
    Direct,
}

fn default_radio_delay() -> f32 { 10.0 }
fn default_norm_type() -> String { "auto".to_string() }
fn default_norm_method() -> String { "dynamic".to_string() }
fn default_norm_pregain() -> f64 { 0.0 }
fn default_norm_threshold() -> f64 { -2.0 }
fn default_norm_knee() -> f64 { 5.0 }
fn default_language_en() -> String { "en".to_string() }

/// Config format matches the Python ttspotify bot's data/config.json
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BotConfig {
    // TeamTalk connection
    pub host: String,
    #[serde(rename = "tcpPort")]
    pub tcp_port: i32,
    #[serde(rename = "udpPort")]
    pub udp_port: i32,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(rename = "botName")]
    pub bot_name: String,
    pub username: String,
    pub password: String,
    #[serde(rename = "ChannelName")]
    pub channel_name: String,
    #[serde(rename = "ChannelPassword")]
    pub channel_password: String,
    #[serde(rename = "botGender")]
    pub bot_gender: String,
    #[serde(default, rename = "adminMode")]
    pub admin_mode: AdminMode,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default = "default_language_en", rename = "defaultLanguage")]
    pub default_language: String,

    // TeamTalk license (optional, overridden by compile-time TT_LICENSE_NAME/TT_LICENSE_KEY)
    #[serde(default, rename = "licenseName", skip_serializing_if = "Option::is_none")]
    pub license_name: Option<String>,
    #[serde(default, rename = "licenseKey", skip_serializing_if = "Option::is_none")]
    pub license_key: Option<String>,

    // Spotify
    #[serde(rename = "spotifyQuality")]
    pub spotify_quality: String,
    #[serde(rename = "spotifyEnableNormalization")]
    pub spotify_enable_normalization: bool,
    #[serde(rename = "spotifyNormalisationType", default = "default_norm_type")]
    pub normalisation_type: String,
    #[serde(rename = "spotifyNormalisationMethod", default = "default_norm_method")]
    pub normalisation_method: String,
    #[serde(rename = "spotifyNormalisationPregainDb", default = "default_norm_pregain")]
    pub normalisation_pregain_db: f64,
    #[serde(rename = "spotifyNormalisationThresholdDbfs", default = "default_norm_threshold")]
    pub normalisation_threshold_dbfs: f64,
    #[serde(rename = "spotifyNormalisationKneeDb", default = "default_norm_knee")]
    pub normalisation_knee_db: f64,

    // Audio
    pub volume: u8,
    #[serde(rename = "spotifyMaxVolume")]
    pub max_volume: u8,
    #[serde(rename = "spotifyJitterBufferSizeMs")]
    pub jitter_buffer_ms: u32,
    #[serde(rename = "spotifyVolumeRampStep")]
    pub volume_ramp_step: f32,

    // Radio/recommendations
    #[serde(rename = "spotifyRadio")]
    pub radio_enabled: bool,
    #[serde(rename = "spotifyRadioBatch")]
    pub radio_batch_size: u8,
    #[serde(rename = "spotifyRadioDelay", default = "default_radio_delay")]
    pub radio_delay: f32,

    // Search
    #[serde(rename = "spotifySearchLimit")]
    pub search_limit: u8,

    // Playback modes (persisted across restarts)
    #[serde(default, rename = "repeatTrack")]
    pub repeat_track: bool,
    #[serde(default, rename = "repeatQueue")]
    pub repeat_queue: bool,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default, rename = "playMode")]
    pub play_mode: PlayMode,
    #[serde(default, rename = "customStatus")]
    pub custom_status: String,

    // Service that the bot starts on and that bare commands (p, search) target.
    #[serde(default, rename = "defaultService")]
    pub default_service: Service,

    // YouTube: path to a Netscape-format cookies file (optional).
    // Empty = check for `<config_dir>/cookies.txt`; if neither set nor
    // present, yt-dlp runs cookie-less and relies on bgutil-pot only.
    // Helps avoid 403s on rate-limited or age-restricted videos.
    #[serde(default, rename = "youtubeCookiesFile")]
    pub youtube_cookies_file: String,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            tcp_port: 10333,
            udp_port: 10333,
            encrypted: false,
            bot_name: "Spotify".to_string(),
            username: String::new(),
            password: String::new(),
            channel_name: "/".to_string(),
            channel_password: String::new(),
            bot_gender: "neutral".to_string(),
            admin_mode: AdminMode::default(),
            admins: Vec::new(),
            default_language: default_language_en(),
            license_name: None,
            license_key: None,

            spotify_quality: "VERY_HIGH".to_string(),
            spotify_enable_normalization: true,
            normalisation_type: "auto".to_string(),
            normalisation_method: "dynamic".to_string(),
            normalisation_pregain_db: 0.0,
            normalisation_threshold_dbfs: -2.0,
            normalisation_knee_db: 5.0,

            volume: 50,
            max_volume: 100,
            jitter_buffer_ms: 400,
            volume_ramp_step: 0.03,

            radio_enabled: false,
            radio_batch_size: 10,
            radio_delay: 10.0,

            search_limit: 5,

            repeat_track: false,
            repeat_queue: false,
            shuffle: false,
            play_mode: PlayMode::default(),
            custom_status: String::new(),

            default_service: Service::default(),
            youtube_cookies_file: String::new(),
        }
    }
}

impl BotConfig {
    /// Clamp out-of-range fields to sane values, returning a list of the
    /// corrections made (for logging). Keeps a hand-edited config from putting
    /// the bot into an unusable state (e.g. volume above the cap, port 0).
    pub fn validate(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.max_volume > 100 {
            warnings.push(format!("max_volume {} > 100, clamped to 100", self.max_volume));
            self.max_volume = 100;
        }
        if self.volume > self.max_volume {
            warnings.push(format!(
                "volume {} > max_volume {}, clamped",
                self.volume, self.max_volume
            ));
            self.volume = self.max_volume;
        }
        if self.radio_batch_size < 1 {
            warnings.push("radio_batch_size < 1, set to 1".to_string());
            self.radio_batch_size = 1;
        }
        if self.search_limit < 1 || self.search_limit > 20 {
            let clamped = self.search_limit.clamp(1, 20);
            warnings.push(format!("search_limit {} out of 1..=20, set to {clamped}", self.search_limit));
            self.search_limit = clamped;
        }
        if self.jitter_buffer_ms > 2000 {
            warnings.push(format!("jitter_buffer_ms {} > 2000, clamped to 2000", self.jitter_buffer_ms));
            self.jitter_buffer_ms = 2000;
        }
        if self.volume_ramp_step <= 0.0 || !self.volume_ramp_step.is_finite() {
            warnings.push(format!("volume_ramp_step {} invalid, reset to 0.03", self.volume_ramp_step));
            self.volume_ramp_step = 0.03;
        }
        if !(1..=65535).contains(&self.tcp_port) {
            warnings.push(format!("tcp_port {} out of range, reset to 10333", self.tcp_port));
            self.tcp_port = 10333;
        }
        if !(1..=65535).contains(&self.udp_port) {
            warnings.push(format!("udp_port {} out of range, reset to 10333", self.udp_port));
            self.udp_port = 10333;
        }
        if self.host.trim().is_empty() {
            warnings.push("host is empty, reset to localhost".to_string());
            self.host = "localhost".to_string();
        }
        if self.bot_name.trim().is_empty() {
            warnings.push("bot_name is empty, reset to Spotify".to_string());
            self.bot_name = "Spotify".to_string();
        }
        warnings
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ::teamtalk::types::UserGender;

    #[test]
    fn botconfig_eq_clone_equal_and_field_change_detected() {
        let a = BotConfig::default();
        let mut b = a.clone();
        assert_eq!(a, b);
        b.volume = a.volume + 1;
        assert_ne!(a, b);
    }

    #[test]
    fn is_valid_gender_male_aliases() {
        for s in ["male", "m", "man", "MALE", "Man"] {
            assert!(is_valid_gender(s), "{s} should be valid");
        }
    }

    #[test]
    fn is_valid_gender_female_aliases() {
        for s in ["female", "f", "woman", "FEMALE", "Woman"] {
            assert!(is_valid_gender(s), "{s} should be valid");
        }
    }

    #[test]
    fn is_valid_gender_neutral_aliases() {
        for s in ["neutral", "n", "nb", "NEUTRAL", "NB"] {
            assert!(is_valid_gender(s), "{s} should be valid");
        }
    }

    #[test]
    fn is_valid_gender_rejects_unknown() {
        for s in ["", "other", "xyz", "ma", "fem", "neutral!"] {
            assert!(!is_valid_gender(s), "{s} should be invalid");
        }
    }

    #[test]
    fn parse_gender_male_aliases() {
        for s in ["male", "m", "man", "MALE", "Man"] {
            assert_eq!(parse_gender(s), UserGender::Male, "{s}");
        }
    }

    #[test]
    fn parse_gender_female_aliases() {
        for s in ["female", "f", "woman", "FEMALE", "Woman"] {
            assert_eq!(parse_gender(s), UserGender::Female, "{s}");
        }
    }

    #[test]
    fn parse_gender_neutral_aliases() {
        for s in ["neutral", "n", "nb", "NEUTRAL"] {
            assert_eq!(parse_gender(s), UserGender::Neutral, "{s}");
        }
    }

    #[test]
    fn parse_gender_unknown_defaults_to_neutral() {
        for s in ["", "xyz", "other"] {
            assert_eq!(parse_gender(s), UserGender::Neutral, "{s}");
        }
    }

    #[test]
    fn validate_default_config_is_clean() {
        let mut cfg = BotConfig::default();
        assert!(cfg.validate().is_empty(), "default config should need no corrections");
    }

    #[test]
    fn validate_clamps_volume_to_max() {
        let mut cfg = BotConfig::default();
        cfg.max_volume = 60;
        cfg.volume = 90;
        let warnings = cfg.validate();
        assert_eq!(cfg.volume, 60);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn validate_clamps_max_volume_over_100() {
        let mut cfg = BotConfig::default();
        cfg.max_volume = 200;
        cfg.volume = 150;
        cfg.validate();
        assert_eq!(cfg.max_volume, 100);
        assert_eq!(cfg.volume, 100);
    }

    #[test]
    fn validate_fixes_zero_ports() {
        let mut cfg = BotConfig::default();
        cfg.tcp_port = 0;
        cfg.udp_port = 99999;
        cfg.validate();
        assert_eq!(cfg.tcp_port, 10333);
        assert_eq!(cfg.udp_port, 10333);
    }

    #[test]
    fn validate_fixes_empty_host_and_name() {
        let mut cfg = BotConfig::default();
        cfg.host = "  ".to_string();
        cfg.bot_name = String::new();
        cfg.validate();
        assert_eq!(cfg.host, "localhost");
        assert_eq!(cfg.bot_name, "Spotify");
    }

    #[test]
    fn validate_fixes_bad_ramp_and_batch() {
        let mut cfg = BotConfig::default();
        cfg.volume_ramp_step = 0.0;
        cfg.radio_batch_size = 0;
        cfg.search_limit = 50;
        cfg.validate();
        assert_eq!(cfg.volume_ramp_step, 0.03);
        assert_eq!(cfg.radio_batch_size, 1);
        assert_eq!(cfg.search_limit, 20);
    }

    #[test]
    fn validate_clamps_jitter_buffer_over_2000() {
        let mut cfg = BotConfig::default();
        cfg.jitter_buffer_ms = 10_000;
        let warnings = cfg.validate();
        assert_eq!(cfg.jitter_buffer_ms, 2000);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn validate_accepts_jitter_buffer_zero_and_default() {
        let mut cfg = BotConfig::default();
        cfg.jitter_buffer_ms = 0;
        assert!(cfg.validate().is_empty());
        cfg.jitter_buffer_ms = 400;
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn admin_mode_defaults_to_both_when_absent() {
        let json = r#"{
            "host": "localhost", "tcpPort": 10333, "udpPort": 10333,
            "botName": "Spotify", "username": "", "password": "",
            "ChannelName": "/", "ChannelPassword": "", "botGender": "neutral",
            "spotifyQuality": "VERY_HIGH", "spotifyEnableNormalization": true
        }"#;
        let cfg: BotConfig = serde_json::from_str(json).expect("config should deserialize");
        assert_eq!(cfg.admin_mode, AdminMode::Both);
        assert!(cfg.admins.is_empty());
    }

    #[test]
    fn default_language_defaults_to_en_and_round_trips() {
        let json = r#"{
            "host": "localhost", "tcpPort": 10333, "udpPort": 10333,
            "botName": "Spotify", "username": "", "password": "",
            "ChannelName": "/", "ChannelPassword": "", "botGender": "neutral",
            "spotifyQuality": "VERY_HIGH", "spotifyEnableNormalization": true
        }"#;
        let cfg: BotConfig = serde_json::from_str(json).expect("config should deserialize");
        assert_eq!(cfg.default_language, "en");

        let mut cfg = BotConfig::default();
        cfg.default_language = "pt".to_string();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"defaultLanguage\":\"pt\""));
        let back: BotConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.default_language, "pt");
    }

    #[test]
    fn admin_mode_round_trips() {
        let mut cfg = BotConfig::default();
        cfg.admin_mode = AdminMode::List;
        cfg.admins = vec!["alice".to_string(), "bob".to_string()];
        let json = serde_json::to_string(&cfg).unwrap();
        let back: BotConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.admin_mode, AdminMode::List);
        assert_eq!(back.admins, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn config_round_trip_preserves_fields() {
        let mut cfg = BotConfig::default();
        cfg.host = "tt.example.com".to_string();
        cfg.tcp_port = 12345;
        cfg.volume = 42;
        cfg.max_volume = 88;
        cfg.radio_enabled = true;
        cfg.default_service = Service::YouTube;
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let parsed: BotConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.host, "tt.example.com");
        assert_eq!(parsed.tcp_port, 12345);
        assert_eq!(parsed.volume, 42);
        assert_eq!(parsed.max_volume, 88);
        assert!(parsed.radio_enabled);
        assert_eq!(parsed.default_service, Service::YouTube);
    }
}
