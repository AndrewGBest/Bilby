//! Everything the store remembers, kept in one JSON file on the user's own device.
//! There is no account and no server: settings, the library of installed apps and
//! recent searches live in `state.json` under the user's config directory.

use crate::error::Result;
use crate::model::AppSummary;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_RECENT_SEARCHES: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `system`, `light` or `dark`. Anything else is treated as `system`.
    pub theme: String,
    /// Optional personal access token; raises GitHub's hourly request limit.
    pub github_token: String,
    /// Offer pre-releases (betas, release candidates) as if they were stable.
    pub include_prereleases: bool,
}

/// An app obtained through the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub summary: String,
    pub version: String,
    /// What was installed, e.g. "AppImage · x86_64".
    #[serde(default)]
    pub label: String,
    /// File name of the asset.
    #[serde(default)]
    pub file: String,
    /// Where the store put it, when the store owns the file (AppImages, downloads).
    #[serde(default)]
    pub path: Option<String>,
    /// True when deleting `path` uninstalls the app (AppImage). Packages handed to the
    /// system installer are not ours to remove.
    #[serde(default)]
    pub store_managed: bool,
    /// Seconds since the Unix epoch.
    #[serde(default)]
    pub installed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub settings: Settings,
    pub library: Vec<InstalledApp>,
    pub recent_searches: Vec<String>,
}

impl State {
    /// Load from `path`. A missing file is a fresh start. A file that cannot be parsed is
    /// set aside as `state.json.corrupt` instead of being overwritten, and a fresh state
    /// is returned: losing preferences beats refusing to start.
    pub fn load(path: &Path) -> State {
        let Ok(text) = std::fs::read_to_string(path) else {
            return State::default();
        };
        match serde_json::from_str(&text) {
            Ok(state) => state,
            Err(_) => {
                let _ = std::fs::rename(path, corrupt_path(path));
                State::default()
            }
        }
    }

    /// Write atomically: a crash mid-save leaves the previous file intact.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        // The file can hold a token: keep it private to the user.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn installed(&self, id: &str) -> Option<&InstalledApp> {
        self.library.iter().find(|a| a.id == id)
    }

    /// Add or replace the library entry for an app; newest first.
    pub fn record_install(&mut self, app: InstalledApp) {
        self.library.retain(|a| a.id != app.id);
        self.library.insert(0, app);
    }

    /// Forget an app. Returns the entry so the caller can delete a store-managed file.
    pub fn remove(&mut self, id: &str) -> Option<InstalledApp> {
        let index = self.library.iter().position(|a| a.id == id)?;
        Some(self.library.remove(index))
    }

    pub fn add_search(&mut self, query: &str) {
        let query = query.trim();
        if query.chars().count() < 2 {
            return;
        }
        self.recent_searches.retain(|q| !q.eq_ignore_ascii_case(query));
        self.recent_searches.insert(0, query.to_string());
        self.recent_searches.truncate(MAX_RECENT_SEARCHES);
    }
}

fn corrupt_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".corrupt");
    path.with_file_name(name)
}

impl InstalledApp {
    pub fn new(summary: &AppSummary, version: &str, label: &str, file: &str, now: u64) -> InstalledApp {
        InstalledApp {
            id: summary.id.to_string(),
            name: summary.name.clone(),
            author: summary.author.clone(),
            summary: summary.summary.clone(),
            version: version.to_string(),
            label: label.to_string(),
            file: file.to_string(),
            path: None,
            store_managed: false,
            installed_at: now,
        }
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ustore-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("state.json")
    }

    fn app(id: &str, version: &str) -> InstalledApp {
        InstalledApp {
            id: id.to_string(),
            name: id.to_string(),
            author: String::new(),
            summary: String::new(),
            version: version.to_string(),
            label: "AppImage · x86_64".to_string(),
            file: "a.AppImage".to_string(),
            path: None,
            store_managed: false,
            installed_at: 1,
        }
    }

    #[test]
    fn round_trip() {
        let path = temp("roundtrip");
        let mut state = State::default();
        state.settings.theme = "light".to_string();
        state.settings.include_prereleases = true;
        state.record_install(app("github:a/b", "v1"));
        state.add_search("ripgrep");
        state.save(&path).unwrap();
        assert_eq!(State::load(&path), state);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn missing_file_is_a_fresh_start() {
        assert_eq!(State::load(&temp("missing")), State::default());
    }

    #[test]
    fn corrupt_file_is_set_aside_not_destroyed() {
        let path = temp("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ this is not json").unwrap();
        assert_eq!(State::load(&path), State::default());
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(corrupt_path(&path)).unwrap(), "{ this is not json");
    }

    #[test]
    fn files_from_older_and_newer_versions_load() {
        let path = temp("compat");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Missing fields get defaults; unknown fields are ignored.
        std::fs::write(&path, r#"{"library":[{"id":"aur:x","name":"x","version":"1"}],"future_field":42}"#).unwrap();
        let state = State::load(&path);
        assert_eq!(state.library[0].id, "aur:x");
        assert!(!state.library[0].store_managed);
        assert_eq!(state.settings, Settings::default());
    }

    #[test]
    fn reinstall_replaces_and_moves_to_front() {
        let mut state = State::default();
        state.record_install(app("github:a/one", "v1"));
        state.record_install(app("github:a/two", "v1"));
        state.record_install(app("github:a/one", "v2"));
        let ids: Vec<_> = state.library.iter().map(|a| (a.id.as_str(), a.version.as_str())).collect();
        assert_eq!(ids, vec![("github:a/one", "v2"), ("github:a/two", "v1")]);
        assert_eq!(state.remove("github:a/two").unwrap().id, "github:a/two");
        assert!(state.remove("github:a/two").is_none());
        assert!(state.installed("github:a/one").is_some());
    }

    #[test]
    fn recent_searches_dedupe_and_cap() {
        let mut state = State::default();
        for q in ["bat", "fd", "Bat", " ", "x", "a1", "a2", "a3", "a4", "a5"] {
            state.add_search(q);
        }
        assert_eq!(state.recent_searches.len(), MAX_RECENT_SEARCHES);
        assert_eq!(state.recent_searches[0], "a5");
        assert_eq!(state.recent_searches.iter().filter(|q| q.eq_ignore_ascii_case("bat")).count(), 1);
        assert!(!state.recent_searches.contains(&"x".to_string()), "one-letter searches are not kept");
    }
}
