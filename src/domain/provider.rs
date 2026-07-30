//! Domain metadata provider port and value objects.
//!
//! Defines the Hexagonal Architecture ports (`MetadataProvider`) for metadata
//! resolution and strongly-typed domain primitives (`TrackId`, `TrackUri`,
//! `DurationMs`) to prevent primitive obsession and decouple service adapters.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use crate::error::BotError;
use crate::track::Track;

/// Strongly-typed identifier for a track across any music service.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);

impl TrackId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Strongly-typed URI or URL used to load a track in a service player.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackUri(pub String);

impl TrackUri {
    pub fn new(uri: impl Into<String>) -> Self {
        Self(uri.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TrackUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Strongly-typed duration in milliseconds with human-readable formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DurationMs(pub u32);

impl DurationMs {
    pub fn new(ms: u32) -> Self {
        Self(ms)
    }

    pub fn as_millis(&self) -> u32 {
        self.0
    }

    pub fn as_secs(&self) -> u32 {
        self.0 / 1000
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Formats the duration as `MM:SS` or `HH:MM:SS` for display in chat.
    pub fn display_formatted(&self) -> String {
        let total_secs = self.as_secs();
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;

        if hours > 0 {
            format!("{}:{:02}:{:02}", hours, mins, secs)
        } else {
            format!("{}:{:02}", mins, secs)
        }
    }
}

impl fmt::Display for DurationMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_formatted())
    }
}

/// Domain port for searching and resolving track metadata from external services.
///
/// Both Spotify and YouTube adapters implement this trait so the application
/// layer can resolve queries without coupling to specific service implementations.
pub trait MetadataProvider: Send + Sync {
    /// Search for tracks matching the query string up to `limit` results.
    fn search_tracks<'a>(
        &'a self,
        query: &'a str,
        limit: u8,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Track>, BotError>> + Send + 'a>>;

    /// Resolve an arbitrary query (URL, URI, or search string) into a list of tracks.
    fn resolve<'a>(
        &'a self,
        query: &'a str,
        limit: u8,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Track>, BotError>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_id_display_and_as_str() {
        let id = TrackId::new("spotify:track:12345");
        assert_eq!(id.as_str(), "spotify:track:12345");
        assert_eq!(id.to_string(), "spotify:track:12345");
    }

    #[test]
    fn track_uri_display_and_as_str() {
        let uri = TrackUri::new("https://youtube.com/watch?v=abcde");
        assert_eq!(uri.as_str(), "https://youtube.com/watch?v=abcde");
        assert_eq!(uri.to_string(), "https://youtube.com/watch?v=abcde");
    }

    #[test]
    fn duration_ms_formatting_mm_ss() {
        let d = DurationMs::new(65_000); // 1 min 5 sec
        assert_eq!(d.display_formatted(), "1:05");
        assert_eq!(d.to_string(), "1:05");
    }

    #[test]
    fn duration_ms_formatting_hh_mm_ss() {
        let d = DurationMs::new(3_665_000); // 1 hr 1 min 5 sec
        assert_eq!(d.display_formatted(), "1:01:05");
    }

    #[test]
    fn duration_ms_zero() {
        let d = DurationMs::default();
        assert!(d.is_zero());
        assert_eq!(d.display_formatted(), "0:00");
    }
}
