//! Input validation and utility execution helpers for the setup wizard.

use crate::config::AdminMode;
use crate::error::BotError;
use crate::youtube::setup;

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

/// Public entry point for the standalone `--setup-yt` flag.
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
pub fn run_update_tools() -> Result<(), BotError> {
    let paths = setup::resolve_paths()?;
    if !setup::is_installed(&paths) {
        println!("  YouTube tools aren't installed yet. Run --setup-yt first.");
        return Ok(());
    }

    println!("Updating yt-dlp...");
    match std::process::Command::new(&paths.yt_dlp).arg("--update").status() {
        Ok(status) if status.success() => println!("  yt-dlp update check complete."),
        Ok(status) => println!("  yt-dlp --update exited with {status}"),
        Err(e) => println!("  Could not run yt-dlp --update: {e}"),
    }

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
pub(crate) fn run_blocking_async<T, F, Fut>(f: F) -> Result<T, BotError>
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
