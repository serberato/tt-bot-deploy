use std::sync::Arc;
use std::time::Instant;
use parking_lot::Mutex;

use crate::services::Service;
use crate::track::Track;
use super::display;
use super::queue::{advance_index, prev_index, remove_index, QueueEntry, QueueState};
use super::search::SearchSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Idle,
    Loading,
    Playing,
    Paused,
}

#[derive(Debug, Default)]
pub struct PlayerState {
    pub queue: QueueState,
    pub current_index: Option<usize>,
    pub status: PlaybackStatus,

    // Modes
    pub repeat_track: bool,
    pub repeat_queue: bool,
    pub shuffle: bool,

    // Radio
    pub radio_enabled: bool,

    // Deconstructed cohesive sub-sessions
    pub search_session: SearchSession,

    // Track position tracking
    pub position_ms: u32,
    pub tracks_played: u32,
    pub active_service: Service,
    pub bulk_load_generation: u64,
}

pub type SharedState = Arc<Mutex<PlayerState>>;

impl PlayerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<&QueueEntry> {
        self.current_index.and_then(|idx| self.queue.get(idx))
    }

    // --- Search delegation ---
    pub fn insert_search_results(&mut self, user_id: i32, tracks: Vec<Track>) {
        self.search_session.insert(user_id, tracks);
    }

    pub fn insert_search_results_at(&mut self, user_id: i32, tracks: Vec<Track>, now: Instant) {
        self.search_session.insert_at(user_id, tracks, now);
    }

    pub fn get_search_results(&self, user_id: i32) -> Option<&Vec<Track>> {
        self.search_session.get(user_id)
    }

    pub fn remove_search_results(&mut self, user_id: i32) -> bool {
        self.search_session.remove(user_id)
    }

    pub fn pick_search_result(&self, user_id: i32, pick: usize) -> Option<Track> {
        self.search_session.pick(user_id, pick)
    }

    // --- Queue delegation & properties ---
    pub fn enqueue(&mut self, track: Track, requester: String, allow_recommend: bool) {
        let was_empty = self.queue.is_empty();
        self.queue.enqueue(track, requester, allow_recommend);
        if was_empty && self.current_index.is_none() {
            self.current_index = Some(0);
        }
    }

    pub fn enqueue_all(&mut self, tracks: Vec<Track>, requester: &str, allow_recommend: bool) {
        let was_empty = self.queue.is_empty();
        self.queue.enqueue_all(tracks, requester, allow_recommend);
        if was_empty && !self.queue.is_empty() {
            self.current_index = Some(0);
        }
    }

    pub fn advance(&mut self) -> Option<&QueueEntry> {
        let idx = advance_index(
            self.queue.len(),
            &mut self.current_index,
            self.repeat_track,
            self.repeat_queue,
            self.shuffle,
        );
        idx.and_then(|i| self.queue.get(i))
    }

    pub fn go_prev(&mut self) -> Option<&QueueEntry> {
        let idx = prev_index(self.queue.len(), &mut self.current_index, self.repeat_queue);
        idx.and_then(|i| self.queue.get(i))
    }

    pub fn clear(&mut self) {
        self.queue.clear();
        self.search_session.clear();
        self.current_index = None;
        self.status = PlaybackStatus::Idle;
        self.position_ms = 0;
        self.bulk_load_generation += 1;
    }

    pub fn clear_upcoming(&mut self) {
        if let Some(idx) = self.current_index {
            self.queue.truncate(idx + 1);
        } else {
            self.queue.clear();
        }
        self.bulk_load_generation += 1;
    }

    pub fn begin_bulk_load(&mut self) -> u64 {
        self.bulk_load_generation += 1;
        self.bulk_load_generation
    }

    pub fn filter_unqueued(&self, tracks: Vec<Track>) -> Vec<Track> {
        self.queue.filter_unqueued(tracks)
    }

    pub fn remove(&mut self, index: usize) -> Option<QueueEntry> {
        if index >= self.queue.len() {
            return None;
        }
        let len_before = self.queue.len();
        let entry = self.queue.remove(index);
        remove_index(len_before, index, &mut self.current_index);
        Some(entry)
    }

    // --- Display delegation ---
    pub fn queue_display(&self) -> String {
        display::queue_display(self)
    }

    pub fn mode_display(&self) -> String {
        display::mode_display(self)
    }
}
