//! Directory and executable path resolution for YouTube tools.

use std::fs;
use std::path::PathBuf;

use crate::error::BotError;

pub(crate) const BGUTIL_VERSION: &str = "v0.8.1";

/// Filename for the sidecar that records which bgutil version is on disk.
/// Lives next to the bgutil binary in `lib/`.
pub(crate) const BGUTIL_VERSION_FILE: &str = ".bgutil-version";

/// Resolved on-disk paths for all three components.
#[derive(Debug, Clone)]
pub struct YoutubeSetupPaths {
    /// Directory for binaries: `<exe-dir>/lib`.
    pub lib_dir: PathBuf,
    /// `lib/yt-dlp` (Linux) or `lib/yt-dlp.exe` (Windows).
    pub yt_dlp: PathBuf,
    /// `lib/bgutil-pot` or `lib/bgutil-pot.exe`.
    pub bgutil_pot: PathBuf,
    /// `lib/yt-dlp-plugins` (the dir we pass to `--plugin-dirs`).
    pub plugin_dir: PathBuf,
}

/// Pick the directory the YouTube tools live in.
/// An exe-side `lib/` that already holds tools wins, so existing installs and
/// dev checkouts keep working. Otherwise use `<data_dir>/ttspotify/lib`, which
/// stays user-writable when the binary itself is installed somewhere
/// root-owned like /usr/local/bin.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn pick_tools_dir(
    legacy: PathBuf,
    legacy_has_tools: bool,
    data_dir: Option<PathBuf>,
) -> PathBuf {
    if legacy_has_tools {
        return legacy;
    }
    match data_dir {
        Some(d) => d.join("ttspotify").join("lib"),
        None => legacy,
    }
}

/// Compute where the binaries should live.
pub fn resolve_paths() -> Result<YoutubeSetupPaths, BotError> {
    let exe = std::env::current_exe()
        .map_err(|e| BotError::Config(format!("current_exe failed: {e}")))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| BotError::Config("current_exe has no parent".to_string()))?;
    let legacy_lib = exe_dir.join("lib");
    #[cfg(windows)]
    let lib_dir = legacy_lib;
    #[cfg(not(windows))]
    let lib_dir = {
        let has_tools =
            legacy_lib.join("yt-dlp").is_file() || legacy_lib.join("bgutil-pot").is_file();
        pick_tools_dir(legacy_lib, has_tools, dirs::data_dir())
    };
    let (yt_dlp_name, bgutil_name) = if cfg!(windows) {
        ("yt-dlp.exe", "bgutil-pot.exe")
    } else {
        ("yt-dlp", "bgutil-pot")
    };
    Ok(YoutubeSetupPaths {
        yt_dlp: lib_dir.join(yt_dlp_name),
        bgutil_pot: lib_dir.join(bgutil_name),
        plugin_dir: lib_dir.join("yt-dlp-plugins"),
        lib_dir,
    })
}

/// True if all three components are present on disk.
pub fn is_installed(paths: &YoutubeSetupPaths) -> bool {
    paths.yt_dlp.is_file() && paths.bgutil_pot.is_file() && paths.plugin_dir.is_dir()
}

/// Pinned version we'd lay down on a fresh install. Read by --update-tools
/// to know what to download if the sidecar is missing.
pub fn pinned_bgutil_version() -> &'static str {
    BGUTIL_VERSION
}

/// Returns the bgutil version actually installed on disk (read from the
/// sidecar). Falls back to the pinned const if the sidecar is missing,
/// which covers older installs that predate the sidecar.
pub fn installed_bgutil_version(paths: &YoutubeSetupPaths) -> String {
    fs::read_to_string(paths.lib_dir.join(BGUTIL_VERSION_FILE))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| BGUTIL_VERSION.to_string())
}

/// Default cookies file path. The bot auto-loads this if it exists when
/// `youtube_cookies_file` is empty.
pub fn default_cookies_path(profile_name: &str) -> PathBuf {
    let name = if profile_name.is_empty() {
        "cookies.txt".to_string()
    } else {
        format!("cookies_{}.txt", profile_name)
    };
    crate::config::config_dir().join(name)
}

/// Look up an executable on PATH. Returns `Some(path)` if found,
/// `None` otherwise. Mirrors `which`/`where` semantics.
pub fn which(exe_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<&str> = if cfg!(windows) {
        vec![".exe", ".cmd", ".bat", ""]
    } else {
        vec![""]
    };
    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let candidate = if ext.is_empty() {
                dir.join(exe_name)
            } else {
                dir.join(format!("{exe_name}{ext}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
