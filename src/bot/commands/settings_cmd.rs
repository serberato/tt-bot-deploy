use std::sync::atomic::Ordering;
use teamtalk::Client;

use crate::bot::commands::{BotCommand, CommandDispatcher, PlaybackMode};
use crate::i18n::Key;
use crate::services::Service;

impl CommandDispatcher {
    pub(crate) fn dispatch_settings_command(
        &self,
        client: &Client,
        sender_id: i32,
        cmd: &str,
        args: &str,
    ) -> bool {
        match cmd {
            "mode" => {
                self.handle_mode_command(client, sender_id, args);
                true
            }
            "radio" => {
                self.handle_radio_command(client, sender_id, args);
                true
            }
            "link" | "url" => {
                self.handle_link_command(client, sender_id);
                true
            }
            "sp" | "spotify" => {
                self.handle_service_command(client, sender_id, Service::Spotify);
                true
            }
            "yt" | "youtube" => {
                self.handle_service_command(client, sender_id, Service::YouTube);
                true
            }
            "stats" => {
                self.handle_stats_command(client, sender_id);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn handle_mode_command(&self, client: &Client, sender_id: i32, args: &str) {
        match args.trim() {
            "r" | "repeat" => {
                self.send(BotCommand::SetMode {
                    mode: PlaybackMode::RepeatTrack,
                    user_id: sender_id,
                });
                self.reply_t(client, sender_id, Key::ModeRepeatTrack, &[]);
            }
            "rq" | "repeat_queue" => {
                self.send(BotCommand::SetMode {
                    mode: PlaybackMode::RepeatQueue,
                    user_id: sender_id,
                });
                self.reply_t(client, sender_id, Key::ModeRepeatQueue, &[]);
            }
            "s" | "shuffle" => {
                self.send(BotCommand::SetMode {
                    mode: PlaybackMode::Shuffle,
                    user_id: sender_id,
                });
                self.reply_t(client, sender_id, Key::ModeShuffle, &[]);
            }
            "off" | "o" | "none" => {
                self.send(BotCommand::SetMode {
                    mode: PlaybackMode::Off,
                    user_id: sender_id,
                });
                self.reply_t(client, sender_id, Key::ModeOff, &[]);
            }
            "direct" => {
                self.send(BotCommand::SetPlayMode {
                    mode: crate::config::PlayMode::Direct,
                    user_id: sender_id,
                });
                self.reply(
                    client,
                    sender_id,
                    "Play Mode set to: Direct (Searches will interrupt current track)",
                );
            }
            "queue" => {
                self.send(BotCommand::SetPlayMode {
                    mode: crate::config::PlayMode::Queue,
                    user_id: sender_id,
                });
                self.reply(
                    client,
                    sender_id,
                    "Play Mode set to: Queue (Searches will add to queue)",
                );
            }
            _ => {
                let state = self.state.lock();
                let display = state.mode_display();
                drop(state);
                self.reply_t(client, sender_id, Key::ModeUsage, &[("modes", display)]);
            }
        }
    }

    pub(crate) fn handle_radio_command(&self, client: &Client, sender_id: i32, args: &str) {
        if self.state.lock().active_service != Service::Spotify {
            return;
        }
        let arg = args.trim().to_lowercase();
        if arg.starts_with("on") {
            if self.state.lock().radio_enabled {
                self.reply_t(client, sender_id, Key::RadioAlreadyOn, &[]);
            } else {
                self.send(BotCommand::RadioToggle {
                    enable: true,
                    user_id: sender_id,
                });
                self.reply_t(client, sender_id, Key::RadioEnabled, &[]);
            }
        } else if arg.starts_with("off") {
            if !self.state.lock().radio_enabled {
                self.reply_t(client, sender_id, Key::RadioAlreadyOff, &[]);
            } else {
                self.send(BotCommand::RadioToggle {
                    enable: false,
                    user_id: sender_id,
                });
                self.reply_t(client, sender_id, Key::RadioDisabled, &[]);
            }
        } else {
            let key = if self.state.lock().radio_enabled {
                Key::RadioStatusOn
            } else {
                Key::RadioStatusOff
            };
            self.reply_t(client, sender_id, key, &[]);
        }
    }

    pub(crate) fn handle_link_command(&self, client: &Client, sender_id: i32) {
        let url = self.state.lock().current().map(|e| e.track.web_url());
        match url {
            Some(u) => self.reply(client, sender_id, &u),
            None => self.reply_t(client, sender_id, Key::NothingPlaying, &[]),
        }
    }

    pub(crate) fn handle_service_command(&self, client: &Client, sender_id: i32, service: Service) {
        if self.state.lock().active_service == service {
            self.reply_t(
                client,
                sender_id,
                Key::AlreadyOnService,
                &[("service", service.name().to_string())],
            );
        } else {
            self.send(BotCommand::SetService {
                service,
                user_id: sender_id,
            });
            self.reply_t(
                client,
                sender_id,
                Key::SwitchedService,
                &[("service", service.name().to_string())],
            );
        }
    }

    pub(crate) fn handle_stats_command(&self, client: &Client, sender_id: i32) {
        let uptime = self.start_time.elapsed();
        let hours = uptime.as_secs() / 3600;
        let mins = (uptime.as_secs() % 3600) / 60;
        let state = self.state.lock();
        let tracks = state.tracks_played;
        let queue_len = state.queue.len();
        let vol = self.volume.load(Ordering::Relaxed);
        drop(state);
        let uptime_str = if hours > 0 {
            format!("{hours}h {mins}m")
        } else {
            format!("{mins}m")
        };
        self.reply_t(
            client,
            sender_id,
            Key::Stats,
            &[
                ("uptime", uptime_str),
                ("tracks", tracks.to_string()),
                ("queue", queue_len.to_string()),
                ("volume", vol.to_string()),
            ],
        );
    }
}
