use std::collections::BTreeSet;

use super::catalog::Catalog;
use super::key::{slots_of, Key, ENGLISH, LANGUAGE_NAME_KEY};

/// Structured result of validating one loaded language against English.
pub struct LangValidation {
    pub code: String,
    /// How many of the bot's keys this language translates.
    pub present: usize,
    /// Total number of translatable keys.
    pub total: usize,
    /// Keys whose `{slot}` set differs from the English template (renamed,
    /// dropped, or invented placeholders).
    pub slot_mismatches: Vec<String>,
    /// Keys in the file that the bot does not know (typos or removed keys).
    pub unknown_keys: Vec<String>,
}

/// Validate a loaded language: coverage count, placeholder-slot mismatches,
/// and unknown keys. Returns structured results so callers can log or test.
pub fn validate(catalog: &Catalog, code: &str) -> LangValidation {
    let total = Key::ALL.len();
    let mut present = 0;
    let mut slot_mismatches = Vec::new();
    for key in Key::ALL {
        let id = key.id();
        if let Some(translated) = catalog.template(code, id) {
            present += 1;
            if let Some(english) = catalog.template(ENGLISH, id) {
                if slots_of(translated) != slots_of(english) {
                    slot_mismatches.push(id.to_string());
                }
            }
        }
    }
    let known: BTreeSet<&str> = Key::ALL.iter().map(|k| k.id()).collect();
    let mut unknown_keys: Vec<String> = catalog
        .langs
        .get(code)
        .map(|entries| {
            entries
                .keys()
                .filter(|k| k.as_str() != LANGUAGE_NAME_KEY && !known.contains(k.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    unknown_keys.sort();
    LangValidation {
        code: code.to_string(),
        present,
        total,
        slot_mismatches,
        unknown_keys,
    }
}
