//! Command-line interface and daemon execution module.

pub mod args;
pub mod run;

pub use args::{parse_args, pin_teamtalk_sdk_version, Args, PINNED_TEAMTALK_SDK_VERSION};
pub use run::run_cli;
