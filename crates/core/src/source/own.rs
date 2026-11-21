//! Sources the person adds themselves: a list of apps kept on their own server or
//! storage.
//!
//! Anyone can publish apps for the store without asking anybody: write a small JSON
//! file saying what the apps are and where their files live, and put it wherever it
//! can be fetched (a web server, a NAS, an object-storage bucket, a file in a
//! repository). Adding its address in Settings makes those apps searchable and
//! installable like any others, with their own row on the home page.
//!
//! The file (the format is in `docs/OWN-SOURCES.md`):
//!
//! ```json
//! { "name": "My apps",
//!   "apps": [ { "id": "my-tool", "name": "My Tool", "summary": "What it does",
//!               "releases": [ { "version": "1.2.0", "date": "2026-11-01",
//!                               "files": [ { "url": "my-tool-1.2.0-x86_64.AppImage",
//!                                            "sha256": "…" } ] } ] } ] }
//! ```
//!
//! Which file suits the device is worked out from its name, exactly as for a release
//! on a code host. Nobody but its owner vouches for such a source, and the store
//! says so where it is added.

use super::Source;
use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::Platform;
use serde::Deserialize;
use std::sync::Arc;

/// One source the person added: what they call it and where its file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct File {
    #[serde(default)]
    name: String,
    #[serde(default)]
    apps: Vec<App>,
}

#[derive(Debug, Clone, Deserialize)]
struct App {
    id: String,
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    releases: Vec<Version>,
}

#[derive(Debug, Clone, Deserialize)]
struct Version {
    version: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    files: Vec<Download>,
}

#[derive(Debug, Clone, Deserialize)]
struct Download {
    url: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size: u64,
}

/// What a file's address must be to be fetched at all.
pub fn valid_address(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|rest| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty() && !host.contains('@') && url.len() < 600 && url.chars().all(|c| c.is_ascii_graphic())
    })
}

/// A short name for a source in app ids, from what it is called: "My apps" is `my-apps`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').chars().take(32).collect::<String>();
    if out.is_empty() {
        "source".to_string()
    } else {
        out
    }
}

/// An address in the file made whole against the file's own: a bare file name sits
/// beside it, `/x` at its server's root. Only http and https come out.
fn whole(base: &str, given: &str) -> Option<String> {
    let url = if given.starts_with("https://") || given.starts_with("http://") {
        given.to_string()
    } else if given.contains("://") || given.starts_with("//") {
        return None;
    } else if let Some(rooted) = given.strip_prefix('/') {
        let scheme_end = base.find("://")? + 3;
        let host_end = base[scheme_end..].find('/').map(|at| scheme_end + at).unwrap_or(base.len());
        format!("{}/{}", &base[..host_end], rooted)
    } else {
        // Beside the list: everything up to its last slash, or the server's root when
        // the list sits there with no path at all.
        let after_scheme = base.find("://")? + 3;
        match base.rfind('/').filter(|at| *at >= after_scheme) {
            Some(at) => format!("{}{given}", &base[..=at]),
            None => format!("{base}/{given}"),
        }
    };
    valid_address(&url).then_some(url)
}

/// Fetch a source's file and say what it calls itself and how many apps it lists.
/// For adding one: a file that is not such a list is refused here, with the reason.
pub fn look(http: &dyn Http, url: &str) -> Result<(String, usize)> {
    if !valid_address(url) {
        return Err(Error::Unsupported(
            "That is not a web address. It should start with https:// (or http:// for a server on your own network)."
                .into(),
        ));
    }
    let file = read(http, url)?;
    let apps = file.apps.iter().filter(|app| usable(app)).count();
    if apps == 0 {
        return Err(Error::Unsupported(
            "That file lists no apps Bilby can use. Each needs an id, a name and at least one release with a file."
                .into(),
        ));
    }
    let host = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
    let name = if file.name.trim().is_empty() { host.to_string() } else { file.name.trim().chars().take(60).collect() };
    Ok((name, apps))
}

fn read(http: &dyn Http, url: &str) -> Result<File> {
    let text = http.get(url)?;
    serde_json::from_str::<File>(&text).map_err(|_| {
        Error::Parse(
            "That address did not give a list of apps in the form Bilby reads (see docs/OWN-SOURCES.md).".into(),
        )
    })
}

fn usable(app: &App) -> bool {
    let id_ok = !app.id.is_empty()
        && app.id.len() <= 80
        && !app.id.starts_with('.')
        && app.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    id_ok && !app.name.trim().is_empty() && app.releases.iter().any(|r| !r.files.is_empty())
}

/// Every source the person added, as one source of apps. An app's id is
/// `own:<source>/<app>`.
pub struct OwnSources {
    http: Arc<dyn Http>,
    added: Vec<Added>,
}

impl OwnSources {
    pub fn new(http: Arc<dyn Http>, added: Vec<Added>) -> OwnSources {
        OwnSources { http, added }
    }

