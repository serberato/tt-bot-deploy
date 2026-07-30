use teamtalk::Client;

/// Maximum reply length before message-chunking kicks in.
pub const MAX_REPLY_LEN: usize = 500;

/// Split a message into chunks no larger than `max_len`, splitting on line
/// boundaries (never mid-line). A line that is itself longer than `max_len` is
/// returned as a single oversized chunk rather than truncated.
///
/// Empty input returns an empty Vec (nothing to send).
pub fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    for line in text.lines() {
        if !chunk.is_empty() && chunk.len() + 1 + line.len() > max_len {
            chunks.push(std::mem::take(&mut chunk));
        }
        if !chunk.is_empty() {
            chunk.push('\n');
        }
        chunk.push_str(line);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

/// Send a reply to a user, splitting at line boundaries if it exceeds MAX_REPLY_LEN.
pub fn send_reply(client: &Client, user_id: i32, text: &str) {
    let uid = ::teamtalk::types::UserId(user_id);
    for chunk in chunk_message(text, MAX_REPLY_LEN) {
        let _ = client.send_to_user(uid, &chunk);
    }
}

/// Sanitize an error for display to a user: collapse to a single line and cap
/// the length, so a raw multi-line `Display` (which may embed internal detail)
/// doesn't flood a TeamTalk PM. Logs keep the full error.
pub fn user_error(e: impl std::fmt::Display) -> String {
    const MAX: usize = 200;
    let one_line: String = e
        .to_string()
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let trimmed = one_line.trim();
    if trimmed.chars().count() > MAX {
        let head: String = trimmed.chars().take(MAX - 3).collect();
        format!("{head}...")
    } else {
        trimmed.to_string()
    }
}

/// Render a search-results numbered listing. Header and footer are passed in
/// (translated by the caller) so this stays a pure formatter.
pub fn format_search_results(
    tracks: &[crate::track::Track],
    header: &str,
    footer: &str,
) -> String {
    use std::fmt::Write as _;
    let mut msg = format!("{header}\n");
    for (i, track) in tracks.iter().enumerate() {
        let _ = writeln!(
            msg,
            "  {}: {} [{}]",
            i + 1,
            track.display_name(),
            track.duration_display()
        );
    }
    msg.push_str(footer);
    msg
}
