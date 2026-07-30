use super::*;

#[test]
fn marker_returns_two_letter_code() {
    assert_eq!(Service::Spotify.marker(), "SP");
    assert_eq!(Service::YouTube.marker(), "YT");
}

#[test]
fn name_is_human_readable() {
    assert_eq!(Service::Spotify.name(), "Spotify");
    assert_eq!(Service::YouTube.name(), "YouTube");
}

#[test]
fn default_is_spotify() {
    assert_eq!(Service::default(), Service::Spotify);
}
