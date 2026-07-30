use teamtalk::Client;

use crate::bot::commands::formatting::format_search_results;
use crate::bot::commands::{BotCommand, CommandDispatcher};
use crate::i18n::Key;

impl CommandDispatcher {
    pub(crate) fn dispatch_queue_command(
        &self,
        client: &Client,
        sender_id: i32,
        cmd: &str,
        args: &str,
    ) -> bool {
        match cmd {
            "c" | "current" => {
                self.handle_current_command(client, sender_id);
                true
            }
            "queue" => {
                self.handle_queue_command(client, sender_id, args);
                true
            }
            "search" => {
                self.handle_search_command(client, sender_id, args);
                true
            }
            "pick" => {
                self.handle_pick_command(client, sender_id, args);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn handle_current_command(&self, client: &Client, sender_id: i32) {
        let state = self.state.lock();
        if let Some(entry) = state.current() {
            let pos_secs = state.position_ms / 1000;
            let pos = format!("{}:{:02}", pos_secs / 60, pos_secs % 60);
            let total = state.queue.len();
            let idx = state.current_index.map(|i| i + 1).unwrap_or(0);
            let args = [
                ("track", entry.track.display_name()),
                ("index", idx.to_string()),
                ("total", total.to_string()),
                ("position", pos),
                ("duration", entry.track.duration_display()),
                ("modes", state.mode_display()),
            ];
            drop(state);
            self.reply_t(client, sender_id, Key::CurrentTrack, &args);
        } else {
            drop(state);
            self.reply_t(client, sender_id, Key::NothingPlaying, &[]);
        }
    }

    pub(crate) fn handle_queue_command(&self, client: &Client, sender_id: i32, args: &str) {
        if args.starts_with("clear") {
            self.send(BotCommand::QueueClear { user_id: sender_id });
            self.reply_t(client, sender_id, Key::QueueCleared, &[]);
        } else if let Some(rest) = args.strip_prefix("rm") {
            let rest = rest.trim();
            if let Ok(n) = rest.parse::<usize>() {
                if n == 0 {
                    self.reply_t(client, sender_id, Key::IndexStartsAtOne, &[]);
                } else {
                    let state = self.state.lock();
                    let base = state.current_index.map(|i| i + 1).unwrap_or(0);
                    let abs_idx = base + n - 1;
                    if abs_idx >= state.queue.len() {
                        drop(state);
                        self.reply_t(
                            client,
                            sender_id,
                            Key::NoTrackAtPosition,
                            &[("position", n.to_string())],
                        );
                    } else {
                        let name = state.queue[abs_idx].track.display_name();
                        drop(state);
                        self.send(BotCommand::QueueRemove {
                            index: abs_idx,
                            user_id: sender_id,
                        });
                        self.reply_t(client, sender_id, Key::Removed, &[("name", name)]);
                    }
                }
            } else {
                self.reply_t(client, sender_id, Key::QueueRmUsage, &[]);
            }
        } else {
            let state = self.state.lock();
            let display = state.queue_display();
            drop(state);
            self.reply(client, sender_id, &display);
        }
    }

    pub(crate) fn handle_search_command(&self, client: &Client, sender_id: i32, args: &str) {
        if !args.is_empty() {
            self.send(BotCommand::SearchOnly {
                query: args.to_string(),
                user_id: sender_id,
            });
            self.reply_t(client, sender_id, Key::Searching, &[]);
        } else {
            let header = self.i18n.tr(sender_id, Key::SearchResultsHeader, &[]);
            let footer = self.i18n.tr(sender_id, Key::SearchResultsFooter, &[]);
            let msg = self
                .state
                .lock()
                .get_search_results(sender_id)
                .map(|results| format_search_results(results, &header, &footer));
            match msg {
                Some(m) => self.reply(client, sender_id, &m),
                None => self.reply_t(client, sender_id, Key::SearchUsage, &[]),
            }
        }
    }

    pub(crate) fn handle_pick_command(&self, client: &Client, sender_id: i32, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.reply_t(client, sender_id, Key::PickUsage, &[]);
        } else if let Ok(n) = trimmed.parse::<usize>() {
            if n > 0 {
                self.send(BotCommand::SearchPick {
                    user_id: sender_id,
                    pick: n - 1,
                    user_name: format!("User#{sender_id}"),
                });
            } else {
                self.reply_t(client, sender_id, Key::PickTooLow, &[]);
            }
        } else {
            self.reply_t(client, sender_id, Key::PickUsage, &[]);
        }
    }
}
