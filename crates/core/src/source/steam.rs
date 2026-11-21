//! Steam, through the storefront's public JSON.
//!
//! * `GET https://store.steampowered.com/api/storesearch/?term=...`
//! * `GET https://store.steampowered.com/api/appdetails?appids=...`
//!
//! The store does not install anything from Steam itself. It hands over to the Steam
//! app with Steam's own links (`steam://install/620`, `steam://rungameid/620`), and
//! Steam does the rest: signing in, payment, downloading, running. What is installed is
//! read from the files Steam keeps beside its library.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{FormFactor, Os, Platform};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const STEAM_API: &str = "https://store.steampowered.com/api";

pub struct Steam {
    http: Arc<dyn Http>,
    api: String,
    /// Where Steam answers questions about many apps at once.
    browse: String,
    os: Os,
    /// Steam's `steamapps` folders on this machine.
    libraries: Vec<PathBuf>,
    /// Whether the Steam app is there to hand over to.
    present: bool,
}

/// Where Steam keeps its library, for the person running the store.
pub fn default_libraries(os: Os) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let mut roots: Vec<PathBuf> = match (os, home) {
        (Os::Linux, Some(home)) => vec![
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            // Steam installed as a Flatpak.
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        ],
        (Os::MacOs, Some(home)) => vec![home.join("Library/Application Support/Steam")],
        (Os::Windows, _) => {
            vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"C:\Program Files\Steam")]
        }
        _ => Vec::new(),
    };
    roots.retain(|root| root.is_dir());
    let mut libraries: Vec<PathBuf> = Vec::new();
    for root in &roots {
        libraries.push(root.join("steamapps"));
        // Games on other drives: each is a "path" line in this file.
        let listed = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")).unwrap_or_default();
        libraries.extend(other_libraries(&listed));
    }
    libraries.dedup();
    libraries
}

/// The `"path"  "D:\\Games"` lines of Steam's list of library folders.
fn other_libraries(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            (quoted.next()? == "path").then(|| quoted.next())?
        })
        .map(|path| PathBuf::from(path.replace("\\\\", "\\")).join("steamapps"))
        .collect()
}

/// Steam writes `appmanifest_<id>.acf` for everything it has installed.
pub fn is_installed(libraries: &[PathBuf], id: &str) -> bool {
    crate::model::valid_key(SourceKind::Steam, id)
        && libraries.iter().any(|library| library.join(format!("appmanifest_{id}.acf")).is_file())
}

/// Everything Steam has installed: its number at Steam and its name, read from the
/// files Steam keeps beside its library. Steam's own tools and runtimes are left out.
pub fn installed_games(libraries: &[PathBuf]) -> Vec<(String, String)> {
    let quoted = |text: &str, key: &str| -> Option<String> {
        text.lines().find_map(|line| {
            let mut parts = line.split('"').skip(1).step_by(2);
            (parts.next()? == key).then(|| parts.next().map(str::to_string))?
        })
    };
    let mut games: Vec<(String, String)> = Vec::new();
    for library in libraries {
        let Ok(entries) = std::fs::read_dir(library) else { continue };
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            let Some(id) = file.strip_prefix("appmanifest_").and_then(|rest| rest.strip_suffix(".acf")) else {
                continue;
            };
            if !crate::model::valid_key(SourceKind::Steam, id) || games.iter().any(|(seen, _)| seen == id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            let Some(name) = quoted(&text, "name").filter(|n| !n.is_empty()) else { continue };
            let lower = name.to_lowercase();
            if lower.starts_with("steam linux runtime")
                || lower.starts_with("proton ")
                || lower.starts_with("steamworks common")
            {
                continue;
            }
            games.push((id.to_string(), name));
        }
    }
    games.sort_by_key(|(_, name)| name.to_lowercase());
    games
}

impl Steam {
    pub fn new(http: Arc<dyn Http>, platform: &Platform) -> Steam {
        let libraries = default_libraries(platform.os);
        // A library folder means Steam is installed, however it was installed.
        let present = !libraries.is_empty();
        Steam::with_api(http, STEAM_API, platform, libraries, present)
    }

    pub fn with_api(
        http: Arc<dyn Http>,
        api: &str,
        platform: &Platform,
        libraries: Vec<PathBuf>,
        present: bool,
    ) -> Steam {
        let api = api.trim_end_matches('/').to_string();
        // The lookup of descriptors lives on another of Steam's addresses.
        let browse =
            if api == STEAM_API { "https://api.steampowered.com".to_string() } else { format!("{api}/browse") };
        Steam { http, api, browse, os: platform.os, libraries, present }
    }

