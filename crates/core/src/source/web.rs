//! Websites as apps, on Linux TVs.
//!
//! A TV has no good way to use a website: no mouse, no keyboard. Longboard
//! (<https://github.com/AndrewGBest/Longboard>) is a browser driven by a remote, and it
//! opens any address it is started with. So on a Linux TV the store can offer a website
//! like any other app: "installing" one puts an entry in the TV's app list that opens
//! the site in Longboard.
//!
//! There is a short list of sites to start from, and any site can be added by typing
//! its address. Nothing is fetched to list them.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::model::{valid_key, AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{FormFactor, Os, Platform};

/// Where Longboard is published; the store installs it like any other app.
pub const LONGBOARD: &str = "AndrewGBest/Longboard";

/// A well-known site, from the list kept in `data/web.json`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Site {
    pub host: String,
    pub name: String,
    pub summary: String,
    /// What it is for: watch, listen, play, read, social, shop, tools, adult.
    #[serde(default)]
    pub kind: String,
    /// For people of 18 or over.
    #[serde(default)]
    pub adult: bool,
    /// Shown on the home page.
    #[serde(default)]
    pub featured: bool,
}

/// The bundled list. A broken file means no list, not a store that will not start;
/// an address that could not be opened is dropped.
pub fn sites() -> &'static [Site] {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        sites: Vec<Site>,
    }
    static LIST: std::sync::OnceLock<Vec<Site>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let mut sites =
            serde_json::from_str::<File>(include_str!("../../data/web.json")).map(|f| f.sites).unwrap_or_default();
        sites.retain(|site| valid_key(SourceKind::Web, &site.host) && !site.name.trim().is_empty());
        sites
    })
}

/// The listed site at this address, with or without a leading "www.".
pub fn listed(host: &str) -> Option<&'static Site> {
    let bare = host.trim_start_matches("www.");
    sites().iter().find(|site| site.host == host || site.host == bare)
}

pub struct Websites;

fn summary_of(host: &str) -> AppSummary {
    let known = listed(host);
    AppSummary {
        id: AppId::new(SourceKind::Web, host),
        name: known.map(|s| s.name.clone()).unwrap_or_else(|| host.trim_start_matches("www.").to_string()),
        author: host.to_string(),
        summary: known
            .map(|s| s.summary.clone())
            .unwrap_or_else(|| format!("Open {host} like an app, driven with the remote.")),
        popularity: None,
        homepage: format!("https://{host}/"),
        license: None,
        category: Some("Web apps".to_string()),
        version: None,
        install_label: Some(Format::WebApp.label().to_string()),
        warning: None,
        origin: None,
        installed_version: None,
        rating: None,
        icon_url: None,
        downloads: None,
        verified: false,
        momentum: None,
        price: None,
        adult: known.is_some_and(|s| s.adult),
    }
}

/// The featured websites, for the Discover rows.
pub fn featured() -> Vec<AppSummary> {
    sites().iter().filter(|s| s.featured && !s.adult).map(|s| summary_of(&s.host)).collect()
}

impl Source for Websites {
    fn kind(&self) -> SourceKind {
        SourceKind::Web
    }

