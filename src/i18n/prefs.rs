use std::collections::HashMap;
use std::path::PathBuf;

/// Per-user language picks, keyed by lowercased TeamTalk username. Stored as
/// machine-written JSON (`<config_dir>/lang_prefs.json`) — unlike `.lang`
/// files, this is never hand-edited.
pub struct LangPrefs {
    map: HashMap<String, String>,
    path: PathBuf,
}

impl LangPrefs {
    /// Load prefs from `path`. A missing or unreadable file yields empty prefs
    /// (never an error — a lost prefs file just means everyone is back on the
    /// server default until they pick again).
    pub fn load(path: PathBuf) -> LangPrefs {
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<HashMap<String, String>>(&text).ok())
            .map(|m| {
                m.into_iter()
                    .map(|(user, code)| (user.to_lowercase(), code))
                    .collect()
            })
            .unwrap_or_default();
        LangPrefs { map, path }
    }

    pub fn get(&self, username: &str) -> Option<&str> {
        self.map.get(&username.to_lowercase()).map(String::as_str)
    }

    /// Set and persist a user's language pick. Persisting is atomic
    /// (temp + rename); a write error is logged, never fatal.
    pub fn set(&mut self, username: &str, code: &str) {
        self.map
            .insert(username.to_lowercase(), code.to_lowercase());
        self.save();
    }

    /// Remove a user's pick (they go back to following the server default).
    /// Returns whether a pick existed. Persists on change.
    pub fn remove(&mut self, username: &str) -> bool {
        let existed = self.map.remove(&username.to_lowercase()).is_some();
        if existed {
            self.save();
        }
        existed
    }

    fn save(&self) {
        let json = match serde_json::to_string_pretty(&self.map) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("Could not serialize language prefs: {e}");
                return;
            }
        };
        let tmp = self.path.with_extension("json.tmp");
        if let Err(e) =
            std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, &self.path))
        {
            tracing::warn!(
                "Could not save language prefs {}: {e}",
                self.path.display()
            );
        }
    }
}
