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
use crate::platform::{FormFactor, Platform};
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
    /// The kinds of device it suits (`desktop`, `tv`, `mobile`). Empty for all of them.
    #[serde(default, rename = "for")]
    pub suits: Vec<String>,
    /// The kinds of device where it is in the first row.
    #[serde(default)]
    pub top: Vec<String>,
}

/// What the lists call this kind of device.
fn form(platform: &Platform) -> &'static str {
    match platform.form_factor {
        FormFactor::Desktop => "desktop",
        FormFactor::Mobile => "mobile",
        FormFactor::Tv => "tv",
    }
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

/// Is this app one of them, at any of its sources, and one that suits this kind of
/// device? An office suite is not lifted in a search on a TV.
pub fn is_essential(id: &AppId, platform: &Platform) -> bool {
    let text = id.to_string();
    all().iter().any(|app| app.suits_device(platform) && app.ids.iter().any(|known| known.eq_ignore_ascii_case(&text)))
}

impl Essential {
    pub fn suits_device(&self, platform: &Platform) -> bool {
        self.suits.is_empty() || self.suits.iter().any(|kind| kind == form(platform))
    }

    fn leads_on(&self, platform: &Platform) -> bool {
        self.top.iter().any(|kind| kind == form(platform))
    }

    /// The version of it this device can use: the first id whose source applies here.
    pub fn here(&self, sources: &[SourceKind]) -> Option<AppSummary> {
        let id = self.ids.iter().filter_map(|id| AppId::parse(id)).find(|id| sources.contains(&id.source))?;
        let mut app = AppSummary::new(id, self.name.clone());
        app.summary = self.summary.clone();
        Some(app)
    }
}

/// The row a listed website belongs in, by what the site is for: websites sit among
/// the apps of their kind, not in rows of their own.
fn kind_of_site(site: &crate::source::web::Site) -> &'static str {
    match site.kind.as_str() {
        "watch" | "listen" => "media",
        "play" => "games",
        "social" => "chat",
        _ => "tools",
    }
}