    /// Which of these apps Steam marks as adult only. One request for the lot; if it
    /// fails, none are marked, and each is still checked when its page is opened.
    fn adult_among(&self, ids: &[u64]) -> Vec<u64> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Items {
            store_items: Vec<Item>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Item {
            appid: u64,
            content_descriptorids: Vec<u32>,
        }
        #[derive(Deserialize)]
        struct Answer {
            response: Items,
        }
        if ids.is_empty() {
            return Vec::new();
        }
        let wanted: Vec<String> = ids.iter().map(|id| format!("{{\"appid\":{id}}}")).collect();
        let input = format!(
            "{{\"ids\":[{}],\"context\":{{\"language\":\"english\",\"country_code\":\"US\"}},\"data_request\":{{\"include_basic_info\":true}}}}",
            wanted.join(",")
        );
        let url = format!("{}/IStoreBrowseService/GetItems/v1?input_json={}", self.browse, url_encode(&input));
        let Ok(text) = self.http.get(&url) else { return Vec::new() };
        let Ok(answer) = serde_json::from_str::<Answer>(&text) else { return Vec::new() };
        answer
            .response
            .store_items
            .into_iter()
            .filter(|item| adult_only(&item.content_descriptorids))
            .map(|item| item.appid)
            .collect()
    }

    fn details(&self, key: &str) -> Result<Details> {
        if !crate::model::valid_key(SourceKind::Steam, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a Steam app number")));
        }
        let text = self.http.get(&format!("{}/appdetails?appids={key}&l=english&cc=US", self.api))?;
        let mut answer: std::collections::HashMap<String, Envelope> = serde_json::from_str(&text)?;
        answer
            .remove(key)
            .filter(|envelope| envelope.success)
            .and_then(|envelope| envelope.data)
            .ok_or_else(|| Error::NotFound(format!("Steam has nothing under number {key}.")))
    }

    /// Does Steam run it on this system? Windows games run on Linux through Proton.
    fn runs_here(&self, on: &Platforms) -> Option<bool> {
        match self.os {
            Os::Windows => on.windows.then_some(true),
            Os::MacOs => on.mac.then_some(true),
            // `true` is native; `false` is "through Proton".
            Os::Linux => (on.linux || on.windows).then_some(on.linux),
            _ => None,
        }
    }

    fn mark_installed(&self, mut app: AppSummary) -> AppSummary {
        if is_installed(&self.libraries, &app.id.key) {
            app.installed_version = Some("installed".to_string());
        }
        app
    }
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    items: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    #[serde(rename = "type", default)]
    kind: String,
    name: String,
    id: u64,
    price: Option<Price>,
    #[serde(default)]
    platforms: Platforms,
    tiny_image: Option<String>,
    /// A critics' score out of 100, as text; empty when there is none.
    #[serde(default)]
    metascore: Option<String>,
    /// "full" or "partial", when the game says it works with a controller.
    #[serde(default)]
    controller_support: Option<String>,
}

#[derive(Deserialize, Default, Clone, Copy)]
struct Platforms {
    #[serde(default)]
    windows: bool,
    #[serde(default)]
    mac: bool,
    #[serde(default)]
    linux: bool,
}

#[derive(Deserialize)]
struct Price {
    #[serde(default)]
    currency: String,
    #[serde(rename = "final", default)]
    amount: u64,
    final_formatted: Option<String>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    success: bool,
    data: Option<Details>,
}

#[derive(Deserialize)]
struct Details {
    name: String,
    steam_appid: u64,
    #[serde(default)]
    is_free: bool,
    short_description: Option<String>,
    #[serde(default)]
    developers: Vec<String>,
    #[serde(default)]
    platforms: Platforms,
    price_overview: Option<Price>,
    release_date: Option<ReleaseDate>,
    header_image: Option<String>,
    recommendations: Option<Recommendations>,
    /// Steam's own notes on what is in it; see [`adult_only`].
    #[serde(default)]
    content_descriptors: Descriptors,
    #[serde(default)]
    screenshots: Vec<Screenshot>,
}

#[derive(Deserialize, Default)]
struct Descriptors {
    #[serde(default)]
    ids: Vec<u32>,
}

/// Steam marks what is in a game with numbered descriptors. 3 is "adult only sexual
/// content" and 4 "gratuitous sexual content"; either makes it something for people of
/// 18 or over. The others (nudity, violence, mature themes in general) do not: they
/// cover half the catalog. [The meanings are from the SteamDatabase project's reading
/// of Steam's own definitions, not a page Valve publishes. Checked 2026-11-21 against
/// three games: Portal 2 has none, Cyberpunk 2077 has 1, 2 and 5, and an adult-only
/// title has all five.]
pub fn adult_only(descriptors: &[u32]) -> bool {
    descriptors.iter().any(|id| matches!(id, 3 | 4))
}

