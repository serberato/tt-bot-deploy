//! Settings and lifecycle command handler module.
//!
//! Handles `JoinChannel`, `ChangeNick`, `SetGender`, `SetStatus`,
//! `SetPlayMode`, `SetService`, `SetDefaultLanguage`, `Quit`, and `Restart`.

use crate::bot::handlers::HandlerContext;
use crate::bot::runner::BotExit;
use crate::bot::state::PlaybackStatus;
use crate::config::PlayMode;
use crate::i18n::Key;
use crate::services::Service;

pub fn handle_join_channel(ctx: &mut HandlerContext, path: String, user_id: i32) {
    let channel_id = ctx.controller.client.get_channel_id_from_path(&path);
    if channel_id == ::teamtalk::types::ChannelId(0) {
        ctx.reply_t(user_id, Key::ChannelNotFound, &[("path", path)]);
    } else {
        let _ = ctx.controller.client.join_channel(channel_id, "");
    }
}

pub fn handle_change_nick(ctx: &mut HandlerContext, name: String, _user_id: i32) {
    let _ = ctx.controller.client.change_nickname(&name);
    ctx.lifecycle.config_store.update(|cfg| {
        cfg.bot_name = name;
    });
}

pub fn handle_set_status(ctx: &mut HandlerContext, status_text: String, _user_id: i32) {
    ctx.lifecycle.config_store.update(|cfg| {
        cfg.custom_status = status_text;
    });
    if ctx.controller.state.lock().status == PlaybackStatus::Idle {
        ctx.announce_idle();
    }
}

pub fn handle_set_gender(ctx: &mut HandlerContext, gender: String, _user_id: i32) {
    let new_gender = crate::config::parse_gender(&gender);
    let current_name = ctx.controller.state.lock().current().map(|e| e.track.display_name());
    let status_text = current_name
        .map(|name| ctx.announcer.now_playing_status(&name, &ctx.controller.state))
        .unwrap_or_else(|| ctx.lifecycle.config_store.get_idle_status());
    let mut status = ::teamtalk::types::UserStatus::default();
    status.gender = new_gender;
    let _ = ctx.controller.client.set_status(status, &status_text);
    ctx.lifecycle.config_store.update(|cfg| {
        cfg.bot_gender = gender;
    });
}

pub fn handle_set_play_mode(ctx: &mut HandlerContext, mode: PlayMode, _user_id: i32) {
    ctx.lifecycle.config_store.update(|cfg| {
        cfg.play_mode = mode;
    });
}

pub fn handle_set_service(ctx: &mut HandlerContext, service: Service, _user_id: i32) {
    ctx.controller.state.lock().active_service = service;
    tracing::info!("Active service switched to {}", service.name());
}

pub fn handle_set_default_language(ctx: &mut HandlerContext, code: String, user_id: i32) {
    ctx.announcer.i18n.set_default(&code);
    ctx.lifecycle.config_store.update(|cfg| {
        cfg.default_language = code.clone();
    });
    tracing::info!("Default language set to {code}");
    ctx.announcer.reply(user_id, &format!("Default language set to {code}"));
}

pub fn handle_quit(ctx: &mut HandlerContext, _user_id: i32) {
    tracing::info!("Quit command received, shutting down...");
    ctx.do_exit(BotExit::Quit);
}

pub fn handle_restart(ctx: &mut HandlerContext, _user_id: i32) {
    tracing::info!("Restart command received...");
    ctx.do_exit(BotExit::Restart);
}
