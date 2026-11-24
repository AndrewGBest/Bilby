//! Flathub, through its public API.
//!
//! Endpoints used, as read from the service's source code
//! (<https://github.com/flathub-infra/website>, `backend/app/routes`); they have not
//! been tried against the live service from where this was written:
//! * `POST /api/v2/search?locale=en` with `{"query": ..., "hits_per_page": N}`
//! * `GET /api/v2/appstream/{app_id}`
//! * `GET /api/v2/summary/{app_id}`
//! * `GET /api/v2/stats/{app_id}`: installs in total and in the last month
//!
//! Flathub apps arrive with what release pages lack: a proper icon, screenshots, and
//! an id that ratings services know.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::{valid_key, AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{Os, Platform};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

pub const FLATHUB_API: &str = "https://flathub.org/api/v2";

pub struct Flathub {
    http: Arc<dyn Http>,
    api: String,
    /// Where Flatpak keeps installed apps (`.../app/<id>`), to mark what is installed.
    roots: Vec<PathBuf>,
    /// The language names and descriptions are asked for in, as Flathub names it.
    locale: String,
}

impl Flathub {
    pub fn new(http: Arc<dyn Http>, roots: Vec<PathBuf>) -> Flathub {
        Flathub::with_api(http, FLATHUB_API, roots)
    }

    pub fn with_api(http: Arc<dyn Http>, api: &str, roots: Vec<PathBuf>) -> Flathub {
        Flathub { http, api: api.trim_end_matches('/').to_string(), roots, locale: "en".to_string() }
    }

    /// Ask for names and descriptions in this language (a code such as `fr`), where
    /// an app's makers have published them in it; English otherwise.
    pub fn in_language(mut self, code: &str) -> Flathub {
        self.locale = match code {
            "" => "en",
            "pt" => "pt-BR",
            "zh" => "zh-Hans-CN",
            other => other,
        }
        .to_string();
        self
    }

    fn installed(&self, id: &str) -> bool {
        is_installed(&self.roots, id)
    }
}

/// Is this Flatpak app on the device, in the system-wide or the per-user installation?
pub fn is_installed(roots: &[PathBuf], id: &str) -> bool {
    valid_key(SourceKind::Flathub, id) && roots.iter().any(|root| root.join("app").join(id).is_dir())
}

/// Every Flatpak app on the device, however it got there: its id and the name its own
/// menu entry gives it. Runtimes are not apps and are not listed.
pub fn installed_apps(roots: &[PathBuf]) -> Vec<(String, String)> {
    let mut apps: Vec<(String, String)> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join("app")) else { continue };
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            if !valid_key(SourceKind::Flathub, &id) || apps.iter().any(|(seen, _)| *seen == id) {
                continue;
            }
            let menu = entry.path().join("current/active/export/share/applications").join(format!("{id}.desktop"));
            let name = std::fs::read_to_string(menu)
                .ok()
                .and_then(|text| text.lines().find_map(|line| line.strip_prefix("Name=").map(|n| n.trim().to_string())))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| id.rsplit('.').next().unwrap_or(&id).to_string());
            apps.push((id, name));
        }
    }
    apps.sort_by_key(|(_, name)| name.to_lowercase());
    apps
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    hits: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    app_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    icon: Option<String>,
    developer_name: Option<String>,
    project_license: Option<String>,
    installs_last_month: Option<u64>,
    /// The developer has proved to Flathub that the app is theirs.
    #[serde(default)]
    verification_verified: bool,
    /// Flathub's own trend score. Its scale is not documented, so only its sign is used:
    /// above zero means the app is being installed more than it was.
    trending: Option<f64>,
}

#[derive(Deserialize)]
struct Appstream {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    developer_name: Option<String>,
    icon: Option<String>,
    project_license: Option<String>,
    #[serde(default)]
    releases: Vec<AppRelease>,
    #[serde(default)]
    screenshots: Vec<Screenshot>,
    urls: Option<Urls>,
    /// Extra facts Flathub adds, among them whether the developer is verified.
    metadata: Option<serde_json::Value>,
    /// The app's own statement of what is in it, worked out by Flathub into a
    /// youngest suitable age, per language.
    content_rating_details: Option<serde_json::Value>,
}

impl Appstream {
    /// For people of 18 or over, by the app's own content rating.
    fn adult(&self) -> bool {
        let Some(details) = self.content_rating_details.as_ref().and_then(|d| d.as_object()) else { return false };
        details.values().any(|language| language["minimumAge"].as_u64().is_some_and(|age| age >= 18))
    }

    fn verified(&self) -> bool {
        let value = self.metadata.as_ref().map(|m| &m["flathub::verification::verified"]);
        value.is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"))
    }
}