    fn find(&self, key: &str) -> Result<(&Added, App)> {
        let (source, id) = key
            .split_once('/')
            .ok_or_else(|| Error::NotFound(format!("\"{key}\" is not an app from one of your sources")))?;
        let added = self
            .added
            .iter()
            .find(|added| slug(&added.name) == source)
            .ok_or_else(|| Error::NotFound(format!("The source “{source}” is no longer among yours.")))?;
        let app = read(self.http.as_ref(), &added.url)?
            .apps
            .into_iter()
            .find(|app| app.id == id && usable(app))
            .ok_or_else(|| Error::NotFound(format!("“{}” no longer lists “{id}”.", added.name)))?;
        Ok((added, app))
    }

    fn summary_of(added: &Added, app: &App) -> AppSummary {
        let mut summary =
            AppSummary::new(AppId::new(SourceKind::Own, format!("{}/{}", slug(&added.name), app.id)), app.name.trim());
        summary.summary = app.summary.trim().to_string();
        summary.author = if app.author.trim().is_empty() { added.name.clone() } else { app.author.trim().to_string() };
        summary.homepage = Some(app.homepage.as_str()).filter(|h| h.starts_with("https://")).unwrap_or("").to_string();
        summary.license = app.license.clone();
        summary.icon_url =
            app.icon.as_deref().and_then(|icon| whole(&added.url, icon)).filter(|u| u.starts_with("https://"));
        summary.version = app.releases.first().map(|r| r.version.clone());
        // Which of the person's sources it is from, for its page and its tile.
        summary.origin = Some(added.name.clone());
        summary
    }

    /// The apps of each source that can be read, with the sources that could not.
    fn all(&self) -> (Vec<(&Added, Vec<App>)>, Option<Error>) {
        let mut lists = Vec::new();
        let mut failed = None;
        for added in &self.added {
            match read(self.http.as_ref(), &added.url) {
                Ok(file) => lists.push((added, file.apps.into_iter().filter(usable).collect())),
                Err(e) => failed = Some(Error::Network(format!("“{}” could not be read: {e}", added.name))),
            }
        }
        (lists, failed)
    }
}

impl Source for OwnSources {
    fn kind(&self) -> SourceKind {
        SourceKind::Own
    }

