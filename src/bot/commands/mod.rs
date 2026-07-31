use std::sync::atomic::AtomicU8;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

use teamtalk::Client;

use crate::bot::state::SharedState;
use crate::i18n::{I18n, Key};
use crate::services::Service;

pub mod formatting;
pub mod help;
pub mod help_text;
pub mod input;
pub mod lang_cmd;
pub mod playback;
pub mod queue_cmd;
pub mod settings_cmd;

pub use formatting::{
    chunk_message, format_search_results, send_reply, user_error, MAX_REPLY_LEN,
};
pub use help::help_text;
pub use input::{classify_input, parse_seek, parse_volume, Input, SeekParse, VolumeParse};

/// Commands sent from the bot thread to the async command processor.
#[derive(Debug)]
#[allow(dead_code)] // user_id fields kept for consistent command protocol + debug logging
pub enum BotCommand {
    SearchAndPlay { query: String, user_id: i32, user_name: String },
    Play { user_id: i32 },
    Pause { user_id: i32 },
    Stop { user_id: i32 },
    /// `after_track`: Some(uri) when sent automatically because that track
    /// ended or failed — the handler drops it if the queue already moved past
    /// that track (races with a manual `n`). None for a user-issued skip.
    Next { user_id: i32, after_track: Option<String> },
    Prev { user_id: i32 },
    Seek { offset_ms: i32, user_id: i32 },
    SetVolume { percent: u8, user_id: i32 },
    SetMode { mode: PlaybackMode, user_id: i32 },
    RadioToggle { enable: bool, user_id: i32 },
    QueueClear { user_id: i32 },
    QueueRemove { index: usize, user_id: i32 },
    SearchOnly { query: String, user_id: i32 },
    SearchPick { user_id: i32, pick: usize, user_name: String },
    JoinChannel { path: String, user_id: i32 },
    ChangeNick { name: String, user_id: i32 },
    SetGender { gender: String, user_id: i32 },
    SetStatus { status_text: String, user_id: i32 },
    SetPlayMode { mode: crate::config::PlayMode, user_id: i32 },
    Quit { user_id: i32 },
    Restart { user_id: i32 },
    SetService { service: Service, user_id: i32 },
    /// Admin: set the server-wide default language (glang). Persisted to config.
    SetDefaultLanguage { code: String, user_id: i32 },
    /// Internal: pre-fetch radio recommendations for the given seed track
    RadioPreFetch { seed_uri: String },
    /// Internal: preload next track for gapless playback
    Replay {
        user_id: i32,
    },
    PreloadNext,
    /// Internal: a YouTube track finished (or errored). `generation` identifies
    /// which load this belongs to so a stale completion (after the user already
    /// skipped/stopped) is dropped instead of double-advancing the queue.
    /// `error` carries a short failure reason when playback did not end cleanly.
    TrackEnded { generation: u64, error: Option<String> },
    /// Internal: debounced stream start after queue change (coalescing rapid clicks)
    StartSettledTrack {
        service: Service,
        uri: String,
        user_id: i32,
        name: String,
    },
    /// Internal: circuit breaker tripped after 3 consecutive failures
    CircuitBreakerTrip {
        service: Service,
    },
}

#[derive(Debug)]
pub enum PlaybackMode {
    RepeatTrack,
    RepeatQueue,
    Shuffle,
    Off,
}

/// Shared resources for command dispatch.
pub struct CommandDispatcher {
    pub state: SharedState,
    pub volume: Arc<AtomicU8>,
    pub cmd_tx: UnboundedSender<BotCommand>,
    pub max_volume: u8,
    pub start_time: std::time::Instant,
    pub auth: crate::bot::auth::AdminAuth,
    pub i18n: Arc<I18n>,
}

impl CommandDispatcher {
    pub(crate) fn send(&self, cmd: BotCommand) {
        if let Err(e) = self.cmd_tx.send(cmd) {
            tracing::error!("Failed to send command: {e}");
        }
    }

    pub(crate) fn reply(&self, client: &Client, user_id: i32, text: &str) {
        send_reply(client, user_id, text);
    }

    /// Reply with a translated message, resolved for the target user's
    /// language (seeded at dispatch). Help and the language-control surface
    /// keep using plain `reply` (always English) per the i18n design.
    pub(crate) fn reply_t(&self, client: &Client, user_id: i32, key: Key, args: &[(&str, String)]) {
        self.reply(client, user_id, &self.i18n.tr(user_id, key, args));
    }

    /// Whether the caller may use admin-gated commands. Resolves the sender's
    /// TeamTalk user_type from the client cache (lazy; only callers that need
    /// it call this). Falls back to non-admin (0) if the user is not cached.
    pub(crate) fn is_caller_admin(&self, client: &Client, sender_id: i32, username: &str) -> bool {
        let user_type = client
            .get_user(::teamtalk::types::UserId(sender_id))
            .map(|u| u.user_type)
            .unwrap_or(0);
        self.auth.is_admin(username, user_type)
    }

    /// Dispatch a text message as a command. Returns true if handled, false to stop the bot.
    pub fn dispatch(&self, client: &Client, text: &str, sender_id: i32, username: &str) -> bool {
        self.i18n.seed(sender_id, username);

        let (cmd, args) = match classify_input(text) {
            Input::Empty => return true,
            Input::Cancel => {
                let mut state = self.state.lock();
                let removed = state.remove_search_results(sender_id);
                drop(state);
                if removed {
                    self.reply_t(client, sender_id, Key::SearchCancelled, &[]);
                }
                return true;
            }
            Input::Number(n) => {
                if n > 0 {
                    self.send(BotCommand::SearchPick {
                        user_id: sender_id,
                        pick: n - 1,
                        user_name: format!("User#{sender_id}"),
                    });
                }
                return true;
            }
            Input::Command { name, args } => (name, args),
        };
        let args = args.as_str();

        if crate::bot::auth::is_admin_command(&cmd)
            && !self.is_caller_admin(client, sender_id, username)
        {
            return true;
        }

        tracing::info!("Command from user {sender_id}: {cmd} {args}");

        if let Some(vol) = parse_volume(&cmd, args) {
            self.handle_volume_command(client, sender_id, vol);
            return true;
        }
        if let Some(seek) = parse_seek(&cmd, args) {
            self.handle_seek_command(client, sender_id, seek);
            return true;
        }

        if let Some(keep_running) = self.dispatch_playback_command(client, sender_id, &cmd, args) {
            return keep_running;
        }
        if self.dispatch_queue_command(client, sender_id, &cmd, args) {
            return true;
        }
        if self.dispatch_settings_command(client, sender_id, &cmd, args) {
            return true;
        }
        if self.dispatch_lang_command(client, sender_id, username, &cmd, args) {
            return true;
        }
        if let Some(keep_running) =
            self.dispatch_help_or_bot_command(client, sender_id, username, &cmd, args)
        {
            return keep_running;
        }

        true
    }
}

#[cfg(test)]
mod tests;
