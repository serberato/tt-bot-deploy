//! Configuration module for ttspotify-rs.
//!
//! Separated into:
//! - `model`: Data structures, serialization attributes, defaults, and validation logic.
//! - `storage`: Disk I/O, path resolution, atomic updates, and thread-safe `ConfigStore`.

mod model;
mod storage;

pub use model::*;
pub use storage::*;
