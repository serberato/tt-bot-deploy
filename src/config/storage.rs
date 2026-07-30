use std::path::{Path, PathBuf};

use crate::error::BotError;
use super::model::BotConfig;

/// Process exit code for "config missing or unreadable" (sysexits EX_CONFIG).
/// The systemd unit lists it in RestartPreventExitStatus: restarting can't fix
/// a missing config, and a 2s crash-restart loop hammers the TeamTalk server
/// with logins.
pub const EXIT_CONFIG_ERROR: i32 = 78;

/// Platform-aware config directory.
/// Linux/macOS: ~/.config/ttspotify/
/// Windows: `data/` next to the executable (not the current working directory),
/// so launching from a shortcut/autostart with a different working dir still
/// finds the right config. Falls back to `<cwd>/data` only if that's where an
/// existing install already lives, keeping older setups working.
pub fn config_dir() -> PathBuf {
    if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ttspotify")
    } else {
        let exe_data = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.join("data")));
        match exe_data {
            Some(exe_data) => {
                if exe_data.exists() {
                    exe_data
                } else {
                    let cwd_data = PathBuf::from("data");
                    if cwd_data.exists() {
                        tracing::warn!(
                            "Using config dir {} (cwd) — consider moving it next to the executable",
                            cwd_data.display()
                        );
                        cwd_data
                    } else {
                        exe_data
                    }
                }
            }
            None => PathBuf::from("data"),
        }
    }
}

/// Read and validate a candidate config file. Returns the parsed config only if
/// it deserializes AND has the essential fields (`host`, `username`) filled — so
/// empty files, junk, and bare `{}` placeholders are rejected.
fn load_valid_config(path: &Path) -> Option<BotConfig> {
    let text = std::fs::read_to_string(path).ok()?;
    let cfg: BotConfig = serde_json::from_str(&text).ok()?;
    if cfg.host.trim().is_empty() || cfg.username.trim().is_empty() {
        return None;
    }
    Some(cfg)
}

/// List config files in the config directory, skipping non-bot files.
pub fn list_configs() -> Vec<(String, PathBuf)> {
    list_configs_in(&config_dir())
}

/// Scan `dir` for bot config files. Skips the name skip-list (auth/session
/// artifacts, app-global settings) and any file that fails content validation
/// (empty host/username, junk, or a bare `{}` placeholder). Split out from
/// `list_configs` so it can be tested against a temp directory.
fn list_configs_in(dir: &Path) -> Vec<(String, PathBuf)> {
    // Non-bot JSON files that share the config directory. "settings" is the
    // app-global settings.json (update-check toggle), "lang_prefs" is the i18n
    // per-user language store; the rest are auth/session artifacts. None are
    // server configs, so they must never appear as bots.
    let skip = ["credentials", "cookies", "sessions", "settings", "lang_prefs"];
    if !dir.exists() {
        return Vec::new();
    }
    let mut configs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if skip.contains(&stem) {
                continue;
            }
            if load_valid_config(&path).is_none() {
                tracing::warn!("Skipping invalid or incomplete config file: {}", path.display());
                continue;
            }
            configs.push((stem.to_string(), path));
        }
    }
    configs.sort_by(|a, b| a.0.cmp(&b.0));
    configs
}

/// After an update, add any newly-added keys (with defaults) to every real config
/// in `dir`, preserving existing values. Idempotent: only rewrites a file whose
/// serialized form differs from what is on disk. Broken/incomplete files (rejected
/// by `load_valid_config`) are left untouched. Returns the number of files rewritten.
fn top_up_configs_in(dir: &Path) -> usize {
    let mut updated = 0;
    // Only ever touch files that `list_configs_in` deems real bot configs: this
    // applies the name skip-list (credentials/settings/cookies/sessions) AND the
    // host+username validation, so an auth/session artifact that happens to carry
    // a "host"/"username" key is never misparsed as a config and overwritten.
    for (_name, path) in list_configs_in(dir) {
        let Some(cfg) = load_valid_config(&path) else { continue };
        let Ok(current) = std::fs::read_to_string(&path) else { continue };
        let Ok(canonical) = serde_json::to_string_pretty(&cfg) else { continue };
        if current.trim() == canonical.trim() {
            continue;
        }
        match cfg.save(&path) {
            Ok(()) => {
                updated += 1;
                tracing::info!("Topped up config with new keys: {}", path.display());
            }
            Err(e) => tracing::warn!("Could not top up config {}: {e}", path.display()),
        }
    }
    updated
}

/// Top up every config in the default config dir with any newly-added keys.
/// Best-effort; per-file errors are logged and skipped, never propagated.
pub fn top_up_configs() {
    let _ = top_up_configs_in(&config_dir());
}

