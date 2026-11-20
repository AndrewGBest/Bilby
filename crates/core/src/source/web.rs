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

struct Site {
    host: &'static str,
    name: &'static str,
    summary: &'static str,
}

/// Free, public sites that make sense on a TV. Written by hand, from memory: the
/// addresses were not checked from where this was written.
const SITES: &[Site] = &[
    Site { host: "wikipedia.org", name: "Wikipedia", summary: "The free encyclopedia." },
    Site {
        host: "archive.org",
        name: "Internet Archive",
        summary: "Films, music, books and old software, free to watch and read.",
    },
    Site { host: "openstreetmap.org", name: "OpenStreetMap", summary: "The free map of the world." },
    Site { host: "gutenberg.org", name: "Project Gutenberg", summary: "Tens of thousands of free books." },
    Site { host: "somafm.com", name: "SomaFM", summary: "Listener-supported internet radio, with no adverts." },
    Site { host: "lichess.org", name: "Lichess", summary: "Free, open-source chess." },
    Site { host: "sepiasearch.org", name: "Sepia Search", summary: "Find videos across PeerTube sites." },
    Site { host: "plus.nasa.gov", name: "NASA+", summary: "NASA's free video service." },
];

pub struct Websites;

fn summary_of(host: &str) -> AppSummary {
    let known = SITES.iter().find(|s| s.host == host);
    AppSummary {
        id: AppId::new(SourceKind::Web, host),
        name: known.map(|s| s.name.to_string()).unwrap_or_else(|| host.trim_start_matches("www.").to_string()),
        author: host.to_string(),
        summary: known
            .map(|s| s.summary.to_string())
            .unwrap_or_else(|| format!("Open {host} like an app, driven with the remote.")),
        popularity: None,
        homepage: format!("https://{host}/"),
        license: None,
        category: Some("Websites".to_string()),
        version: None,
        install_label: Some(Format::WebApp.label().to_string()),
        warning: None,
        origin: None,
        installed_version: None,
        rating: None,
        icon_url: None,
    }
}

/// The featured websites, for the Discover rows.
pub fn featured() -> Vec<AppSummary> {
    SITES.iter().map(|s| summary_of(s.host)).collect()
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
        let mut hits: Vec<AppSummary> = SITES
            .iter()
            .filter(|s| {
                let text = format!("{} {} {}", s.name, s.host, s.summary).to_lowercase();
                !words.is_empty() && words.iter().all(|w| text.contains(w.as_str()))
            })
            .map(|s| summary_of(s.host))
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
        assert_eq!(ids("chess"), ["web:lichess.org"]);
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
