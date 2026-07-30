//! Systemd runtime unit and process detection utilities.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::daemon::ipc::prompt_yes_no;
use crate::daemon::service::SERVICE_NAME;

pub(crate) fn systemd_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("systemd")
        .join("user")
}

/// True when the system was booted under systemd.
pub fn systemd_booted() -> bool {
    Path::new("/run/systemd/system").exists()
}

/// True if the ttspotify@ systemd user unit file is installed.
pub fn service_installed() -> bool {
    systemd_dir().join(SERVICE_NAME).exists()
}

/// Parse `systemctl --user list-units 'ttspotify@*' --state=running` output into unit names.
pub(crate) fn parse_running_units(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|unit| unit.starts_with("ttspotify@") && unit.ends_with(".service"))
        .map(str::to_string)
        .collect()
}

/// Names of the `ttspotify@` user units currently running.
pub fn running_bot_units() -> Vec<String> {
    let out = Command::new("systemctl")
        .args([
            "--user",
            "list-units",
            "ttspotify@*",
            "--state=running",
            "--plain",
            "--no-legend",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => parse_running_units(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// After a successful self-update, offer to restart the running bot units.
pub fn offer_restart_running_bots() {
    let units = running_bot_units();
    if units.is_empty() {
        println!("If running as a service, restart it: systemctl --user restart ttspotify@<name>");
        return;
    }
    if !prompt_yes_no(&format!("Restart {} running bot(s) now?", units.len())) {
        println!("Restart later with: systemctl --user restart ttspotify@<name>");
        return;
    }
    for unit in &units {
        let ok = Command::new("systemctl")
            .args(["--user", "restart", unit])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("  {unit} restarted.");
        } else {
            println!("  {unit} failed to restart - check: systemctl --user status {unit}");
        }
    }
}
