//! CLI argument parsing and SDK version pinning.

use clap::Parser;

/// TeamTalk SDK version pinned by default.
pub const PINNED_TEAMTALK_SDK_VERSION: &str = "v5.19a";

/// Pin the TeamTalk SDK version unless explicitly overridden in the environment.
pub fn pin_teamtalk_sdk_version() {
    if std::env::var_os("TEAMTALK_SDK_VERSION").is_none() {
        std::env::set_var("TEAMTALK_SDK_VERSION", PINNED_TEAMTALK_SDK_VERSION);
    }
    crate::tt::sdk::pin_sdk_dir();
}

#[derive(Parser)]
#[command(name = "tt-spotify-bot", about = "TeamTalk Spotify Bot")]
pub struct Args {
    /// Path to config file
    #[arg(short, long)]
    pub config: Option<String>,

    /// Run the interactive config setup wizard
    #[arg(long, value_name = "NAME", num_args = 0..=1, default_missing_value = "")]
    pub setup: Option<String>,

    /// Install systemd user service (Linux only)
    #[cfg(target_os = "linux")]
    #[arg(long)]
    pub install_service: bool,

    /// Remove systemd user service (Linux only)
    #[cfg(target_os = "linux")]
    #[arg(long)]
    pub uninstall_service: bool,

    /// Authenticate with Spotify and exit (no bot startup)
    #[arg(long)]
    pub auth: bool,

    /// Check if Spotify credentials are cached and exit
    #[arg(long)]
    pub auth_status: bool,

    /// Download YouTube support binaries into the bot's lib/ folder
    #[arg(long)]
    pub setup_yt: bool,

    /// Update YouTube tools (yt-dlp and bgutil-pot)
    #[arg(long)]
    pub update_tools: bool,

    /// Check GitHub for a newer release and update interactively
    #[arg(long)]
    pub update: bool,
}

pub fn parse_args() -> Args {
    Args::parse()
}
