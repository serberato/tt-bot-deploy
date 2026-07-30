//! Interactive configuration wizard module.

pub mod config_builder;
pub mod prompts;
pub mod validation;

#[cfg(test)]
mod tests;

pub use config_builder::{run_wizard, SetupWizard};
pub use validation::{run_update_tools, run_youtube_setup};
#[allow(unused_imports)]
pub(crate) use validation::{parse_admin_mode_input, parse_lang_code, parse_yes_no_input};
