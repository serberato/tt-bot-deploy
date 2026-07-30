//! Core domain layer for Clean Architecture.
//!
//! Contains service-agnostic value objects and Hexagonal Architecture ports
//! (`MetadataProvider`) for audio and track metadata resolution.

pub mod provider;

pub use provider::{DurationMs, MetadataProvider, TrackId, TrackUri};
