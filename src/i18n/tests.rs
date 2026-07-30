use super::*;

// -- parse_lang --

#[test]
fn parse_lang_basic_trim_and_comments() {
    let map = parse_lang(
        "# a comment\n\n  paused =  Pausiert  \nlang_set = Sprache: {language}\n",
    );
    assert_eq!(map.get("paused").unwrap(), "Pausiert");
    assert_eq!(map.get("lang_set").unwrap(), "Sprache: {language}");
    assert_eq!(map.len(), 2);
}

#[test]
fn parse_lang_keeps_equals_in_value() {
    let map = parse_lang("formula = a = b + c");
    assert_eq!(map.get("formula").unwrap(), "a = b + c");
}

#[test]
fn parse_lang_skips_malformed_line_not_whole_file() {
    let map = parse_lang("good = ok\nthis line has no equals sign\nalso = fine");
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("good").unwrap(), "ok");
    assert_eq!(map.get("also").unwrap(), "fine");
}

#[test]
fn parse_lang_skips_empty_key() {
    let map = parse_lang("= orphan value\nok = yes");
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("ok"));
}

#[test]
fn parse_lang_unescapes_newline() {
    let map = parse_lang(r"two_lines = first\nsecond");
    assert_eq!(map.get("two_lines").unwrap(), "first\nsecond");
}

#[test]
fn parse_lang_reads_language_name() {
    let map = parse_lang("language_name = Deutsch");
    assert_eq!(map.get("language_name").unwrap(), "Deutsch");
}

// -- fill --

#[test]
fn fill_substitutes_named_slots() {
    assert_eq!(
        fill("Volume: {percent}%", &[("percent", "40".to_string())]),
        "Volume: 40%"
    );
}

#[test]
fn fill_allows_reordered_slots() {
    // Translator moved the slots around; substitution is by name.
    assert_eq!(
        fill(
            "Max {max}%, now {percent}%",
            &[("percent", "30".to_string()), ("max", "90".to_string())]
        ),
        "Max 90%, now 30%"
    );
}

#[test]
fn fill_leaves_unknown_slot_visible() {
    assert_eq!(fill("Hello {nobody}", &[]), "Hello {nobody}");
}

#[test]
fn fill_handles_no_slots_and_unmatched_brace() {
    assert_eq!(fill("Paused", &[]), "Paused");
    assert_eq!(fill("odd { brace", &[]), "odd { brace");
}

#[test]
fn fill_does_not_rescan_substituted_values() {
    // A value containing a slot-shaped string must not be substituted again.
    assert_eq!(
        fill(
            "{a} {b}",
            &[("a", "{b}".to_string()), ("b", "two".to_string())]
        ),
        "{b} two"
    );
}

// -- Catalog::t --

fn catalog_with_de() -> Catalog {
    let mut c = Catalog::new_embedded();
    c.add_language(
        "de",
        parse_lang("language_name = Deutsch\nlang_set = Sprache auf {language} gesetzt"),
    );
    c
}

#[test]
fn t_uses_language_when_present() {
    let c = catalog_with_de();
    assert_eq!(
        c.t("de", Key::LangSet, &[("language", "Deutsch".to_string())]),
        "Sprache auf Deutsch gesetzt"
    );
}

#[test]
fn t_falls_back_to_english_for_unknown_language() {
    let c = Catalog::new_embedded();
    assert_eq!(
        c.t("xx", Key::LangSet, &[("language", "English".to_string())]),
        "Language set to English"
    );
}

#[test]
fn t_falls_back_to_english_for_missing_key() {
    let mut c = Catalog::new_embedded();
    // A language file with no lang_set entry at all.
    c.add_language("pt", parse_lang("language_name = Portugues"));
    assert_eq!(
        c.t("pt", Key::LangSet, &[("language", "Portugues".to_string())]),
        "Language set to Portugues"
    );
}

#[test]
fn language_name_falls_back_to_code() {
    let c = catalog_with_de();
    assert_eq!(c.language_name("de"), "Deutsch");
    assert_eq!(c.language_name("zz"), "zz");
}

