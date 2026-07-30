use tt_spotify_bot::i18n::{I18n, Key};

#[test]
fn test_i18n_integration_session_lifecycle() {
    let dir = std::env::temp_dir().join(format!("ttspotify_integration_i18n_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let i18n = I18n::load(&dir, "en");

    // 1. Unseeded user should use server default language (en)
    assert_eq!(i18n.lang_of(1001), "en");
    assert_eq!(i18n.tr(1001, Key::Paused, &[]), "Paused");

    // 2. Seed a user and verify Spanish translation works out of the box (embedded)
    i18n.seed(1002, "carlos");
    i18n.set_pref(1002, "carlos", "es");
    assert_eq!(i18n.lang_of(1002), "es");
    assert_eq!(i18n.tr(1002, Key::Paused, &[]), "Pausado");

    // 3. Changing server default language does not override explicit user preference
    i18n.set_default("de");
    assert_eq!(i18n.lang_of(1002), "es");

    // 4. Clearing preference reverts user to server default
    assert!(i18n.clear_pref(1002, "carlos"));
    assert_eq!(i18n.lang_of(1002), "de");

    let _ = std::fs::remove_dir_all(&dir);
}
