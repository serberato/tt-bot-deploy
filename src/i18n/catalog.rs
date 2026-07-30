use std::collections::HashMap;
use std::path::Path;

use super::key::{fill, parse_lang, Key, ENGLISH, LANGUAGE_NAME_KEY};

pub(crate) const EMBEDDED_EN: &str = include_str!("en.lang");

/// Translations bundled into the binary, so they work identically for release
/// downloads and source builds with no files to install. A same-code file in
/// `<config_dir>/lang/` overrides these per key. English is NOT in this list:
/// it is the authoritative fallback and cannot be overridden.
pub(crate) const EMBEDDED_LANGS: &[(&str, &str)] = &[
    ("es", include_str!("es.lang")),
    ("pt", include_str!("pt.lang")),
    ("ru", include_str!("ru.lang")),
];

/// All loaded languages: the embedded English plus any runtime `.lang` files.
pub struct Catalog {
    pub(crate) langs: HashMap<String, HashMap<String, String>>,
}

impl Catalog {
    /// A catalog holding only the embedded English.
    pub fn new_embedded() -> Catalog {
        let mut langs = HashMap::new();
        langs.insert(ENGLISH.to_string(), parse_lang(EMBEDDED_EN));
        Catalog { langs }
    }

    /// Register a runtime language (code is lowercased), replacing any
    /// existing map for that code.
    pub fn add_language(&mut self, code: &str, entries: HashMap<String, String>) {
        self.langs.insert(code.to_lowercase(), entries);
    }

    /// Merge entries into a language: given keys override, everything else is
    /// kept. Used for `<config_dir>/lang/` files so a partial file can patch
    /// individual messages of a bundled translation (a new code just inserts).
    pub fn merge_language(&mut self, code: &str, entries: HashMap<String, String>) {
        self.langs
            .entry(code.to_lowercase())
            .or_default()
            .extend(entries);
    }

    pub(crate) fn template(&self, lang: &str, id: &str) -> Option<&str> {
        self.langs.get(lang)?.get(id).map(String::as_str)
    }

    /// Translate `key` into `lang`, falling back to English on a missing key
    /// or unknown language, then fill the `{slot}` placeholders. As a last
    /// resort (a key absent even from English — prevented by the completeness
    /// test) the key id itself is returned so the gap is visible.
    pub fn t(&self, lang: &str, key: Key, args: &[(&str, String)]) -> String {
        let id = key.id();
        let template = self
            .template(lang, id)
            .or_else(|| self.template(ENGLISH, id))
            .unwrap_or(id);
        fill(template, args)
    }

    /// The language's self-declared display name, or its code if absent.
    pub fn language_name(&self, code: &str) -> String {
        self.langs
            .get(code)
            .and_then(|m| m.get(LANGUAGE_NAME_KEY))
            .cloned()
            .unwrap_or_else(|| code.to_string())
    }

    /// All loaded language codes, sorted.
    pub fn codes(&self) -> Vec<String> {
        let mut codes: Vec<String> = self.langs.keys().cloned().collect();
        codes.sort();
        codes
    }

    pub fn has_language(&self, code: &str) -> bool {
        self.langs.contains_key(code)
    }
}

/// Write the embedded English template to `<lang_dir>/en.lang` so translators
/// have a commented, always-current file to copy (the loader ignores an
/// en.lang file — the embedded English stays authoritative, so overwriting is
/// safe and keeps the template in sync after updates). Best-effort: failure is
/// logged, never fatal.
pub(crate) fn export_english_template(lang_dir: &Path) {
    let path = lang_dir.join("en.lang");
    // Skip the write when the on-disk copy is already current.
    if std::fs::read_to_string(&path).is_ok_and(|current| current == EMBEDDED_EN) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(lang_dir)
        .and_then(|()| std::fs::write(&path, EMBEDDED_EN))
    {
        tracing::warn!("Could not write English template {}: {e}", path.display());
    }
}

/// Language codes available: embedded (English + bundled translations) plus
/// any on-disk `.lang` files, sorted. Used by the config editor and setup
/// wizard, which need the list without loading a full catalog (the bot itself
/// uses `I18n::load`).
pub fn installed_language_codes(config_dir: &Path) -> Vec<String> {
    let mut codes = vec![ENGLISH.to_string()];
    for (code, _) in EMBEDDED_LANGS {
        codes.push((*code).to_string());
    }
    if let Ok(entries) = std::fs::read_dir(config_dir.join("lang")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lang") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let code = stem.to_lowercase();
                if !codes.contains(&code) {
                    codes.push(code);
                }
            }
        }
    }
    codes.sort();
    codes
}
