use super::*;
use crate::bot::state::PlayerState;
use crate::spotify::types::SpotifyTrack;
use crate::track::Track;

// -- startup_auth_plan --

#[test]
fn youtube_only_user_without_creds_skips_eager_connect() {
    assert_eq!(startup_auth_plan(false, false, true), StartupAuthPlan::Skip);
    assert_eq!(startup_auth_plan(false, false, false), StartupAuthPlan::Skip);
}

#[test]
fn interactive_contexts_keep_fatal_eager_connect() {
    // Cached creds present, spotify default, or both — with OAuth feasible
    // a startup failure should still abort (user is there to see/fix it).
    assert_eq!(startup_auth_plan(true, false, true), StartupAuthPlan::ConnectFatal);
    assert_eq!(startup_auth_plan(false, true, true), StartupAuthPlan::ConnectFatal);
    assert_eq!(startup_auth_plan(true, true, true), StartupAuthPlan::ConnectFatal);
}

#[test]
fn noninteractive_contexts_never_die_on_spotify_failure() {
    // systemd: OAuth infeasible. Failure must disable Spotify, not kill the
    // bot — a fatal exit here becomes a TT login/logout crash-restart loop.
    assert_eq!(startup_auth_plan(true, false, false), StartupAuthPlan::ConnectBestEffort);
    assert_eq!(startup_auth_plan(false, true, false), StartupAuthPlan::ConnectBestEffort);
    assert_eq!(startup_auth_plan(true, true, false), StartupAuthPlan::ConnectBestEffort);
}

// -- DrainWait --

#[test]
fn drain_wait_needs_two_consecutive_drained_polls() {
    let mut w = DrainWait::new();
    assert!(!w.observe(true));
    assert!(w.observe(true));
}

#[test]
fn drain_wait_resets_on_a_busy_poll() {
    // A chunk can be in flight between the channel and the framer: one
    // empty poll isn't proof. A busy poll restarts the count.
    let mut w = DrainWait::new();
    assert!(!w.observe(true));
    assert!(!w.observe(false));
    assert!(!w.observe(true));
    assert!(w.observe(true));
}

// -- auto_advance_is_stale --

#[test]
fn manual_next_is_never_stale() {
    assert!(!auto_advance_is_stale(None, Some("spotify:track:a")));
    assert!(!auto_advance_is_stale(None, None));
}

#[test]
fn auto_advance_runs_when_ended_track_is_still_current() {
    assert!(!auto_advance_is_stale(Some("spotify:track:a"), Some("spotify:track:a")));
}

#[test]
fn auto_advance_is_stale_after_queue_moved() {
    // Track A ended naturally, but a manual `n` (processed first) already
    // advanced the queue to B — the auto-advance must not fire again.
    assert!(auto_advance_is_stale(Some("spotify:track:a"), Some("spotify:track:b")));
    assert!(auto_advance_is_stale(Some("spotify:track:a"), None));
}

// -- StartFailureBrake --

#[test]
fn brake_trips_after_cap_consecutive_failures() {
    let mut brake = StartFailureBrake::new(3);
    assert!(!brake.on_failure());
    assert!(!brake.on_failure());
    assert!(brake.on_failure());
    // Tripping resets the streak.
    assert!(!brake.on_failure());
}

#[test]
fn brake_resets_on_immediate_success() {
    let mut brake = StartFailureBrake::new(3);
    assert!(!brake.on_failure());
    assert!(!brake.on_failure());
    brake.on_success();
    assert!(!brake.on_failure());
    assert!(!brake.on_failure());
    assert!(brake.on_failure());
}

#[test]
fn brake_progressive_backoff_and_consec() {
    use std::time::Duration;
    let mut brake = StartFailureBrake::new(5);
    assert_eq!(brake.consec(), 0);
    assert_eq!(brake.backoff_duration(), Duration::from_millis(500)); // 500 * 2^0 = 500ms

    assert!(!brake.on_failure());
    assert_eq!(brake.consec(), 1);
    assert_eq!(brake.backoff_duration(), Duration::from_millis(1000)); // 500 * 2^1 = 1000ms

    assert!(!brake.on_failure());
    assert_eq!(brake.consec(), 2);
    assert_eq!(brake.backoff_duration(), Duration::from_millis(2000)); // 500 * 2^2 = 2000ms

    assert!(!brake.on_failure());
    assert_eq!(brake.consec(), 3);
    assert_eq!(brake.backoff_duration(), Duration::from_millis(4000)); // 500 * 2^3 = 4000ms

    assert!(!brake.on_failure());
    assert_eq!(brake.consec(), 4);
    assert_eq!(brake.backoff_duration(), Duration::from_millis(8000)); // 500 * 2^4 = 8000ms

    brake.on_success();
    assert_eq!(brake.consec(), 0);
    assert_eq!(brake.backoff_duration(), Duration::from_millis(500));
}

