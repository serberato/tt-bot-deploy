//! Interactive config setup wizard.
//!
//! Modular, CLI/terminal-focused configuration wizard that guides the user
//! through creating a valid `BotConfig`, validating input, and offering
//! post-setup integrations (Spotify auth, YouTube support, systemd service).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::{config_dir, AdminMode, BotConfig};
use crate::error::BotError;
use crate::services::Service;
use crate::youtube::setup;

// ============================================================================
// CLI I/O and Defensive Input Prompting
// ============================================================================

/// Parse a "yes/no" user input string into a boolean, falling back to `default`.
pub(crate) fn parse_yes_no_input(input: &str, default: bool) -> bool {
    let s = input.trim();
    if s.is_empty() {
        return default;
    }
    matches!(s.to_lowercase().as_str(), "y" | "yes")
}

/// Parse an admin mode selection string into an `AdminMode`.
pub(crate) fn parse_admin_mode_input(input: &str) -> AdminMode {
    match input.trim().to_lowercase().as_str() {
        "everyone" => AdminMode::Everyone,
        "ttrights" => AdminMode::TtRights,
        "list" => AdminMode::List,
        _ => AdminMode::Both,
    }
}

/// Parse and clean a default language selection code.
pub(crate) fn parse_lang_code(input: &str) -> String {
    let code = input.trim().to_lowercase();
    if code.is_empty() {
        "en".to_string()
    } else {
        code
    }
}

/// Prompt the user for string input with a display default and optional required flag.
fn ask(prompt: &str, default: &str, required: bool) -> Option<String> {
    loop {
        if default.is_empty() {
            print!("  {prompt}: ");
        } else {
            print!("  {prompt} [{default}]: ");
        }
        io::stdout().flush().ok();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) | Err(_) => {
                println!("\nSetup cancelled.");
                return None;
            }
            _ => {}
        }

        let input = input.trim().to_string();
        if input.is_empty() && !default.is_empty() {
            return Some(default.to_string());
        }
        if input.is_empty() && required {
            println!("    This field is required.");
            continue;
        }
        return Some(input);
    }
}

/// Prompt the user for an integer input, retrying until valid.
fn ask_int(prompt: &str, default: i32) -> Option<i32> {
    loop {
        let raw = ask(prompt, &default.to_string(), true)?;
        match raw.parse::<i32>() {
            Ok(v) => return Some(v),
            Err(_) => println!("    Invalid input. Expected a number."),
        }
    }
}

/// Prompt for a boolean yes/no choice.
fn ask_yes_no(prompt: &str, default: bool) -> Option<bool> {
    let default_str = if default { "Y/n" } else { "y/N" };
    let prompt_text = format!("{prompt} ({default_str})");
    let ans = ask(&prompt_text, if default { "y" } else { "n" }, false)?;
    Some(parse_yes_no_input(&ans, default))
}

// ============================================================================
// Setup Wizard State & Step Functions
// ============================================================================

/// Encapsulates the state and sequential step execution of the CLI setup wizard.
struct SetupWizard {
    name: String,
    config_path: PathBuf,
    config: BotConfig,
}

impl SetupWizard {
    /// Prepare a wizard instance, resolving file paths and handling overwrite checks.
    fn prepare(config_name: Option<&str>) -> Result<Option<Self>, BotError> {
        println!();
        println!("TTSpotify Configuration Setup");
        println!();

        let name = if let Some(n) = config_name {
            n.to_string()
        } else {
            match ask("Config name (used for file name and service name)", "config", true) {
                Some(n) => n.replace(".json", ""),
                None => return Ok(None),
            }
        };

        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let config_path = dir.join(format!("{name}.json"));

        if config_path.exists() {
            let prompt = format!("{} already exists. Overwrite?", config_path.display());
            let overwrite = ask_yes_no(&prompt, false);
            if overwrite != Some(true) {
                println!("Setup cancelled.");
                return Ok(None);
            }
        }

        Ok(Some(Self {
            name,
            config_path,
            config: BotConfig::default(),
        }))
    }

    /// Run all interactive prompt steps in sequence.
    fn prompt_all(&mut self) -> Option<()> {
        self.prompt_server_settings()?;
        self.prompt_credentials()?;
        self.prompt_bot_settings()?;
        self.prompt_admin_permissions()?;
        self.prompt_language()?;
        self.prompt_license()?;
        self.prompt_default_service()?;
        self.prompt_youtube_cookies()?;
        Some(())
    }

