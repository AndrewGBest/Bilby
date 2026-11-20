//! F-Droid, the catalog of free Android apps.
//!
//! * Search: `GET https://search.f-droid.org/api/search_apps?q=...`
//! * Versions: `GET https://f-droid.org/api/v1/packages/{id}`
//! * The file itself: `https://f-droid.org/repo/{id}_{versionCode}.apk`
//!
//! F-Droid builds and signs every app itself, so each version is one APK.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{Os, Platform};
use serde::Deserialize;
use std::sync::Arc;

pub const FDROID_SEARCH: &str = "https://search.f-droid.org/api";
pub const FDROID_SITE: &str = "https://f-droid.org";
/// How many past versions are offered.
const VERSIONS: usize = 3;

pub struct FDroid {
    http: Arc<dyn Http>,
    search: String,
    site: String,
}

impl FDroid {
    pub fn new(http: Arc<dyn Http>) -> FDroid {
        FDroid::with_api(http, FDROID_SEARCH, FDROID_SITE)
    }

    pub fn with_api(http: Arc<dyn Http>, search: &str, site: &str) -> FDroid {
        FDroid { http, search: search.trim_end_matches('/').to_string(), site: site.trim_end_matches('/').to_string() }
    }

    fn find(&self, query: &str) -> Result<Vec<AppSummary>> {
        let url = format!("{}/search_apps?q={}", self.search, url_encode(query.trim()));
        let found: Found = serde_json::from_str(&self.http.get(&url)?)?;
        Ok(found
            .apps
            .into_iter()
            .filter_map(|hit| {
                // The app's id is the end of its page address.
                let id = hit.url.trim_end_matches('/').rsplit('/').next()?.to_string();
                if !crate::model::valid_key(SourceKind::FDroid, &id) {
                    return None;
                }
                let mut app = AppSummary::new(AppId::new(SourceKind::FDroid, id.clone()), hit.name);
                app.summary = hit.summary.unwrap_or_default();
                app.homepage = format!("https://f-droid.org/packages/{id}");
                app.icon_url = hit.icon.filter(|u| u.starts_with("https://") && u.ends_with(".png"));
                Some(app)
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    apps: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    name: String,
    summary: Option<String>,
    icon: Option<String>,
    url: String,
}

#[derive(Deserialize)]
struct Package {
    #[serde(rename = "suggestedVersionCode")]
    suggested: Option<u64>,
    #[serde(default)]
    packages: Vec<Version>,
}

#[derive(Deserialize)]
struct Version {
    #[serde(rename = "versionName")]
    name: String,
    #[serde(rename = "versionCode")]
    code: u64,
}

fn check(key: &str) -> Result<()> {
    if crate::model::valid_key(SourceKind::FDroid, key) {
        Ok(())
    } else {
        Err(Error::NotFound(format!("\"{key}\" is not an Android package name")))
    }
}

impl Source for FDroid {
    fn kind(&self) -> SourceKind {
        SourceKind::FDroid
    }

    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::Android
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.find(query)?.into_iter().take(limit).collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        check(key)?;
        // There is no "one app" address that answers with its name; searching for the
        // id finds it. If it does not, the id itself has to do.
        let found = self.find(key).unwrap_or_default().into_iter().find(|app| app.id.key == key);
        Ok(found.unwrap_or_else(|| {
            let mut app = AppSummary::new(AppId::new(SourceKind::FDroid, key), key.rsplit('.').next().unwrap_or(key));
            app.homepage = format!("https://f-droid.org/packages/{key}");
            app
        }))
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        check(key)?;
        let package: Package = serde_json::from_str(&self.http.get(&format!("{}/api/v1/packages/{key}", self.site))?)?;
        let mut versions = package.packages;
        versions.dedup_by_key(|v| v.code);
        // The version F-Droid suggests first, then newer to older.
        versions.sort_by_key(|v| (Some(v.code) != package.suggested, std::cmp::Reverse(v.code)));
        Ok(versions
            .into_iter()
            .take(VERSIONS)
            .map(|v| Release {
                version: v.name,
                published: String::new(),
                prerelease: package.suggested.is_some_and(|suggested| v.code > suggested),
                downloads: None,
                notes: String::new(),
                assets: vec![Asset {
                    name: format!("{key}_{}.apk", v.code),
                    size: 0,
                    url: format!("{}/repo/{key}_{}.apk", self.site, v.code),
                    digest: None,
                    info: Some(AssetInfo {
                        format: Format::Apk,
                        os: TargetOs::Known(Os::Android),
                        arch: TargetArch::Universal,
                        libc: None,
                        tv: false,
                        debug: false,
                    }),
                }],
            })
            .collect())
    }
}
