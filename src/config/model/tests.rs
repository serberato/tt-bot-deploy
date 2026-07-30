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
    let mut cfg = BotConfig {
        max_volume: 60,
        volume: 90,
        ..Default::default()
    };
    let warnings = cfg.validate();
    assert_eq!(cfg.volume, 60);
    assert!(!warnings.is_empty());
}

#[test]
fn validate_clamps_max_volume_over_100() {
    let mut cfg = BotConfig {
        max_volume: 200,
        volume: 150,
        ..Default::default()
    };
    cfg.validate();
    assert_eq!(cfg.max_volume, 100);
    assert_eq!(cfg.volume, 100);
}

#[test]
fn validate_fixes_zero_ports() {
    let mut cfg = BotConfig {
        tcp_port: 0,
        udp_port: 99999,
        ..Default::default()
    };
    cfg.validate();
    assert_eq!(cfg.tcp_port, 10333);
    assert_eq!(cfg.udp_port, 10333);
}

#[test]
fn validate_fixes_empty_host_and_name() {
    let mut cfg = BotConfig {
        host: "  ".to_string(),
        bot_name: String::new(),
        ..Default::default()
    };
    cfg.validate();
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.bot_name, "Spotify");
}

#[test]
fn validate_fixes_bad_ramp_and_batch() {
    let mut cfg = BotConfig {
        volume_ramp_step: 0.0,
        radio_batch_size: 0,
        search_limit: 50,
        ..Default::default()
    };
    cfg.validate();
    assert_eq!(cfg.volume_ramp_step, 0.03);
    assert_eq!(cfg.radio_batch_size, 1);
    assert_eq!(cfg.search_limit, 20);
}

#[test]
fn validate_clamps_jitter_buffer_over_2000() {
    let mut cfg = BotConfig {
        jitter_buffer_ms: 10_000,
        ..Default::default()
    };
    let warnings = cfg.validate();
    assert_eq!(cfg.jitter_buffer_ms, 2000);
    assert!(!warnings.is_empty());
}

#[test]
fn validate_accepts_jitter_buffer_zero_and_default() {
    let mut cfg = BotConfig {
        jitter_buffer_ms: 0,
        ..Default::default()
    };
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

    let cfg = BotConfig {
        default_language: "pt".to_string(),
        ..Default::default()
    };
    let json = serde_json::to_string(&cfg).unwrap();
    assert!(json.contains("\"defaultLanguage\":\"pt\""));
    let back: BotConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.default_language, "pt");
}

#[test]
fn admin_mode_round_trips() {
    let cfg = BotConfig {
        admin_mode: AdminMode::List,
        admins: vec!["alice".to_string(), "bob".to_string()],
        ..Default::default()
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let back: BotConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.admin_mode, AdminMode::List);
    assert_eq!(back.admins, vec!["alice".to_string(), "bob".to_string()]);
}

#[test]
fn config_round_trip_preserves_fields() {
    let cfg = BotConfig {
        host: "tt.example.com".to_string(),
        tcp_port: 12345,
        volume: 42,
        max_volume: 88,
        radio_enabled: true,
        default_service: Service::YouTube,
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&cfg).unwrap();
    let parsed: BotConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.host, "tt.example.com");
    assert_eq!(parsed.tcp_port, 12345);
    assert_eq!(parsed.volume, 42);
    assert_eq!(parsed.max_volume, 88);
    assert!(parsed.radio_enabled);
    assert_eq!(parsed.default_service, Service::YouTube);
}
