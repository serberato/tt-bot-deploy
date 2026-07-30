use super::*;

fn cmd(name: &str, args: &str) -> Input {
    Input::Command { name: name.to_string(), args: args.to_string() }
}

// -- help_text admin gating --

#[test]
fn help_hides_admin_commands_from_non_admins() {
    // Admins see the gated jc/rs/q lines.
    let admin = help_text(Service::Spotify, true);
    assert!(admin.contains("jc <path>"), "admin help should list jc");
    assert!(admin.contains("Join channel"), "admin help should list jc");
    assert!(admin.contains("Restart\n"), "admin help should list rs/Restart");
    assert!(admin.contains("Quit"), "admin help should list q/Quit");
    assert!(admin.contains("glang"), "admin help should list glang");

    // Non-admins must not even see that those commands exist.
    let plain = help_text(Service::Spotify, false);
    assert!(!plain.contains("jc <path>"), "non-admin help must hide jc");
    assert!(!plain.contains("Join channel"), "non-admin help must hide jc");
    assert!(!plain.contains("Restart\n"), "non-admin help must hide rs");
    assert!(!plain.contains("Quit"), "non-admin help must hide q");
    assert!(!plain.contains("glang"), "non-admin help must hide glang");

    // Non-gated Bot lines stay visible for everyone.
    assert!(plain.contains("Change nickname"), "cn stays visible");
    assert!(plain.contains("Bot info"), "info stays visible");
    assert!(plain.contains("lang "), "lang stays visible for everyone");
}

// -- classify_input --

#[test]
fn classify_empty_and_whitespace() {
    assert_eq!(classify_input(""), Input::Empty);
    assert_eq!(classify_input("   "), Input::Empty);
}

#[test]
fn classify_strips_slash_and_bang_prefix() {
    assert_eq!(classify_input("/next"), cmd("next", ""));
    assert_eq!(classify_input("!p photograph"), cmd("p", "photograph"));
}

#[test]
fn classify_cancel_words_case_insensitive() {
    for w in ["a", "cancel", "abort", "exit", "CANCEL", "Exit"] {
        assert_eq!(classify_input(w), Input::Cancel, "{w}");
    }
}

#[test]
fn classify_bare_number_is_pick() {
    assert_eq!(classify_input("3"), Input::Number(3));
    assert_eq!(classify_input("0"), Input::Number(0));
}

#[test]
fn classify_lowercases_command_but_preserves_args() {
    assert_eq!(classify_input("PLAY Hello World"), cmd("play", "Hello World"));
    assert_eq!(classify_input("Search Photograph"), cmd("search", "Photograph"));
}

#[test]
fn classify_command_without_args() {
    assert_eq!(classify_input("stop"), cmd("stop", ""));
}

// -- parse_volume --

#[test]
fn volume_forms() {
    assert_eq!(parse_volume("v", ""), Some(VolumeParse::Show));
    assert_eq!(parse_volume("volume", ""), Some(VolumeParse::Show));
    assert_eq!(parse_volume("v", "50"), Some(VolumeParse::Set(50)));
    assert_eq!(parse_volume("v50", ""), Some(VolumeParse::Set(50)));
    assert_eq!(parse_volume("volume", "30"), Some(VolumeParse::Set(30)));
    // Above-range still parses as Set; caller enforces the cap.
    assert_eq!(parse_volume("v101", ""), Some(VolumeParse::Set(101)));
    // Not a volume command.
    assert_eq!(parse_volume("view", ""), None);
    assert_eq!(parse_volume("next", ""), None);
}

// -- parse_seek --

#[test]
fn seek_forms() {
    assert_eq!(parse_seek("sf", ""), Some(SeekParse::Seconds(10)));
    assert_eq!(parse_seek("sb", ""), Some(SeekParse::Seconds(-10)));
    assert_eq!(parse_seek("sf30", ""), Some(SeekParse::Seconds(30)));
    assert_eq!(parse_seek("sb", "5"), Some(SeekParse::Seconds(-5)));
    assert_eq!(parse_seek("sf", "abc"), Some(SeekParse::Usage));
}