#[derive(Deserialize)]
struct Stats {
    installs_total: Option<u64>,
    installs_last_month: Option<u64>,
}

#[derive(Deserialize)]
struct AppRelease {
    version: Option<String>,
    /// Seconds since 1970, as a number or as text depending on the entry.
    timestamp: Option<serde_json::Value>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct Screenshot {
    #[serde(default)]
    sizes: Vec<ScreenshotSize>,
}

#[derive(Deserialize)]
struct ScreenshotSize {
    src: Option<String>,
    /// Width in pixels, as a number or as text.
    width: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Urls {
    homepage: Option<String>,
}

#[derive(Deserialize)]
struct Summary {
    download_size: Option<u64>,
    #[serde(default)]
    arches: Vec<String>,
}

/// Flathub serves pictures from its own media host; nothing else is ever fetched.
fn own_media(url: Option<String>) -> Option<String> {
    url.filter(|u| u.starts_with("https://dl.flathub.org/") || u.starts_with("https://flathub.org/"))
}

fn number(value: &Option<serde_json::Value>) -> Option<u64> {
    match value.as_ref()? {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

impl Flathub {
    fn summary_from(&self, id: String, name: String, summary: String, developer: Option<String>) -> AppSummary {
        AppSummary {
            name: if name.is_empty() { id.rsplit('.').next().unwrap_or(&id).to_string() } else { name },
            author: developer.unwrap_or_default(),
            summary,
            popularity: None,
            homepage: format!("https://flathub.org/apps/{id}"),
            license: None,
            category: None,
            version: None,
            install_label: Some(Format::FlathubApp.label().to_string()),
            warning: None,
            origin: None,
            installed_version: self.installed(&id).then(|| "installed".to_string()),
            rating: None,
            icon_url: None,
            downloads: None,
            verified: false,
            momentum: None,
            price: None,
            adult: false,
            restricted: false,
            id: AppId::new(SourceKind::Flathub, id),
        }
    }

    fn appstream(&self, key: &str) -> Result<Appstream> {
        if !valid_key(SourceKind::Flathub, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a Flathub app id")));
        }
        Ok(serde_json::from_str(&self.http.get(&match self.locale.as_str() {
            "en" => format!("{}/appstream/{key}", self.api),
            locale => format!("{}/appstream/{key}?locale={locale}", self.api),
        })?)?)
    }

    /// Up to four screenshot addresses for an app, each the size nearest to what a page shows.
    pub fn screenshots(&self, key: &str) -> Vec<String> {
        let Ok(app) = self.appstream(key) else { return Vec::new() };
        app.screenshots
            .into_iter()
            .filter_map(|shot| {
                let mut sizes: Vec<(u64, String)> =
                    shot.sizes.into_iter().filter_map(|s| Some((number(&s.width)?, own_media(s.src)?))).collect();
                // The smallest that is still at least 600 wide; else the largest there is.
                sizes.sort_by_key(|(width, _)| *width);
                // Wide enough to stay sharp when shown large (on a TV a picture fills
                // the screen): the first of about 1,200 pixels or more, else the widest.
                let pick = sizes.iter().find(|(width, _)| *width >= 1200).or(sizes.last()).cloned();
                pick.map(|(_, src)| src)
            })
            .filter(|src| {
                let path = src.split('?').next().unwrap_or(src).to_ascii_lowercase();
                path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg")
            })
            .take(crate::screenshots::MAX_PICTURES)
            .collect()
    }
}

impl Flathub {
    fn summaries_of(&self, hits: Vec<Hit>) -> Vec<AppSummary> {
        hits.into_iter()
            .filter(|h| valid_key(SourceKind::Flathub, &h.app_id))
            .map(|h| {
                let mut app = self.summary_from(h.app_id, h.name, h.summary, h.developer_name);
                app.popularity = h.installs_last_month;
                app.license = h.project_license;
                app.icon_url = own_media(h.icon);
                app.verified = h.verification_verified;
                app.momentum = h.trending.map(|t| if t > 0.0 { 0.75 } else { 0.25 });
                app
            })
            .collect()
    }
}

const LISTS: [(&str, &str); 3] = [
    ("Popular on Flathub", "popular"),
    ("Trending on Flathub", "trending"),
    ("New and updated on Flathub", "recently-updated"),
];
// Flathub files every app under what it is for. Those lists, most used first,
// fill the home page's rows of the same kind: the row is then everything of
// that kind worth having, not only the handful the store names itself.
const KINDS: [(&str, &[&str]); 7] = [
    ("Web browsers", &["category/Network/subcategories?subcategory=WebBrowser&"]),
    ("Music and video", &["category/AudioVideo?"]),
    (
        "Chat, calls and mail",
        &["category/Network/subcategories?subcategory=Chat&subcategory=InstantMessaging&subcategory=Email&subcategory=VideoConference&"],
    ),
    ("Games", &["category/Game?"]),
    ("Work and create", &["category/Office?", "category/Graphics?"]),
    ("Handy tools", &["category/Utility?"]),
    ("Developer tools", &["category/Development?"]),
];

impl Source for Flathub {
    fn kind(&self) -> SourceKind {
        SourceKind::Flathub
    }

    /// Wherever Flatpak is set up.
    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::Linux && platform.linux.as_ref().is_some_and(|l| l.flatpak)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let body = serde_json::json!({ "query": query, "hits_per_page": limit.clamp(1, 50), "page": 1 }).to_string();
        let resp: SearchResponse =
            serde_json::from_str(&self.http.post_json(&format!("{}/search?locale={}", self.api, self.locale), &body)?)?;
        Ok(self.summaries_of(resp.hits))
    }

    /// Flathub's own lists, for a home page with more on it than the bundled few.
    fn collections(&self, limit: usize) -> Vec<(String, Vec<AppSummary>)> {
        let fetch = |path: &str| -> Result<Vec<AppSummary>> {
            let url =
                format!("{}/collection/{path}?page=1&per_page={}&locale={}", self.api, limit.clamp(1, 50), self.locale);
            let resp: SearchResponse = serde_json::from_str(&self.http.get(&url)?)?;
            Ok(self.summaries_of(resp.hits))
        };
        let of_kind = |paths: &[&str]| -> Result<Vec<AppSummary>> {
            let per_list = (limit * 2 / paths.len()).clamp(1, 50);
            let mut lists = Vec::new();
            for path in paths {
                let url = format!("{}/collection/{path}page=1&per_page={per_list}&locale={}", self.api, self.locale);
                let resp: SearchResponse = serde_json::from_str(&self.http.get(&url)?)?;
                lists.push(self.summaries_of(resp.hits).into_iter());
            }
            // Two lists for one row (documents, pictures) take turns.
            let mut apps = Vec::new();
            while lists.iter().any(|list| list.len() > 0) {
                apps.extend(lists.iter_mut().filter_map(Iterator::next));
            }
            Ok(apps)
        };
        // Asked all at once: the page waits as long as the slowest list, not their sum.
        // A list that cannot be fetched is left out; the rest does not wait on it.
        let (kinds, lists) = std::thread::scope(|scope| {
            let kinds: Vec<_> = KINDS.iter().map(|(title, paths)| (title, scope.spawn(|| of_kind(paths)))).collect();
            let lists: Vec<_> = LISTS.iter().map(|(title, path)| (title, scope.spawn(|| fetch(path)))).collect();
            let done = |all: Vec<(&&str, std::thread::ScopedJoinHandle<Result<Vec<AppSummary>>>)>| {
                all.into_iter()
                    .filter_map(|(title, list)| Some((title.to_string(), list.join().ok()?.ok()?)))
                    .filter(|(_, apps)| !apps.is_empty())
                    .collect::<Vec<_>>()
            };
            (done(kinds), done(lists))
        });
        kinds.into_iter().chain(lists).collect()
    }

    /// One list in full: page after page of it, until there is enough or no more.
    fn collection(&self, title: &str, limit: usize) -> Vec<AppSummary> {
        let kind = KINDS.iter().find(|(name, _)| *name == title).map(|(_, paths)| paths.to_vec());
        let list = LISTS.iter().find(|(name, _)| *name == title).map(|(_, path)| vec![format!("{path}?")]);
        // Or one of the store's categories, which says itself what Flathub calls it.
        let category = crate::categories::find(title).filter(|c| !c.flathub.is_empty()).map(|c| c.flathub.to_vec());
        let Some(paths) =
            category.or(kind).map(|paths| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>()).or(list)
        else {
            return Vec::new();
        };
        const PAGE: usize = 50;
        let mut apps: Vec<AppSummary> = Vec::new();
        for page in 1..=limit.div_ceil(PAGE).max(1) {
            let mut lists = Vec::new();
            for path in &paths {
                let url = format!("{}/collection/{path}page={page}&per_page={PAGE}&locale={}", self.api, self.locale);
                let hits = self.http.get(&url).ok().and_then(|text| serde_json::from_str::<SearchResponse>(&text).ok());
                lists.push(hits.map(|resp| self.summaries_of(resp.hits)).unwrap_or_default().into_iter());
            }
            let before = apps.len();
            while lists.iter().any(|list| list.len() > 0) {
                for app in lists.iter_mut().filter_map(Iterator::next) {
                    if !apps.iter().any(|seen| seen.id == app.id) {
                        apps.push(app);
                    }
                }
            }
            if apps.len() == before || apps.len() >= limit {
                break;
            }
        }
        apps.truncate(limit);
        apps
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        let a = self.appstream(key)?;
        let verified = a.verified();
        let adult = a.adult();
        let mut app = self.summary_from(a.id, a.name, a.summary, a.developer_name);
        app.verified = verified;
        app.adult = adult;
        // How much it is installed comes from a separate small document. Without it the
        // page simply has no such figures.
        let stats = self.http.get(&format!("{}/stats/{key}", self.api)).ok();
        if let Some(stats) = stats.and_then(|text| serde_json::from_str::<Stats>(&text).ok()) {
            app.popularity = stats.installs_last_month;
            app.downloads = stats.installs_total;
        }
        app.license = a.project_license;
        app.icon_url = own_media(a.icon);
        if let Some(homepage) = a.urls.and_then(|u| u.homepage).filter(|u| u.starts_with("https://")) {
            app.homepage = homepage;
        }
        Ok(app)
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let app = self.appstream(key)?;
        // Size and processors come from a second, small document. Without it the app is
        // still listed, just with no size.
        let summary: Option<Summary> = self
            .http
            .get(&format!("{}/summary/{key}", self.api))
            .ok()
            .and_then(|body| serde_json::from_str(&body).ok());
        let (size, arches) = summary.map(|s| (s.download_size.unwrap_or(0), s.arches)).unwrap_or_default();
        let latest = app.releases.first();
        let published = latest
            .and_then(|r| number(&r.timestamp))
            .map(|secs| crate::model::date_from_unix(secs as i64))
            .unwrap_or_default();
        // Many releases say nothing about what changed. Then there is no "What's new":
        // repeating the app's description under that heading would be untrue.
        let notes = strip_html(latest.and_then(|r| r.description.as_deref()).unwrap_or(""));
        Ok(vec![Release {
            version: latest.and_then(|r| r.version.clone()).unwrap_or_else(|| "latest".to_string()),
            published,
            prerelease: false,
            downloads: None,
            notes,
            assets: vec![Asset {
                name: key.to_string(),
                size,
                url: String::new(),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::FlathubApp,
                    os: TargetOs::Known(Os::Linux),
                    arch: arch_for(&arches),
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }

    fn about(&self, key: &str) -> Result<Option<String>> {
        Ok(self.appstream(key).ok().map(|a| strip_html(&a.description)).filter(|text| !text.is_empty()))
    }

    fn pictures(&self, key: &str) -> Vec<String> {
        self.screenshots(key)
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some(Format::FlathubApp.label())
    }
}

/// Which processors the app is built for. When Flathub lists none, it is not second-guessed.
fn arch_for(arches: &[String]) -> TargetArch {
    use crate::platform::Arch;
    let known: Vec<Arch> = arches
        .iter()
        .filter_map(|a| match a.as_str() {
            "x86_64" => Some(Arch::X86_64),
            "aarch64" => Some(Arch::Aarch64),
            _ => None,
        })
        .collect();
    match known.as_slice() {
        [only] => TargetArch::One(*only),
        _ => TargetArch::Any,
    }
}

/// AppStream descriptions are a small subset of HTML: paragraphs and lists.
pub fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close].trim().to_ascii_lowercase();
        match tag.as_str() {
            "li" => out.push_str("\n•  "),
            "/p" | "/ul" | "/ol" | "br" | "br/" => out.push('\n'),
            "p" => out.push('\n'),
            _ => {}
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    let text = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        // One blank line between paragraphs at most.
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_lose_their_markup_and_keep_their_shape() {
        let html = "<p>A &amp; B make notes.</p><p>Features:</p><ul><li>Fast</li><li>Works &lt;offline&gt;</li></ul><p>Enjoy</p>";
        assert_eq!(strip_html(html), "A & B make notes.\n\nFeatures:\n\n•  Fast\n•  Works <offline>\n\nEnjoy");
        assert_eq!(strip_html("plain"), "plain");
        assert_eq!(strip_html("broken <p"), "broken");
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn processors_are_only_narrowed_when_flathub_names_exactly_one() {
        assert_eq!(arch_for(&["x86_64".into()]), TargetArch::One(crate::platform::Arch::X86_64));
        assert_eq!(arch_for(&["x86_64".into(), "aarch64".into()]), TargetArch::Any);
        assert_eq!(arch_for(&[]), TargetArch::Any);
    }

    #[test]
    fn pictures_come_only_from_flathubs_own_host() {
        assert!(own_media(Some("https://dl.flathub.org/media/a/icon.png".into())).is_some());
        assert!(own_media(Some("https://evil.example/icon.png".into())).is_none());
        assert!(own_media(Some("http://dl.flathub.org/a.png".into())).is_none());
        assert!(own_media(None).is_none());
    }
}