impl BotConfig {
    /// Read and parse a config file. No wizard prompt, no validation — pure I/O
    /// plus deserialization. Safe to call from async/background contexts (never
    /// blocks on stdin).
    pub(crate) fn parse_file(path: &Path) -> Result<Self, BotError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| BotError::Config(format!("Failed to read {}: {e}", path.display())))?;
        serde_json::from_str(&contents)
            .map_err(|e| BotError::Config(format!("Failed to parse {}: {e}", path.display())))
    }

    /// Load and validate a config without any interactive prompt. Fails if the
    /// file is missing. Use this from any runtime/background path.
    pub fn load_noninteractive(path: &str) -> Result<Self, BotError> {
        let mut config = Self::parse_file(Path::new(path))?;
        for warning in config.validate() {
            tracing::warn!("Config {path}: {warning}");
        }
        Ok(config)
    }

    /// Load config for startup. If the file is missing, offer the interactive
    /// setup wizard (blocks on stdin — startup only, never from a worker task).
    /// Non-interactive contexts (systemd: stdin is /dev/null) skip the prompt
    /// and fail immediately with a clear error, so a missing config becomes a
    /// clean exit instead of a hung or crash-looping service.
    pub fn load(path: &str) -> Result<Self, BotError> {
        use std::io::IsTerminal;
        let path_ref = Path::new(path);
        if !path_ref.exists() {
            eprintln!("Config file not found: {}", path_ref.display());
            if std::io::stdin().is_terminal() {
                eprint!("Would you like to run the setup wizard? [y/N] ");
                use std::io::Write;
                std::io::stderr().flush().ok();
                let mut input = String::new();
                if std::io::stdin().read_line(&mut input).is_ok()
                    && matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
                {
                    // offer_service = false: this path continues into running the
                    // bot in the foreground; also starting a systemd instance
                    // would run the same config twice.
                    crate::wizard::run_wizard(None, false)?;
                    // Re-check if a config was created in the default config dir
                    let configs = list_configs();
                    if let Some((_, created_path)) = configs.first() {
                        let mut config = Self::parse_file(created_path)
                            .map_err(|e| BotError::Config(format!("Failed to load created config: {e}")))?;
                        for warning in config.validate() {
                            tracing::warn!("Config: {warning}");
                        }
                        return Ok(config);
                    }
                }
            }
            return Err(BotError::Config(format!(
                "Config not found: {}\nRun: tt-spotify-bot --setup",
                path_ref.display()
            )));
        }
        Self::load_noninteractive(path)
    }

    /// Write the config atomically: serialize to a temp file, then rename over
    /// the target. A crash mid-write can never leave a truncated config, and
    /// concurrent writers see whole files (last writer wins) rather than torn
    /// ones. `std::fs::rename` replaces the destination on both Unix and Windows.
    pub fn save(&self, path: &Path) -> Result<(), BotError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| BotError::Config(format!("Failed to serialize config: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Single owner of a bot's on-disk config during runtime. All runtime config
/// mutations (volume debounce, mode/radio/gender saves, exit persistence) go
/// through `update()` under one lock, eliminating the read-modify-write races
/// the old `BotConfig::update(path, ..)` free function had (each call reloaded,
/// mutated, and rewrote the whole file, clobbering concurrent writers).
pub struct ConfigStore {
    path: PathBuf,
    cfg: parking_lot::Mutex<BotConfig>,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>, cfg: BotConfig) -> Self {
        Self {
            path: path.into(),
            cfg: parking_lot::Mutex::new(cfg),
        }
    }

    pub fn get(&self) -> parking_lot::MutexGuard<'_, BotConfig> {
        self.cfg.lock()
    }

    pub fn get_idle_status(&self) -> String {
        let text = self.get().custom_status.clone();
        if text.trim().is_empty() {
            "Send h for help".to_string()
        } else {
            text
        }
    }

    /// Apply a mutation to the config and persist it atomically, all under one
    /// lock. Before mutating, re-sync from disk so edits made externally (e.g.
    /// the tray GUI's config editor writing the same file in another thread)
    /// are preserved rather than clobbered by a stale in-memory copy. Falls
    /// back to the cached copy if the file is momentarily unreadable.
    pub fn update(&self, f: impl FnOnce(&mut BotConfig)) {
        let mut guard = self.cfg.lock();
        if let Ok(on_disk) = BotConfig::parse_file(&self.path) {
            *guard = on_disk;
        }
        f(&mut guard);
        if let Err(e) = guard.save(&self.path) {
            tracing::error!("Failed to save config {}: {e}", self.path.display());
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::config::AdminMode;

    #[test]
    fn config_store_update_persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("ttspotify_cfgtest_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store_test.json");
        let mut cfg = BotConfig::default();
        cfg.volume = 30;
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
        let mut good = BotConfig::default();
        good.host = "srv.example.com".to_string();
        good.username = "botacct".to_string();
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
        let mut good = BotConfig::default();
        good.host = "srv.example.com".to_string();
        good.username = "botacct".to_string();
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
}