    fn prompt_server_settings(&mut self) -> Option<()> {
        println!("TeamTalk Server Settings");
        self.config.host = ask("Server address", "", true)?;
        self.config.tcp_port = ask_int("TCP port", 10333)?;
        self.config.udp_port = ask_int("UDP port", self.config.tcp_port)?;
        Some(())
    }

    fn prompt_credentials(&mut self) -> Option<()> {
        println!();
        println!("Bot Credentials");
        self.config.username = ask("Bot username", "", true)?;
        self.config.password = ask("Bot password", "", false)?;
        Some(())
    }

    fn prompt_bot_settings(&mut self) -> Option<()> {
        println!();
        println!("Bot Settings");
        self.config.bot_name = ask("Bot nickname", "Spotify", true)?;
        let channel = ask("Channel to join (path or leave blank for root)", "/", false)?;
        self.config.channel_name = if channel.is_empty() {
            "/".to_string()
        } else {
            channel
        };
        self.config.channel_password = ask("Channel password (if any)", "", false)?;
        Some(())
    }

    fn prompt_admin_permissions(&mut self) -> Option<()> {
        println!();
        println!("Admin Permissions");
        let mode_str = ask("Admin mode [everyone/ttrights/list/both]", "both", false)?;
        self.config.admin_mode = parse_admin_mode_input(&mode_str);

        if matches!(self.config.admin_mode, AdminMode::List | AdminMode::Both) {
            let list_str = ask("Admin usernames (comma separated)", "", false)?;
            self.config.admins = crate::bot::auth::parse_admin_list(&list_str);
        } else {
            self.config.admins.clear();
        }
        Some(())
    }

    fn prompt_language(&mut self) -> Option<()> {
        println!();
        println!("Language");
        let lang_codes = crate::i18n::installed_language_codes(&config_dir());
        let prompt = format!("Default language [{}]", lang_codes.join("/"));
        let lang_str = ask(&prompt, "en", false)?;
        self.config.default_language = parse_lang_code(&lang_str);
        Some(())
    }

    fn prompt_license(&mut self) -> Option<()> {
        println!();
        println!("License (optional)");
        let license_name = ask("License name", "", false)?;
        let license_key = ask("License key", "", false)?;
        if !license_name.is_empty() {
            self.config.license_name = Some(license_name);
        }
        if !license_key.is_empty() {
            self.config.license_key = Some(license_key);
        }
        Some(())
    }

    fn prompt_default_service(&mut self) -> Option<()> {
        println!();
        println!("Default Service");
        let svc_str = ask("Which service should bare commands target? (spotify/youtube)", "spotify", true)?;
        self.config.default_service = Service::parse_or_default(&svc_str);
        Some(())
    }

    fn prompt_youtube_cookies(&mut self) -> Option<()> {
        println!();
        println!("YouTube Cookies (optional)");
        println!("  Cookies help with rate-limited or age-restricted videos.");
        println!("  Playback works without them in most cases.");
        if ask_yes_no("Configure a cookies file path?", false)? {
            let default_path = setup::default_cookies_path(&self.name).to_string_lossy().into_owned();
            let path_str = ask("Cookies file path", &default_path, false)?;
            if !path_str.is_empty() && !Path::new(&path_str).is_file() {
                println!("  Warning: {path_str} doesn't exist yet. Saving anyway — drop the file there later.");
            }
            self.config.youtube_cookies_file = path_str;
        } else {
            self.config.youtube_cookies_file.clear();
        }
        Some(())
    }

    /// Persist the configured `BotConfig` to disk.
    fn save(&self) -> Result<(), BotError> {
        self.config.save(&self.config_path)?;
        println!();
        println!("  Config saved to: {}", self.config_path.display());
        Ok(())
    }
}

// ============================================================================
// Post-Setup Workflow Handlers
// ============================================================================

