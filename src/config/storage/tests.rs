use super::*;
use crate::config::AdminMode;

#[test]
fn config_store_update_persists_and_reloads() {
    let dir = std::env::temp_dir().join(format!("ttspotify_cfgtest_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store_test.json");
    let cfg = BotConfig {
        volume: 30,
        ..Default::default()
    };
    cfg.save(&path).unwrap();

    let store = ConfigStore::new(path.clone(), cfg);
    store.update(|c| c.volume = 55);
    store.update(|c| c.radio_enabled = true);

    let reloaded = BotConfig::parse_file(&path).unwrap();
    assert_eq!(reloaded.volume, 55);
    assert!(reloaded.radio_enabled);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_store_update_preserves_external_edits() {
    let dir = std::env::temp_dir().join(format!("ttspotify_cfgext_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ext_test.json");
    let cfg = BotConfig::default();
    cfg.save(&path).unwrap();
    let store = ConfigStore::new(path.clone(), cfg);

    let mut external = BotConfig::parse_file(&path).unwrap();
    external.host = "edited.example.com".to_string();
    external.save(&path).unwrap();

    store.update(|c| c.volume = 42);

    let reloaded = BotConfig::parse_file(&path).unwrap();
    assert_eq!(reloaded.host, "edited.example.com");
    assert_eq!(reloaded.volume, 42);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_configs_skips_invalid_files() {
    let dir = std::env::temp_dir().join(format!("ttspotify_listcfg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.as_path();
    let good = BotConfig {
        host: "srv.example.com".to_string(),
        username: "botacct".to_string(),
        ..Default::default()
    };
    good.save(&p.join("good.json")).unwrap();
    std::fs::write(p.join("empty.json"), "").unwrap();
    std::fs::write(p.join("junk.json"), "not json at all").unwrap();
    std::fs::write(p.join("blank.json"), "{}").unwrap();
    std::fs::write(p.join("nouser.json"), r#"{"host":"h"}"#).unwrap();
    std::fs::write(p.join("settings.json"), r#"{"host":"h","username":"u"}"#).unwrap();

    let listed: Vec<String> = list_configs_in(p).into_iter().map(|(name, _)| name).collect();
    assert_eq!(listed, vec!["good".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_configs_skips_lang_prefs_by_name() {
    let dir = std::env::temp_dir().join(format!("ttspotify_cfglangprefs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.as_path();
    let good = BotConfig {
        host: "srv.example.com".to_string(),
        username: "botacct".to_string(),
        ..Default::default()
    };
    good.save(&p.join("good.json")).unwrap();
    std::fs::write(p.join("lang_prefs.json"), r#"{"host":"h","username":"u"}"#).unwrap();

    let listed: Vec<String> = list_configs_in(p).into_iter().map(|(name, _)| name).collect();
    assert_eq!(listed, vec!["good".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn top_up_adds_missing_keys_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("ttspotify_topup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.as_path();
    let path = p.join("srv.json");
    std::fs::write(&path, r#"{"host":"h","username":"u"}"#).unwrap();
    let junk = p.join("junk.json");
    std::fs::write(&junk, "garbage").unwrap();
    let junk_before = std::fs::read_to_string(&junk).unwrap();

    assert_eq!(top_up_configs_in(p), 1);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("adminMode"));
    assert!(text.contains("admins"));
    let cfg: BotConfig = serde_json::from_str(&text).unwrap();
    assert_eq!(cfg.host, "h");
    assert_eq!(cfg.username, "u");
    assert_eq!(cfg.admin_mode, AdminMode::Both);

    assert_eq!(top_up_configs_in(p), 0);
    assert_eq!(std::fs::read_to_string(&junk).unwrap(), junk_before);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn top_up_never_touches_skip_listed_files() {
    let dir = std::env::temp_dir().join(format!("ttspotify_topupskip_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.as_path();
    let cred = p.join("credentials.json");
    std::fs::write(&cred, r#"{"host":"h","username":"u","refresh_token":"secret"}"#).unwrap();
    let cred_before = std::fs::read_to_string(&cred).unwrap();
    let settings = p.join("settings.json");
    std::fs::write(&settings, r#"{"host":"h","username":"u","check_updates_on_startup":true}"#).unwrap();
    let settings_before = std::fs::read_to_string(&settings).unwrap();

    let updated = top_up_configs_in(p);

    assert_eq!(updated, 0, "no bot configs present, nothing should be rewritten");
    assert_eq!(std::fs::read_to_string(&cred).unwrap(), cred_before, "credentials.json must be untouched");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), settings_before, "settings.json must be untouched");

    let _ = std::fs::remove_dir_all(&dir);
}
