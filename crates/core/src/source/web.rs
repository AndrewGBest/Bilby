//! Websites as apps.
//!
//! A website can be offered like any other app: "installing" one puts an entry among
//! the device's apps that opens the site. On a Linux TV it opens in Longboard
//! (<https://github.com/AndrewGBest/Longboard>), a browser driven by a remote; anywhere
//! else, in the browser the person already uses.
//!
//! Three lists are bundled, and nothing is fetched to consult them:
//! * `data/web.json`: about ninety well-known sites, written by hand, with a name and
//!   what each is for. These are what the home page recommends.
//! * `data/web-top.txt`: the hosts among the thousand most visited by Chrome's users,
//!   so that a site is found by typing part of its name. Bare addresses, in no order,
//!   and not vetted by anyone: they are found by searching, never recommended.
//! * `data/adult-hosts.txt.gz`: some sixty thousand addresses of sites for adults, so
//!   that one is recognised whether or not it is on the other two lists.
//!
//! Any other address can be typed and added.

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

/// The source. `tv`: the device is a TV, where sites open in Longboard.
pub struct Websites {
    pub tv: bool,
    os: Os,
}

impl Websites {
    pub fn new(platform: &Platform) -> Websites {
        Websites { tv: platform.os == Os::Linux && platform.form_factor == FormFactor::Tv, os: platform.os }
    }
}

/// The hosts among the thousand most visited, tidied. See the module's notes.
pub fn top_hosts() -> &'static [&'static str] {
    static LIST: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        include_str!("../../data/web-top.txt")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#') && valid_key(SourceKind::Web, line))
            .collect()
    })
}

/// Is this a site for adults, by the bundled list of such addresses? A site counts
/// when it or any domain it sits under is listed, so `fr.example.com` follows
/// `example.com`. [The list is the "porn" extension of StevenBlack's hosts project
/// (MIT), which merges lists by Sinfonietta, BigDargon and Clefspeare13 (MIT) and by
/// Tiuxo and brijrajparmar27 (CC BY 4.0); fetched 2026-11-21.]
pub fn adult_host(host: &str) -> bool {
    static LIST: std::sync::OnceLock<Hosts> = std::sync::OnceLock::new();
    on_list(LIST.get_or_init(|| unpack(include_bytes!("../../data/adult-hosts.txt.gz"))), host)
}

/// Is this a gambling site, or one known for piracy? Such sites are shown like any
/// other when searched for, never recommended, and gone in Family mode. [Gambling:
/// the "gambling" extension of StevenBlack's hosts project, from lists by BigDargon
/// and Sinfonietta (MIT). Piracy: The Block List Project's piracy list (MIT by its own
/// header). Fetched 2026-11-21. The piracy list is short and misses a good deal.]
pub fn restricted_host(host: &str) -> bool {
    static GAMBLING: std::sync::OnceLock<Hosts> = std::sync::OnceLock::new();
    static PIRACY: std::sync::OnceLock<Hosts> = std::sync::OnceLock::new();
    on_list(GAMBLING.get_or_init(|| unpack(include_bytes!("../../data/gambling-hosts.txt.gz"))), host)
        || on_list(PIRACY.get_or_init(|| unpack(include_bytes!("../../data/piracy-hosts.txt.gz"))), host)
}

type Hosts = std::collections::HashSet<String>;

fn unpack(packed: &[u8]) -> Hosts {
    use std::io::Read;
    let mut text = String::new();
    let _ = flate2::read::GzDecoder::new(packed).read_to_string(&mut text);
    text.lines().map(str::to_string).collect()
}

/// A site counts when it or any domain it sits under is listed.
fn on_list(list: &Hosts, host: &str) -> bool {
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    // The whole address, then each domain above it, stopping before the bare ending.
    (0..labels.len().saturating_sub(1)).any(|from| list.contains(&labels[from..].join(".")))
}

/// A website as the lists show it: the hand-written name and summary where there is one.
pub fn summary_of(host: &str) -> AppSummary {
    let known = listed(host);
    AppSummary {
        id: AppId::new(SourceKind::Web, host),
        name: known.map(|s| s.name.clone()).unwrap_or_else(|| host.trim_start_matches("www.").to_string()),
        author: host.to_string(),
        summary: known.map(|s| s.summary.clone()).unwrap_or_else(|| format!("The website {host}, opened like an app.")),
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
        adult: known.is_some_and(|s| s.adult) || adult_host(host),
        restricted: restricted_host(host),
    }
}

