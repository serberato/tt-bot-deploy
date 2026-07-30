use crate::track::Track;
use super::{PlaybackStatus, PlayerState};

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub track: Track,
    #[allow(dead_code)] // stored for future "who queued this" display
    pub requester: String,
    /// Only allow radio recommendations for single-track plays (not playlists/albums)
    pub allow_recommend: bool,
}

impl PlayerState {
    pub fn enqueue(&mut self, track: Track, requester: String, allow_recommend: bool) {
        self.queue.push(QueueEntry { track, requester, allow_recommend });
        if self.current_index.is_none() {
            self.current_index = Some(0);
        }
    }

    pub fn enqueue_all(&mut self, tracks: Vec<Track>, requester: &str, allow_recommend: bool) {
        let was_empty = self.queue.is_empty();
        for track in tracks {
            self.queue.push(QueueEntry {
                track,
                requester: requester.to_string(),
                allow_recommend,
            });
        }
        if was_empty && !self.queue.is_empty() {
            self.current_index = Some(0);
        }
    }

    /// Advance to the next track. Returns the next entry if available.
    pub fn advance(&mut self) -> Option<&QueueEntry> {
        if self.queue.is_empty() {
            self.current_index = None;
            return None;
        }

        if self.repeat_track {
            return self.current();
        }

        if self.shuffle {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let current = self.current_index.unwrap_or(0);
            // Only shuffle among upcoming tracks (after current).
            let remaining: Vec<usize> = ((current + 1)..self.queue.len()).collect();
            if !remaining.is_empty() {
                let idx = remaining[rng.gen_range(0..remaining.len())];
                self.current_index = Some(idx);
                return self.queue.get(idx);
            } else if self.repeat_queue && self.queue.len() > 1 {
                // All tracks played, re-shuffle from start (excluding the one that just played)
                let others: Vec<usize> = (0..self.queue.len()).filter(|&i| i != current).collect();
                if !others.is_empty() {
                    let idx = others[rng.gen_range(0..others.len())];
                    self.current_index = Some(idx);
                    return self.queue.get(idx);
                }
            }
            // Fallthrough: no more tracks
            self.current_index = None;
            return None;
        }

        if let Some(idx) = self.current_index {
            let next = idx + 1;
            if next < self.queue.len() {
                self.current_index = Some(next);
                return self.queue.get(next);
            } else if self.repeat_queue {
                self.current_index = Some(0);
                return self.queue.first();
            } else {
                self.current_index = None;
                return None;
            }
        }

        None
    }

    /// Go to previous track.
    pub fn go_prev(&mut self) -> Option<&QueueEntry> {
        if self.queue.is_empty() {
            return None;
        }

        if let Some(idx) = self.current_index {
            if idx > 0 {
                self.current_index = Some(idx - 1);
            } else if self.repeat_queue {
                self.current_index = Some(self.queue.len() - 1);
            }
        } else {
            self.current_index = Some(self.queue.len() - 1);
        }

        self.current()
    }

    pub fn clear(&mut self) {
        self.queue.clear();
        self.current_index = None;
        self.status = PlaybackStatus::Idle;
        self.position_ms = 0;
        self.bulk_load_generation += 1;
    }

    /// Drop everything after the current track (or the whole queue when
    /// nothing is playing). Also invalidates any in-flight background bulk
    /// loader — otherwise it would keep re-filling the queue the user just
    /// cleared.
    pub fn clear_upcoming(&mut self) {
        if let Some(idx) = self.current_index {
            self.queue.truncate(idx + 1);
        } else {
            self.queue.clear();
        }
        self.bulk_load_generation += 1;
    }

    /// Start a new bulk load: invalidates any in-flight background loader and
    /// returns the generation the new loader must carry.
    pub fn begin_bulk_load(&mut self) -> u64 {
        self.bulk_load_generation += 1;
        self.bulk_load_generation
    }

    /// Drop incoming tracks that are already in the queue (by track id), so
    /// repeating a bulk source (liked songs, a playlist) doesn't duplicate it.
    pub fn filter_unqueued(&self, tracks: Vec<Track>) -> Vec<Track> {
        if self.queue.len() < 32 {
            tracks
                .into_iter()
                .filter(|t| !self.queue.iter().any(|e| e.track.id() == t.id()))
                .collect()
        } else {
            let queued: std::collections::HashSet<&str> =
                self.queue.iter().map(|e| e.track.id()).collect();
            tracks
                .into_iter()
                .filter(|t| !queued.contains(t.id()))
                .collect()
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<QueueEntry> {
        if index >= self.queue.len() {
            return None;
        }
        let entry = self.queue.remove(index);

        // Adjust current index
        if let Some(ref mut cur) = self.current_index {
            if index < *cur {
                *cur -= 1;
            } else if index == *cur {
                if self.queue.is_empty() {
                    self.current_index = None;
                } else if *cur >= self.queue.len() {
                    *cur = self.queue.len() - 1;
                }
            }
        }

        Some(entry)
    }
}
