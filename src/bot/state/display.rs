use std::fmt::Write;
use super::PlayerState;

impl PlayerState {
    pub fn queue_display(&self) -> String {
        if self.queue.is_empty() {
            return "Queue is empty".to_string();
        }

        let mut out = String::new();
        for (i, entry) in self.queue.iter().enumerate() {
            let marker = if self.current_index == Some(i) { "> " } else { "  " };
            if i > 0 { out.push('\n'); }
            let _ = write!(out, "{}{} [{}]: {} [{}]",
                marker, i + 1, entry.track.service().marker(),
                entry.track.display_name(), entry.track.duration_display());
        }
        out
    }

    pub fn mode_display(&self) -> String {
        let mut modes = Vec::new();
        if self.repeat_track {
            modes.push("Repeat Track");
        }
        if self.repeat_queue {
            modes.push("Repeat Queue");
        }
        if self.shuffle {
            modes.push("Shuffle");
        }
        if modes.is_empty() {
            "No modes active".to_string()
        } else {
            modes.join(", ")
        }
    }
}