#[derive(Deserialize)]
struct ReleaseDate {
    #[serde(default)]
    coming_soon: bool,
}

#[derive(Deserialize)]
struct Recommendations {
    #[serde(default)]
    total: u64,
}

#[derive(Deserialize)]
struct Screenshot {
    path_full: Option<String>,
}

/// "$9.99", from Steam's own wording where it gives one.
fn price_text(price: Option<&Price>, free: bool) -> Option<String> {
    match price {
        Some(p) if p.amount == 0 => Some("Free".to_string()),
        Some(p) => Some(p.final_formatted.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| {
            let amount = format!("{}.{:02}", p.amount / 100, p.amount % 100);
            if p.currency == "USD" {
                format!("${amount}")
            } else {
                format!("{amount} {}", p.currency)
            }
        })),
        None if free => Some("Free".to_string()),
        None => None,
    }
}

/// Only pictures Steam itself serves.
fn own_picture(url: Option<String>) -> Option<String> {
    url.filter(|u| {
        u.starts_with("https://shared.akamai.steamstatic.com/") || u.starts_with("https://cdn.akamai.steamstatic.com/")
    })
}

impl Source for Steam {
    fn kind(&self) -> SourceKind {
        SourceKind::Steam
    }

    /// Computers, including one used as a TV. Steam has no phone store to hand over to.
    fn available_on(&self, platform: &Platform) -> bool {
        matches!(platform.os, Os::Windows | Os::MacOs | Os::Linux) && platform.form_factor != FormFactor::Mobile
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        if query.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        let url = format!("{}/storesearch/?term={}&l=english&cc=US", self.api, url_encode(query.trim()));
        let found: Found = serde_json::from_str(&self.http.get(&url)?)?;
        let hits: Vec<_> = found
            .items
            .into_iter()
            .filter(|hit| hit.kind == "app" && self.runs_here(&hit.platforms).is_some())
            .take(limit)
            .collect();
        let adult = self.adult_among(&hits.iter().map(|hit| hit.id).collect::<Vec<u64>>());
        Ok(hits
            .into_iter()
            .map(|hit| {
                let mut app = AppSummary::new(AppId::new(SourceKind::Steam, hit.id.to_string()), hit.name);
                app.adult = adult.contains(&hit.id);
                app.homepage = format!("https://store.steampowered.com/app/{}", hit.id);
                // A search hit's price being absent means free to play.
                app.price = price_text(hit.price.as_ref(), true);
                app.icon_url = own_picture(hit.tiny_image);
                // What the search itself says about it, so its tile is not blank.
                let mut on: Vec<&str> = Vec::new();
                if hit.platforms.windows {
                    on.push("Windows");
                }
                if hit.platforms.mac {
                    on.push("macOS");
                }
                if hit.platforms.linux {
                    on.push("Linux");
                }
                let mut says = vec![format!("On Steam for {}.", on.join(", "))];
                if let Some(score) = hit.metascore.as_deref().filter(|s| !s.is_empty() && *s != "0") {
                    says.push(format!("Metascore {score}."));
                }
                match hit.controller_support.as_deref() {
                    Some("full") => says.push("Plays fully with a controller.".to_string()),
                    Some("partial") => says.push("Plays partly with a controller.".to_string()),
                    _ => {}
                }
                app.summary = says.join(" ");
                app.author = "Steam".to_string();
                self.mark_installed(app)
            })
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        let d = self.details(key)?;
        let mut app = AppSummary::new(AppId::new(SourceKind::Steam, d.steam_appid.to_string()), d.name.clone());
        app.author = d.developers.first().cloned().unwrap_or_default();
        app.summary = super::flathub::strip_html(d.short_description.as_deref().unwrap_or(""));
        app.homepage = format!("https://store.steampowered.com/app/{}", d.steam_appid);
        app.popularity = d.recommendations.as_ref().map(|r| r.total).filter(|n| *n > 0);
        app.price = price_text(d.price_overview.as_ref(), d.is_free);
        app.icon_url = own_picture(d.header_image.clone());
        app.adult = adult_only(&d.content_descriptors.ids);
        // What the person should know before pressing the button, most important first.
        let mut notices = Vec::new();
        if d.release_date.as_ref().is_some_and(|r| r.coming_soon) {
            notices.push("Not released yet.");
        }
        if !self.present {
            notices.push("Steam is not installed on this device. Install Steam first, then come back.");
        }
        if self.runs_here(&d.platforms) == Some(false) {
            notices.push(
                "Made for Windows. On Linux, Steam runs it through Proton, which works for most games but not all.",
            );
        }
        if !notices.is_empty() {
            app.warning = Some(notices.join(" "));
        }
        Ok(self.mark_installed(app))
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some("Steam")
    }

    fn pictures(&self, key: &str) -> Vec<String> {
        let Ok(d) = self.details(key) else { return Vec::new() };
        d.screenshots
            .into_iter()
            .filter_map(|s| own_picture(s.path_full))
            // The size is a query on the address; the file itself is a JPEG.
            .filter(|url| url.split('?').next().is_some_and(|path| path.ends_with(".jpg")))
            .take(crate::screenshots::MAX_PICTURES)
            .collect()
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let d = self.details(key)?;
        if self.runs_here(&d.platforms).is_none() {
            return Ok(Vec::new());
        }
        let id = d.steam_appid;
        let link = |name: &str, url: String| Asset {
            name: name.to_string(),
            size: 0,
            url,
            digest: None,
            info: Some(AssetInfo {
                format: Format::StoreApp,
                os: TargetOs::Known(self.os),
                arch: TargetArch::Unknown,
                libc: None,
                tv: false,
                debug: false,
            }),
        };
        // With Steam here, ask it to install; its store page is the second choice.
        // Without it, the page on the web says how to get Steam.
        let assets = if self.present {
            vec![
                link("Install with Steam", format!("steam://install/{id}")),
                link("Its page in Steam", format!("steam://store/{id}")),
            ]
        } else {
            vec![link("Its page on Steam's website", format!("https://store.steampowered.com/app/{id}"))]
        };
        // Steam keeps games up to date itself and publishes no version here.
        Ok(vec![Release {
            version: String::new(),
            published: String::new(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets,
        }])
    }
}

/// Where this device would get the Steam app itself from, among the sources it has.
pub fn steam_itself(platform: &Platform, sources: &[SourceKind]) -> Option<AppId> {
    use crate::platform::LinuxFamily;
    let has = |kind: SourceKind| sources.contains(&kind);
    let id = match platform.os {
        Os::Windows if has(SourceKind::Winget) => AppId::new(SourceKind::Winget, "Valve.Steam"),
        Os::MacOs if has(SourceKind::Homebrew) => AppId::new(SourceKind::Homebrew, "steam"),
        Os::Linux if has(SourceKind::Flathub) => AppId::new(SourceKind::Flathub, "com.valvesoftware.Steam"),
        Os::Linux if has(SourceKind::Pacman) => AppId::new(SourceKind::Pacman, "steam"),
        Os::Linux if has(SourceKind::Apt) => AppId::new(SourceKind::Apt, "steam-installer"),
        Os::Linux if has(SourceKind::Dnf) || has(SourceKind::Zypper) => {
            let suse = platform.linux.as_ref().is_some_and(|l| l.family == LinuxFamily::Suse);
            AppId::new(if suse { SourceKind::Zypper } else { SourceKind::Dnf }, "steam")
        }
        _ => return None,
    };
    Some(id)
}

/// The link that asks Steam to start something.
pub fn run_link(id: &str) -> Option<PathBuf> {
    crate::model::valid_key(SourceKind::Steam, id).then(|| Path::new(&format!("steam://rungameid/{id}")).to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_folders_on_other_drives_are_read() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/me/.local/share/Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Games\"\n\t}\n}\n";
        assert_eq!(
            other_libraries(vdf),
            [PathBuf::from("/home/me/.local/share/Steam/steamapps"), PathBuf::from("D:\\Games").join("steamapps")]
        );
        assert!(other_libraries("").is_empty());
    }

    #[test]
    fn installed_is_what_steam_has_a_manifest_for() {
        let root = std::env::temp_dir().join(format!("bilby-steam-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("appmanifest_620.acf"), "\"AppState\"{}").unwrap();
        assert!(is_installed(std::slice::from_ref(&root), "620"));
        assert!(!is_installed(std::slice::from_ref(&root), "400"));
        assert!(!is_installed(&[root], "../620"), "only numbers are looked up");
        assert_eq!(run_link("620").unwrap().to_str(), Some("steam://rungameid/620"));
        assert_eq!(run_link("620; rm"), None);
    }

    #[test]
    fn prices_read_as_steam_words_them() {
        let price = |amount, formatted: Option<&str>| Price {
            currency: "USD".into(),
            amount,
            final_formatted: formatted.map(String::from),
        };
        assert_eq!(price_text(Some(&price(999, Some("$9.99"))), false).as_deref(), Some("$9.99"));
        assert_eq!(price_text(Some(&price(999, None)), false).as_deref(), Some("$9.99"));
        assert_eq!(price_text(Some(&price(0, None)), false).as_deref(), Some("Free"));
        assert_eq!(price_text(None, true).as_deref(), Some("Free"));
        assert_eq!(price_text(None, false), None, "no price given and not free: say nothing");
    }
}
