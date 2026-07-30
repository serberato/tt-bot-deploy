//! Downloading and installing YouTube binaries and plugins.

use std::collections::HashMap;
use std::fs;

use crate::error::BotError;
use super::paths::{
    installed_bgutil_version, resolve_paths, which, YoutubeSetupPaths, BGUTIL_VERSION,
    BGUTIL_VERSION_FILE,
};
use super::verify::{
    download_verified, extract_plugin_zip, fetch_release_asset_digests, fetch_text,
    make_executable, parse_sums_file,
};

/// Detected versions of the YouTube tools, for the startup version log.
/// `None` means the tool isn't installed.
pub struct ToolVersions {
    pub yt_dlp: Option<String>,
    pub bgutil: Option<String>,
}

/// Detect installed YouTube tool versions: `yt-dlp --version` (bundled first,
/// then PATH) and the bgutil sidecar version file.
pub fn installed_tool_versions() -> ToolVersions {
    let paths = resolve_paths().ok();

    let yt_dlp_exe = paths
        .as_ref()
        .map(|p| p.yt_dlp.clone())
        .filter(|p| p.is_file())
        .or_else(|| which("yt-dlp"));
    let yt_dlp = yt_dlp_exe.and_then(|exe| {
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("--version");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let out = cmd.output().ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    });

    let bgutil = paths.as_ref().and_then(|p| {
        if p.bgutil_pot.is_file() {
            Some(installed_bgutil_version(p))
        } else {
            None
        }
    });

    ToolVersions { yt_dlp, bgutil }
}

async fn install_yt_dlp(
    client: &reqwest::Client,
    paths: &YoutubeSetupPaths,
    progress: &impl Fn(&str),
) -> Result<(), BotError> {
    progress("Downloading yt-dlp (latest)...");
    let yt_dlp_asset = if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    };
    let yt_dlp_url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/{yt_dlp_asset}"
    );
    let yt_dlp_hash = match fetch_text(
        client,
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS",
    )
    .await
    {
        Ok(sums) => parse_sums_file(&sums, yt_dlp_asset),
        Err(e) => {
            tracing::warn!("Could not fetch yt-dlp checksums: {e}");
            None
        }
    };
    download_verified(
        client,
        &yt_dlp_url,
        &paths.yt_dlp,
        yt_dlp_hash.as_deref(),
        true,
    )
    .await?;
    make_executable(&paths.yt_dlp)?;
    progress("  yt-dlp installed.");
    Ok(())
}

async fn install_bgutil(
    client: &reqwest::Client,
    paths: &YoutubeSetupPaths,
    progress: &impl Fn(&str),
    digests: &HashMap<String, String>,
    version: &str,
) -> Result<(), BotError> {
    progress(&format!("Downloading bgutil-pot {version}..."));
    let bgutil_asset = if cfg!(windows) {
        "bgutil-pot-windows-x86_64.exe"
    } else if cfg!(target_arch = "aarch64") {
        "bgutil-pot-linux-aarch64"
    } else {
        "bgutil-pot-linux-x86_64"
    };
    let bgutil_url = format!(
        "https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs/releases/download/{version}/{bgutil_asset}"
    );
    download_verified(
        client,
        &bgutil_url,
        &paths.bgutil_pot,
        digests.get(bgutil_asset).map(|s| s.as_str()),
        true,
    )
    .await?;
    make_executable(&paths.bgutil_pot)?;
    progress("  bgutil-pot installed.");
    Ok(())
}

async fn install_bgutil_plugin(
    client: &reqwest::Client,
    paths: &YoutubeSetupPaths,
    progress: &impl Fn(&str),
    digests: &HashMap<String, String>,
    version: &str,
) -> Result<(), BotError> {
    progress(&format!("Downloading bgutil yt-dlp plugin {version}..."));
    let zip_asset = "bgutil-ytdlp-pot-provider-rs.zip";
    let plugin_url = format!(
        "https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs/releases/download/{version}/{zip_asset}"
    );
    let zip_path = paths.lib_dir.join("bgutil-plugin.zip");
    download_verified(
        client,
        &plugin_url,
        &zip_path,
        digests.get(zip_asset).map(|s| s.as_str()),
        false,
    )
    .await?;
    let _ = fs::remove_dir_all(&paths.plugin_dir);
    extract_plugin_zip(&zip_path, &paths.plugin_dir)?;
    let _ = fs::remove_file(&zip_path);
    progress("  Plugin extracted.");
    Ok(())
}

/// Download + install yt-dlp, bgutil-pot, and the plugin zip.
pub async fn install(
    paths: &YoutubeSetupPaths,
    progress: impl Fn(&str),
) -> Result<(), BotError> {
    fs::create_dir_all(&paths.lib_dir)
        .map_err(|e| BotError::Config(format!("create lib dir: {e}")))?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("tt-spotify-bot/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| BotError::Config(format!("HTTP client: {e}")))?;

    install_yt_dlp(&client, paths, &progress).await?;

    let bgutil_digests = fetch_release_asset_digests(
        &client,
        "jim60105/bgutil-ytdlp-pot-provider-rs",
        BGUTIL_VERSION,
    )
    .await;
    install_bgutil(&client, paths, &progress, &bgutil_digests, BGUTIL_VERSION).await?;
    install_bgutil_plugin(&client, paths, &progress, &bgutil_digests, BGUTIL_VERSION).await?;

    let _ = fs::write(paths.lib_dir.join(BGUTIL_VERSION_FILE), BGUTIL_VERSION);
    progress(&format!(
        "YouTube support ready in {}",
        paths.lib_dir.display()
    ));
    Ok(())
}

/// Re-download just the bgutil binary + plugin at a specific version.
pub async fn install_bgutil_version(
    paths: &YoutubeSetupPaths,
    version: &str,
    progress: impl Fn(&str),
) -> Result<(), BotError> {
    fs::create_dir_all(&paths.lib_dir)
        .map_err(|e| BotError::Config(format!("create lib dir: {e}")))?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("tt-spotify-bot/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| BotError::Config(format!("HTTP client: {e}")))?;

    let digests = fetch_release_asset_digests(
        &client,
        "jim60105/bgutil-ytdlp-pot-provider-rs",
        version,
    )
    .await;

    install_bgutil(&client, paths, &progress, &digests, version).await?;
    install_bgutil_plugin(&client, paths, &progress, &digests, version).await?;

    let _ = fs::write(paths.lib_dir.join(BGUTIL_VERSION_FILE), version);
    progress(&format!("bgutil-pot updated to {version}."));
    Ok(())
}

/// Hit the GitHub API for the latest bgutil release tag.
pub async fn latest_bgutil_version() -> Result<String, BotError> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("tt-spotify-bot/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| BotError::Config(format!("HTTP client: {e}")))?;
    let response = client
        .get("https://api.github.com/repos/jim60105/bgutil-ytdlp-pot-provider-rs/releases/latest")
        .send()
        .await
        .map_err(|e| BotError::Config(format!("GitHub API: {e}")))?;
    if !response.status().is_success() {
        return Err(BotError::Config(format!(
            "GitHub API returned {}",
            response.status()
        )));
    }
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| BotError::Config(format!("GitHub API JSON: {e}")))?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BotError::Config("GitHub API: missing tag_name".to_string()))?
        .to_string();
    Ok(tag)
}
