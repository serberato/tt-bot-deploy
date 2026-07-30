//! Interactive configuration builder and wizard runner.

use std::path::{Path, PathBuf};

use crate::config::{config_dir, AdminMode, BotConfig};
use crate::error::BotError;
use crate::services::Service;
use crate::wizard::prompts::{ask, ask_int, ask_yes_no};
use crate::wizard::validation::{parse_admin_mode_input, parse_lang_code, run_youtube_setup};
use crate::youtube::setup;

pub struct SetupWizard {
    pub(crate) name: String,
    pub(crate) config_path: PathBuf,
    pub(crate) config: BotConfig,
}

impl SetupWizard {
    pub fn prepare(config_name: Option<&str>) -> Result<Option<Self>, BotError> {
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
            if ask_yes_no(&prompt, false) != Some(true) {
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

    pub fn prompt_all(&mut self) -> Option<()> {
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
        println!("\nBot Credentials");
        self.config.username = ask("Bot username", "", true)?;
        self.config.password = ask("Bot password", "", false)?;
        Some(())
    }

    fn prompt_bot_settings(&mut self) -> Option<()> {
        println!("\nBot Settings");
        self.config.bot_name = ask("Bot nickname", "Spotify", true)?;
        let channel = ask("Channel to join (path or leave blank for root)", "/", false)?;
        self.config.channel_name = if channel.is_empty() { "/".to_string() } else { channel };
        self.config.channel_password = ask("Channel password (if any)", "", false)?;
        Some(())
    }

    fn prompt_admin_permissions(&mut self) -> Option<()> {
        println!("\nAdmin Permissions");
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
        println!("\nLanguage");
        let lang_codes = crate::i18n::installed_language_codes(&config_dir());
        let prompt = format!("Default language [{}]", lang_codes.join("/"));
        let lang_str = ask(&prompt, "en", false)?;
        self.config.default_language = parse_lang_code(&lang_str);
        Some(())
    }

    fn prompt_license(&mut self) -> Option<()> {
        println!("\nLicense (optional)");
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
        println!("\nDefault Service");
        let svc_str = ask("Which service should bare commands target? (spotify/youtube)", "spotify", true)?;
        self.config.default_service = Service::parse_or_default(&svc_str);
        Some(())
    }

    fn prompt_youtube_cookies(&mut self) -> Option<()> {
        println!("\nYouTube Cookies (optional)");
        println!("  Cookies help with rate-limited or age-restricted videos.");
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

    pub fn save(&self) -> Result<(), BotError> {
        self.config.save(&self.config_path)?;
        println!("\n  Config saved to: {}", self.config_path.display());
        Ok(())
    }
}

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

    println!("\n  Run the bot with: tt-spotify-bot --config {}\n", wizard.config_path.display());
    Ok(())
}

fn offer_spotify_auth(config_name: &str) -> Option<()> {
    println!("\nSpotify Authentication");
    if ask_yes_no("Authenticate with Spotify now?", false)? {
        println!("  Starting Spotify authentication...");
        let name_clone = config_name.to_string();
        let auth_result = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().ok()?;
            let mut auth = crate::spotify::auth::SpotifyAuth::new(&name_clone);
            Some(rt.block_on(auth.connect()))
        })
        .join()
        .ok()
        .flatten();

        match auth_result {
            Some(Ok(_)) => println!("  Spotify authentication successful! Credentials cached."),
            Some(Err(e)) => println!("  Spotify authentication failed: {e}\n  You can try again with: tt-spotify-bot --auth"),
            None => println!("  Could not initialize authentication.\n  You can authenticate later with: tt-spotify-bot --auth"),
        }
    } else {
        println!("  Skipping Spotify authentication.\n  You can authenticate later with: tt-spotify-bot --auth");
    }
    Some(())
}

fn offer_youtube_support(default_service: Service) -> Option<()> {
    let yt_already_installed = setup::resolve_paths()
        .map(|p| setup::is_installed(&p))
        .unwrap_or(false);
    if yt_already_installed {
        return Some(());
    }
    println!("\nYouTube Support");
    let yt_default = default_service == Service::YouTube;
    let prompt = if yt_default {
        "YouTube support requires extra binaries (~50 MB: yt-dlp, bgutil-pot, plugin). Download now?"
    } else {
        "You can also enable YouTube support. Downloads ~50 MB of binaries. Install YouTube support?"
    };
    if ask_yes_no(prompt, yt_default)? {
        if let Err(e) = run_youtube_setup() {
            println!("  YouTube setup failed: {e}\n  You can retry later with: tt-spotify-bot --setup-yt");
        }
    } else {
        println!("  Skipping YouTube setup. You can run it later with: tt-spotify-bot --setup-yt");
    }
    Some(())
}

#[cfg(target_os = "linux")]
fn offer_systemd_service(config_name: &str, offer_service: bool) -> Option<()> {
    if !offer_service || !crate::daemon::systemd_booted() {
        return Some(());
    }
    println!("\nSystemd Service");
    if crate::daemon::service_installed() {
        crate::daemon::offer_enable_instance(config_name);
    } else if ask_yes_no("Systemd service not installed. Install it now?", false)? {
        if let Err(e) = crate::daemon::install_service() {
            println!("  Service install failed: {e}\n  You can retry later with: tt-spotify-bot --install-service");
        }
    }
    Some(())
}
