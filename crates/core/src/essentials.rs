//! The apps and websites nearly everyone expects.
//!
//! The sources' own numbers say what is popular on each source. They do not say that
//! a person setting up a computer wants a browser, a video player and the handful of
//! names everyone knows, whichever source happens to carry them. That is a human
//! judgement, so it is kept by hand in `data/essentials.json`: some sixty apps and
//! sites, each with the ids it goes by at every source.
//!
//! The store uses the list in two ways: rows on the home page showing, for each, the
//! version this device can use; and extra weight for them in a search. Both can be
//! switched off in Settings, which leaves only what the sources themselves say.

use crate::model::{AppId, AppSummary, SourceKind};
use crate::platform::Platform;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
pub struct Essential {
    pub name: String,
    pub summary: String,
    /// browse, media, chat, games, work or tools.
    pub kind: String,
    /// The ids it goes by, best first.
    pub ids: Vec<String>,
    /// In the first row.
    #[serde(default)]
    pub top: bool,
}

/// The bundled list. A broken file means no list, not a store that will not start;
/// ids that could not be opened are dropped.
pub fn all() -> &'static [Essential] {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        apps: Vec<Essential>,
    }
    static LIST: OnceLock<Vec<Essential>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut apps =
            serde_json::from_str::<File>(include_str!("../data/essentials.json")).map(|f| f.apps).unwrap_or_default();
        for app in &mut apps {
            app.ids.retain(|id| AppId::parse(id).is_some_and(|id| crate::model::valid_key(id.source, &id.key)));
        }
        apps.retain(|app| !app.name.trim().is_empty() && !app.ids.is_empty());
        apps
    })
}

/// Is this app one of them, at any of its sources?
pub fn is_essential(id: &AppId) -> bool {
    let text = id.to_string();
    all().iter().any(|app| app.ids.iter().any(|known| known.eq_ignore_ascii_case(&text)))
}

impl Essential {
    /// The version of it this device can use: the first id whose source applies here.
    pub fn here(&self, sources: &[SourceKind]) -> Option<AppSummary> {
        let id = self.ids.iter().filter_map(|id| AppId::parse(id)).find(|id| sources.contains(&id.source))?;
        let mut app = AppSummary::new(id, self.name.clone());
        app.summary = self.summary.clone();
        Some(app)
    }
}

/// Rows for the home page: the ones nearly everyone wants first, then by kind. Only
/// what this device can use; a row with a single thing in it is left out.
pub fn rows(sources: &[SourceKind], _platform: &Platform) -> Vec<(String, Vec<AppSummary>)> {
    let pick = |wanted: &dyn Fn(&Essential) -> bool| -> Vec<AppSummary> {
        all().iter().filter(|app| wanted(app)).filter_map(|app| app.here(sources)).collect()
    };
    let mut rows = vec![("Essentials".to_string(), pick(&|app| app.top))];
    for (kind, title) in [
        ("browse", "Web browsers"),
        ("media", "Music and video"),
        ("chat", "Chat, calls and mail"),
        ("games", "Games"),
        ("work", "Work and create"),
        ("tools", "Handy tools"),
    ] {
        // Everything of the kind, the best known included: a row of browsers without
        // the two most used would be an odd row.
        rows.push((title.to_string(), pick(&|app| app.kind == kind)));
    }
    rows.retain(|(_, apps)| apps.len() >= 2);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sound() {
        let list = all();
        assert!(list.len() >= 50, "{}", list.len());
        let mut names: Vec<&str> = list.iter().map(|a| a.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), list.len(), "each app once");
        assert!(list.iter().all(|a| !a.summary.is_empty()
            && ["browse", "media", "chat", "games", "work", "tools"].contains(&a.kind.as_str())));
        // Every website on it is one the store lists by hand, and none is for adults
        // or otherwise unfit to recommend.
        for id in list.iter().flat_map(|a| &a.ids).filter_map(|id| id.strip_prefix("web:")) {
            let site = crate::source::web::listed(id).unwrap_or_else(|| panic!("{id} is not in web.json"));
            assert!(!site.adult && !crate::source::web::restricted_host(id), "{id}");
        }
        assert!(is_essential(&AppId::parse("flathub:org.videolan.VLC").unwrap()));
        assert!(is_essential(&AppId::parse("winget:videolan.vlc").unwrap()), "at any of its sources");
        assert!(!is_essential(&AppId::parse("github:someone/vlc").unwrap()));
    }

    #[test]
    fn each_device_gets_the_version_it_can_use() {
        let on = |sources: &[SourceKind]| -> Vec<(String, Vec<String>)> {
            rows(sources, &Platform::preset("linux-x64-debian").unwrap())
                .into_iter()
                .map(|(title, apps)| (title, apps.into_iter().map(|a| a.id.to_string()).collect()))
                .collect()
        };
        let linux = on(&[SourceKind::Flathub, SourceKind::Apt, SourceKind::GitHub, SourceKind::Web]);
        assert_eq!(linux[0].0, "Essentials");
        assert!(
            linux[0].1.contains(&"flathub:org.mozilla.firefox".to_string())
                && linux[0].1.contains(&"web:youtube.com".to_string())
        );
        assert!(!linux.iter().flat_map(|(_, ids)| ids).any(|id| id.starts_with("winget:") || id.starts_with("brew:")));
        let windows = on(&[SourceKind::Winget, SourceKind::GitHub, SourceKind::Web]);
        assert!(windows[0].1.contains(&"winget:Mozilla.Firefox".to_string()));
        assert!(
            windows.iter().any(|(_, ids)| ids.contains(&"winget:7zip.7zip".to_string())),
            "Windows-only tools are there on Windows"
        );
        assert!(!linux.iter().any(|(_, ids)| ids.iter().any(|id| id.contains("7zip"))), "and nowhere else");
        // With no web apps and no package source, what is left is what there is.
        assert!(on(&[SourceKind::GitHub]).is_empty());
    }
}
