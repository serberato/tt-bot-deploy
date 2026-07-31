use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
#[cfg(test)]
use std::time::Duration;

use parking_lot::Mutex;

use crate::services::Service;
use crate::track::Track;

mod display;
mod queue;
mod search;

pub use queue::QueueEntry;
#[cfg(test)]
pub(crate) use search::SEARCH_RESULT_TTL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Idle,
    Loading,
    Playing,
    Paused,
}

#[derive(Debug)]
pub struct PlayerState {
    pub queue: Vec<QueueEntry>,
    pub current_index: Option<usize>,
    pub status: PlaybackStatus,

    // Modes
    pub repeat_track: bool,
    pub repeat_queue: bool,
    pub shuffle: bool,

    // Radio
    pub radio_enabled: bool,

    // Search session (user_id → (inserted_at, results)). Access via the
    // search-result helper methods so stale entries get swept.
    pub search_results: HashMap<i32, (Instant, Vec<Track>)>,

    // Track position tracking
    pub position_ms: u32,

    // Stats
    pub tracks_played: u32,

    // The service that bare commands target (e.g. `p <query>`).
    // Switched via `/sp` or `/yt`. In-memory only — resets on restart.
    pub active_service: Service,

    /// Bumped on stop/clear and each new bulk load; a background bulk loader
    /// captures the value at spawn and dies when it no longer matches.
    pub bulk_load_generation: u64,
}

pub type SharedState = Arc<Mutex<PlayerState>>;

impl Default for PlayerState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerState {
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
            current_index: None,
            status: PlaybackStatus::Idle,
            repeat_track: false,
            repeat_queue: false,
            shuffle: false,
            radio_enabled: false,
            search_results: HashMap::new(),
            position_ms: 0,
            tracks_played: 0,
            active_service: Service::default(),
            bulk_load_generation: 0,
        }
    }

    pub fn current(&self) -> Option<&QueueEntry> {
        self.current_index.and_then(|i| self.queue.get(i))
    }

    pub fn is_idle_or_no_track(&self) -> bool {
        self.status == PlaybackStatus::Idle || self.current().is_none()
    }
}

#[cfg(test)]
mod tests;
