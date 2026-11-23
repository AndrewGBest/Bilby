//! Google Play, as far as Google lets a program look.
//!
//! * `GET https://play.google.com/store/apps/details?id=<package>&hl=en&gl=<country>`
//!
//! Google publishes no catalog for other programs to ask. Its `robots.txt` (read on
//! 2026-11-23) lets programs read an app's own page and forbids its search, its lists
//! of reviews and everything behind them, so this source does exactly that much: it
//! reads the page of an app whose name it already has, and never searches Google
//! Play. An app gets here by its link being pasted, or by being one of the names
//! everyone knows (`essentials`), which carry their Google Play names. A search finds
//! those by name, from the list Bilby carries, without asking Google anything.
//!
//! A page describes its app in a block of structured data (schema.org's
//! `SoftwareApplication`): name, maker, picture, rating, how many rated it, price and
//! currency. That block is what is read; the rest of the page is Google's to change.
//! Only Google Play can install from Google Play, so the store hands over to it.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::{AppId, AppSummary, Asset, Rating, Release, SourceKind};
use crate::platform::{Os, Platform};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub const PLAY: &str = "https://play.google.com";

pub struct GooglePlay {
    http: Arc<dyn Http>,
    site: String,
    /// Whose prices to ask for.
    country: String,
}

/// An app's page on Google Play, which Android opens in Google Play itself.
pub fn store_page(package: &str) -> String {
    format!("{PLAY}/store/apps/details?id={package}")
}

/// The block of structured data in a page, if it has one.
fn described(page: &str) -> Option<Described> {
    let mut rest = page;
    while let Some(at) = rest.find("<script type=\"application/ld+json\"") {
        let block = &rest[at..];
        let start = block.find('>')? + 1;
        let end = block.find("</script>")?;
        if let Ok(found) = serde_json::from_str::<Described>(block.get(start..end)?) {
            if found.kind == "SoftwareApplication" {
                return Some(found);
            }
        }
        rest = &block[end..];
    }
    None
}

/// "500M+" beside the word Downloads, as a number: at least this many.
fn installs(page: &str) -> Option<u64> {
    let before = &page[..page.find(">Downloads<")?];
    let tail = &before[before.len().saturating_sub(300)..];
    // The figure is the last piece of text before the label.
    let text = tail.rsplit('>').map(|piece| piece.split('<').next().unwrap_or("").trim()).find(|t| t.ends_with('+'))?;
    let figure = text.trim_end_matches('+').replace(',', "");
    let (digits, times) = match figure.chars().last()? {
        'K' => (&figure[..figure.len() - 1], 1e3),
        'M' => (&figure[..figure.len() - 1], 1e6),
        'B' => (&figure[..figure.len() - 1], 1e9),
        _ => (figure.as_str(), 1.0),
    };
    digits.parse::<f64>().ok().filter(|n| *n > 0.0).map(|n| (n * times) as u64)
}

#[derive(Deserialize)]
struct Described {
    #[serde(rename = "@type", default)]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    author: Option<Named>,
    #[serde(rename = "aggregateRating", default)]
    rating: Option<Rated>,
    #[serde(default)]
    offers: Option<Vec<Offer>>,
}

#[derive(Deserialize)]
struct Named {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Rated {
    #[serde(rename = "ratingValue", default)]
    value: Value,
    #[serde(rename = "ratingCount", default)]
    count: Value,
}

#[derive(Deserialize)]
struct Offer {
    #[serde(default)]
    price: Value,
    #[serde(rename = "priceCurrency", default)]
    currency: Option<String>,
}

/// A number the page may write as a number or as text.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Only pictures Google Play itself serves.
fn own_picture(url: &str) -> Option<String> {
    let host = url.strip_prefix("https://")?.split('/').next()?;
    let own = host == "play-lh.googleusercontent.com";
    (own && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '?')).then(|| format!("{url}=s256"))
}

impl GooglePlay {
    pub fn new(http: Arc<dyn Http>) -> GooglePlay {
        GooglePlay::with_site(http, PLAY)
    }

    pub fn with_site(http: Arc<dyn Http>, site: &str) -> GooglePlay {
        GooglePlay { http, site: site.trim_end_matches('/').to_string(), country: "US".to_string() }
    }

    /// Ask for the prices of this country (two letters, as in `CA`); anything that is
    /// not a country code leaves it as it was.
    pub fn in_country(mut self, code: &str) -> Self {
        if let Some(country) = crate::region::country(code) {
            self.country = country;
        }
        self
    }

    fn page(&self, key: &str) -> Result<String> {
        if !crate::model::valid_key(SourceKind::GooglePlay, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not an Android app's name")));
        }
        let url = format!("{}/store/apps/details?id={key}&hl=en&gl={}", self.site, self.country);
        match self.http.get(&url) {
            Ok(page) => Ok(page),
            Err(Error::NotFound(_)) | Err(Error::Status { .. }) => {
                Err(Error::NotFound(format!("Google Play has nothing called {key}.")))
            }
            Err(e) => Err(e),
        }
    }
}

