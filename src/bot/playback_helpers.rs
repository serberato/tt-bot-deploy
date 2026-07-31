//! Compatibility re-exports for playback and queue helper utilities.
//!
//! Authoritative definitions live in `crate::bot::controller::helpers`
//! and `crate::bot::handlers::queue`.

pub use crate::bot::controller::{
    auto_advance_is_stale, channel_move_needs_flush, spawn_drained_advance, startup_auth_plan,
    DrainWait, StartFailureBrake, StartupAuthPlan,
};
pub use crate::bot::handlers::queue::queue_wait_info;
