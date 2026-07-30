//! CLI execution logic and workflow coordinators.

use std::io::{IsTerminal, Write};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::bot::runner::BotExit;
use crate::cli::args::{parse_args, pin_teamtalk_sdk_version, Args};
use crate::config::BotConfig;
use crate::error::BotError;

pub async fn run_cli() -> Result<(), BotError> {
    pin_teamtalk_sdk_version();
    crate::youtube::setup::migrate_legacy_tools();
    crate::logging::install_panic_hook();
    let args = parse_args();

    if let Some(res) = handle_setup_flags(&args) {
        return res;
    }

    let (config_path, profile_name) = resolve_config_and_profile(&args);
    handle_auth_status(&args, &profile_name);
    handle_tool_flags(&args);

    if args.update {
        return run_cli_update().await;
    }
    if args.auth {
        handle_auth_connect(&profile_name).await;
    }

    run_daemon_loop(config_path).await
}

fn handle_setup_flags(args: &Args) -> Option<Result<(), BotError>> {
    if let Some(ref name) = args.setup {
        let name = if name.is_empty() { None } else { Some(name.as_str()) };
        return Some(crate::wizard::run_wizard(name, true));
    }
    #[cfg(target_os = "linux")]
    if args.install_service {
        return Some(crate::daemon::install_service());
    }
    #[cfg(target_os = "linux")]
    if args.uninstall_service {
        return Some(crate::daemon::uninstall_service());
    }
    None
}

fn resolve_config_and_profile(args: &Args) -> (String, String) {
    let config_path = args.config.clone().unwrap_or_else(|| {
        let configs = crate::config::list_configs();
        if let Some((_, path)) = configs.first() {
            path.to_string_lossy().into_owned()
        } else {
            crate::config::config_dir()
                .join("config.json")
                .to_string_lossy()
                .into_owned()
        }
    });
    let profile_name = std::path::Path::new(&config_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    (config_path, profile_name)
}

fn handle_auth_status(args: &Args, profile_name: &str) {
    if !args.auth_status {
        return;
    }
    let auth = crate::spotify::auth::SpotifyAuth::new(profile_name);
    if auth.has_cached_credentials() {
        println!("Spotify: Cached credentials found for profile {profile_name}.");
        println!("  (Note: credentials may be expired or revoked.)");
        std::process::exit(0);
    } else {
        println!("Spotify: No cached credentials for profile {profile_name}.");
        println!("  Run with --auth to authenticate.");
        std::process::exit(1);
    }
}

fn handle_tool_flags(args: &Args) {
    if args.setup_yt {
        match crate::wizard::run_youtube_setup() {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("YouTube setup failed: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.update_tools {
        match crate::wizard::run_update_tools() {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("Tool update failed: {e}");
                std::process::exit(1);
            }
        }
    }
}

async fn handle_auth_connect(profile_name: &str) {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut auth = crate::spotify::auth::SpotifyAuth::new(profile_name);
    match auth.connect().await {
        Ok(_) => {
            println!("Spotify authentication successful. Credentials cached.");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Spotify authentication failed: {e}");
            std::process::exit(1);
        }
    }
}

async fn run_daemon_loop(config_path: String) -> Result<(), BotError> {
    let _log_guard = crate::logging::init_logging(&config_path);
    let last_channel = Arc::new(parking_lot::Mutex::new(None));
    loop {
        let config = match BotConfig::load(&config_path) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(crate::config::EXIT_CONFIG_ERROR);
            }
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        match crate::bot::runner::run_bot(
            config,
            config_path.clone(),
            shutdown,
            None,
            last_channel.clone(),
        )
        .await?
        {
            BotExit::Restart => {
                tracing::info!("Restarting bot...");
                continue;
            }
            _ => std::process::exit(0),
        }
    }
}

pub async fn run_cli_update() -> Result<(), BotError> {
    let info = match crate::update::check().await {
        Ok(Some(info)) => info,
        Ok(None) => {
            println!("Already up to date (v{}).", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Err(e) => {
            eprintln!("Update check failed: {e}");
            std::process::exit(1);
        }
    };

    println!("Update available: {} (you have v{})", info.tag, env!("CARGO_PKG_VERSION"));
    println!("\n{}\n", crate::update::plain_changelog(&info.changelog));

    if !std::io::stdin().is_terminal() {
        eprintln!("Not a terminal; refusing to update non-interactively. Run `ttspotify --update` from a shell.");
        std::process::exit(1);
    }

    print!("Download and install {}? [y/N] ", info.tag);
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).ok();
    if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
        println!("Cancelled.");
        return Ok(());
    }

    apply_interactive_update(&info).await
}

async fn apply_interactive_update(info: &crate::update::UpdateInfo) -> Result<(), BotError> {
    let cancel = AtomicBool::new(false);
    let progress = |done: u64, total: Option<u64>| {
        match total {
            Some(t) if t > 0 => print!("\rDownloading... {}%   ", done * 100 / t),
            _ => print!("\rDownloading... {done} bytes   "),
        }
        let _ = std::io::stdout().flush();
    };
    match crate::update::download_and_apply(info, &progress, &cancel).await {
        Ok(()) => {
            println!("\nUpdated to {}.", info.tag);
            #[cfg(target_os = "linux")]
            crate::daemon::offer_unit_refresh();
            #[cfg(target_os = "linux")]
            crate::daemon::offer_restart_running_bots();
            #[cfg(not(target_os = "linux"))]
            println!("Restart the bot to use the new version.");
            Ok(())
        }
        Err(e) => {
            eprintln!("\nUpdate failed: {e}");
            std::process::exit(1);
        }
    }
}