/// Rows for the home page: the ones nearly everyone wants first, then by kind. Only
/// what this device can use; a row with a single thing in it is left out. Nothing is
/// in two rows: what leads is not repeated in the row of its kind beside it.
pub fn rows(sources: &[SourceKind], platform: &Platform) -> Vec<(String, Vec<AppSummary>)> {
    // Only what suits this kind of device: a TV is for watching and playing, not for
    // spreadsheets.
    let pick = |wanted: &dyn Fn(&Essential) -> bool| -> Vec<AppSummary> {
        all()
            .iter()
            .filter(|app| app.suits_device(platform) && wanted(app))
            .filter_map(|app| app.here(sources))
            .collect()
    };
    let mut rows = vec![("Essentials".to_string(), pick(&|app| app.leads_on(platform)))];
    // The rows in the order that suits the device: things to watch and play first on
    // a TV, a browser first on a computer.
    let kinds: [(&str, &str); 6] = match platform.form_factor {
        FormFactor::Tv => [
            ("media", "Music and video"),
            ("games", "Games"),
            ("tools", "Handy tools"),
            ("chat", "Chat, calls and mail"),
            ("browse", "Web browsers"),
            ("work", "Work and create"),
        ],
        FormFactor::Mobile => [
            ("chat", "Chat, calls and mail"),
            ("media", "Music and video"),
            ("games", "Games"),
            ("browse", "Web browsers"),
            ("tools", "Handy tools"),
            ("work", "Work and create"),
        ],
        FormFactor::Desktop => [
            ("browse", "Web browsers"),
            ("media", "Music and video"),
            ("chat", "Chat, calls and mail"),
            ("games", "Games"),
            ("work", "Work and create"),
            ("tools", "Handy tools"),
        ],
    };
    // Websites the list does not name go in the row of their kind too, after the apps.
    let named: std::collections::HashSet<&str> =
        all().iter().flat_map(|app| &app.ids).filter_map(|id| id.strip_prefix("web:")).collect();
    let sites: Vec<&crate::source::web::Site> = match sources.contains(&SourceKind::Web) {
        true => crate::source::web::sites().iter().filter(|s| !s.adult && !named.contains(s.host.as_str())).collect(),
        false => Vec::new(),
    };
    for (kind, title) in kinds {
        // What already leads the page is not shown a second time just below it.
        let mut row = pick(&|app| app.kind == kind && !app.leads_on(platform));
        row.extend(sites.iter().filter(|s| kind_of_site(s) == kind).map(|s| crate::source::web::summary_of(&s.host)));
        rows.push((title.to_string(), row));
    }
    rows.retain(|(_, apps)| apps.len() >= 2);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(preset: &str) -> Platform {
        Platform::preset(preset).unwrap()
    }

    fn tv() -> Platform {
        let mut p = device("linux-x64-debian");
        p.form_factor = FormFactor::Tv;
        p
    }

    fn touch() -> Platform {
        let mut p = device("linux-x64-debian");
        p.form_factor = FormFactor::Mobile;
        p
    }

    #[test]
    fn the_list_is_sound() {
        let list = all();
        assert!(list.len() >= 120, "{}", list.len());
        let mut names: Vec<&str> = list.iter().map(|a| a.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), list.len(), "each app once");
        let kinds = ["browse", "media", "chat", "games", "work", "tools"];
        let forms = ["desktop", "tv", "mobile"];
        for app in list {
            assert!(!app.summary.is_empty() && kinds.contains(&app.kind.as_str()), "{}", app.name);
            assert!(app.suits.iter().chain(&app.top).all(|f| forms.contains(&f.as_str())), "{}", app.name);
            // It cannot lead on a kind of device it does not suit.
            assert!(app.top.iter().all(|f| app.suits.is_empty() || app.suits.contains(f)), "{}", app.name);
        }
        // Every website on it is one the store lists by hand, and none is for adults
        // or otherwise unfit to recommend.
        for id in list.iter().flat_map(|a| &a.ids).filter_map(|id| id.strip_prefix("web:")) {
            let site = crate::source::web::listed(id).unwrap_or_else(|| panic!("{id} is not in web.json"));
            assert!(!site.adult && !crate::source::web::restricted_host(id), "{id}");
        }
        let desk = device("linux-x64-debian");
        assert!(is_essential(&AppId::parse("flathub:org.videolan.VLC").unwrap(), &desk));
        assert!(is_essential(&AppId::parse("winget:videolan.vlc").unwrap(), &desk), "at any of its sources");
        assert!(!is_essential(&AppId::parse("github:someone/vlc").unwrap(), &desk));
    }

    #[test]
    fn nothing_is_in_two_rows_and_websites_sit_with_the_apps_of_their_kind() {
        let sources = [SourceKind::Flathub, SourceKind::Apt, SourceKind::GitHub, SourceKind::Web];
        for platform in [device("linux-x64-debian"), tv(), touch()] {
            let rows = rows(&sources, &platform);
            assert_eq!(rows[0].0, "Essentials");
            let mut seen = std::collections::HashSet::new();
            for (title, apps) in &rows {
                for app in apps {
                    assert!(seen.insert(app.name.to_lowercase()), "{} is shown twice (again in {title})", app.name);
                }
            }
            // No row is only websites, and none is called after them.
            assert!(rows.iter().all(|(title, _)| !title.contains("Web apps")));
            let row = |title: &str| &rows.iter().find(|(t, _)| t == title).unwrap_or_else(|| panic!("{title}")).1;
            // A site the essentials do not name is in the row of its kind, after the apps.
            let media = row("Music and video");
            let vimeo =
                media.iter().position(|a| a.id.to_string() == "web:vimeo.com").expect("Vimeo with the video apps");
            assert!(media.iter().position(|a| a.id.source != SourceKind::Web).is_some_and(|app| app < vimeo));
            assert!(row("Games").iter().any(|a| a.id.to_string() == "web:lichess.org"));
            assert!(row("Handy tools").iter().any(|a| a.id.to_string() == "web:duckduckgo.com"));
        }
        // Without the websites source there are none.
        let without = rows(&[SourceKind::Flathub], &device("linux-x64-debian"));
        assert!(without.iter().flat_map(|(_, apps)| apps).all(|a| a.id.source != SourceKind::Web));
    }

    #[test]
    fn each_device_gets_the_version_it_can_use() {
        let on = |sources: &[SourceKind]| -> Vec<(String, Vec<String>)> {
            rows(sources, &device("linux-x64-debian"))
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

    #[test]
    fn what_leads_depends_on_the_kind_of_device() {
        let sources = [SourceKind::Flathub, SourceKind::Web, SourceKind::GitHub];
        let names = |platform: &Platform| -> Vec<(String, Vec<String>)> {
            rows(&sources, platform)
                .into_iter()
                .map(|(t, apps)| (t, apps.into_iter().map(|a| a.name).collect()))
                .collect()
        };
        let has = |row: &[String], name: &str| row.iter().any(|n| n == name);

        // A computer leads with a browser and what everyone installs first.
        let desk = names(&device("linux-x64-debian"));
        assert!(has(&desk[0].1, "Firefox") && has(&desk[0].1, "LibreOffice") && has(&desk[0].1, "Zoom"));
        assert_eq!(desk[1].0, "Web browsers");

        // A TV leads with things to watch and play, and has no office suite at all.
        let tv = names(&tv());
        assert!(
            has(&tv[0].1, "Netflix") && has(&tv[0].1, "Kodi") && has(&tv[0].1, "Prime Video") && has(&tv[0].1, "Steam")
        );
        assert!(!has(&tv[0].1, "Firefox") && !has(&tv[0].1, "Zoom"));
        assert_eq!(tv[1].0, "Music and video");
        assert!(!tv.iter().flat_map(|(_, row)| row).any(|n| n == "LibreOffice" || n == "Visual Studio Code"));
        assert!(!tv.iter().any(|(title, _)| title == "Work and create"));

        // A touch screen leads with what people use on phones.
        let touch_rows = names(&touch());
        assert!(
            has(&touch_rows[0].1, "Instagram")
                && has(&touch_rows[0].1, "WhatsApp")
                && has(&touch_rows[0].1, "Google Maps")
        );
        assert_eq!(touch_rows[1].0, "Chat, calls and mail");

        // The lift in a search follows the device too.
        let office = AppId::parse("flathub:org.libreoffice.LibreOffice").unwrap();
        assert!(is_essential(&office, &device("linux-x64-debian")) && !is_essential(&office, &self::tv()));
        let kodi = AppId::parse("flathub:tv.kodi.Kodi").unwrap();
        assert!(is_essential(&kodi, &self::tv()) && is_essential(&kodi, &device("linux-x64-debian")));
    }
}
