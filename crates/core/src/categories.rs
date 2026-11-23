//! The kinds of app there are, as a store's list of categories has them.
//!
//! The home page's rows are recommendations: what was made for this device, the names
//! everyone knows, what is popular at a source this week. A list of categories is a
//! different thing. It answers "what is there to watch" or "show me the games", so
//! each entry is a plain kind of app under the name other stores give it (Flathub's
//! Games and Audio & Video, the Video, Music, Games and Shopping of a TV's own store),
//! and choosing one shows everything of that kind: the hand-kept names first, then
//! all that the sources file under it.

use crate::essentials;
use crate::model::{AppSummary, SourceKind};
use crate::platform::{FormFactor, Platform};
use crate::source::web;

pub struct Category {
    /// Its name, as shown.
    pub title: &'static str,
    /// What the hand-kept lists call it (see [`essentials::Essential::shelf`]).
    key: &'static str,
    /// Flathub's lists of it, as paths under its `collection/`.
    pub flathub: &'static [&'static str],
    /// Offered on a TV. Nobody writes code from the sofa.
    tv: bool,
}

/// Every category, in the order they are shown: what people watch, hear and play
/// first, then what they do, then what they work with.
pub const ALL: [Category; 12] = [
    Category {
        title: "Video",
        key: "video",
        flathub: &["category/AudioVideo/subcategories?subcategory=Video&subcategory=TV&"],
        tv: true,
    },
    Category {
        title: "Music",
        key: "music",
        flathub: &["category/AudioVideo/subcategories?subcategory=Audio&subcategory=Music&"],
        tv: true,
    },
    Category { title: "Games", key: "games", flathub: &["category/Game?"], tv: true },
    Category {
        title: "Chat and social",
        key: "chat",
        flathub: &["category/Network/subcategories?subcategory=Chat&subcategory=InstantMessaging&subcategory=Email&subcategory=VideoConference&"],
        tv: true,
    },
    Category { title: "News and reading", key: "reading", flathub: &[], tv: true },
    Category { title: "Shopping", key: "shopping", flathub: &[], tv: true },
    Category {
        title: "Web browsers",
        key: "browsers",
        flathub: &["category/Network/subcategories?subcategory=WebBrowser&"],
        tv: true,
    },
    Category { title: "Office", key: "office", flathub: &["category/Office?"], tv: true },
    Category { title: "Photos and graphics", key: "graphics", flathub: &["category/Graphics?"], tv: true },
    Category {
        title: "Education and science",
        key: "learning",
        flathub: &["category/Education?", "category/Science?"],
        tv: true,
    },
    Category { title: "Developer tools", key: "develop", flathub: &["category/Development?"], tv: false },
    Category { title: "Utilities", key: "utilities", flathub: &["category/Utility?", "category/System?"], tv: true },
];

/// Every key a hand-kept list may file an app under.
pub fn keys() -> Vec<&'static str> {
    ALL.iter().map(|category| category.key).collect()
}

pub fn find(title: &str) -> Option<&'static Category> {
    ALL.iter().find(|category| category.title == title)
}

/// The category a listed website belongs in, by what the site is for.
fn key_of_site(site: &web::Site) -> &'static str {
    match site.kind.as_str() {
        "watch" => "video",
        "listen" => "music",
        "play" => "games",
        "social" => "chat",
        "read" => "reading",
        "shop" => "shopping",
        _ => "utilities",
    }
}

impl Category {
    /// Is it offered on this kind of device at all?
    pub fn suits(&self, platform: &Platform) -> bool {
        self.tv || platform.form_factor != FormFactor::Tv
    }

    /// The names everyone knows of this kind, in the version this device can use, and
    /// then the listed websites of the kind. Costs no network requests.
    pub fn known(&self, sources: &[SourceKind], platform: &Platform) -> Vec<AppSummary> {
        let list = essentials::all();
        let mut apps: Vec<AppSummary> = list
            .iter()
            .filter(|app| app.suits_device(platform) && app.shelf() == self.key)
            .filter_map(|app| app.here(sources, platform))
            .collect();
        if sources.contains(&SourceKind::Web) {
            // A site the list names is already there, as whatever suits this device best.
            let named: std::collections::HashSet<&str> =
                list.iter().flat_map(|app| &app.ids).filter_map(|id| id.strip_prefix("web:")).collect();
            apps.extend(
                web::sites()
                    .iter()
                    .filter(|site| !site.adult && !named.contains(site.host.as_str()) && key_of_site(site) == self.key)
                    .map(|site| web::summary_of(&site.host)),
            );
        }
        let mut seen = std::collections::HashSet::new();
        apps.retain(|app| seen.insert(app.id.to_string()));
        apps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_is_one_plain_kind_of_app() {
        let titles: Vec<&str> = ALL.iter().map(|c| c.title).collect();
        let mut once = titles.clone();
        once.sort_unstable();
        once.dedup();
        assert_eq!(once.len(), titles.len());
        // Not the home page's recommendations, and not where an app comes from.
        for not_a_kind in ["Essentials", "Made for TV", "Web apps", "Popular on Flathub", "Arch User Repository"] {
            assert!(find(not_a_kind).is_none(), "{not_a_kind}");
        }
        for wanted in ["Video", "Music", "Games", "Office"] {
            assert!(find(wanted).is_some(), "{wanted}");
        }
    }

    #[test]
    fn the_names_everyone_knows_are_each_in_the_category_people_would_look_in() {
        let desktop = Platform::preset("linux-x64-debian").unwrap();
        let sources = [SourceKind::Flathub, SourceKind::Web];
        let names = |title: &str| -> Vec<String> {
            find(title).unwrap().known(&sources, &desktop).into_iter().map(|app| app.name).collect()
        };
        let has = |title: &str, name: &str| names(title).iter().any(|n| n == name);
        assert!(has("Video", "Netflix") && has("Video", "VLC") && has("Video", "Sportsnet"));
        assert!(has("Music", "Spotify") && has("Music", "Bandcamp") && !has("Video", "Spotify"));
        assert!(has("Games", "Steam") && has("Games", "Lichess"));
        assert!(has("Chat and social", "Discord") && has("Chat and social", "Mastodon"));
        assert!(has("News and reading", "Wikipedia") && has("News and reading", "Reuters"));
        assert!(has("Shopping", "Amazon") && has("Shopping", "eBay"));
        assert!(has("Web browsers", "Firefox"));
        assert!(has("Office", "LibreOffice") && !has("Office", "GIMP"));
        assert!(has("Photos and graphics", "GIMP"));
        assert!(has("Developer tools", "Visual Studio Code"));
        assert!(has("Utilities", "Flatseal") && !has("Utilities", "Amazon"));
        // Nothing is in two categories.
        let mut all: Vec<String> = ALL.iter().flat_map(|c| names(c.title)).collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count);

        // A TV is offered what suits a TV: no office suite among its names, no code.
        let mut tv = desktop.clone();
        tv.form_factor = FormFactor::Tv;
        assert!(!find("Developer tools").unwrap().suits(&tv) && find("Developer tools").unwrap().suits(&desktop));
        assert!(find("Office").unwrap().known(&sources, &tv).is_empty());
        assert!(find("Video").unwrap().known(&sources, &tv).len() > 10);
    }
}
