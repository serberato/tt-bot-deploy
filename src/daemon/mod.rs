//! Systemd service and process management module.

pub mod ipc;
pub mod pid;
pub mod service;

#[cfg(test)]
mod tests;

pub use ipc::offer_enable_instance;
pub use pid::{
    offer_restart_running_bots, running_bot_units, service_installed, systemd_booted,
};
pub use service::{install_service, offer_unit_refresh, uninstall_service};