/// Offer Spotify OAuth flow in a background runtime.
fn offer_spotify_auth(config_name: &str) -> Option<()> {
    println!();
    println!("Spotify Authentication");
    if ask_yes_no("Authenticate with Spotify now?", false)? {
        println!("  Starting Spotify authentication...");
        let name_clone = config_name.to_string();
        let auth_result = std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("  Failed to create async runtime: {e}");
                    return None;
                }
            };
            let mut auth = crate::spotify::auth::SpotifyAuth::new(&name_clone);
            Some(rt.block_on(auth.connect()))
        })
        .join()
        .ok()
        .flatten();

        match auth_result {
            Some(Ok(_)) => println!("  Spotify authentication successful! Credentials cached."),
            Some(Err(e)) => {
                println!("  Spotify authentication failed: {e}");
                println!("  You can try again with: tt-spotify-bot --auth");
            }
            None => {
                println!("  Could not initialize authentication.");
                println!("  You can authenticate later with: tt-spotify-bot --auth");
            }
        }
    } else {
        println!("  Skipping Spotify authentication.");
        println!("  You can authenticate later with: tt-spotify-bot --auth");
    }
    Some(())
}

/// Offer YouTube support and binary download if not yet installed.
fn offer_youtube_support(default_service: Service) -> Option<()> {
    let yt_already_installed = setup::resolve_paths()
        .map(|p| setup::is_installed(&p))
        .unwrap_or(false);

    if yt_already_installed {
        return Some(());
    }

    println!();
    println!("YouTube Support");
    let yt_default = default_service == Service::YouTube;
    let prompt = if yt_default {
        "YouTube support requires extra binaries (~50 MB: yt-dlp, bgutil-pot, plugin). Download now?"
    } else {
        "You can also enable YouTube support. Downloads ~50 MB of binaries (yt-dlp, bgutil-pot, plugin). Skip if you only need Spotify. Install YouTube support?"
    };

    if ask_yes_no(prompt, yt_default)? {
        if let Err(e) = run_youtube_setup() {
            println!("  YouTube setup failed: {e}");
            println!("  You can retry later with: tt-spotify-bot --setup-yt");
        }
    } else {
        println!("  Skipping YouTube setup. You can run it later with: tt-spotify-bot --setup-yt");
    }
    Some(())
}

/// Offer systemd service installation and wiring on Linux.
#[cfg(target_os = "linux")]
fn offer_systemd_service(config_name: &str, offer_service: bool) -> Option<()> {
    if !offer_service || !crate::daemon::systemd_booted() {
        return Some(());
    }

    println!();
    println!("Systemd Service");
    if crate::daemon::service_installed() {
        crate::daemon::offer_enable_instance(config_name);
    } else if ask_yes_no("Systemd service not installed. Install it now?", false)? {
        if let Err(e) = crate::daemon::install_service() {
            println!("  Service install failed: {e}");
            println!("  You can retry later with: tt-spotify-bot --install-service");
        }
    }
    Some(())
}

// ============================================================================
// Public Wizard and Setup Entry Points
// ============================================================================

/// Run the interactive setup wizard.
///
/// `offer_service` should be true only for the standalone `--setup` flow.
/// The first-run wizard inside `BotConfig::load` must pass false: that path
/// continues into running the bot in the foreground, and starting a systemd
/// instance there too would run the same config twice.
pub fn run_wizard(config_name: Option<&str>, offer_service: bool) -> Result<(), BotError> {
    #[cfg(not(target_os = "linux"))]
    let _ = offer_service;

    let Some(mut wizard) = SetupWizard::prepare(config_name)? else {
        return Ok(());
    };

    if wizard.prompt_all().is_none() {
        return Ok(());
    }

    wizard.save()?;

    offer_spotify_auth(&wizard.name);
    offer_youtube_support(wizard.config.default_service);

    #[cfg(target_os = "linux")]
    offer_systemd_service(&wizard.name, offer_service);

    println!();
    println!("  Run the bot with: tt-spotify-bot --config {}", wizard.config_path.display());
    println!();

    Ok(())
}