// -- completeness --

#[test]
fn every_key_has_an_english_entry() {
    let c = Catalog::new_embedded();
    for key in Key::ALL {
        assert!(
            c.template(ENGLISH, key.id()).is_some(),
            "en.lang is missing an entry for key `{}`",
            key.id()
        );
    }
}

// -- validation --

#[test]
fn validate_reports_coverage_and_mismatches() {
    let mut c = Catalog::new_embedded();
    // lang_set drops {language} and adds a typo'd key.
    c.add_language("de", parse_lang("lang_set = Sprache gesetzt\npausd = Pausiert"));
    let v = validate(&c, "de");
    assert_eq!(v.present, 1);
    assert_eq!(v.total, Key::ALL.len());
    assert_eq!(v.slot_mismatches, vec!["lang_set".to_string()]);
    assert_eq!(v.unknown_keys, vec!["pausd".to_string()]);
}

// -- LangPrefs --

#[test]
fn lang_prefs_set_get_persist_round_trip() {
    let dir = std::env::temp_dir().join(format!("ttspotify_langprefs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lang_prefs.json");

    // Missing file -> empty prefs.
    let mut prefs = LangPrefs::load(path.clone());
    assert!(prefs.get("alice").is_none());

    // Set persists; lookups are case-insensitive on username.
    prefs.set("Alice", "PT");
    assert_eq!(prefs.get("alice"), Some("pt"));
    assert_eq!(prefs.get("ALICE"), Some("pt"));

    // A fresh load reads the same pick back.
    let reloaded = LangPrefs::load(path.clone());
    assert_eq!(reloaded.get("alice"), Some("pt"));

    // A corrupt file degrades to empty prefs, not a crash.
    std::fs::write(&path, "not json").unwrap();
    let broken = LangPrefs::load(path);
    assert!(broken.get("alice").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validate_accepts_moved_slots() {
    let mut c = Catalog::new_embedded();
    // Same slot, different position: valid, no mismatch.
    c.add_language("de", parse_lang("lang_set = {language} ist jetzt aktiv"));
    let v = validate(&c, "de");
    assert!(v.slot_mismatches.is_empty());
}

// -- I18n runtime --

/// A temp config dir with a de.lang translation, torn down by the caller.
fn runtime_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ttspotify_i18n_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lang")).unwrap();
    std::fs::write(
        dir.join("lang").join("de.lang"),
        "language_name = Deutsch\nlang_set = Sprache auf {language} gesetzt\n",
    )
    .unwrap();
    dir
}

#[test]
fn runtime_resolution_order_pref_then_default_then_english() {
    let dir = runtime_dir("resolve");
    let i18n = I18n::load(&dir, "de");

    // Unseeded user id -> server default (de).
    assert_eq!(i18n.lang_of(99), "de");

    // Seeded user without a pref -> server default.
    i18n.seed(1, "alice");
    assert_eq!(i18n.lang_of(1), "de");

    // A personal pick beats the default and survives re-seeding.
    i18n.set_pref(1, "alice", "en");
    assert_eq!(i18n.lang_of(1), "en");
    i18n.seed(1, "alice");
    assert_eq!(i18n.lang_of(1), "en");

    // Changing the default (glang) moves un-preffed users only.
    i18n.seed(2, "bob");
    i18n.set_default("en");
    i18n.seed(2, "bob");
    assert_eq!(i18n.lang_of(2), "en");
    assert_eq!(i18n.lang_of(1), "en"); // alice's own pick still stands

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_tr_translates_and_falls_back() {
    let dir = runtime_dir("tr");
    let i18n = I18n::load(&dir, "de");

    i18n.seed(1, "alice");
    assert_eq!(
        i18n.tr(1, Key::LangSet, &[("language", "Deutsch".to_string())]),
        "Sprache auf Deutsch gesetzt"
    );

    // tr_in renders in an explicit language regardless of session.
    assert_eq!(
        i18n.tr_in("en", Key::LangSet, &[("language", "English".to_string())]),
        "Language set to English"
    );

    // Availability and names. Embedded bundles (es/pt/ru) are always
    // present alongside English and the on-disk de file.
    assert!(i18n.is_available("de"));
    assert!(i18n.is_available("EN"));
    assert!(!i18n.is_available("xx"));
    assert_eq!(i18n.language_name("de"), "Deutsch");
    let codes: Vec<String> = i18n.available().into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        codes,
        vec!["de", "en", "es", "pt", "ru"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn embedded_translations_work_without_any_files() {
    let dir = std::env::temp_dir().join(format!(
        "ttspotify_i18n_embed_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // No lang files at all: bundled Portuguese still translates.
    let i18n = I18n::load(&dir, "en");
    assert!(i18n.is_available("pt"));
    assert!(i18n.is_available("es"));
    assert!(i18n.is_available("ru"));
    assert_eq!(
        i18n.tr_in("pt", Key::Paused, &[]),
        "Pausado"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lang_file_overrides_embedded_translation_per_key() {
    let dir = std::env::temp_dir().join(format!(
        "ttspotify_i18n_override_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lang")).unwrap();
    // A one-line pt.lang: overrides that key, keeps the rest of the
    // bundled Portuguese (merge, not replace).
    std::fs::write(dir.join("lang").join("pt.lang"), "paused = Em pausa\n").unwrap();
    let i18n = I18n::load(&dir, "en");
    assert_eq!(i18n.tr_in("pt", Key::Paused, &[]), "Em pausa"); // overridden
    assert_eq!(i18n.tr_in("pt", Key::Resuming, &[]), "Retomando"); // still bundled
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn embedded_translations_have_valid_keys_and_slots() {
    // Guards bundled translations (and future community PRs to them)
    // against typo'd keys or broken {placeholders}.
    let mut c = Catalog::new_embedded();
    for (code, text) in EMBEDDED_LANGS {
        c.add_language(code, parse_lang(text));
        let v = validate(&c, code);
        assert!(
            v.slot_mismatches.is_empty(),
            "{code}.lang has placeholder mismatches: {:?}",
            v.slot_mismatches
        );
        assert!(
            v.unknown_keys.is_empty(),
            "{code}.lang has unknown keys: {:?}",
            v.unknown_keys
        );
    }
}

#[test]
fn load_exports_english_template_to_lang_dir() {
    let dir = std::env::temp_dir().join(format!("ttspotify_i18n_tpl_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // No lang dir yet: load creates it and writes the template.
    let _ = I18n::load(&dir, "en");
    let exported = std::fs::read_to_string(dir.join("lang").join("en.lang")).unwrap();
    assert_eq!(exported, EMBEDDED_EN);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_clear_pref_removes_pick_and_follows_default() {
    let dir = runtime_dir("clear");
    {
        let i18n = I18n::load(&dir, "en");
        i18n.seed(1, "alice");
        i18n.set_pref(1, "alice", "de");
        assert_eq!(i18n.lang_of(1), "de");
        // Clear: pick removed, session follows the server default at once.
        assert!(i18n.clear_pref(1, "alice"));
        assert_eq!(i18n.lang_of(1), "en");
        assert!(!i18n.clear_pref(1, "alice")); // nothing left to remove
    }
    // The removal persisted: a fresh runtime no longer knows the pick.
    let i18n = I18n::load(&dir, "en");
    i18n.seed(7, "alice");
    assert_eq!(i18n.lang_of(7), "en");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_set_pref_persists_across_reload() {
    let dir = runtime_dir("persist");
    {
        let i18n = I18n::load(&dir, "en");
        i18n.seed(1, "alice");
        i18n.set_pref(1, "Alice", "de");
    }
    // A fresh runtime (bot restart) still knows alice's pick.
    let i18n = I18n::load(&dir, "en");
    i18n.seed(7, "alice"); // new session id after reconnect
    assert_eq!(i18n.lang_of(7), "de");

    let _ = std::fs::remove_dir_all(&dir);
}
