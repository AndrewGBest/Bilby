//! The Microsoft Store: Windows apps, and the PC games the Xbox app sells from the
//! same catalog.
//!
//! * `GET https://storeedgefd.dsx.mp.microsoft.com/v9.0/search?market=US&locale=en-US&deviceFamily=Windows.Desktop&query=...`
//! * `GET https://storeedgefd.dsx.mp.microsoft.com/v9.0/products/<id>?market=US&locale=en-US&deviceFamily=Windows.Desktop`
//!
//! * `GET https://storeedgefd.dsx.mp.microsoft.com/v9.0/ratings/product/<id>?market=US&locale=en-US&deviceFamily=Windows.Desktop`
//!
//! This is the service the Store app itself and winget's `msstore` source ask. It
//! needs no account, but Microsoft does not document it for others, so its answers
//! are read loosely: a field that is missing or has changed its type costs that one
//! fact, not the app. What is read here was checked against the live service on
//! 2026-11-23.
//!
//! Only the Store app can install from the Store, so the store hands over to it with
//! `ms-windows-store://pdp/?ProductId=<id>`. Prices and availability are those of the
//! country the device is set to (see `region`).
//!
//! It answers from anywhere, not only from Windows: the catalog can be searched on
//! any device, which is what running Windows software elsewhere (Wine, Proton) would
//! start from.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Rating, Release, SourceKind};
use crate::platform::{Os, Platform};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub const STORE_API: &str = "https://storeedgefd.dsx.mp.microsoft.com/v9.0";

pub struct MicrosoftStore {
    http: Arc<dyn Http>,
    api: String,
    /// Whose prices to ask for.
    country: String,
}

/// The link that opens the Store app on one product.
pub fn store_link(id: &str) -> String {
    format!("ms-windows-store://pdp/?ProductId={id}")
}

impl MicrosoftStore {
    pub fn new(http: Arc<dyn Http>) -> MicrosoftStore {
        MicrosoftStore::with_api(http, STORE_API)
    }

    pub fn with_api(http: Arc<dyn Http>, api: &str) -> MicrosoftStore {
        MicrosoftStore { http, api: api.trim_end_matches('/').to_string(), country: "US".to_string() }
    }

    /// Ask for the prices of this country (two letters, as in `CA`); anything that is
    /// not a country code leaves it as it was.
    pub fn in_country(mut self, code: &str) -> Self {
        if let Some(country) = crate::region::country(code) {
            self.country = country;
        }
        self
    }

    /// The part of every question that says whose store is meant. The words stay
    /// English; the market decides the prices and what is sold.
    fn market(&self) -> String {
        format!("market={}&locale=en-US&deviceFamily=Windows.Desktop", self.country)
    }

    fn lookup(&self, key: &str) -> Result<Product> {
        if !crate::model::valid_key(SourceKind::MicrosoftStore, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a Microsoft Store product id")));
        }
        let url = format!("{}/products/{key}?{}", self.api, self.market());
        let nothing = || Error::NotFound(format!("The Microsoft Store has nothing under {key}."));
        let text = match self.http.get(&url) {
            Ok(text) => text,
            Err(Error::NotFound(_)) | Err(Error::Status { .. }) => return Err(nothing()),
            Err(e) => return Err(e),
        };
        let answer: Answer<Product> = serde_json::from_str(&text)?;
        answer.payload.filter(|product| product.product_id.eq_ignore_ascii_case(key)).ok_or_else(nothing)
    }
}

/// Everything the service sends is wrapped like this.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Answer<T> {
    payload: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Found {
    #[serde(default)]
    search_results: Vec<Product>,
}

/// A product, as a search result or as its own page: the two name some things
/// differently, and both are read here.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Product {
    product_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    publisher_name: Option<String>,
    #[serde(default)]
    display_price: Option<String>,
    #[serde(default)]
    strikethrough_price: Option<String>,
    /// "Xbox Game Pass", where the price shown is "Included".
    #[serde(default)]
    plaintext_pass_name: Option<String>,
    #[serde(default)]
    average_rating: Option<Value>,
    /// A number on a product's page; in a search, text such as "18K".
    #[serde(default)]
    rating_count: Option<Value>,
    #[serde(default)]
    ratings_count: Option<Value>,
    #[serde(default)]
    images: Option<Vec<Image>>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    long_description: Option<String>,
    #[serde(default)]
    categories: Option<Vec<String>>,
    #[serde(default)]
    last_update_date_utc: Option<String>,
    #[serde(default)]
    last_modified_date: Option<String>,
    #[serde(default)]
    approximate_size_in_bytes: Option<Value>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Image {
    #[serde(default)]
    image_type: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

/// Only pictures Microsoft's store itself serves.
fn own_picture(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host == "store-images.s-microsoft.com" || host == "store-images.microsoft.com")
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '?')
}

