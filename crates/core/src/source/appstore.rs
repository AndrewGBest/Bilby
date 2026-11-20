//! The Mac App Store, through Apple's public search service.
//! <https://performance-partners.apple.com/search-api>
//!
//! * `GET https://itunes.apple.com/search?term=...&entity=macSoftware`
//! * `GET https://itunes.apple.com/lookup?id=...`
//!
//! Only the App Store app can install from the App Store, so the store hands over to
//! it with `macappstore://apps.apple.com/app/id<number>`. Prices and availability are
//! the United States store's; another country's may differ.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Rating, Release, SourceKind};
use crate::platform::{Os, Platform};
use serde::Deserialize;
use std::sync::Arc;

pub const APPLE_API: &str = "https://itunes.apple.com";

pub struct MacAppStore {
    http: Arc<dyn Http>,
    api: String,
}

impl MacAppStore {
    pub fn new(http: Arc<dyn Http>) -> MacAppStore {
        MacAppStore::with_api(http, APPLE_API)
    }

    pub fn with_api(http: Arc<dyn Http>, api: &str) -> MacAppStore {
        MacAppStore { http, api: api.trim_end_matches('/').to_string() }
    }

    fn lookup(&self, key: &str) -> Result<App> {
        if !crate::model::valid_key(SourceKind::MacAppStore, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not an App Store number")));
        }
        let url = format!("{}/lookup?id={key}&entity=macSoftware&country=us", self.api);
        let found: Found = serde_json::from_str(&self.http.get(&url)?)?;
        found
            .results
            .into_iter()
            .find(|app| app.track_id.to_string() == key)
            .ok_or_else(|| Error::NotFound(format!("The Mac App Store has nothing under number {key}.")))
    }
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    results: Vec<App>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct App {
    track_id: u64,
    track_name: String,
    seller_name: Option<String>,
    artist_name: Option<String>,
    formatted_price: Option<String>,
    version: Option<String>,
    average_user_rating: Option<f32>,
    user_rating_count: Option<u32>,
    artwork_url512: Option<String>,
    artwork_url100: Option<String>,
    file_size_bytes: Option<serde_json::Value>,
    current_version_release_date: Option<String>,
    description: Option<String>,
    release_notes: Option<String>,
    #[serde(default)]
    screenshot_urls: Vec<String>,
}

/// Only pictures Apple itself serves.
fn own_picture(url: Option<String>) -> Option<String> {
    url.filter(|u| u.starts_with("https://") && u.split('/').nth(2).is_some_and(|host| host.ends_with(".mzstatic.com")))
}

impl App {
    fn summary(&self) -> AppSummary {
        let mut app =
            AppSummary::new(AppId::new(SourceKind::MacAppStore, self.track_id.to_string()), self.track_name.clone());
        app.author = self.seller_name.clone().or_else(|| self.artist_name.clone()).unwrap_or_default();
        // The first line of the description is the nearest thing to a one-line summary.
        app.summary =
            self.description.as_deref().unwrap_or("").lines().next().unwrap_or("").chars().take(160).collect();
        app.homepage = format!("https://apps.apple.com/app/id{}", self.track_id);
        app.version = self.version.clone();
        app.price = self.formatted_price.clone().filter(|p| !p.is_empty());
        app.icon_url = own_picture(self.artwork_url512.clone().or_else(|| self.artwork_url100.clone()));
        if let (Some(stars), Some(count)) = (self.average_user_rating, self.user_rating_count) {
            if count >= 5 && stars > 0.0 {
                // Apple gives the average and the count, not how the stars are spread.
                app.rating = Some(Rating { stars, count, spread: [0; 5] });
                app.popularity = Some(count as u64);
            }
        }
        app
    }
}

impl Source for MacAppStore {
    fn kind(&self) -> SourceKind {
        SourceKind::MacAppStore
    }

    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::MacOs
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        if query.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/search?term={}&entity=macSoftware&country=us&limit={}",
            self.api,
            url_encode(query.trim()),
            limit.clamp(1, 25)
        );
        let found: Found = serde_json::from_str(&self.http.get(&url)?)?;
        Ok(found.results.iter().take(limit).map(App::summary).collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        Ok(self.lookup(key)?.summary())
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some("Mac App Store")
    }

    fn about(&self, key: &str) -> Result<Option<String>> {
        Ok(self.lookup(key)?.description.filter(|d| !d.trim().is_empty()))
    }

    fn pictures(&self, key: &str) -> Vec<String> {
        let Ok(app) = self.lookup(key) else { return Vec::new() };
        app.screenshot_urls
            .into_iter()
            .filter_map(|url| own_picture(Some(url)))
            .filter(|url| url.ends_with(".jpg") || url.ends_with(".png"))
            .take(crate::screenshots::MAX_PICTURES)
            .collect()
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let app = self.lookup(key)?;
        let size = match &app.file_size_bytes {
            Some(serde_json::Value::String(text)) => text.parse().unwrap_or(0),
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
            _ => 0,
        };
        Ok(vec![Release {
            version: app.version.clone().unwrap_or_else(|| "latest".to_string()),
            published: app.current_version_release_date.clone().unwrap_or_default().chars().take(10).collect(),
            prerelease: false,
            downloads: None,
            notes: app.release_notes.clone().unwrap_or_default(),
            assets: vec![Asset {
                name: "Get on the App Store".to_string(),
                size,
                url: format!("macappstore://apps.apple.com/app/id{}", app.track_id),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::StoreApp,
                    os: TargetOs::Known(Os::MacOs),
                    arch: TargetArch::Unknown,
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }
}
