//! Bot announcer module.
//!
//! Encapsulates TeamTalk chat notifications, i18n translations, status line updates,
//! and runner status event notifications.

use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::bot::commands::send_reply;
use crate::bot::runner::RunnerEvent;
use crate::bot::state::{PlaybackStatus, SharedState};
use crate::i18n::{I18n, Key};

/// Responsible for sending chat notifications, internationalized messages,
/// TeamTalk status text updates, and runner lifecycle events.
#[derive(Clone)]
pub struct Announcer {
    pub client: Arc<::teamtalk::Client>,
    pub i18n: Arc<I18n>,
    pub bot_gender: ::teamtalk::types::UserGender,
    pub event_tx: Option<Sender<RunnerEvent>>,
}

impl Announcer {
    pub fn new(
        client: Arc<::teamtalk::Client>,
        i18n: Arc<I18n>,
        bot_gender: ::teamtalk::types::UserGender,
        event_tx: Option<Sender<RunnerEvent>>,
    ) -> Self {
        Self {
            client,
            i18n,
            bot_gender,
            event_tx,
        }
    }

    /// Sends a raw text reply to a user, automatically splitting at line boundaries if long.
    /// Does nothing if `user_id <= 0`.
    pub fn reply(&self, user_id: i32, text: &str) {
        if user_id > 0 {
            send_reply(&self.client, user_id, text);
        }
    }

    /// Translates a message key for the target `user_id` and sends it as a reply.
    pub fn reply_t(&self, user_id: i32, key: Key, args: &[(&str, String)]) {
        if user_id > 0 {
            let text = self.i18n.tr(user_id, key, args);
            send_reply(&self.client, user_id, &text);
        }
    }

    /// Updates the bot's TeamTalk status line text, preserving its configured gender.
    pub fn set_status(&self, text: &str) {
        let mut status = ::teamtalk::types::UserStatus::default();
        status.gender = self.bot_gender;
        let _ = self.client.set_status(status, text);
    }

    /// Emits a runner status event if an observer channel is configured.
    pub fn send_event(&self, evt: RunnerEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(evt);
        }
    }

    /// Formats the "Playing/Paused: <track_name> [pos/total]" status string.
    pub fn now_playing_status(&self, track_name: &str, state: &SharedState) -> String {
        let s = state.lock();
        let total = s.queue.len();
        let prefix = match s.status {
            PlaybackStatus::Paused => "Paused",
            _ => "Playing",
        };
        if total > 1 {
            let pos = s.current_index.map(|i| i + 1).unwrap_or(1);
            format!("{prefix}: {track_name} [{pos}/{total}]")
        } else {
            format!("{prefix}: {track_name}")
        }
    }

    /// Updates the TT status line and emits a `Playing` event for the given track name.
    pub fn announce_playing(&self, track_name: &str, state: &SharedState) {
        let status_text = self.now_playing_status(track_name, state);
        self.set_status(&status_text);
        self.send_event(RunnerEvent::Playing(track_name.to_string()));
    }

    /// Updates the TT status line to the given idle text and emits an `Idle` event.
    pub fn announce_idle(&self, idle_text: &str) {
        self.set_status(idle_text);
        self.send_event(RunnerEvent::Idle);
    }

    /// Formats a user-facing error string.
    pub fn user_error(&self, err: impl std::fmt::Display) -> String {
        crate::bot::commands::user_error(err)
    }
}