/// A count as the service writes it: 34828, "34828", "18K", "1.2M".
fn count_of(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Value::String(text) => {
            let text = text.trim().replace(',', "");
            let (digits, times) = match text.chars().last()? {
                'K' | 'k' => (&text[..text.len() - 1], 1_000.0),
                'M' | 'm' => (&text[..text.len() - 1], 1_000_000.0),
                _ => (text.as_str(), 1.0),
            };
            digits.trim().parse::<f64>().ok().filter(|n| *n >= 0.0).map(|n| (n * times) as u64)
        }
        _ => None,
    }
}

impl Product {
    /// An add-on to a game is not something to install by itself.
    fn stands_alone(&self) -> bool {
        !self.categories.as_deref().unwrap_or(&[]).iter().any(|category| category.eq_ignore_ascii_case("Add-on"))
            && self.title.as_deref().is_some_and(|title| !title.trim().is_empty())
            && crate::model::valid_key(SourceKind::MicrosoftStore, &self.product_id.to_ascii_uppercase())
    }

    /// Its square logo, at a size that suits a tile: the smallest of 256 pixels or
    /// more, else the largest there is. A game may have only box art, which is square too.
    fn icon(&self) -> Option<String> {
        let square = |kind: &str| {
            let mut found: Vec<&Image> = self
                .images
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .filter(|image| image.image_type.eq_ignore_ascii_case(kind))
                .filter(|image| image.width > 0 && image.width == image.height && own_picture(&image.url))
                .collect();
            found.sort_by_key(|image| {
                (image.width < 256, if image.width < 256 { u32::MAX - image.width } else { image.width })
            });
            found.first().map(|image| format!("{}?w=256&h=256", image.url))
        };
        square("logo").or_else(|| square("tile")).or_else(|| square("BoxArt"))
    }

    fn price(&self) -> Option<String> {
        let shown = self.display_price.as_deref().map(str::trim).filter(|price| !price.is_empty())?;
        if shown.eq_ignore_ascii_case("Included") {
            // Included with a subscription: say which, or the price without it.
            let pass = self.plaintext_pass_name.as_deref().map(str::trim).filter(|name| !name.is_empty());
            let full = self
                .strikethrough_price
                .as_deref()
                .map(|price| price.trim_matches(|c: char| c.is_whitespace() || c == '\u{200b}'))
                .filter(|price| !price.is_empty());
            return match (pass, full) {
                (Some(pass), _) => Some(format!("Included with {pass}")),
                (None, Some(full)) => Some(full.to_string()),
                (None, None) => None,
            };
        }
        Some(shown.to_string())
    }

    fn about(&self) -> Option<&str> {
        self.description.as_deref().or(self.long_description.as_deref()).filter(|text| !text.trim().is_empty())
    }

    fn summary(&self) -> AppSummary {
        let id = self.product_id.to_ascii_uppercase();
        let mut app = AppSummary::new(
            AppId::new(SourceKind::MicrosoftStore, id.clone()),
            self.title.clone().unwrap_or_default().trim().to_string(),
        );
        app.author = self.publisher_name.clone().unwrap_or_default();
        // The first line of the description is the nearest thing to a one-line summary.
        app.summary = self.about().unwrap_or("").trim().lines().next().unwrap_or("").chars().take(160).collect();
        app.homepage = format!("https://apps.microsoft.com/detail/{id}");
        app.version = self.version.clone().filter(|version| !version.trim().is_empty());
        app.price = self.price();
        app.icon_url = self.icon();
        app.category = self.categories.as_ref().and_then(|all| all.first().cloned());
        let stars = self.average_rating.as_ref().and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let count = self.rating_count.as_ref().or(self.ratings_count.as_ref()).and_then(count_of).unwrap_or(0);
        if count >= 5 && stars > 0.0 {
            // The average and the count, not how the stars are spread.
            app.rating = Some(Rating { stars, count: count.min(u32::MAX as u64) as u32, spread: [0; 5] });
            app.popularity = Some(count);
        }
        app
    }
}

impl Source for MicrosoftStore {
    fn kind(&self) -> SourceKind {
        SourceKind::MicrosoftStore
    }

