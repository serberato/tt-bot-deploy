use teamtalk::Client;

use crate::bot::commands::{BotCommand, CommandDispatcher};
use crate::i18n::Key;

impl CommandDispatcher {
    pub(crate) fn dispatch_lang_command(
        &self,
        client: &Client,
        sender_id: i32,
        username: &str,
        cmd: &str,
        args: &str,
    ) -> bool {
        match cmd {
            "lang" => {
                self.handle_lang_command(client, sender_id, username, args);
                true
            }
            "glang" => {
                self.handle_glang_command(client, sender_id, args);
                true
            }
            _ => false,
        }
    }

    fn available_language_listing(&self) -> String {
        self.i18n
            .available()
            .into_iter()
            .map(|(code, name)| format!("  {code} - {name}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn available_language_codes(&self) -> String {
        self.i18n
            .available()
            .into_iter()
            .map(|(code, _)| code)
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(crate) fn handle_lang_command(
        &self,
        client: &Client,
        sender_id: i32,
        username: &str,
        args: &str,
    ) {
        let code = args.trim().to_lowercase();
        if code.is_empty() {
            let listing = self.available_language_listing();
            let current = self.i18n.lang_of(sender_id);
            self.reply(
                client,
                sender_id,
                &format!(
                    "Available languages:\n{listing}\nYour language: {current}\n\
                     Use: lang <code>, or lang clear to follow the server default"
                ),
            );
        } else if code == "clear" {
            self.i18n.clear_pref(sender_id, username);
            self.reply(
                client,
                sender_id,
                &format!(
                    "Language preference cleared. You now follow the server default ({})",
                    self.i18n.default_language()
                ),
            );
        } else if self.i18n.is_available(&code) {
            self.i18n.set_pref(sender_id, username, &code);
            let name = self.i18n.language_name(&code);
            let msg = self.i18n.tr_in(&code, Key::LangSet, &[("language", name)]);
            self.reply(client, sender_id, &msg);
        } else {
            let codes = self.available_language_codes();
            self.reply(
                client,
                sender_id,
                &format!("Unknown language: {code}. Available: {codes}"),
            );
        }
    }

    pub(crate) fn handle_glang_command(&self, client: &Client, sender_id: i32, args: &str) {
        let code = args.trim().to_lowercase();
        if code.is_empty() {
            self.reply(
                client,
                sender_id,
                &format!(
                    "Default language: {}\nUse: glang <code>",
                    self.i18n.default_language()
                ),
            );
        } else if self.i18n.is_available(&code) {
            self.send(BotCommand::SetDefaultLanguage {
                code,
                user_id: sender_id,
            });
        } else {
            let codes = self.available_language_codes();
            self.reply(
                client,
                sender_id,
                &format!("Unknown language: {code}. Available: {codes}"),
            );
        }
    }
}