// -- channel_move_needs_flush --

#[test]
fn initial_join_does_not_flush() {
    use ::teamtalk::types::ChannelId;
    // prev == 0 means we had no channel yet (first join after login).
    assert!(!channel_move_needs_flush(ChannelId(0), ChannelId(5)));
}

#[test]
fn rejoining_same_channel_does_not_flush() {
    use ::teamtalk::types::ChannelId;
    assert!(!channel_move_needs_flush(ChannelId(3), ChannelId(3)));
}

#[test]
fn moving_between_channels_flushes() {
    use ::teamtalk::types::ChannelId;
    assert!(channel_move_needs_flush(ChannelId(1), ChannelId(5)));
    assert!(channel_move_needs_flush(ChannelId(5), ChannelId(1)));
}

fn track(id: &str, duration_ms: u32) -> Track {
    Track::Spotify(SpotifyTrack {
        id: id.to_string(),
        name: format!("T{id}"),
        artists: vec!["A".to_string()],
        album: "Album".to_string(),
        duration_ms,
        uri: format!("spotify:track:{id}"),
    })
}

fn enqueue(state: &mut PlayerState, durations_ms: &[u32]) {
    for (i, d) in durations_ms.iter().enumerate() {
        state.enqueue(track(&i.to_string(), *d), "u".into(), true);
    }
}

// -- empty / not-applicable cases --

#[test]
fn queue_wait_info_empty_when_no_current() {
    let state = PlayerState::new();
    assert_eq!(queue_wait_info(&state), "");
}

#[test]
fn queue_wait_info_empty_when_only_current_track() {
    let mut state = PlayerState::new();
    enqueue(&mut state, &[180_000]);
    assert_eq!(queue_wait_info(&state), "");
}

// -- "next" position (1 upcoming) --

#[test]
fn queue_wait_info_one_upcoming_zero_position_says_next() {
    let mut state = PlayerState::new();
    // Two tracks: current full duration unplayed, one upcoming.
    // Wait = 60s remaining on current → rounds to 1 min.
    enqueue(&mut state, &[60_000, 120_000]);
    // position_ms=0 (default) → wait = 60_000 - 0 = 60_000ms → 1 min.
    assert_eq!(queue_wait_info(&state), " (next, ~1 min)");
}

#[test]
fn queue_wait_info_subtracts_position_from_current_track_wait() {
    let mut state = PlayerState::new();
    enqueue(&mut state, &[180_000, 60_000]);
    state.position_ms = 150_000; // 30s left on current
    // Wait = 30s → (30000+30000)/60000 = 1 min.
    assert_eq!(queue_wait_info(&state), " (next, ~1 min)");
}

#[test]
fn queue_wait_info_under_thirty_seconds_drops_minute_suffix() {
    let mut state = PlayerState::new();
    enqueue(&mut state, &[20_000, 60_000]);
    // Wait = 20s → (20000+30000)/60000 = 0 min → no "~N min".
    assert_eq!(queue_wait_info(&state), " (next)");
}

// -- multi-upcoming --

#[test]
fn queue_wait_info_multi_upcoming_uses_ahead_form() {
    let mut state = PlayerState::new();
    // queue [A=120s, B=60s, C=60s, D=60s], current=A, asking about D's wait.
    // upcoming_pos = total(4) - current_idx(0) - 1 = 3.
    // Wait = remaining(A=120s) + B(60s) + C(60s) = 240s = 4 min.
    // (D itself is not summed — wait is "until D starts".)
    enqueue(&mut state, &[120_000, 60_000, 60_000, 60_000]);
    assert_eq!(queue_wait_info(&state), " (3 ahead, ~4 min)");
}

#[test]
fn queue_wait_info_does_not_count_last_upcoming_track_duration() {
    // Defensive test for the "wait until the newly-queued (last) track starts"
    // semantic: skip(current+1).take(upcoming_pos - 1) excludes the final entry.
    let mut state = PlayerState::new();
    // queue [A=60s, B=60s, C=999_999_000ms (huge)], current=A.
    // wait = 60s (remaining A) + 60s (B). C is excluded.
    enqueue(&mut state, &[60_000, 60_000, 999_999_000]);
    // Wait = 120s → (120000+30000)/60000 = 2 min.
    assert_eq!(queue_wait_info(&state), " (2 ahead, ~2 min)");
}

#[test]
fn queue_wait_info_position_past_current_duration_saturates_to_zero() {
    // Edge: position_ms > current.duration_ms (shouldn't happen but
    // saturating_sub guards it). With upcoming_pos=1, only the (saturated)
    // remainder of the current track is summed → wait_ms=0 → "(next)".
    let mut state = PlayerState::new();
    enqueue(&mut state, &[10_000, 60_000]);
    state.position_ms = 99_999_999;
    assert_eq!(queue_wait_info(&state), " (next)");
}
