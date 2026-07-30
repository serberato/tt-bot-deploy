//! Lightweight i18n engine.
//!
//! English is embedded in the binary (`src/i18n/en.lang`) and is both the
//! fallback and the translator's template. Other languages are plain
//! `key = value` text files (`<config_dir>/lang/<code>.lang`) loaded at
//! startup. Any missing key or unknown language falls back to English, so a
//! partial translation never breaks a reply.
//!
//! Templates use named `{slot}` placeholders. Translators may move a slot
//! anywhere in their sentence (substitution is by name, not position), but
//! must not rename slots or invent new ones; an unknown slot is left visible
//! rather than silently dropped.

use std::collections::HashMap;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use parking_lot::Mutex;

mod catalog;
mod key;
mod prefs;
mod validation;

pub use catalog::{installed_language_codes, Catalog};
#[allow(unused_imports)]
pub(crate) use catalog::{export_english_template, EMBEDDED_EN, EMBEDDED_LANGS};
pub use key::{fill, parse_lang, Key, ENGLISH};
#[allow(unused_imports)]
pub(crate) use key::slots_of;
pub use prefs::LangPrefs;
pub use validation::{validate, LangValidation};

/// Shared i18n runtime: the loaded catalog plus per-user state. Wrapped in an
/// `Arc` and shared by the command dispatcher and the command processor.
///
/// Locks are only ever taken one at a time (never nested), and every critical
/// section is a quick map access — safe to call from async contexts.
pub struct I18n {
    catalog: Catalog,
    prefs: Mutex<LangPrefs>,
    default_lang: Mutex<String>,
    /// Session cache: TeamTalk user id -> resolved language code. Seeded at
    /// dispatch time (where the sender's username is known) so every later
    /// reply site can resolve by user id alone.
    session: Mutex<HashMap<i32, String>>,
}

impl I18n {
    /// Build the runtime: embedded English plus every `<config_dir>/lang/*.lang`
    /// file, and per-user prefs from `<config_dir>/lang_prefs.json`. Each loaded
    /// file gets a coverage log line and placeholder-mismatch warnings; a broken
    /// file degrades, it never fails startup.
    pub fn load(config_dir: &Path, default_language: &str) -> I18n {
        let mut catalog = Catalog::new_embedded();
        // Bundled translations first; files below override them per key.
        for (code, text) in EMBEDDED_LANGS {
            catalog.add_language(code, parse_lang(text));
        }
        let lang_dir = config_dir.join("lang");
        export_english_template(&lang_dir);
        if let Ok(entries) = std::fs::read_dir(&lang_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("lang") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let code = stem.to_lowercase();
                if code == ENGLISH {
                    // English is embedded and authoritative; an en.lang file
                    // in the lang dir is ignored.
                    continue;
                }
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        // Merge, not replace: a partial file patches individual
                        // messages of a bundled translation instead of wiping it.
                        catalog.merge_language(&code, parse_lang(&text));
                        let v = validate(&catalog, &code);
                        tracing::info!(
                            "Loaded translation {}: {}/{} messages",
                            code, v.present, v.total
                        );
                        for id in &v.slot_mismatches {
                            tracing::warn!(
                                "{code}.lang `{id}`: {{placeholders}} differ from English (renamed or dropped)"
                            );
                        }
                        for id in &v.unknown_keys {
                            tracing::warn!("{code}.lang has unknown key `{id}` (ignored)");
                        }
                    }
                    Err(e) => tracing::warn!("Could not read {}: {e}", path.display()),
                }
            }
        }
        let default_lang = default_language.to_lowercase();
        if !catalog.has_language(&default_lang) {
            tracing::warn!(
                "Default language `{default_lang}` has no {default_lang}.lang file; English will be shown"
            );
        }
        I18n {
            catalog,
            prefs: Mutex::new(LangPrefs::load(config_dir.join("lang_prefs.json"))),
            default_lang: Mutex::new(default_lang),
            session: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve and cache the language for a user id. Called at dispatch, where
    /// the sender's username is known. Order: user pref -> server default.
    pub fn seed(&self, user_id: i32, username: &str) {
        let lang = self.resolve_for(username);
        self.session.lock().insert(user_id, lang);
    }

    fn resolve_for(&self, username: &str) -> String {
        if let Some(code) = self.prefs.lock().get(username) {
            return code.to_string();
        }
        self.default_lang.lock().clone()
    }

    /// The cached language for a user id; server default if never seeded.
    pub fn lang_of(&self, user_id: i32) -> String {
        self.session
            .lock()
            .get(&user_id)
            .cloned()
            .unwrap_or_else(|| self.default_lang.lock().clone())
    }

    /// Translate `key` for the user behind `user_id`.
    pub fn tr(&self, user_id: i32, key: Key, args: &[(&str, String)]) -> String {
        self.catalog.t(&self.lang_of(user_id), key, args)
    }

    /// Translate directly into an explicit language (used for the `lang_set`
    /// confirmation, which renders in the just-picked language).
    pub fn tr_in(&self, code: &str, key: Key, args: &[(&str, String)]) -> String {
        self.catalog.t(&code.to_lowercase(), key, args)
    }

    /// Persist a user's language pick and update their session immediately.
    pub fn set_pref(&self, user_id: i32, username: &str, code: &str) {
        let code = code.to_lowercase();
        self.prefs.lock().set(username, &code);
        self.session.lock().insert(user_id, code);
    }

    /// Drop a user's pick so they follow the server default again. Updates
    /// their session immediately. Returns whether a pick existed.
    pub fn clear_pref(&self, user_id: i32, username: &str) -> bool {
        let existed = self.prefs.lock().remove(username);
        let default = self.default_lang.lock().clone();
        self.session.lock().insert(user_id, default);
        existed
    }

    /// Change the server default (glang). Personal picks are untouched.
    pub fn set_default(&self, code: &str) {
        *self.default_lang.lock() = code.to_lowercase();
    }

    /// The current server default language code.
    pub fn default_language(&self) -> String {
        self.default_lang.lock().clone()
    }

    pub fn is_available(&self, code: &str) -> bool {
        self.catalog.has_language(&code.to_lowercase())
    }

    /// All loaded languages as (code, display name), sorted by code.
    pub fn available(&self) -> Vec<(String, String)> {
        self.catalog
            .codes()
            .into_iter()
            .map(|code| {
                let name = self.catalog.language_name(&code);
                (code, name)
            })
            .collect()
    }

    pub fn language_name(&self, code: &str) -> String {
        self.catalog.language_name(&code.to_lowercase())
    }
}

#[cfg(test)]
mod tests;
