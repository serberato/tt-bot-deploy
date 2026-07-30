/// Result of the first-pass classification of an incoming message, before any
/// command-specific handling. Pure and unit-tested.
#[derive(Debug, PartialEq)]
pub enum Input {
    /// Empty/whitespace-only message.
    Empty,
    /// Search cancellation word (a / cancel / abort / exit).
    Cancel,
    /// Bare number (search pick). `n` is as typed (1-based; 0 is a no-op).
    Number(usize),
    /// A command word plus its (case-preserved) argument string.
    Command { name: String, args: String },
}

/// Classify raw message text: strip an optional `/`/`!` prefix, detect cancel
/// words and bare-number picks, otherwise split into a lowercased command word
/// and its trimmed argument string.
pub fn classify_input(text: &str) -> Input {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Input::Empty;
    }
    let stripped = trimmed
        .strip_prefix('/')
        .or_else(|| trimmed.strip_prefix('!'))
        .unwrap_or(trimmed);

    match stripped.to_lowercase().as_str() {
        "a" | "cancel" | "abort" | "exit" => return Input::Cancel,
        _ => {}
    }
    if let Ok(n) = stripped.parse::<usize>() {
        return Input::Number(n);
    }
    let (cmd, args) = stripped
        .split_once(|c: char| c.is_whitespace())
        .map(|(c, a)| (c, a.trim()))
        .unwrap_or((stripped, ""));
    Input::Command {
        name: cmd.to_lowercase(),
        args: args.to_string(),
    }
}

/// Parsed volume command. `Set` carries the raw requested percent (unbounded;
/// the caller clamps against `max_volume`).
#[derive(Debug, PartialEq)]
pub enum VolumeParse {
    Show,
    Set(u16),
}

/// Parse a volume command word + args, matching `v`, `volume`, `v50`, `v 50`.
/// Returns `None` if the command word is not a volume command at all.
pub fn parse_volume(cmd: &str, args: &str) -> Option<VolumeParse> {
    let is_vol_cmd = cmd == "v"
        || cmd == "volume"
        || (cmd.starts_with('v') && cmd.len() > 1 && cmd[1..].chars().all(|c| c.is_ascii_digit()));
    if !is_vol_cmd {
        return None;
    }
    let vol_str = if cmd.len() > 1 && cmd.starts_with('v') && cmd != "volume" {
        &cmd[1..]
    } else {
        args
    };
    match vol_str.parse::<u16>() {
        Ok(v) => Some(VolumeParse::Set(v)),
        Err(_) => Some(VolumeParse::Show),
    }
}

/// Parsed seek command. `Seconds` is signed (negative = backward).
#[derive(Debug, PartialEq)]
pub enum SeekParse {
    Seconds(i32),
    Usage,
}

/// Parse a seek command word + args. Matches bare `sf`/`sb` (default 10s) or
/// `sf`/`sb` immediately followed by digits (`sf10`); a non-numeric explicit
/// arg yields `Usage`. Returns `None` for anything that is not a seek command
/// (notably "sblah", which must not silently seek).
pub fn parse_seek(cmd: &str, args: &str) -> Option<SeekParse> {
    let is_seek = (cmd == "sf" || cmd == "sb")
        || ((cmd.starts_with("sf") || cmd.starts_with("sb"))
            && cmd.len() > 2
            && cmd[2..].chars().all(|c| c.is_ascii_digit()));
    if !is_seek {
        return None;
    }
    let direction: i32 = if cmd.starts_with("sf") { 1 } else { -1 };
    let num_str = if cmd.len() > 2 { &cmd[2..] } else { args };
    let secs: i32 = if num_str.is_empty() {
        10
    } else {
        match num_str.parse() {
            Ok(n) => n,
            Err(_) => return Some(SeekParse::Usage),
        }
    };
    Some(SeekParse::Seconds(direction * secs))
}