    fn available_on(&self, _platform: &Platform) -> bool {
        !self.added.is_empty()
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let (lists, failed) = self.all();
        let mut hits = Vec::new();
        for (added, apps) in &lists {
            for app in apps {
                let text = format!("{} {} {} {}", app.name, app.id, app.summary, app.author).to_lowercase();
                if !words.is_empty() && words.iter().all(|word| text.contains(word.as_str())) {
                    hits.push(Self::summary_of(added, app));
                }
            }
        }
        hits.truncate(limit.max(1));
        // One source being down does not hide what the others have.
        match (hits.is_empty(), failed) {
            (true, Some(e)) => Err(e),
            _ => Ok(hits),
        }
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        let (added, app) = self.find(key)?;
        Ok(Self::summary_of(added, &app))
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let (added, app) = self.find(key)?;
        Ok(app
            .releases
            .into_iter()
            .map(|release| Release {
                version: release.version,
                published: release.date,
                prerelease: release.prerelease,
                downloads: None,
                notes: release.notes,
                assets: release
                    .files
                    .into_iter()
                    .filter_map(|file| {
                        let url = whole(&added.url, &file.url)?;
                        // What the file is called decides what it is for, so a name
                        // can be given where the address does not end in one.
                        let from_url =
                            url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                        let name = file.name.filter(|n| !n.trim().is_empty()).unwrap_or(from_url);
                        let digest = file
                            .sha256
                            .map(|hex| hex.trim().to_lowercase())
                            .filter(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
                            .map(|hex| format!("sha256:{hex}"));
                        (!name.is_empty()).then_some(Asset { name, size: file.size, url, digest, info: None })
                    })
                    .collect(),
            })
            .collect())
    }

    fn about(&self, key: &str) -> Result<Option<String>> {
        Ok(self.find(key)?.1.description.filter(|text| !text.trim().is_empty()))
    }

    /// Each source is a row on the home page, under its own name.
    fn collections(&self, limit: usize) -> Vec<(String, Vec<AppSummary>)> {
        let (lists, _) = self.all();
        lists
            .into_iter()
            .map(|(added, apps)| {
                (
                    added.name.clone(),
                    apps.iter().take(limit).map(|app| Self::summary_of(added, app)).collect::<Vec<_>>(),
                )
            })
            .filter(|(_, apps)| !apps.is_empty())
            .collect()
    }

    fn origins(&self) -> Vec<String> {
        self.added.iter().map(|added| added.name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A made-up server: each address answers with what the test gave it.
    struct Web(HashMap<&'static str, &'static str>);
    impl Http for Web {
        fn get(&self, url: &str) -> Result<String> {
            self.0.get(url).map(|body| body.to_string()).ok_or_else(|| Error::NotFound(url.to_string()))
        }
        fn download(
            &self,
            _: &str,
            _: &std::path::Path,
            _: &mut dyn FnMut(u64, Option<u64>) -> bool,
        ) -> Result<String> {
            unreachable!()
        }
    }

    const LIST: &str = r#"{
        "name": "Workshop apps",
        "apps": [
          { "id": "gauge", "name": "Gauge", "summary": "Reads the workshop's sensors", "author": "Workshop",
            "icon": "icons/gauge.png", "description": "Longer words about Gauge.",
            "releases": [
              { "version": "2.1.0", "date": "2026-11-01", "notes": "Faster.",
                "files": [
                  { "url": "gauge-2.1.0-x86_64.AppImage", "sha256": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "size": 100 },
                  { "url": "/files/gauge-2.1.0-setup.exe" },
                  { "url": "https://cdn.example.net/gauge-2.1.0.dmg" },
                  { "url": "ftp://old.example/gauge.zip" },
                  { "url": "https://dl.example.org/get?id=7", "name": "gauge_2.1.0_amd64.deb" }
                ] },
              { "version": "2.0.0", "files": [ { "url": "gauge-2.0.0-x86_64.AppImage" } ] }
            ] },
          { "id": "../escape", "name": "Bad id", "releases": [ { "version": "1", "files": [ { "url": "x.zip" } ] } ] },
          { "id": "empty", "name": "No files", "releases": [ { "version": "1", "files": [] } ] }
        ] }"#;
    const AT: &str = "https://example.org/apps/index.json";

    fn sources() -> OwnSources {
        let web = Web(HashMap::from([(AT, LIST)]));
        OwnSources::new(Arc::new(web), vec![Added { name: "Workshop apps".into(), url: AT.into() }])
    }

    #[test]
    fn a_list_on_the_persons_own_server_is_a_source_of_apps() {
        let own = sources();
        assert_eq!(
            look(own.http.as_ref(), AT).unwrap(),
            ("Workshop apps".to_string(), 1),
            "only the usable app counts"
        );
        let hits = own.search("sensors", 10).unwrap();
        assert_eq!(hits.len(), 1);
        let app = &hits[0];
        assert_eq!(app.id.to_string(), "own:workshop-apps/gauge");
        assert_eq!(
            (app.name.as_str(), app.author.as_str(), app.origin.as_deref()),
            ("Gauge", "Workshop", Some("Workshop apps"))
        );
        assert_eq!(app.icon_url.as_deref(), Some("https://example.org/apps/icons/gauge.png"), "beside the list");
        assert_eq!(app.version.as_deref(), Some("2.1.0"));
        assert!(own.search("escape", 10).unwrap().is_empty() && own.search("files", 10).unwrap().is_empty());
        assert_eq!(own.about("workshop-apps/gauge").unwrap().as_deref(), Some("Longer words about Gauge."));

        // Its files: addresses made whole, checksums kept, anything but http left out.
        let releases = own.releases("workshop-apps/gauge").unwrap();
        assert_eq!(releases.len(), 2);
        let files: Vec<(&str, &str)> = releases[0].assets.iter().map(|a| (a.name.as_str(), a.url.as_str())).collect();
        assert_eq!(
            files,
            [
                ("gauge-2.1.0-x86_64.AppImage", "https://example.org/apps/gauge-2.1.0-x86_64.AppImage"),
                ("gauge-2.1.0-setup.exe", "https://example.org/files/gauge-2.1.0-setup.exe"),
                ("gauge-2.1.0.dmg", "https://cdn.example.net/gauge-2.1.0.dmg"),
                ("gauge_2.1.0_amd64.deb", "https://dl.example.org/get?id=7"),
            ]
        );
        assert_eq!(releases[0].assets[0].digest.as_deref(), Some(format!("sha256:{}", "a".repeat(64)).as_str()));
        assert!(releases[0].assets[1].digest.is_none());
        // And a row on the home page under its own name.
        let rows = own.collections(10);
        assert_eq!((rows[0].0.as_str(), rows[0].1.len()), ("Workshop apps", 1));
    }

    #[test]
    fn what_is_not_such_a_list_is_refused_with_the_reason() {
        let web = Web(HashMap::from([
            ("https://example.org/page.html", "<html>not json</html>"),
            ("https://example.org/empty.json", r#"{"name":"Empty","apps":[]}"#),
        ]));
        assert!(look(&web, "ftp://example.org/x").unwrap_err().to_string().contains("not a web address"));
        assert!(look(&web, "https://user@example.org/x").is_err());
        assert!(look(&web, "https://example.org/page.html").unwrap_err().to_string().contains("did not give a list"));
        assert!(look(&web, "https://example.org/empty.json").unwrap_err().to_string().contains("lists no apps"));
        assert!(look(&web, "https://example.org/gone.json").is_err());
        // A source that has gone away does not break the others, and says so when alone.
        let gone = OwnSources::new(
            Arc::new(web),
            vec![Added { name: "Gone".into(), url: "https://example.org/gone.json".into() }],
        );
        assert!(gone.search("anything", 5).unwrap_err().to_string().contains("could not be read"));
        assert!(gone.collections(5).is_empty());
        assert_eq!(slug("My Apps & Tools!"), "my-apps-tools");
        assert_eq!(slug("***"), "source");
    }
}
