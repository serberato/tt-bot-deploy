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
                let (is_idle, q_len) = {
                    let s = self.state.lock();
                    (s.is_idle_or_no_track(), s.queue.len())
                };
                if is_idle {
                    self.reply_t(client, sender_id, Key::NothingPlaying, &[]);
                } else if q_len == 1 {
                    self.reply_t(client, sender_id, Key::NoMoreItemsInQueue, &[]);
                } else {
                    self.send(BotCommand::Next {
                        user_id: sender_id,
                        after_track: None,
                    });
                }
                Some(true)
            }
            "b" | "prev" => {
                let (is_idle, q_len) = {
                    let s = self.state.lock();
                    (s.is_idle_or_no_track(), s.queue.len())
                };
                if is_idle {
                    self.reply_t(client, sender_id, Key::NothingPlaying, &[]);
                } else if q_len == 1 {
                    self.reply_t(client, sender_id, Key::NoMoreItemsInQueue, &[]);
                } else {
                    self.send(BotCommand::Prev { user_id: sender_id });
                }
                Some(true)
            }
            "replay" | "rp" => {
                if self.state.lock().is_idle_or_no_track() {
                    self.reply_t(client, sender_id, Key::NothingPlaying, &[]);
                } else {
                    self.send(BotCommand::Replay { user_id: sender_id });
                    self.reply_t(client, sender_id, Key::RestartingTrack, &[]);
                }
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
                    Key::CurrentVolume,
                    &[("volume", vol.to_string())],
                );
            }
        }
    }

    pub(crate) fn handle_seek_command(&self, client: &Client, sender_id: i32, seek: SeekParse) {
        match seek {
            SeekParse::Seconds(secs) => {
                let (is_idle, status, current_pos_ms, duration_ms) = {
                    let s = self.state.lock();
                    let dur = s.current().map(|e| e.track.duration_ms()).unwrap_or(0);
                    (s.is_idle_or_no_track(), s.status, s.position_ms, dur)
                };
                if is_idle || (status != PlaybackStatus::Playing && status != PlaybackStatus::Paused) {
                    self.reply_t(client, sender_id, Key::NothingPlaying, &[]);
                    return;
                }
                let target_ms = current_pos_ms as i64 + (secs as i64 * 1000);
                if target_ms < 0 || target_ms > duration_ms as i64 {
                    self.reply_t(client, sender_id, Key::SeekExceedsTimeline, &[]);
                    return;
                }
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
            let (is_idle, status) = {
                let s = self.state.lock();
                (s.is_idle_or_no_track(), s.status)
            };
            if is_idle {
                self.reply_t(client, sender_id, Key::NothingPlaying, &[]);
                return;
            }
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
                    self.reply_t(client, sender_id, Key::NothingPlaying, &[]);
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
