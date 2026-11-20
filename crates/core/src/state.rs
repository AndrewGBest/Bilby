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
    /// The first-run explanation was read and dismissed.
    pub welcomed: bool,
    /// Do not look for updates when the store starts (saves requests on a tight allowance).
    pub skip_update_check_on_start: bool,
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
    /// A newer version the user chose not to be told about. Later versions are
    /// announced as usual.
    #[serde(default)]
    pub skipped_version: String,
}

/// An app marked to come back to. Enough is kept to show it without the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favourite {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub summary: String,
}

impl Favourite {
    pub fn of(app: &AppSummary) -> Favourite {
        Favourite {
            id: app.id.to_string(),
            name: app.name.clone(),
            author: app.author.clone(),
            summary: app.summary.clone(),
        }
    }
}

/// What `export` writes and `import` reads: the library and favourites, no settings
/// (the file is meant to be shared or moved, and settings can hold a token).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Backup {
    pub bilby_backup: u32,
    pub library: Vec<InstalledApp>,
    pub favourites: Vec<Favourite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub settings: Settings,
    pub library: Vec<InstalledApp>,
    pub recent_searches: Vec<String>,
    pub favourites: Vec<Favourite>,
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

    pub fn is_favourite(&self, id: &str) -> bool {
        self.favourites.iter().any(|f| f.id == id)
    }

    /// Mark or unmark an app. Returns whether it is a favourite afterwards.
    pub fn toggle_favourite(&mut self, favourite: Favourite) -> bool {
        if self.is_favourite(&favourite.id) {
            self.favourites.retain(|f| f.id != favourite.id);
            return false;
        }
        self.favourites.insert(0, favourite);
        true
    }

    /// Stop announcing `version` of a library app. Returns false if it is not installed.
    pub fn skip_version(&mut self, id: &str, version: &str) -> bool {
        match self.library.iter_mut().find(|a| a.id == id) {
            Some(app) => {
                app.skipped_version = version.to_string();
                true
            }
            None => false,
        }
    }

    pub fn backup(&self) -> Backup {
        let mut library = self.library.clone();
        // Paths belong to this device; on another one they would point at nothing.
        for app in &mut library {
            app.path = None;
            app.store_managed = false;
        }
        Backup { bilby_backup: 1, library, favourites: self.favourites.clone() }
    }

    /// The backup as the text of a file.
    pub fn backup_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.backup())?)
    }

    /// Merge a backup in. What is already here wins; returns `(apps, favourites)` added.
    pub fn restore(&mut self, text: &str) -> Result<(usize, usize)> {
        let backup: Backup = serde_json::from_str(text)?;
        if backup.bilby_backup == 0 {
            return Err(crate::error::Error::Parse("this is not a Bilby backup file".into()));
        }
        let ok = |id: &str| crate::model::AppId::parse(id).is_some_and(|i| crate::model::valid_key(i.source, &i.key));
        let (mut apps, mut favourites) = (0, 0);
        for app in backup.library {
            if ok(&app.id) && self.installed(&app.id).is_none() {
                self.library.push(app);
                apps += 1;
            }
        }
        for favourite in backup.favourites {
            if ok(&favourite.id) && !self.is_favourite(&favourite.id) {
                self.favourites.push(favourite);
                favourites += 1;
            }
        }
        Ok((apps, favourites))
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
            skipped_version: String::new(),
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
        let dir = std::env::temp_dir().join(format!("bilby-state-{tag}-{}", std::process::id()));
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
            skipped_version: String::new(),
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

    fn summary(id: &str) -> AppSummary {
        AppSummary {
            id: crate::model::AppId::parse(id).unwrap(),
            name: "Thing".into(),
            author: "someone".into(),
            summary: "Does things".into(),
            popularity: None,
            homepage: String::new(),
            license: None,
            category: None,
            version: None,
            install_label: None,
            warning: None,
            origin: None,
            installed_version: None,
            rating: None,
            icon_url: None,
        }
    }

    #[test]
    fn favourites_toggle_and_keep_what_is_needed_to_show_them() {
        let mut state = State::default();
        assert!(state.toggle_favourite(Favourite::of(&summary("github:a/b"))));
        assert!(state.toggle_favourite(Favourite::of(&summary("codeberg:c/d"))));
        assert_eq!(state.favourites[0].id, "codeberg:c/d", "newest first");
        assert_eq!(state.favourites[1].name, "Thing");
        assert!(!state.toggle_favourite(Favourite::of(&summary("github:a/b"))), "a second press removes it");
        assert!(!state.is_favourite("github:a/b") && state.is_favourite("codeberg:c/d"));
    }

    #[test]
    fn a_skipped_version_is_remembered_until_the_next_install() {
        let mut state = State::default();
        state.record_install(app("github:a/b", "v1"));
        assert!(state.skip_version("github:a/b", "v2"));
        assert_eq!(state.installed("github:a/b").unwrap().skipped_version, "v2");
        assert!(!state.skip_version("github:not/here", "v2"));
        state.record_install(app("github:a/b", "v3"));
        assert_eq!(state.installed("github:a/b").unwrap().skipped_version, "");
    }

    #[test]
    fn a_backup_moves_the_library_but_not_settings_or_paths() {
        let mut old = State::default();
        old.settings.github_token = "secret".into();
        let mut owned = app("github:a/b", "v1");
        owned.path = Some("/home/me/Applications/a.AppImage".into());
        owned.store_managed = true;
        old.record_install(owned);
        old.toggle_favourite(Favourite::of(&summary("aur:paru")));
        let text = serde_json::to_string(&old.backup()).unwrap();
        assert!(!text.contains("secret") && !text.contains("/home/me"));

        let mut new = State::default();
        new.record_install(app("github:a/b", "v9"));
        assert_eq!(new.restore(&text).unwrap(), (0, 1), "what is already here wins");
        assert_eq!(new.installed("github:a/b").unwrap().version, "v9");
        assert_eq!(new.restore(&text).unwrap(), (0, 0), "restoring twice adds nothing");

        assert!(new.restore("{\"library\":[]}").is_err(), "some other JSON file is refused");
        assert!(new.restore("not json").is_err());
        let hostile = r#"{"bilby_backup":1,"library":[{"id":"github:../x","name":"x","version":"1"}]}"#;
        assert_eq!(new.restore(hostile).unwrap(), (0, 0), "ids that could not be real are dropped");
    }
}
