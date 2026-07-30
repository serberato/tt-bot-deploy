//! Checksum verification, downloading, and archive extraction for YouTube tools.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::error::BotError;

/// Compute the lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Verify `bytes` hash against an expected hex digest (case-insensitive).
pub(crate) fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool {
    sha256_hex(bytes).eq_ignore_ascii_case(expected_hex.trim())
}

/// Parse a `SHA2-256SUMS`-style file (`<hex>  <filename>` per line) and return
/// the digest for `asset_name`, if present.
pub(crate) fn parse_sums_file(text: &str, asset_name: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next().unwrap_or("").trim_start_matches('*');
        if name == asset_name && hash.len() == 64 {
            return Some(hash.to_string());
        }
    }
    None
}

/// Basic executable magic-byte sanity check, used as a fallback when no hash
/// is available: PE ("MZ") on Windows, ELF ("\x7fELF") on Unix.
pub(crate) fn looks_like_executable(bytes: &[u8]) -> bool {
    if cfg!(windows) {
        bytes.starts_with(b"MZ")
    } else {
        bytes.starts_with(b"\x7fELF")
    }
}

/// Fetch a URL as text (used for the SHA2-256SUMS manifest).
pub(crate) async fn fetch_text(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, BotError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| BotError::Config(format!("fetch {url}: {e}")))?;
    if !response.status().is_success() {
        return Err(BotError::Config(format!(
            "fetch {url} returned {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|e| BotError::Config(format!("read {url}: {e}")))
}

/// Fetch a GitHub release's asset SHA-256 digests, keyed by asset filename.
pub(crate) async fn fetch_release_asset_digests(
    client: &reqwest::Client,
    repo: &str,
    tag: &str,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
    let json: serde_json::Value = match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("bgutil release JSON parse failed: {e}");
                return map;
            }
        },
        Ok(resp) => {
            tracing::warn!("bgutil release API returned {}", resp.status());
            return map;
        }
        Err(e) => {
            tracing::warn!("bgutil release API request failed: {e}");
            return map;
        }
    };
    if let Some(assets) = json.get("assets").and_then(|a| a.as_array()) {
        for asset in assets {
            let name = asset.get("name").and_then(|v| v.as_str());
            let digest = asset
                .get("digest")
                .and_then(|v| v.as_str())
                .and_then(|d| d.strip_prefix("sha256:"));
            if let (Some(name), Some(digest)) = (name, digest) {
                map.insert(name.to_string(), digest.to_string());
            }
        }
    }
    map
}

pub(crate) async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    verify_executable_magic: bool,
) -> Result<(), BotError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| BotError::Config(format!("download {url}: {e}")))?;
    if !response.status().is_success() {
        return Err(BotError::Config(format!(
            "download {url} returned {}",
            response.status()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| BotError::Config(format!("read body of {url}: {e}")))?;

    match expected_sha256 {
        Some(expected) => {
            if !verify_sha256(&bytes, expected) {
                return Err(BotError::Config(format!(
                    "checksum mismatch for {url}: expected {expected}, got {}",
                    sha256_hex(&bytes)
                )));
            }
        }
        None => {
            tracing::warn!("No checksum available for {url}; skipping hash verification");
            if verify_executable_magic && !looks_like_executable(&bytes) {
                return Err(BotError::Config(format!(
                    "{url} does not look like a valid executable for this platform"
                )));
            }
        }
    }

    let tmp = dest.with_extension("download.tmp");
    {
        let mut f = fs::File::create(&tmp)
            .map_err(|e| BotError::Config(format!("create {}: {e}", tmp.display())))?;
        f.write_all(&bytes)
            .map_err(|e| BotError::Config(format!("write {}: {e}", tmp.display())))?;
    }
    fs::rename(&tmp, dest)
        .map_err(|e| BotError::Config(format!("rename to {}: {e}", dest.display())))?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn make_executable(path: &Path) -> Result<(), BotError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .map_err(|e| BotError::Config(format!("stat {}: {e}", path.display())))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
        .map_err(|e| BotError::Config(format!("chmod {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn make_executable(_path: &Path) -> Result<(), BotError> {
    Ok(())
}

pub(crate) fn extract_plugin_zip(
    zip_path: &Path,
    dest_dir: &Path,
) -> Result<(), BotError> {
    let file =
        fs::File::open(zip_path).map_err(|e| BotError::Config(format!("open zip: {e}")))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| BotError::Config(format!("read zip: {e}")))?;

    fs::create_dir_all(dest_dir)
        .map_err(|e| BotError::Config(format!("mkdir plugin dir: {e}")))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| BotError::Config(format!("zip entry {i}: {e}")))?;
        let outpath = match entry.enclosed_name() {
            Some(p) => dest_dir.join(p),
            None => continue,
        };
        if entry.is_dir() {
            fs::create_dir_all(&outpath)
                .map_err(|e| BotError::Config(format!("mkdir {}: {e}", outpath.display())))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    BotError::Config(format!("mkdir {}: {e}", parent.display()))
                })?;
            }
            let mut out = fs::File::create(&outpath)
                .map_err(|e| BotError::Config(format!("create {}: {e}", outpath.display())))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| BotError::Config(format!("write {}: {e}", outpath.display())))?;
        }
    }
    Ok(())
}