    /// Where the Store app is. Elsewhere the catalog can still be looked through (see
    /// `Catalog::searched`), and what it finds is marked as not for this device.
    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::Windows
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        if query.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        // Apps and games together: asked for apps alone, the service ranks oddly.
        let url = format!("{}/search?{}&query={}", self.api, self.market(), url_encode(query.trim()));
        let answer: Answer<Found> = serde_json::from_str(&self.http.get(&url)?)?;
        let found = answer.payload.map(|found| found.search_results).unwrap_or_default();
        Ok(found.iter().filter(|product| product.stands_alone()).take(limit.max(1)).map(Product::summary).collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        Ok(self.lookup(key)?.summary())
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some("Microsoft Store")
    }

    fn about(&self, key: &str) -> Result<Option<String>> {
        Ok(self.lookup(key)?.about().map(str::to_string))
    }

    fn pictures(&self, key: &str) -> Vec<String> {
        let Ok(product) = self.lookup(key) else { return Vec::new() };
        product
            .images
            .unwrap_or_default()
            .into_iter()
            // Pictures of the app in use, the wide ones: a phone's tall one does not suit the page.
            .filter(|image| image.image_type.eq_ignore_ascii_case("screenshot") && image.width > image.height)
            .filter(|image| own_picture(&image.url))
            .map(|image| format!("{}?w=1280&format=jpg", image.url))
            .take(crate::screenshots::MAX_PICTURES)
            .collect()
    }

    fn reviews(&self, key: &str) -> Vec<crate::ratings::Review> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct List {
            #[serde(default)]
            reviews: Vec<Written>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Written {
            #[serde(default)]
            reviewer_name: Option<String>,
            #[serde(default)]
            rating: Option<Value>,
            #[serde(default)]
            review_text: Option<String>,
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            submitted_date_time_utc: Option<String>,
        }
        if !crate::model::valid_key(SourceKind::MicrosoftStore, key) {
            return Vec::new();
        }
        let url = format!("{}/ratings/product/{key}?{}", self.api, self.market());
        let answer = self.http.get(&url).ok().and_then(|text| serde_json::from_str::<Answer<List>>(&text).ok());
        let Some(list) = answer.and_then(|answer| answer.payload) else { return Vec::new() };
        list.reviews
            .into_iter()
            .filter_map(|review| {
                let stars = review.rating.as_ref().and_then(Value::as_f64).unwrap_or(0.0).round().clamp(0.0, 5.0) as u8;
                // The Store writes line breaks as bare carriage returns.
                let text = review.review_text.unwrap_or_default().replace("\r\n", "\n").replace('\r', "\n");
                (stars >= 1 && !text.trim().is_empty()).then(|| crate::ratings::Review {
                    stars,
                    title: review.title.unwrap_or_default().trim().to_string(),
                    text: text.trim().to_string(),
                    author: review.reviewer_name.unwrap_or_default().trim().to_string(),
                    date: review.submitted_date_time_utc.unwrap_or_default().chars().take(10).collect(),
                    version: String::new(),
                })
            })
            .take(20)
            .collect()
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let product = self.lookup(key)?;
        let id = product.product_id.to_ascii_uppercase();
        let updated = product.last_update_date_utc.as_deref().or(product.last_modified_date.as_deref()).unwrap_or("");
        Ok(vec![Release {
            version: product.version.clone().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "latest".to_string()),
            published: updated.chars().take(10).collect(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets: vec![Asset {
                name: "Get from the Microsoft Store".to_string(),
                size: product.approximate_size_in_bytes.as_ref().and_then(count_of).unwrap_or(0),
                url: store_link(&id),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::StoreApp,
                    os: TargetOs::Known(Os::Windows),
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
    fn counts_are_read_however_they_are_written() {
        assert_eq!(count_of(&serde_json::json!(34828)), Some(34828));
        assert_eq!(count_of(&serde_json::json!("18K")), Some(18_000));
        assert_eq!(count_of(&serde_json::json!("1.2M")), Some(1_200_000));
        assert_eq!(count_of(&serde_json::json!("1,204")), Some(1204));
        assert_eq!(count_of(&serde_json::json!("many")), None);
        assert_eq!(count_of(&serde_json::json!(null)), None);
    }

    #[test]
    fn only_the_stores_own_pictures_are_used() {
        assert!(own_picture("https://store-images.s-microsoft.com/image/apps.1.2.a-b"));
        assert!(!own_picture("http://store-images.s-microsoft.com/image/apps.1"));
        assert!(!own_picture("https://store-images.s-microsoft.com.example.org/image/apps.1"));
        assert!(!own_picture("https://example.org/store-images.s-microsoft.com/a.png"));
        assert!(!own_picture("https://store-images.s-microsoft.com/image/a?redirect=elsewhere"));
    }
}
