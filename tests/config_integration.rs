use tt_spotify_bot::config::{AdminMode, BotConfig, ConfigStore};

#[test]
fn test_config_integration_lifecycle() {
    let dir = std::env::temp_dir().join(format!("ttspotify_integration_cfg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let path = dir.join("bot1.json");
    let cfg = BotConfig {
        bot_name: "IntegrationBot".to_string(),
        volume: 45,
        ..Default::default()
    };
    cfg.save(&path).unwrap();

    // Verify file exists on disk and can be read back via JSON
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("IntegrationBot"));

    // Test ConfigStore live updates and thread-safe get
    let store = ConfigStore::new(path.clone(), cfg);
    store.update(|c| {
        c.volume = 70;
    });

    let live = store.get();
    assert_eq!(live.volume, 70);
    assert_eq!(live.bot_name, "IntegrationBot");
    drop(live);

    // Test that the update persisted to disk
    let text_after = std::fs::read_to_string(&path).unwrap();
    let parsed: BotConfig = serde_json::from_str(&text_after).unwrap();
    assert_eq!(parsed.volume, 70);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_config_validation_integration() {
    let mut cfg = BotConfig {
        tcp_port: 0,
        udp_port: 99999,
        volume: 250,
        max_volume: 100,
        host: "  ".to_string(),
        bot_name: "".to_string(),
        ..Default::default()
    };

    let corrections = cfg.validate();
    assert!(!corrections.is_empty());
    assert_eq!(cfg.tcp_port, 10333);
    assert_eq!(cfg.udp_port, 10333);
    assert_eq!(cfg.volume, 100);
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.bot_name, "Spotify");
    assert_eq!(cfg.admin_mode, AdminMode::Both);
}
