# Phase 8: Master Refactoring Plan for ttspotify-rs

- [x] 1. Deconstruct `src/bot/commands.rs` into a modular directory `src/bot/commands/`
  - [x] 1.1 Create `src/bot/commands/mod.rs` re-exporting symbols and declaring submodules
  - [x] 1.2 Create `src/bot/commands/input.rs` (`Input` enum, `classify_input`, `parse_volume`, `parse_seek`)
  - [x] 1.3 Create `src/bot/commands/formatting.rs` (`send_reply`, `user_error`, `chunk_message`)
  - [x] 1.4 Create `src/bot/commands/playback.rs` (`handle_volume_command`, `handle_seek_command`, `handle_play_command`, `handle_liked_command`)
  - [x] 1.5 Create `src/bot/commands/queue_cmd.rs` (`handle_current_command`, `handle_queue_command`, `handle_pick_command`)
  - [x] 1.6 Create `src/bot/commands/settings_cmd.rs` (`handle_mode_command`, `handle_radio_command`, `handle_link_command`, `handle_service_command`, `handle_stats_command`)
  - [x] 1.7 Create `src/bot/commands/lang_cmd.rs` (`handle_lang_command`, `handle_glang_command`, language listing helpers)
  - [x] 1.8 Create `src/bot/commands/help.rs` (`handle_help_command`, `help_text`, dynamic help strings)
  - [x] 1.9 Create `src/bot/commands/tests.rs` with relocated unit tests
  - [x] 1.10 Delete `src/bot/commands.rs` and refactor `CommandDispatcher::dispatch`
- [x] 2. Deconstruct `src/bot/runner.rs` into a modular directory `src/bot/runner/`
  - [x] 2.1 Create `src/bot/runner/mod.rs` (`run_bot` facade orchestrator, `BotExit`, `RunnerEvent`, re-exports, `#[cfg(test)] mod tests;`)
  - [x] 2.2 Create `src/bot/runner/context.rs` (`CmdContext`, atomic shared flags, channel tracking)
  - [x] 2.3 Create `src/bot/runner/lifecycle.rs` (Connection lifecycle, channel join/move logic, shutdown trapping)
  - [x] 2.4 Create `src/bot/runner/command_loop.rs` (`command_processor` async loop)
  - [x] 2.5 Create `src/bot/runner/player_loop.rs` (`player_event_loop` and auto-advance rules)
  - [x] 2.6 Create `src/bot/runner/spotify_recovery.rs` (`SpotifyRecovery`, `rebuild_spotify_engine`, `recover_spotify`, `spotify_supervisor`)
  - [x] 2.7 Create `src/bot/runner/tests.rs` with relocated unit tests
  - [x] 2.8 Delete `src/bot/runner.rs` and refactor `run_bot` function
- [x] 3. Verify that all existing crate imports (`crate::bot::commands::*`, `crate::bot::runner::*`) continue to work without breaking external references
- [x] 4. Verify that `cargo check --tests` and `cargo test --all` pass 100% cleanly in `e:\reescrito\tt-bot\ttspotify-rs` with 0 errors and 0 warnings
- [x] 5. Ensure Clean Architecture, SOLID, and Clean Code principles are followed

# Phase 9: Domain Core Deconstruct & Phase 7 Cleanup

- [x] 1. Complete Domain Core Deconstruct & Cleanup (`src/i18n/`, `src/bot/state/`, `src/config/model/`, `src/config/storage/`, `src/services/`)
  - [x] 1.1 Deconstruct `src/i18n.rs` into `src/i18n/` directory (`mod.rs`, `key.rs`, `catalog.rs`, `prefs.rs`, `validation.rs`, `tests.rs`) and DELETE `src/i18n.rs`
  - [x] 1.2 Deconstruct `src/bot/state.rs` into `src/bot/state/` directory (`mod.rs`, `queue.rs`, `search.rs`, `display.rs`, `tests.rs`) and DELETE `src/bot/state.rs`
  - [x] 1.3 Clean up `src/config/model/`, `src/config/storage/`, and `src/services/` by DELETING `src/config/model.rs`, `src/config/storage.rs`, `src/services.rs` and creating `mod.rs` in each directory
  - [x] 1.4 Refactor `src/bot/controller.rs` into small, cohesive helpers so it is strictly under 200-250 lines
