//! Help command handler and general bot command dispatcher.

use teamtalk::Client;

pub use crate::bot::commands::help_text::help_text;
use crate::bot::commands::help_text::help_topic_detail;
use crate::bot::commands::{BotCommand, CommandDispatcher};
use crate::i18n::Key;

impl CommandDispatcher {
    pub(crate) fn dispatch_help_or_bot_command(
        &self,
        client: &Client,
        sender_id: i32,
        username: &str,
        cmd: &str,
        args: &str,
    ) -> Option<bool> {
        if let Some(res) = self.dispatch_bot_settings_command(client, sender_id, cmd, args) {
            return Some(res);
        }
        if let Some(res) = self.dispatch_info_or_lifecycle_command(client, sender_id, cmd, args) {
            return Some(res);
        }
        if matches!(cmd, "h" | "help") {
            let res = self.handle_help_command(client, sender_id, username, args);
            return Some(res);
        }
        None
    }

    fn dispatch_bot_settings_command(
        &self,
        client: &Client,
        sender_id: i32,
        cmd: &str,
        args: &str,
    ) -> Option<bool> {
        match cmd {
            "jc" => {
                if !args.is_empty() {
                    self.send(BotCommand::JoinChannel {
                        path: args.to_string(),
                        user_id: sender_id,
                    });
                }
                Some(true)
            }
            "cn" => {
                if !args.is_empty() {
                    self.send(BotCommand::ChangeNick {
                        name: args.to_string(),
                        user_id: sender_id,
                    });
                    self.reply_t(
                        client,
                        sender_id,
                        Key::Nickname,
                        &[("name", args.to_string())],
                    );
                }
                Some(true)
            }
            "gender" => {
                let g = args.trim().to_lowercase();
                if crate::config::is_valid_gender(&g) {
                    self.send(BotCommand::SetGender {
                        gender: g.clone(),
                        user_id: sender_id,
                    });
                    self.reply_t(client, sender_id, Key::GenderSet, &[("gender", g)]);
                } else {
                    self.reply_t(client, sender_id, Key::GenderUsage, &[]);
                }
                Some(true)
            }
            "status" => {
                let status_text = args.trim().to_string();
                self.send(BotCommand::SetStatus {
                    status_text: status_text.clone(),
                    user_id: sender_id,
                });
                if status_text.is_empty() {
                    self.reply(
                        client,
                        sender_id,
                        "Status cleared. Default idle text will be used.",
                    );
                } else {
                    self.reply(client, sender_id, &format!("Status set to: {status_text}"));
                }
                Some(true)
            }
            _ => None,
        }
    }

    fn dispatch_info_or_lifecycle_command(
        &self,
        client: &Client,
        sender_id: i32,
        cmd: &str,
        _args: &str,
    ) -> Option<bool> {
        match cmd {
            "info" | "about" => {
                self.reply_t(
                    client,
                    sender_id,
                    Key::Info,
                    &[("version", env!("CARGO_PKG_VERSION").to_string())],
                );
                Some(true)
            }
            "q" | "quit" => {
                self.send(BotCommand::Quit { user_id: sender_id });
                Some(false)
            }
            "rs" | "restart" => {
                self.send(BotCommand::Restart { user_id: sender_id });
                Some(false)
            }
            _ => None,
        }
    }

    pub(crate) fn handle_help_command(
        &self,
        client: &Client,
        sender_id: i32,
        username: &str,
        args: &str,
    ) -> bool {
        let active = self.state.lock().active_service;
        let is_admin = self.is_caller_admin(client, sender_id, username);
        if args.is_empty() {
            let text = help_text(active, is_admin);
            self.reply(client, sender_id, &text);
        } else {
            let topic = args.trim().to_lowercase();
            if !is_admin
                && matches!(
                    topic.as_str(),
                    "q" | "quit" | "rs" | "restart" | "jc" | "glang"
                )
            {
                self.reply(
                    client,
                    sender_id,
                    "Unknown command. Type h for the command list.",
                );
                return true;
            }
            if let Some(detail) = help_topic_detail(&topic, active) {
                self.reply(client, sender_id, detail);
            } else {
                self.reply(
                    client,
                    sender_id,
                    "Unknown command. Type h for the command list.",
                );
            }
        }
        true
    }
}
