//! Homebrew casks: Mac apps, through Homebrew's public JSON.
//! <https://formulae.brew.sh/docs/api/>
//!
//! A cask says where the app's own publisher serves its download and what its SHA-256
//! is. So the store does not need Homebrew installed: it fetches that file itself,
//! checks it against the checksum and hands it to macOS, exactly as for a file from a
//! release page.
//!
//! There is no search service. The list of casks with how often each was installed in
//! the last thirty days is one file of about a megabyte; names are matched against it,
//! and the details of the few best matches are then fetched one by one.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{Arch, Os, Platform};
use serde::Deserialize;
use std::sync::Arc;

pub const BREW_API: &str = "https://formulae.brew.sh/api";
/// How many casks' details one search fetches.
const DETAILS_PER_SEARCH: usize = 5;

pub struct Homebrew {
    http: Arc<dyn Http>,
    api: String,
    arch: Arch,
}

impl Homebrew {
    pub fn new(http: Arc<dyn Http>, platform: &Platform) -> Homebrew {
        Homebrew::with_api(http, BREW_API, platform)
    }

    pub fn with_api(http: Arc<dyn Http>, api: &str, platform: &Platform) -> Homebrew {
        Homebrew { http, api: api.trim_end_matches('/').to_string(), arch: platform.arch }
    }

    fn cask(&self, key: &str) -> Result<Cask> {
        if !crate::model::valid_key(SourceKind::Homebrew, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a cask name")));
        }
        Ok(serde_json::from_str(&self.http.get(&format!("{}/cask/{key}.json", self.api))?)?)
    }

    /// Cask names with their installs in the last thirty days, most installed first.
    fn installs(&self) -> Result<Vec<(String, u64)>> {
        let list: Analytics =
            serde_json::from_str(&self.http.get(&format!("{}/analytics/cask-install/30d.json", self.api))?)?;
        Ok(list
            .items
            .into_iter()
            .map(|i| (i.cask, i.count.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)))
            .collect())
    }
}

#[derive(Deserialize)]
struct Analytics {
    #[serde(default)]
    items: Vec<Installs>,
}

#[derive(Deserialize)]
struct Installs {
    cask: String,
    /// With thousands separators: "80,981".
    #[serde(default)]
    count: String,
}

#[derive(Deserialize)]
struct Cask {
    token: String,
    #[serde(default)]
    name: Vec<String>,
    desc: Option<String>,
    homepage: Option<String>,
    url: Option<String>,
    version: Option<String>,
    sha256: Option<String>,
    #[serde(default)]
    deprecated: bool,
    #[serde(default)]
    disabled: bool,
    /// Other downloads by macOS version and processor, e.g. `sequoia`, `arm64_sonoma`.
    #[serde(default)]
    variations: serde_json::Map<String, serde_json::Value>,
}

impl Cask {
    fn summary(&self) -> AppSummary {
        let name = self.name.first().cloned().unwrap_or_else(|| self.token.clone());
        let mut app = AppSummary::new(AppId::new(SourceKind::Homebrew, self.token.clone()), name);
        app.summary = self.desc.clone().unwrap_or_default();
        app.homepage = self
            .homepage
            .clone()
            .filter(|u| u.starts_with("https://"))
            .unwrap_or_else(|| format!("https://formulae.brew.sh/cask/{}", self.token));
        app.version = self.version.clone();
        // A cask names no author; the site it comes from says whose app it is.
        app.author = app
            .homepage
            .trim_start_matches("https://")
            .trim_start_matches("www.")
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        if self.disabled || self.deprecated {
            app.warning = Some("Homebrew marks this app as no longer maintained.".to_string());
        }
        app
    }

    /// The download for this processor and its checksum. The main entry is for Apple
    /// silicon where the two differ; Intel Macs are listed under a macOS version name.
    fn download(&self, arch: Arch) -> Option<(String, Option<String>)> {
        let main = self.url.clone().map(|url| (url, self.sha256.clone()));
        if arch != Arch::X86_64 {
            return main;
        }
        let intel = self.variations.iter().find(|(name, v)| !name.starts_with("arm64") && v.get("url").is_some());
        match intel {
            Some((_, v)) => Some((
                v["url"].as_str()?.to_string(),
                v.get("sha256").and_then(|s| s.as_str()).map(str::to_string).or_else(|| self.sha256.clone()),
            )),
            None => main,
        }
    }
}

/// What kind of file an address ends in, as far as macOS is concerned.
fn format_of(file: &str) -> Option<Format> {
    let lower = file.to_lowercase();
    if lower.ends_with(".dmg") {
        Some(Format::Dmg)
    } else if lower.ends_with(".pkg") {
        Some(Format::MacPkg)
    } else if [".zip", ".tar.gz", ".tgz", ".tar.xz", ".tar.bz2"].iter().any(|ext| lower.ends_with(ext)) {
        Some(Format::Archive)
    } else {
        None
    }
}

impl Source for Homebrew {
    fn kind(&self) -> SourceKind {
        SourceKind::Homebrew
    }

    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::MacOs
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.iter().map(String::len).sum::<usize>() < 2 {
            return Ok(Vec::new());
        }
        // The list is already most-installed first, so the first matches are the best known.
        let matches: Vec<(String, u64)> = self
            .installs()?
            .into_iter()
            .filter(|(cask, _)| words.iter().all(|w| cask.contains(w.as_str())))
            .filter(|(cask, _)| crate::model::valid_key(SourceKind::Homebrew, cask))
            .take(limit.min(DETAILS_PER_SEARCH))
            .collect();
        Ok(matches
            .into_iter()
            .filter_map(|(token, count)| {
                // A cask whose details cannot be fetched is left out, not an error.
                let mut app = self.cask(&token).ok()?.summary();
                app.popularity = Some(count);
                Some(app)
            })
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        Ok(self.cask(key)?.summary())
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let cask = self.cask(key)?;
        let Some((url, sha256)) = cask.download(self.arch).filter(|(url, _)| url.starts_with("https://")) else {
            return Ok(Vec::new());
        };
        let file = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("").replace("%20", " ");
        let Some(format) = format_of(&file) else { return Ok(Vec::new()) };
        // "no_check" means the publisher replaces the file in place, so there is nothing to check against.
        let digest = sha256.filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()));
        Ok(vec![Release {
            version: cask.version.clone().unwrap_or_else(|| "latest".to_string()),
            published: String::new(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets: vec![Asset {
                name: file,
                size: 0,
                url,
                digest: digest.map(|hex| format!("sha256:{hex}")),
                info: Some(AssetInfo {
                    format,
                    os: TargetOs::Known(Os::MacOs),
                    // The cask already chose the download for this processor.
                    arch: TargetArch::One(self.arch),
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }
}