/// Every listed website, for the one row that is only websites: the best known first,
/// then the rest in the list's order. Nothing for adults.
pub fn featured() -> Vec<AppSummary> {
    let fit = || sites().iter().filter(|s| !s.adult);
    fit().filter(|s| s.featured).chain(fit().filter(|s| !s.featured)).map(|s| summary_of(&s.host)).collect()
}

impl Source for Websites {
    fn kind(&self) -> SourceKind {
        SourceKind::Web
    }

    /// Wherever there is a browser to open them in: Longboard on a Linux TV, the
    /// person's own browser on a computer or a phone.
    fn available_on(&self, platform: &Platform) -> bool {
        matches!(platform.os, Os::Linux | Os::Windows | Os::MacOs | Os::Android)
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
        // Then the widely visited ones, by a part of their address. Three letters at
        // least, or everything with an "a" in it would match.
        let typed_word = query.trim().to_lowercase();
        if typed_word.chars().count() >= 3 && !typed_word.contains(char::is_whitespace) {
            let mut more: Vec<&str> = top_hosts()
                .iter()
                .copied()
                .filter(|host| host.contains(typed_word.as_str()) && !hits.iter().any(|h| h.id.key == *host))
                // A listed site's other addresses (fr.wikipedia.org) are that site.
                .filter(|host| !sites().iter().any(|site| host.ends_with(&format!(".{}", site.host))))
                .collect();
            // The address that starts with what was typed first, shorter before longer.
            more.sort_by_key(|host| (!host.starts_with(typed_word.as_str()), host.len()));
            hits.extend(more.into_iter().take(8).map(summary_of));
        }
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
                "This is a website, not a program. Adding it puts {} among this device's apps; it opens in a browser (Longboard on a TV). Nothing is downloaded, and the site can be removed again from your library.",
                site.name
            ),
            assets: vec![Asset {
                name: site.name,
                size: 0,
                url: site.homepage,
                digest: None,
                info: Some(AssetInfo {
                    format: Format::WebApp,
                    // Any system: it is an address. Named as this one so that it is
                    // offered here.
                    os: TargetOs::Known(self.os),
                    arch: TargetArch::Any,
                    libc: None,
                    // On a TV it opens in Longboard; elsewhere in the usual browser.
                    tv: self.tv,
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

    const ON_TV: Websites = Websites { tv: true, os: Os::Linux };

    fn tv() -> Platform {
        let mut p = Platform::preset("linux-x64-debian").unwrap();
        p.form_factor = FormFactor::Tv;
        p
    }

    #[test]
    fn websites_are_offered_wherever_there_is_a_browser() {
        assert!(ON_TV.available_on(&tv()));
        for preset in ["linux-x64-debian", "windows-x64", "macos-arm64", "android-arm64"] {
            let platform = Platform::preset(preset).unwrap();
            let sites = Websites::new(&platform);
            assert!(sites.available_on(&platform), "{preset}");
            assert!(!sites.tv, "{preset}: its own browser, not Longboard");
        }
        assert!(Websites::new(&tv()).tv);
    }

    #[test]
    fn a_site_is_found_by_name_or_by_typing_its_address() {
        let ids =
            |q: &str| -> Vec<String> { ON_TV.search(q, 8).unwrap().into_iter().map(|a| a.id.to_string()).collect() };
        assert_eq!(ids("chess"), ["web:lichess.org", "web:chess.com"]);
        assert_eq!(ids("free books"), ["web:archive.org", "web:gutenberg.org"]);
        assert_eq!(ids("https://Example.org/"), ["web:example.org"], "any address can be added");
        assert_eq!(ids("wikipedia.org"), ["web:wikipedia.org"], "a listed site is not offered twice");
        assert!(ids("zzzz").is_empty());
        assert!(ids("not a site").is_empty());
    }

    #[test]
    fn a_site_is_one_thing_to_add_on_a_tv_or_a_computer() {
        let release = &ON_TV.releases("lichess.org").unwrap()[0];
        let asset = &release.assets[0];
        assert_eq!((asset.name.as_str(), asset.url.as_str()), ("Lichess", "https://lichess.org/"));
        let info = asset.info.clone().unwrap();
        assert!(crate::compat::evaluate(&info, &tv()).usable());
        assert!(info.tv, "on a TV it opens in Longboard");
        // On a computer it is offered too, to open in the browser already there.
        let debian = Platform::preset("linux-x64-debian").unwrap();
        let there = Websites::new(&debian).releases("lichess.org").unwrap()[0].assets[0].info.clone().unwrap();
        assert!(!there.tv && crate::compat::evaluate(&there, &debian).usable());
        let windows = Platform::preset("windows-x64").unwrap();
        let on_windows = Websites::new(&windows).releases("lichess.org").unwrap()[0].assets[0].info.clone().unwrap();
        assert!(crate::compat::evaluate(&on_windows, &windows).usable());
        assert!(ON_TV.releases("../etc").is_err());
        assert!(ON_TV.summary("no-dot").is_err());
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;
    const ON_TV: Websites = Websites { tv: true, os: Os::Linux };

    #[test]
    fn sites_for_adults_are_recognised_from_the_bundled_list() {
        assert!(adult_host("pornhub.com") && adult_host("www.pornhub.com") && adult_host("fr.pornhub.com"));
        for ordinary in ["wikipedia.org", "example.org", "youtube.com", "com", "", "bbc.co.uk"] {
            assert!(!adult_host(ordinary), "{ordinary}");
        }
        // A typed address nobody listed by hand is still marked.
        let typed = ON_TV.summary("fr.pornhub.com").unwrap();
        assert!(typed.adult);
    }

    #[test]
    fn gambling_and_piracy_sites_are_marked_and_none_is_recommended() {
        assert!(
            restricted_host("bet365.com") || restricted_host("pokerstars.com"),
            "a well-known betting site is listed"
        );
        assert!(restricted_host("1337x.to") || restricted_host("thepiratebay.org"), "and a well-known piracy site");
        assert!(!restricted_host("wikipedia.org") && !restricted_host("netflix.com"));
        // Nothing written into the list by hand is one, and so nothing on the home page.
        assert!(
            sites().iter().all(|site| !restricted_host(&site.host)),
            "{:?}",
            sites().iter().find(|s| restricted_host(&s.host)).map(|s| &s.host)
        );
        assert!(featured().iter().all(|app| !app.restricted && !app.adult));
        // The ones people most expect are there to be promoted.
        let home: Vec<String> = featured().into_iter().map(|app| app.name).collect();
        for name in ["YouTube", "Netflix", "Twitch", "Spotify", "Disney+"] {
            assert!(home.contains(&name.to_string()), "{name}: {home:?}");
        }
    }

    #[test]
    fn widely_visited_sites_are_found_by_part_of_their_address_but_not_recommended() {
        let top = top_hosts();
        assert!(top.len() > 800 && top.contains(&"archive.org"), "{}", top.len());
        assert!(!top.iter().any(|h| h.starts_with("www.") || h.starts_with("accounts.")));
        // A site only on that list is found by searching.
        let only_there = top.iter().find(|h| listed(h).is_none() && !adult_host(h) && h.len() > 8).unwrap();
        let word = only_there.split('.').next().unwrap();
        let found: Vec<String> = ON_TV.search(word, 20).unwrap().into_iter().map(|a| a.id.key).collect();
        assert!(found.iter().any(|h| h == only_there), "{word}: {found:?}");
        // The home page's row of websites comes only from the hand-written list: all
        // of it that is fit to recommend, the best known first.
        let row = featured();
        assert_eq!(row.len(), sites().iter().filter(|s| !s.adult).count());
        assert!(row.iter().all(|site| !site.adult && listed(&site.id.key).is_some()));
        assert_eq!(row[0].name, "YouTube");
    }

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
        assert!(ON_TV.summary(&adult[0].host).unwrap().adult);
        assert!(ON_TV.summary(&format!("www.{}", adult[0].host)).unwrap().adult, "with or without www");
        assert!(!ON_TV.summary("wikipedia.org").unwrap().adult);
        // The home page's row is the featured ones, none of them adult.
        let home = featured();
        assert!(home.len() >= 8 && home.iter().all(|app| !app.adult));
        assert!(home.iter().any(|app| app.name == "YouTube"));
        // Found by what they are for, too.
        let music: Vec<String> = ON_TV.search("listen", 50).unwrap().into_iter().map(|a| a.name).collect();
        assert!(music.contains(&"Spotify".to_string()), "{music:?}");
    }
}
