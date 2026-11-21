//! GitLab.com projects, through GitLab's REST API (version 4).
//! <https://docs.gitlab.com/api/projects/> and <https://docs.gitlab.com/api/releases/>
//!
//! A GitLab release lists its downloads as named links rather than uploaded files, so
//! a file's name is taken from the end of its address when that looks like a file, and
//! from the link's name otherwise. No size is published.

use super::Source;
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::Platform;
use serde::Deserialize;
use std::sync::Arc;

pub const GITLAB_API: &str = "https://gitlab.com/api/v4";
const RELEASES_PER_APP: usize = 6;

pub struct GitLab {
    http: Arc<dyn Http>,
    api: String,
}

impl GitLab {
    pub fn new(http: Arc<dyn Http>) -> GitLab {
        GitLab::with_api(http, GITLAB_API)
    }

    pub fn with_api(http: Arc<dyn Http>, api: &str) -> GitLab {
        GitLab { http, api: api.trim_end_matches('/').to_string() }
    }
}

#[derive(Deserialize)]
struct Project {
    name: String,
    path_with_namespace: String,
    description: Option<String>,
    #[serde(default)]
    star_count: u64,
    web_url: Option<String>,
    avatar_url: Option<String>,
    archived: Option<bool>,
    forked_from_project: Option<serde_json::Value>,
    namespace: Option<Namespace>,
}

#[derive(Deserialize)]
struct Namespace {
    name: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    released_at: Option<String>,
    description: Option<String>,
    #[serde(default)]
    upcoming_release: bool,
    assets: Option<Assets>,
}

#[derive(Deserialize)]
struct Assets {
    #[serde(default)]
    links: Vec<Link>,
}

#[derive(Deserialize)]
struct Link {
    name: String,
    url: String,
    direct_asset_url: Option<String>,
}

impl From<Project> for AppSummary {
    fn from(p: Project) -> AppSummary {
        let mut app = AppSummary::new(AppId::new(SourceKind::GitLab, p.path_with_namespace.clone()), p.name);
        app.author = p
            .namespace
            .and_then(|n| n.name)
            .unwrap_or_else(|| p.path_with_namespace.split('/').next().unwrap_or("").to_string());
        app.summary = p.description.unwrap_or_default();
        app.popularity = Some(p.star_count);
        app.homepage = p.web_url.unwrap_or_else(|| format!("https://gitlab.com/{}", p.path_with_namespace));
        // Only pictures GitLab itself serves.
        app.icon_url = p.avatar_url.filter(|u| u.starts_with("https://gitlab.com/"));
        if p.archived == Some(true) {
            app.warning = Some("Archived on GitLab: no longer maintained.".to_string());
        }
        app
    }
}

fn check(key: &str) -> Result<()> {
    if crate::model::valid_key(SourceKind::GitLab, key) {
        Ok(())
    } else {
        Err(Error::NotFound(format!("\"{key}\" is not a GitLab project path")))
    }
}

/// The name to classify a download by: the end of its address when that is a file name.
fn file_name(link: &Link) -> String {
    let url = link.direct_asset_url.as_deref().unwrap_or(&link.url);
    let last = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    let looks_like_a_file = last.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    if looks_like_a_file {
        last.to_string()
    } else {
        link.name.clone()
    }
}

impl Source for GitLab {
    fn kind(&self) -> SourceKind {
        SourceKind::GitLab
    }

    fn available_on(&self, _platform: &Platform) -> bool {
        true
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        // GitLab refuses searches shorter than three characters.
        if query.trim().chars().count() < 3 {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/projects?search={}&order_by=star_count&sort=desc&visibility=public&per_page={}",
            self.api,
            url_encode(query.trim()),
            limit.clamp(1, 50)
        );
        // GitLab limits how often an anonymous client may ask. Being turned away is
        // not something to show the person searching; GitLab's apps are just absent.
        let body = match self.http.get(&url) {
            Err(Error::Status { code: 429 | 403, .. }) | Err(Error::RateLimited { .. }) => return Ok(Vec::new()),
            other => other?,
        };
        let projects: Vec<Project> = serde_json::from_str(&body)?;
        Ok(projects
            .into_iter()
            .filter(|p| p.forked_from_project.is_none() && p.archived != Some(true))
            .filter(|p| crate::model::valid_key(SourceKind::GitLab, &p.path_with_namespace))
            .map(AppSummary::from)
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        check(key)?;
        let project: Project =
            serde_json::from_str(&self.http.get(&format!("{}/projects/{}", self.api, url_encode(key)))?)?;
        Ok(project.into())
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        check(key)?;
        let url = format!("{}/projects/{}/releases?per_page={RELEASES_PER_APP}", self.api, url_encode(key));
        let body = match self.http.get(&url) {
            // Turned away for asking too often: nothing to offer from here for now.
            Err(Error::Status { code: 429 | 403, .. }) | Err(Error::RateLimited { .. }) => return Ok(Vec::new()),
            other => other?,
        };
        let releases: Vec<ApiRelease> = serde_json::from_str(&body)?;
        Ok(releases
            .into_iter()
            .filter(|r| !r.upcoming_release)
            .map(|r| Release {
                prerelease: {
                    let tag = r.tag_name.to_lowercase();
                    ["alpha", "beta", "-rc", "nightly", "preview"].iter().any(|word| tag.contains(word))
                },
                version: r.tag_name,
                published: r.released_at.unwrap_or_default().chars().take(10).collect(),
                downloads: None,
                notes: r.description.unwrap_or_default(),
                assets: r
                    .assets
                    .map(|a| a.links)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|l| l.url.starts_with("https://"))
                    .map(|l| Asset {
                        name: file_name(&l),
                        size: 0,
                        url: l
                            .direct_asset_url
                            .clone()
                            .filter(|u| u.starts_with("https://"))
                            .unwrap_or_else(|| l.url.clone()),
                        digest: None,
                        info: None,
                    })
                    .collect(),
            })
            .collect())
    }
}
