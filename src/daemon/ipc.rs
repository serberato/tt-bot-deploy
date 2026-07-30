//! Systemd instance escaping, user session interaction, and prompts.

use std::io::{self, Write};
use std::process::Command;

/// Escape a config name for use as a systemd template instance.
pub(crate) fn systemd_escape_instance(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, b) in name.bytes().enumerate() {
        let allowed = b.is_ascii_alphanumeric()
            || b == b':'
            || b == b'_'
            || (b == b'.' && i != 0);
        if b == b'/' {
            out.push('-');
        } else if allowed {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out
}

/// Offer (y/N prompt) to enable and start `ttspotify@<name>` now.
pub fn offer_enable_instance(name: &str) {
    let instance = systemd_escape_instance(name);
    if prompt_yes_no(&format!("Enable and start ttspotify@{instance} now?")) {
        if let Err(e) = Command::new("systemctl")
            .args(["--user", "enable", &format!("ttspotify@{instance}")])
            .status()
        {
            tracing::warn!("Failed to enable systemd service ttspotify@{instance}: {e}");
        }
        if let Err(e) = Command::new("systemctl")
            .args(["--user", "start", &format!("ttspotify@{instance}")])
            .status()
        {
            tracing::warn!("Failed to start systemd service ttspotify@{instance}: {e}");
        }
        println!("  ttspotify@{instance} enabled and started.");
    }
}

/// Current login name, for loginctl calls.
pub(crate) fn current_user() -> String {
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return u;
        }
    }
    Command::new("id")
        .arg("-un")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Whether systemd lingering is enabled for `user`.
pub(crate) fn linger_enabled(user: &str) -> bool {
    Command::new("loginctl")
        .args(["show-user", user, "--property=Linger"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "Linger=yes")
        .unwrap_or(false)
}

pub(crate) fn prompt_yes_no(message: &str) -> bool {
    print!("{message} [y/N] ");
    io::stdout().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}
