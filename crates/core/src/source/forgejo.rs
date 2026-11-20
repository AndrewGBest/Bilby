//! Codeberg, through the Forgejo REST API (Gitea's API has the same shape).
//!
//! Endpoints used (documented at <https://codeberg.org/api/swagger>):
//! * `GET /api/v1/repos/search?q=...&limit=N`
//! * `GET /api/v1/repos/{owner}/{repo}`
//! * `GET /api/v1/repos/{owner}/{repo}/releases?limit=N`

use super::Source;
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::Platform;
use serde::Deserialize;
use std::sync::Arc;

pub const CODEBERG_API: &str = "https://codeberg.org/api/v1";

/// Same reasoning as for GitHub: some projects publish each platform as its own release.
const RELEASES_PER_APP: usize = 6;

pub struct Forgejo {
    http: Arc<dyn Http>,
    api: String,
}

impl Forgejo {
    pub fn codeberg(http: Arc<dyn Http>) -> Forgejo {
        Forgejo::with_api(http, CODEBERG_API)
    }

    /// Talk to a different instance's `/api/v1` (a test server).
    pub fn with_api(http: Arc<dyn Http>, api: &str) -> Forgejo {
        Forgejo { http, api: api.trim_end_matches('/').to_string() }
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<Repo>,
}

#[derive(Deserialize)]
struct Repo {
    name: String,
    full_name: String,
    owner: Owner,
    #[serde(default)]
    description: String,
    html_url: String,
    #[serde(default)]
    stars_count: u64,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
}

#[derive(Deserialize)]
struct Owner {
    login: String,
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
}

impl From<Repo> for AppSummary {
    fn from(r: Repo) -> AppSummary {
        AppSummary {
            id: AppId::new(SourceKind::Codeberg, r.full_name),
            name: r.name,
            author: r.owner.login,
            summary: r.description,
            popularity: Some(r.stars_count),
            homepage: r.html_url,
            license: None,
            category: None,
            version: None,
            install_label: None,
            warning: None,
            origin: None,
            installed_version: None,
            rating: None,
            icon_url: None,
        }
    }
}

fn check(key: &str) -> Result<()> {
    if crate::model::valid_key(SourceKind::Codeberg, key) {
        Ok(())
    } else {
        Err(Error::NotFound(format!("\"{key}\" is not an owner/repository name")))
    }
}

impl Source for Forgejo {
    fn kind(&self) -> SourceKind {
        SourceKind::Codeberg
    }

    fn available_on(&self, _platform: &Platform) -> bool {
        true
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let url = format!(
            "{}/repos/search?q={}&limit={}&sort=stars&order=desc",
            self.api,
            url_encode(query),
            limit.clamp(1, 50)
        );
        let resp: SearchResponse = serde_json::from_str(&self.http.get(&url)?)?;
        // As on GitHub: forks and archived repositories rarely carry the releases people want.
        Ok(resp.data.into_iter().filter(|r| !r.fork && !r.archived).map(AppSummary::from).collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        check(key)?;
        let repo: Repo = serde_json::from_str(&self.http.get(&format!("{}/repos/{key}", self.api))?)?;
        Ok(repo.into())
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        check(key)?;
        let url = format!("{}/repos/{key}/releases?limit={RELEASES_PER_APP}", self.api);
        let releases: Vec<ApiRelease> = serde_json::from_str(&self.http.get(&url)?)?;
        Ok(releases
            .into_iter()
            .filter(|r| !r.draft)
            .map(|r| Release {
                version: r.tag_name,
                published: r.published_at.unwrap_or_default().chars().take(10).collect(),
                prerelease: r.prerelease,
                notes: r.body.unwrap_or_default(),
                assets: r
                    .assets
                    .into_iter()
                    .map(|a| Asset {
                        name: a.name,
                        size: a.size,
                        url: a.browser_download_url,
                        digest: None,
                        info: None,
                    })
                    .collect(),
            })
            .collect())
    }
}
