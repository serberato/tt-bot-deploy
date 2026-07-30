use std::sync::atomic::Ordering;
use teamtalk::Client;

use crate::bot::commands::input::{SeekParse, VolumeParse};
use crate::bot::commands::{BotCommand, CommandDispatcher};
use crate::bot::state::PlaybackStatus;
use crate::i18n::Key;
use crate::services::Service;

impl CommandDispatcher {
    pub(crate) fn dispatch_playback_command(
        &self,
        client: &Client,
        sender_id: i32,
        cmd: &str,
        args: &str,
    ) -> Option<bool> {
        match cmd {
            "p" | "play" => {
                self.handle_play_command(client, sender_id, args);
                Some(true)
            }
            "s" | "stop" => {
                self.send(BotCommand::Stop { user_id: sender_id });
                Some(true)
            }
            "n" | "next" => {
                self.send(BotCommand::Next {
                    user_id: sender_id,
                    after_track: None,
                });
                Some(true)
            }
            "b" | "prev" => {
                self.send(BotCommand::Prev { user_id: sender_id });
                Some(true)
            }
            "replay" | "rp" => {
                self.send(BotCommand::Replay { user_id: sender_id });
                self.reply_t(client, sender_id, Key::RestartingTrack, &[]);
                Some(true)
            }
            "liked" | "fav" => {
                self.handle_liked_command(client, sender_id);
                Some(true)
            }
            _ => None,
        }
    }

    pub(crate) fn handle_volume_command(&self, client: &Client, sender_id: i32, vol: VolumeParse) {
        match vol {
            VolumeParse::Set(v) => {
                if v > self.max_volume as u16 {
                    self.reply_t(
                        client,
                        sender_id,
                        Key::VolumeRange,
                        &[
                            ("max", self.max_volume.to_string()),
                            ("got", v.to_string()),
                        ],
                    );
                } else {
                    let capped = (v as u8).min(self.max_volume);
                    self.volume.store(capped, Ordering::Relaxed);
                    self.send(BotCommand::SetVolume {
                        percent: capped,
                        user_id: sender_id,
                    });
                    self.reply_t(
                        client,
                        sender_id,
                        Key::VolumeSet,
                        &[("percent", capped.to_string())],
                    );
                }
            }
            VolumeParse::Show => {
                let vol = self.volume.load(Ordering::Relaxed);
                self.reply_t(
                    client,
                    sender_id,
                    Key::VolumeShow,
                    &[
                        ("percent", vol.to_string()),
                        ("max", self.max_volume.to_string()),
                    ],
                );
            }
        }
    }

    pub(crate) fn handle_seek_command(&self, client: &Client, sender_id: i32, seek: SeekParse) {
        match seek {
            SeekParse::Seconds(secs) => {
                self.send(BotCommand::Seek {
                    offset_ms: secs * 1000,
                    user_id: sender_id,
                });
                let key = if secs >= 0 {
                    Key::SeekForward
                } else {
                    Key::SeekBackward
                };
                self.reply_t(
                    client,
                    sender_id,
                    key,
                    &[("seconds", secs.abs().to_string())],
                );
            }
            SeekParse::Usage => {
                self.reply_t(client, sender_id, Key::SeekUsage, &[]);
            }
        }
    }

    pub(crate) fn handle_play_command(&self, client: &Client, sender_id: i32, args: &str) {
        if !args.is_empty() {
            self.send(BotCommand::SearchAndPlay {
                query: args.to_string(),
                user_id: sender_id,
                user_name: format!("User#{sender_id}"),
            });
            self.reply_t(client, sender_id, Key::Searching, &[]);
        } else {
            let status = self.state.lock().status;
            match status {
                PlaybackStatus::Loading | PlaybackStatus::Playing => {
                    self.send(BotCommand::Pause { user_id: sender_id });
                    self.reply_t(client, sender_id, Key::Paused, &[]);
                }
                PlaybackStatus::Paused => {
                    self.send(BotCommand::Play { user_id: sender_id });
                    self.reply_t(client, sender_id, Key::Resuming, &[]);
                }
                PlaybackStatus::Idle => {
                    self.reply_t(client, sender_id, Key::NothingToPlay, &[]);
                }
            }
        }
    }

    pub(crate) fn handle_liked_command(&self, client: &Client, sender_id: i32) {
        if self.state.lock().active_service == Service::Spotify {
            self.send(BotCommand::SearchAndPlay {
                query: "spotify:collection:liked".to_string(),
                user_id: sender_id,
                user_name: format!("User#{sender_id}"),
            });
            self.reply_t(client, sender_id, Key::LoadingLiked, &[]);
        }
    }
}