/// Public entry point for the standalone `--setup-yt` flag.
/// Downloads the binaries (skipping if already installed). Cookies are a
/// separate, optional config-time concern — not part of this flow.
pub fn run_youtube_setup() -> Result<(), BotError> {
    let paths = setup::resolve_paths()?;

    if setup::is_installed(&paths) {
        println!("  YouTube binaries already installed at {}", paths.lib_dir.display());
        return Ok(());
    }

    println!("  Installing into {}", paths.lib_dir.display());
    run_blocking_async(|| async {
        let paths = setup::resolve_paths()?;
        setup::install(&paths, |line| println!("  {line}")).await
    })?;

    println!();
    println!("  YouTube support installed.");
    println!("  Tip: cookies are optional. If you want them, edit your config and");
    println!("  set youtubeCookiesFile, or drop a cookies.txt in the config dir.");
    Ok(())
}

/// Public entry point for `--update-tools`.
///
/// 1. Self-update yt-dlp via its built-in `--update` command.
/// 2. Compare installed bgutil version (sidecar) with the latest GitHub release.
///    Re-download bgutil + plugin only if newer is available.
pub fn run_update_tools() -> Result<(), BotError> {
    let paths = setup::resolve_paths()?;

    if !setup::is_installed(&paths) {
        println!("  YouTube tools aren't installed yet. Run --setup-yt first.");
        return Ok(());
    }

    // 1. yt-dlp self-update.
    println!("Updating yt-dlp...");
    match std::process::Command::new(&paths.yt_dlp)
        .arg("--update")
        .status()
    {
        Ok(status) if status.success() => {
            println!("  yt-dlp update check complete.");
        }
        Ok(status) => {
            println!("  yt-dlp --update exited with {status}");
        }
        Err(e) => {
            println!("  Could not run yt-dlp --update: {e}");
        }
    }

    // 2. bgutil version check vs GitHub releases.
    println!();
    println!("Checking bgutil-pot for updates...");
    let installed = setup::installed_bgutil_version(&paths);
    let latest = run_blocking_async(|| async { setup::latest_bgutil_version().await })?;

    if installed == latest {
        println!("  bgutil-pot already on {installed} (latest).");
    } else {
        println!("  Installed: {installed}, latest: {latest}. Updating...");
        let target = latest.clone();
        run_blocking_async(move || async move {
            let paths = setup::resolve_paths()?;
            setup::install_bgutil_version(&paths, &target, |line| println!("  {line}")).await
        })?;
    }

    println!();
    println!("  Done.");
    Ok(())
}

/// Run an async closure on a fresh tokio runtime in a worker thread.
/// The wizard is sync but may be invoked from an async context (e.g. `main`),
/// so spinning up our own runtime avoids the nested-runtime panic.
fn run_blocking_async<T, F, Fut>(f: F) -> Result<T, BotError>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, BotError>>,
{
    std::thread::spawn(move || -> Result<T, BotError> {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| BotError::Config(format!("tokio runtime: {e}")))?;
        rt.block_on(f())
    })
    .join()
    .map_err(|_| BotError::Config("async worker thread panicked".to_string()))?
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yes_no_input_matches_yes_variants() {
        assert!(parse_yes_no_input("y", false));
        assert!(parse_yes_no_input("Y", false));
        assert!(parse_yes_no_input("yes", false));
        assert!(parse_yes_no_input("YES", false));
    }

    #[test]
    fn parse_yes_no_input_defaults_on_empty() {
        assert!(parse_yes_no_input("", true));
        assert!(!parse_yes_no_input("", false));
        assert!(!parse_yes_no_input("   ", false));
    }

    #[test]
    fn parse_yes_no_input_rejects_no_variants() {
        assert!(!parse_yes_no_input("n", true));
        assert!(!parse_yes_no_input("NO", true));
        assert!(!parse_yes_no_input("other", true));
    }

    #[test]
    fn parse_admin_mode_input_matches_modes() {
        assert_eq!(parse_admin_mode_input("everyone"), AdminMode::Everyone);
        assert_eq!(parse_admin_mode_input("ttrights"), AdminMode::TtRights);
        assert_eq!(parse_admin_mode_input("list"), AdminMode::List);
        assert_eq!(parse_admin_mode_input("both"), AdminMode::Both);
        assert_eq!(parse_admin_mode_input("unknown"), AdminMode::Both);
    }

    #[test]
    fn parse_lang_code_defaults_to_en() {
        assert_eq!(parse_lang_code(""), "en");
        assert_eq!(parse_lang_code("   "), "en");
        assert_eq!(parse_lang_code("ES"), "es");
        assert_eq!(parse_lang_code("pt-BR"), "pt-br");
    }
}
