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
    /// Do not fetch or show star ratings.
    pub hide_ratings: bool,
    /// The language to show the store in, as a code such as `fr`. Empty: the
    /// system's own.
    pub language: String,
    /// The country whose prices the stores are asked for, as two letters such as
    /// `CA`. Empty: the one the system is set to.
    pub country: String,
    /// Do not offer Windows programs on Linux, set up to run through Proton.
    pub hide_windows_apps: bool,
    /// Which interface to use: `desktop`, `tv`, or anything else for "decide for me".
    pub interface: String,
    /// The "is this a TV?" question was answered or dismissed; it is asked once.
    pub asked_about_tv: bool,
    /// Do not add the store itself to the app menu.
    pub hide_from_menu: bool,
    /// Change things at once instead of animating them.
    pub reduce_motion: bool,
    /// Sources switched off, by their short names (`flathub`, `steam`, ...).
    pub disabled_sources: Vec<String>,
    /// No sounds as the person moves about. They are on, and quiet, unless this is set.
    pub mute_sounds: bool,
    /// Lists of apps on the person's own servers or storage, added by address.
    pub own_sources: Vec<OwnSource>,
    /// Where downloaded files go, when the person chose somewhere other than the
    /// usual folder. Empty for the usual one.
    pub downloads_dir: String,
    /// Ask where to save each download. Off unless the person switched it on.
    pub ask_where: bool,
    /// Do not recommend the hand-kept list of apps nearly everyone expects: no rows
    /// of them on the home page, no extra weight in a search.
    pub hide_essentials: bool,
    /// Also show apps that do not run on this device: marked as such, last in a
    /// search, and not installable here.
    pub show_everything: bool,
    /// Install updates to the library's apps as they are found, without being asked.
    /// Off unless the person switched it on.
    pub auto_update: bool,
    /// Closing the window ends the store. Unless this is set it stays in the system
    /// tray instead, where there is one, and goes on keeping things up to date.
    pub quit_on_close: bool,
    /// The kind of app to offer first on Linux: "" (what suits the system),
    /// "flatpak", "appimage" or "system".
    pub prefer_kind: String,
    /// Family mode: nothing for adults anywhere, in any form.
    pub family_mode: bool,
    /// Adult things may appear on the home page (covered, as everywhere).
    pub adult_on_home: bool,
    /// Adult things are shown as they are, like anything else: no cover, no section
    /// of their own, no question on opening.
    pub adult_uncovered: bool,
    /// The person has confirmed, once, that they are an adult.
    pub adult_confirmed: bool,
    /// The PIN that guards these settings and adult content, as a salted hash.
    /// Empty when no PIN is set.
    pub pin_hash: String,
    pub pin_salt: String,
    /// Ask for the PIN before installing or removing anything.
    pub pin_for_installs: bool,
}

/// A source the person added themselves: what it calls itself and where its list is.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OwnSource {
    pub name: String,
    pub url: String,
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
    /// The Flatpak app this became, when it was installed from a downloaded bundle:
    /// Flatpak runs and removes it by this name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flatpak: String,
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
    /// The Steam account the person signed in to, and what it owns.
    pub steam: Option<crate::steam_account::Account>,
    /// The GOG account the person signed in to, and what it owns.
    pub gog: Option<crate::gog_account::Account>,
    /// A made-up id sent with reviews written here, so that the ratings service can
    /// tell one person's reviews together without knowing whose they are. Made the
    /// first time a review is sent.
    pub reviewer: String,
    /// The name last shown next to a review, offered again the next time.
    pub reviewer_name: String,
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
        match serde_json::from_str::<State>(&text) {
            Ok(mut state) => {
                state.merge_duplicates();
                state
            }
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

    /// An app's entry. Ids are compared without regard to capitals: the sources do
    /// not tell `owner/App` from `owner/app`, and an app must not be in the library
    /// twice because two places spelt it differently.
    pub fn installed(&self, id: &str) -> Option<&InstalledApp> {
        self.library.iter().find(|a| a.id.eq_ignore_ascii_case(id))
    }

    /// Add or replace the library entry for an app; newest first.
    pub fn record_install(&mut self, app: InstalledApp) {
        self.library.retain(|a| !a.id.eq_ignore_ascii_case(&app.id));
        self.library.insert(0, app);
    }

    /// Forget an app. Returns the entry so the caller can delete a store-managed file.
    pub fn remove(&mut self, id: &str) -> Option<InstalledApp> {
        let index = self.library.iter().position(|a| a.id.eq_ignore_ascii_case(id))?;
        Some(self.library.remove(index))
    }

    /// One entry per app. A library written by an earlier version could hold the same
    /// app twice, under two spellings of its id (the store itself, after updating):
    /// the copy the store placed is the one kept, else the first.
    fn merge_duplicates(&mut self) {
        let mut kept: Vec<InstalledApp> = Vec::new();
        for app in std::mem::take(&mut self.library) {
            match kept.iter_mut().find(|k| k.id.eq_ignore_ascii_case(&app.id)) {
                Some(have) => {
                    let placed = |a: &InstalledApp| a.store_managed && a.path.is_some();
                    if placed(&app) && !placed(have) {
                        *have = app;
                    }
                }
                None => kept.push(app),
            }
        }
        self.library = kept;
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
        match self.library.iter_mut().find(|a| a.id.eq_ignore_ascii_case(id)) {
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
            flatpak: String::new(),
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

    /// The store listed itself as `owner/bilby` and recorded its own update as
    /// `owner/Bilby`: two entries, the old one for ever "waiting for an update".
    #[test]
    fn an_app_is_in_the_library_once_however_its_id_is_spelt() {
        let mut state = State::default();
        state.record_install(app("github:AndrewGBest/bilby", "v0.1.24"));
        let mut updated = app("github:AndrewGBest/Bilby", "v0.1.25");
        updated.store_managed = true;
        updated.path = Some("/home/me/Applications/bilby.AppImage".to_string());
        state.record_install(updated.clone());
        assert_eq!(state.library.len(), 1, "the update replaces the entry");
        assert_eq!(state.installed("github:andrewgbest/BILBY").unwrap().version, "v0.1.25");
        assert!(state.skip_version("github:AndrewGBest/bilby", "v0.1.26"));

        // A file an earlier version wrote, with both: the copy the store placed is kept,
        // whichever came first.
        for order in [[0, 1], [1, 0]] {
            let both = [app("github:AndrewGBest/bilby", "v0.1.24"), updated.clone()];
            let file = temp(&format!("twice-{}", order[0]));
            let written = State { library: order.iter().map(|n| both[*n].clone()).collect(), ..State::default() };
            written.save(&file).unwrap();
            let read = State::load(&file);
            assert_eq!(read.library.len(), 1);
            assert_eq!(read.library[0].version, "v0.1.25");
        }
        assert!(state.remove("GITHUB:andrewgbest/bilby").is_some() && state.library.is_empty());
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
            flatpak: String::new(),
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
            downloads: None,
            verified: false,
            momentum: None,
            price: None,
            adult: false,
            restricted: false,
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
