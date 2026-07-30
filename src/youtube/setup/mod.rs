//! YouTube binaries auto-installer.

pub mod downloader;
pub mod migration;
pub mod paths;
pub mod verify;

#[cfg(test)]
mod tests;

pub use downloader::{
    install, install_bgutil_version, installed_tool_versions, latest_bgutil_version, ToolVersions,
};
pub use migration::{migrate_legacy_tools, migrate_tools_dir};
pub use paths::{
    default_cookies_path, installed_bgutil_version, is_installed, pinned_bgutil_version,
    resolve_paths, which, YoutubeSetupPaths,
};
