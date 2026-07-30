use std::time::{Duration, Instant};
use crate::track::Track;
use super::PlayerState;

/// How long a user's search results stay pickable before being swept.
/// Prevents `search_results` growing unbounded when users search and walk away.
pub(crate) const SEARCH_RESULT_TTL: Duration = Duration::from_secs(600);

impl PlayerState {
    /// Store a user's search results, timestamped, sweeping any entries older
    /// than `SEARCH_RESULT_TTL` first.
    pub fn insert_search_results(&mut self, user_id: i32, tracks: Vec<Track>) {
        self.insert_search_results_at(user_id, tracks, Instant::now());
    }

    /// Timestamp-injectable variant for tests.
    pub fn insert_search_results_at(&mut self, user_id: i32, tracks: Vec<Track>, now: Instant) {
        self.search_results
            .retain(|_, (t, _)| now.duration_since(*t) < SEARCH_RESULT_TTL);
        self.search_results.insert(user_id, (now, tracks));
    }

    /// Borrow a user's current search results, if any.
    pub fn get_search_results(&self, user_id: i32) -> Option<&Vec<Track>> {
        self.search_results.get(&user_id).map(|(_, v)| v)
    }

    /// Remove a user's search results; returns whether an entry existed.
    pub fn remove_search_results(&mut self, user_id: i32) -> bool {
        self.search_results.remove(&user_id).is_some()
    }

    /// Clone the `pick`-th result of a user's search, if present.
    pub fn pick_search_result(&self, user_id: i32, pick: usize) -> Option<Track> {
        self.search_results
            .get(&user_id)
            .and_then(|(_, v)| v.get(pick).cloned())
    }
}
