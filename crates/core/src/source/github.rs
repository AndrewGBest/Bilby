//! GitHub Releases, through the public REST API.
//!
//! Endpoints used (all documented at <https://docs.github.com/en/rest>):
//! * `GET /search/repositories?q=...`
//! * `GET /repos/{owner}/{repo}`
//! * `GET /repos/{owner}/{repo}/releases?per_page=N`
//! * `GET /repos/{owner}/{repo}/readme` (for its `download_url`)

use super::Source;
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::Platform;
use serde::Deserialize;
use std::sync::Arc;

/// How many releases to look back through for one that has a file for this machine.
/// Some projects publish each platform as its own release.
const RELEASES_PER_APP: usize = 6;

const RAW_HOST: &str = "https://raw.githubusercontent.com/";

pub struct GitHub {
    http: Arc<dyn Http>,
    api: String,
}

impl GitHub {
    pub fn new(http: Arc<dyn Http>) -> GitHub {
        GitHub::with_api(http, crate::http::GITHUB_API)
    }

    /// Talk to a different API host (GitHub Enterprise, a test server).
    pub fn with_api(http: Arc<dyn Http>, api: &str) -> GitHub {
        GitHub { http, api: api.trim_end_matches('/').to_string() }
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    items: Vec<Repo>,
}

#[derive(Deserialize)]
struct Repo {
    name: String,
    full_name: String,
    owner: Owner,
    description: Option<String>,
    html_url: String,
    #[serde(default)]
    stargazers_count: u64,
    license: Option<License>,
    /// The owner has marked the project as no longer maintained.
    #[serde(default)]
    archived: bool,
}

#[derive(Deserialize)]
struct Owner {
    login: String,
}

#[derive(Deserialize)]
struct License {
    spdx_id: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
    /// `sha256:...`; present on assets uploaded since mid-2025.
    digest: Option<String>,
    /// How many times the file was downloaded.
    download_count: Option<u64>,
}

impl From<Repo> for AppSummary {
    fn from(r: Repo) -> AppSummary {
        AppSummary {
            id: AppId::new(SourceKind::GitHub, r.full_name),
            name: r.name,
            author: r.owner.login,
            summary: r.description.unwrap_or_default(),
            popularity: Some(r.stargazers_count),
            homepage: r.html_url,
            license: r.license.and_then(|l| l.spdx_id).filter(|id| id != "NOASSERTION"),
            category: None,
            version: None,
            install_label: None,
            warning: r.archived.then(|| "Its owner has archived this project: it is no longer maintained.".to_string()),
            origin: None,
            installed_version: None,
            rating: None,
            icon_url: None,
            downloads: None,
            verified: false,
            momentum: None,
            price: None,
        }
    }
}

#[derive(Deserialize)]
struct Readme {
    download_url: Option<String>,
}

fn valid_key(key: &str) -> bool {
    crate::model::valid_key(SourceKind::GitHub, key)
}

impl Source for GitHub {
    fn kind(&self) -> SourceKind {
        SourceKind::GitHub
    }

    fn available_on(&self, _platform: &Platform) -> bool {
        true
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        // Forks and archived repos practically never carry the releases people want.
        let q = format!("{query} in:name,description fork:false archived:false");
        let url = format!("{}/search/repositories?q={}&per_page={}", self.api, url_encode(&q), limit.clamp(1, 100));
        let resp: SearchResponse = serde_json::from_str(&self.http.get(&url)?)?;
        Ok(resp.items.into_iter().map(AppSummary::from).collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        if !valid_key(key) {
            return Err(Error::NotFound(format!("\"{key}\" is not an owner/repository name")));
        }
        let repo: Repo = serde_json::from_str(&self.http.get(&format!("{}/repos/{key}", self.api))?)?;
        Ok(repo.into())
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        if !valid_key(key) {
            return Err(Error::NotFound(format!("\"{key}\" is not an owner/repository name")));
        }
        let url = format!("{}/repos/{key}/releases?per_page={RELEASES_PER_APP}", self.api);
        let releases: Vec<ApiRelease> = serde_json::from_str(&self.http.get(&url)?)?;
        Ok(releases
            .into_iter()
            .filter(|r| !r.draft)
            .map(|r| Release {
                version: r.tag_name,
                published: r.published_at.unwrap_or_default().chars().take(10).collect(),
                prerelease: r.prerelease,
                downloads: super::counted(r.assets.iter().map(|a| a.download_count)),
                notes: r.body.unwrap_or_default(),
                assets: r
                    .assets
                    .into_iter()
                    .map(|a| Asset {
                        name: a.name,
                        size: a.size,
                        url: a.browser_download_url,
                        digest: a.digest,
                        info: None,
                    })
                    .collect(),
            })
            .collect())
    }

    fn about(&self, key: &str) -> Result<Option<String>> {
        if !valid_key(key) {
            return Ok(None);
        }
        let readme: Readme = match self.http.get(&format!("{}/repos/{key}/readme", self.api)) {
            Ok(body) => serde_json::from_str(&body)?,
            // Plenty of projects have no README; that is not a failure.
            Err(Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        // The file itself comes from GitHub's raw host and nowhere else, whatever the
        // response says.
        match readme.download_url.filter(|u| u.starts_with(RAW_HOST)) {
            Some(url) => Ok(Some(self.http.get(&url)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_validated_before_they_reach_a_url() {
        assert!(valid_key("sharkdp/bat"));
        assert!(valid_key("obsidianmd/obsidian-releases"));
        assert!(!valid_key("bat"));
        assert!(!valid_key("a/b/c"));
        assert!(!valid_key("a/b?per_page=100"));
        assert!(!valid_key("../x"));
    }
}
