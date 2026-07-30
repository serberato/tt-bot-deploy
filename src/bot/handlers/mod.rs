//! Command handler domain submodules and routing facade.
//!
//! Exposes domain submodules (`playback`, `queue`, `radio`, `settings`)
//! and routing entry point (`handle_command`).

pub mod context;
pub mod playback;
pub mod queue;
pub mod radio;
pub mod settings;

pub use context::*;

use crate::bot::commands::BotCommand;

/// Dispatch a `BotCommand` to the appropriate domain handler.
pub async fn handle_command(cmd: BotCommand, ctx: &mut HandlerContext) {
    match cmd {
        // Playback domain
        BotCommand::Play { user_id } => playback::handle_play(ctx, user_id),
        BotCommand::Pause { user_id } => playback::handle_pause(ctx, user_id),
        BotCommand::Stop { user_id } => playback::handle_stop(ctx, user_id),
        BotCommand::Next { user_id, after_track } => playback::handle_next(ctx, user_id, after_track).await,
        BotCommand::Prev { user_id } => playback::handle_prev(ctx, user_id),
        BotCommand::Seek { offset_ms, user_id } => playback::handle_seek(ctx, offset_ms, user_id),
        BotCommand::SetVolume { percent, user_id } => playback::handle_set_volume(ctx, percent, user_id),
        BotCommand::SetMode { mode, user_id } => playback::handle_set_mode(ctx, mode, user_id),
        BotCommand::Replay { user_id } => playback::handle_replay(ctx, user_id),
        BotCommand::PreloadNext => playback::handle_preload_next(ctx),
        BotCommand::TrackEnded { generation, error } => playback::handle_track_ended(ctx, generation, error),

        // Queue domain
        BotCommand::SearchAndPlay { query, user_id, user_name } => {
            queue::handle_search_and_play(ctx, query, user_id, user_name).await;
        }
        BotCommand::SearchOnly { query, user_id } => {
            queue::handle_search_only(ctx, query, user_id).await;
        }
        BotCommand::SearchPick { user_id, pick, user_name } => {
            queue::handle_search_pick(ctx, user_id, pick, user_name);
        }
        BotCommand::QueueClear { user_id } => queue::handle_queue_clear(ctx, user_id),
        BotCommand::QueueRemove { index, user_id } => queue::handle_queue_remove(ctx, index, user_id),

        // Radio domain
        BotCommand::RadioToggle { enable, user_id } => radio::handle_radio_toggle(ctx, enable, user_id),
        BotCommand::RadioPreFetch { seed_uri } => {
            radio::handle_radio_prefetch(ctx, seed_uri).await;
        }

        // Settings / lifecycle domain
        BotCommand::JoinChannel { path, user_id } => settings::handle_join_channel(ctx, path, user_id),
        BotCommand::ChangeNick { name, user_id } => settings::handle_change_nick(ctx, name, user_id),
        BotCommand::SetGender { gender, user_id } => settings::handle_set_gender(ctx, gender, user_id),
        BotCommand::SetStatus { status_text, user_id } => settings::handle_set_status(ctx, status_text, user_id),
        BotCommand::SetPlayMode { mode, user_id } => settings::handle_set_play_mode(ctx, mode, user_id),
        BotCommand::SetService { service, user_id } => settings::handle_set_service(ctx, service, user_id),
        BotCommand::SetDefaultLanguage { code, user_id } => settings::handle_set_default_language(ctx, code, user_id),
        BotCommand::Quit { user_id } => settings::handle_quit(ctx, user_id),
        BotCommand::Restart { user_id } => settings::handle_restart(ctx, user_id),
    }
}