impl Source for GooglePlay {
    fn kind(&self) -> SourceKind {
        SourceKind::GooglePlay
    }

    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::Android
    }

    /// Never asks Google: its search is closed to programs. The names everyone knows
    /// that are on Google Play are found in the list Bilby carries.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.iter().map(String::len).sum::<usize>() < 2 {
            return Ok(Vec::new());
        }
        Ok(crate::essentials::all()
            .iter()
            .filter(|app| {
                let name = app.name.to_lowercase();
                words.iter().all(|word| name.split(|c: char| !c.is_alphanumeric()).any(|part| part.starts_with(word)))
            })
            .filter_map(|app| {
                let package = app.ids.iter().find_map(|id| id.strip_prefix("play:"))?;
                let mut found = AppSummary::new(AppId::new(SourceKind::GooglePlay, package), app.name.clone());
                found.summary = app.summary.clone();
                found.homepage = store_page(package);
                Some(found)
            })
            .take(limit.max(1))
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        let page = self.page(key)?;
        let described = described(&page)
            .filter(|d| !d.name.trim().is_empty())
            .ok_or_else(|| Error::Parse(format!("Google Play's page for {key} could not be read.")))?;
        let mut app = AppSummary::new(AppId::new(SourceKind::GooglePlay, key), described.name.trim().to_string());
        app.author = described.author.map(|author| author.name.trim().to_string()).unwrap_or_default();
        app.summary = described.description.unwrap_or_default().trim().chars().take(160).collect();
        app.homepage = store_page(key);
        app.icon_url = described.image.as_deref().and_then(own_picture);
        if let Some(rated) = described.rating {
            let (stars, count) = (number(&rated.value).unwrap_or(0.0) as f32, number(&rated.count).unwrap_or(0.0));
            if count >= 5.0 && stars > 0.0 {
                // The average and the count, not how the stars are spread.
                app.rating =
                    Some(Rating { stars: stars.min(5.0), count: count.min(u32::MAX as f64) as u32, spread: [0; 5] });
            }
        }
        // How many have installed it says more about an Android app than how many rated it.
        app.downloads = installs(&page);
        app.popularity = app.downloads.or(app.rating.map(|rating| rating.count as u64));
        app.price = described.offers.as_ref().and_then(|offers| offers.first()).and_then(|offer| {
            let hundredths = (number(&offer.price)? * 100.0).round();
            (hundredths >= 0.0)
                .then(|| crate::region::money(hundredths as u64, offer.currency.as_deref().unwrap_or("USD")))
        });
        Ok(app)
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some("Google Play")
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        // The page is read to know the app is there; a version is not on it.
        self.summary(key)?;
        Ok(vec![Release {
            version: "latest".to_string(),
            published: String::new(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets: vec![Asset {
                name: "Get it on Google Play".to_string(),
                size: 0,
                url: store_page(key),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::StoreApp,
                    os: TargetOs::Known(Os::Android),
                    arch: TargetArch::Unknown,
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_are_read_as_at_least_that_many() {
        let page = |figure: &str| format!("<div><i class='x'>info</i></div><div>{figure}</div><div>Downloads</div>");
        assert_eq!(installs(&page("500M+")), Some(500_000_000));
        assert_eq!(installs(&page("1B+")), Some(1_000_000_000));
        assert_eq!(installs(&page("10K+")), Some(10_000));
        assert_eq!(installs(&page("1,000+")), Some(1000));
        assert_eq!(installs(&page("many")), None);
        assert_eq!(installs("<div>no such label</div>"), None);
    }

    #[test]
    fn only_a_description_of_an_app_is_read_and_only_googles_pictures() {
        let other = r#"<script type="application/ld+json" nonce="a">{"@type":"Organization","name":"Google"}</script>"#;
        let app =
            r#"<script type="application/ld+json" nonce="b">{"@type":"SoftwareApplication","name":"Thing"}</script>"#;
        assert_eq!(described(&format!("{other}{app}")).unwrap().name, "Thing");
        assert!(described(other).is_none() && described("<html></html>").is_none());
        assert!(described(r#"<script type="application/ld+json">not json</script>"#).is_none());
        assert_eq!(
            own_picture("https://play-lh.googleusercontent.com/abc").as_deref(),
            Some("https://play-lh.googleusercontent.com/abc=s256")
        );
        assert!(own_picture("https://play-lh.googleusercontent.com.example.org/abc").is_none());
        assert!(own_picture("https://example.org/play-lh.googleusercontent.com/abc").is_none());
    }
}