#[test]
fn seek_rejects_non_seek_words() {
    // Regression: "sblah" must NOT be treated as a seek.
    assert_eq!(parse_seek("sblah", ""), None);
    assert_eq!(parse_seek("sfx", ""), None);
    assert_eq!(parse_seek("stop", ""), None);
}

#[test]
fn user_error_collapses_and_caps() {
    assert_eq!(user_error("simple error"), "simple error");
    assert_eq!(user_error("line one\nline two\r\nthree"), "line one line two  three");
    let long = "x".repeat(500);
    let out = user_error(long);
    assert_eq!(out.chars().count(), 200);
    assert!(out.ends_with("..."));
}

#[test]
fn chunk_message_empty_returns_empty_vec() {
    assert!(chunk_message("", 500).is_empty());
}

#[test]
fn chunk_message_short_returns_single_chunk() {
    let chunks = chunk_message("hello", 500);
    assert_eq!(chunks, vec!["hello".to_string()]);
}

#[test]
fn chunk_message_exactly_max_len_returns_single_chunk() {
    let text = "a".repeat(500);
    let chunks = chunk_message(&text, 500);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), 500);
}

#[test]
fn chunk_message_multiline_under_max_returns_single_chunk() {
    let text = "line one\nline two\nline three";
    let chunks = chunk_message(text, 500);
    assert_eq!(chunks, vec![text.to_string()]);
}

#[test]
fn chunk_message_splits_on_line_boundary_not_mid_line() {
    // Build a message where each line is 60 chars; with max_len 100,
    // each chunk should hold exactly one line (since 60+1+60 = 121 > 100).
    let line = "x".repeat(60);
    let text = format!("{line}\n{line}\n{line}");
    let chunks = chunk_message(&text, 100);
    assert_eq!(chunks.len(), 3);
    for chunk in &chunks {
        assert_eq!(chunk.len(), 60);
        assert!(!chunk.contains('\n'), "chunk must not span line boundaries");
    }
}

#[test]
fn chunk_message_packs_multiple_lines_per_chunk_when_they_fit() {
    // Three 30-char lines, max 100. First two fit in one chunk
    // (30 + 1 + 30 = 61), third forces a new chunk
    // (61 + 1 + 30 = 92 fits actually). Use sizes that force 2 chunks:
    // 40-char lines, max 100. 40 + 1 + 40 = 81 fits;
    // 81 + 1 + 40 = 122 > 100 → second chunk.
    let line = "y".repeat(40);
    let text = format!("{line}\n{line}\n{line}");
    let chunks = chunk_message(&text, 100);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0], format!("{line}\n{line}"));
    assert_eq!(chunks[1], line);
}

#[test]
fn chunk_message_oversized_single_line_returned_as_one_chunk() {
    // Single line longer than max_len: current behavior is to return it as
    // one oversized chunk rather than truncate or split mid-line.
    let line = "z".repeat(700);
    let chunks = chunk_message(&line, 500);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), 700);
}

#[test]
fn chunk_message_short_input_is_returned_verbatim() {
    // Short inputs (≤ max) round-trip exactly, including any trailing newline.
    let text = "hello\n";
    let chunks = chunk_message(text, 500);
    assert_eq!(chunks, vec!["hello\n".to_string()]);
}

#[test]
fn chunk_message_long_input_with_trailing_newline_drops_empty_final_chunk() {
    // When the message is split via `lines()`, a trailing newline does not
    // emit an empty final element — `"a\n".lines()` yields just `["a"]`.
    // Build something long enough to force the split path.
    let line = "q".repeat(200);
    let text = format!("{line}\n{line}\n{line}\n");
    let chunks = chunk_message(&text, 250);
    // Each line is 200 chars; 200+1+200=401 > 250, so each chunk = 1 line.
    assert_eq!(chunks.len(), 3);
    for c in &chunks {
        assert_eq!(c.len(), 200);
        assert!(!c.ends_with('\n'));
    }
}

#[test]
fn chunk_message_blank_lines_in_middle_are_preserved() {
    let text = "alpha\n\nbeta";
    let chunks = chunk_message(text, 500);
    assert_eq!(chunks, vec!["alpha\n\nbeta".to_string()]);
}