    /// Only where Longboard runs and is needed: Linux, used as a TV.
    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::Linux && platform.form_factor == FormFactor::Tv
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        // A site is found by its name, its address or what it is for ("watch", "music").
        let mut hits: Vec<AppSummary> = sites()
            .iter()
            .filter(|s| {
                let text = format!("{} {} {} {}", s.name, s.host, s.summary, s.kind).to_lowercase();
                !words.is_empty() && words.iter().all(|w| text.contains(w.as_str()))
            })
            .map(|s| summary_of(&s.host))
            .collect();
        // Someone typed an address: offer exactly that site.
        let typed = query.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
        let typed = typed.to_lowercase();
        if valid_key(SourceKind::Web, &typed) && !hits.iter().any(|h| h.id.key == typed) {
            hits.insert(0, summary_of(&typed));
        }
        hits.truncate(limit.max(1));
        Ok(hits)
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        if !valid_key(SourceKind::Web, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a website's address")));
        }
        Ok(summary_of(key))
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let site = self.summary(key)?;
        Ok(vec![Release {
            // A website has no versions: it is whatever the site serves today.
            version: "web".to_string(),
            published: String::new(),
            prerelease: false,
            downloads: None,
            notes: format!(
                "This is a website, not a program. Adding it puts {} in this TV's app list; it opens in Longboard, a browser made for remotes. Nothing is downloaded, and the site can be removed again from your library.",
                site.name
            ),
            assets: vec![Asset {
                name: site.name,
                size: 0,
                url: site.homepage,
                digest: None,
                info: Some(AssetInfo {
                    format: Format::WebApp,
                    os: TargetOs::Known(Os::Linux),
                    arch: TargetArch::Any,
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some(Format::WebApp.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv() -> Platform {
        let mut p = Platform::preset("linux-x64-debian").unwrap();
        p.form_factor = FormFactor::Tv;
        p
    }

    #[test]
    fn websites_are_offered_on_linux_tvs_and_nowhere_else() {
        assert!(Websites.available_on(&tv()));
        for preset in ["linux-x64-debian", "android-tv", "windows-x64", "macos-arm64", "android-arm64"] {
            assert!(!Websites.available_on(&Platform::preset(preset).unwrap()), "{preset}");
        }
    }

    #[test]
    fn a_site_is_found_by_name_or_by_typing_its_address() {
        let ids =
            |q: &str| -> Vec<String> { Websites.search(q, 8).unwrap().into_iter().map(|a| a.id.to_string()).collect() };
        assert_eq!(ids("chess"), ["web:lichess.org", "web:chess.com"]);
        assert_eq!(ids("free books"), ["web:archive.org", "web:gutenberg.org"]);
        assert_eq!(ids("https://Example.org/"), ["web:example.org"], "any address can be added");
        assert_eq!(ids("wikipedia.org"), ["web:wikipedia.org"], "a listed site is not offered twice");
        assert!(ids("zzzz").is_empty());
        assert!(ids("not a site").is_empty());
    }

    #[test]
    fn a_site_is_one_thing_to_add_that_only_fits_a_linux_tv() {
        let release = &Websites.releases("lichess.org").unwrap()[0];
        let asset = &release.assets[0];
        assert_eq!((asset.name.as_str(), asset.url.as_str()), ("Lichess", "https://lichess.org/"));
        let info = asset.info.clone().unwrap();
        assert!(crate::compat::evaluate(&info, &tv()).usable());
        let desktop = crate::compat::evaluate(&info, &Platform::preset("linux-x64-debian").unwrap());
        assert!(!desktop.usable(), "on a desktop a website is opened in the browser already there");
        assert!(Websites.releases("../etc").is_err());
        assert!(Websites.summary("no-dot").is_err());
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;

    #[test]
    fn the_bundled_list_is_sound_and_keeps_adult_sites_off_the_home_page() {
        let all = sites();
        assert!(all.len() >= 80, "{}", all.len());
        let mut hosts: Vec<&str> = all.iter().map(|s| s.host.as_str()).collect();
        hosts.sort_unstable();
        hosts.dedup();
        assert_eq!(hosts.len(), all.len(), "each site once");
        assert!(all.iter().all(|s| !s.summary.is_empty() && !s.kind.is_empty()));
        // Sites for adults are on the list so that they can be recognised, marked as such.
        let adult: Vec<&Site> = all.iter().filter(|s| s.adult).collect();
        assert!(!adult.is_empty() && adult.iter().all(|s| s.kind == "adult" && !s.featured));
        assert!(Websites.summary(&adult[0].host).unwrap().adult);
        assert!(Websites.summary(&format!("www.{}", adult[0].host)).unwrap().adult, "with or without www");
        assert!(!Websites.summary("wikipedia.org").unwrap().adult);
        // The home page's row is the featured ones, none of them adult.
        let home = featured();
        assert!(home.len() >= 8 && home.iter().all(|app| !app.adult));
        assert!(home.iter().any(|app| app.name == "YouTube"));
        // Found by what they are for, too.
        let music: Vec<String> = Websites.search("listen", 50).unwrap().into_iter().map(|a| a.name).collect();
        assert!(music.contains(&"Spotify".to_string()), "{music:?}");
    }
}