- [x] 2. Verify that all existing crate references (`crate::i18n::*`, `crate::bot::state::*`, `crate::config::*`, etc.) continue to work via re-exports
- [x] 3. Verify that `cargo check --tests` and `cargo test --all` pass 100% cleanly in `e:\reescrito\tt-bot\ttspotify-rs` with 0 errors and 0 warnings
- [x] 4. Follow Clean Architecture, SOLID, and Clean Code principles

# Phase 10: Final Deconstruction & CLI Clean Entry

- [x] 1. Deconstruct `src/youtube/player.rs` into `src/youtube/player/` (`mod.rs`, `streamer.rs`, `decoder.rs`, `interleave.rs`, `tests.rs`), break down `decode_and_stream` (<50 lines each), and DELETE `src/youtube/player.rs`
- [x] 2. Deconstruct `src/wizard.rs` into `src/wizard/` (`mod.rs`, `prompts.rs`, `config_builder.rs`, `validation.rs`, `tests.rs`) and DELETE `src/wizard.rs`
- [x] 3. Deconstruct `src/audio/pipeline.rs` into `src/audio/pipeline/` (`mod.rs`, `encoder.rs`, `resampler.rs`, `buffer.rs`, `tests.rs`) and DELETE `src/audio/pipeline.rs`
- [x] 4. Deconstruct `src/daemon.rs` into `src/daemon/` (`mod.rs`, `service.rs`, `pid.rs`, `ipc.rs`, `tests.rs`) and DELETE `src/daemon.rs`
- [x] 5. Break down `handle_search_and_play` God Function in `src/bot/handlers/queue.rs` into focused helper functions (<50 lines each)
- [x] 6. Extract CLI Logic out of `src/main.rs` into `src/cli/` (`mod.rs`, `args.rs`, `run.rs`) so `src/main.rs` is < 50 lines
- [x] 7. Verify all existing crate references continue to work via re-exports and check `build.rs`
- [x] 8. Verify that `cargo check --tests` and `cargo test --all` pass 100% cleanly with 0 errors and 0 warnings

# Phase 11: Exhaustive Anti-Pattern, God-Function, Unwrap, Error-Traceability & Zombie-Prevention Sweep

- [x] 1. Pillar 1: Zero God Functions & God Objects Sweep (`src/`)
  - [x] 1.1 Inspect `src/bot/handlers/queue.rs` and refactor/break down any remaining helper >50 lines or file length >200-250 lines
  - [x] 1.2 Inspect `src/bot/runner/mod.rs`, `src/bot/handlers/playback.rs`, and other modules to ensure functions <= 50 lines and clean structure
  - [x] 1.3 Verify no production file exceeds 200–250 lines and all functions are focused (<50 lines)
- [x] 2. Pillar 2: Zero `.unwrap()` / `.expect()` in Production Code
  - [x] 2.1 Audit all non-test code in `src/` for `.unwrap()` / `.expect()` calls
  - [x] 2.2 Replace production `.unwrap()` / `.expect()` in `src/bot/commands/formatting.rs`, `src/spotify/types.rs`, and anywhere else with safe pattern matching, `Result` propagation (`?`), or graceful fallbacks
- [x] 3. Pillar 3: Full Error Traceability (`tracing::error!` / `warn!`)
  - [x] 3.1 Inspect YouTube (`src/youtube/player/streamer.rs` and related) to ensure `stderr` is captured and/or non-zero exit codes are logged with `tracing::error!` / `tracing::warn!`
  - [x] 3.2 Inspect Spotify (`src/spotify/`), Network (`src/tt/`), Config (`src/config/`), and Audio (`src/audio/`) to ensure errors are explicitly logged with context instead of being silently swallowed
- [x] 4. Pillar 4: Zombie Process Prevention (`kill_on_drop(true)` & Clean Cleanup)
  - [x] 4.1 Check `src/youtube/player/streamer.rs` and all child process spawners (`tokio::process::Command` / `std::process::Command`)
  - [x] 4.2 Ensure `.kill_on_drop(true)` is set on all spawned `tokio::process::Command` instances and child processes are explicitly cleaned up on drop, skip, or channel switch
- [x] 5. Pillar 5: Zero Duplication & Idiomatic Rust Sweep
  - [x] 5.1 Sweep `src/` to remove redundant `.clone()`, clean up unidiomatic patterns, and eliminate boilerplate duplication
- [x] 6. Verification & Validation
  - [x] 6.1 Verify `cargo check --tests` and `cargo test --all` pass 100% cleanly with 0 errors and 0 warnings

