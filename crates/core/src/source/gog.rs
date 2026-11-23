//! GOG: games sold without copy protection, for Windows, macOS and Linux.
//!
//! * `GET https://catalog.gog.com/v1/catalog?query=like:...`   search, with price and rating
//! * `GET https://api.gog.com/products/<id>?expand=description,screenshots,downloads`
//! * `GET https://api.gog.com/products/<id>/prices?countryCode=US`
//! * `GET https://api.gog.com/v2/games/<id>`                    who made it
//! * `GET https://reviews.gog.com/v1/products/<id>/averageRating`
//! * `GET https://reviews.gog.com/v1/products/<id>/reviews`
//!
//! These are the services GOG's own website asks; they need no account. What is read
//! here was checked against the live services on 2026-11-23. A game is bought and
//! downloaded with the person's GOG account, so the store hands over to the game's
//! page on gog.com. Prices are those of the country the device is set to (see `region`).

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Rating, Release, SourceKind};
use crate::platform::{Os, Platform};
use crate::ratings::Review;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub struct Gog {
    http: Arc<dyn Http>,
    os: Os,
    /// Whose prices to ask for.
    country: String,
    catalog: String,
    api: String,
    reviews: String,
}

/// A game's page on gog.com, from the name GOG gives it in addresses.
pub fn store_page(slug: &str) -> String {
    format!("https://www.gog.com/en/game/{slug}")
}

/// A name as GOG writes them in addresses: small letters, digits and underscores.
pub fn slug_ok(slug: &str) -> bool {
    (1..=120).contains(&slug.len()) && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Only pictures GOG itself serves, with or without the "https:" in front.
fn own_picture(url: &str) -> Option<String> {
    let url = if url.starts_with("//") { format!("https:{url}") } else { url.to_string() };
    let host = url.strip_prefix("https://")?.split('/').next()?;
    let own = host == "images.gog.com"
        || host == "images.gog-statics.com"
        || (host.starts_with("images-") && host.ends_with(".gog-statics.com"));
    (own && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '?')).then_some(url)
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    products: Vec<Listed>,
}

/// A game as the catalog lists it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    id: Value,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    developers: Option<Vec<String>>,
    #[serde(default)]
    publishers: Option<Vec<String>>,
    #[serde(default)]
    cover_vertical: Option<String>,
    #[serde(default)]
    price: Option<ListedPrice>,
    /// Out of fifty.
    #[serde(default)]
    reviews_rating: Option<f32>,
    #[serde(default)]
    reviews_count: Option<u32>,
    #[serde(default)]
    product_type: Option<String>,
    #[serde(default)]
    operating_systems: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ListedPrice {
    #[serde(rename = "final", default)]
    now: Option<String>,
}

/// A game's own record.
#[derive(Deserialize)]
struct Product {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    content_system_compatibility: Option<Systems>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    images: Option<Images>,
    #[serde(default)]
    description: Option<Description>,
    #[serde(default)]
    screenshots: Option<Vec<Screenshot>>,
    #[serde(default)]
    downloads: Option<Downloads>,
}

#[derive(Deserialize, Default)]
struct Systems {
    #[serde(default)]
    windows: bool,
    #[serde(default)]
    osx: bool,
    #[serde(default)]
    linux: bool,
}

#[derive(Deserialize)]
struct Images {
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    logo2x: Option<String>,
}

#[derive(Deserialize)]
struct Description {
    #[serde(default)]
    lead: Option<String>,
    #[serde(default)]
    full: Option<String>,
}

#[derive(Deserialize)]
struct Screenshot {
    #[serde(default)]
    formatter_template_url: Option<String>,
}

#[derive(Deserialize)]
struct Downloads {
    #[serde(default)]
    installers: Vec<Installer>,
}

#[derive(Deserialize)]
struct Installer {
    #[serde(default)]
    os: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    total_size: u64,
}

#[derive(Deserialize)]
struct Average {
    #[serde(default)]
    value: f32,
    #[serde(default)]
    count: u32,
}

/// "$0.00" is free; anything else is shown as GOG wrote it.
fn price_text(price: &str) -> Option<String> {
    let price = price.trim();
    if price.is_empty() {
        return None;
    }
    let nothing = price.chars().filter(char::is_ascii_digit).all(|digit| digit == '0');
    Some(if nothing { "Free".to_string() } else { price.to_string() })
}

/// "1499 USD", as the prices service writes it, is $14.99. The country's own money
/// comes first in its answer, so that is the one read.
fn cents_text(price: &str) -> Option<String> {
    let (amount, currency) = price.trim().split_once(' ')?;
    let cents: u64 = amount.parse().ok()?;
    let known = currency.len() == 3 && currency.chars().all(|c| c.is_ascii_uppercase());
    known.then(|| crate::region::money(cents, currency))
}

fn stars(out_of_five: f32, count: u32) -> Option<Rating> {
    (count >= 5 && out_of_five > 0.0).then_some(Rating { stars: out_of_five.min(5.0), count, spread: [0; 5] })
}

impl Listed {
    fn key(&self) -> Option<String> {
        let key = match &self.id {
            Value::String(text) => text.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        crate::model::valid_key(SourceKind::Gog, &key).then_some(key)
    }

    fn summary(&self) -> Option<AppSummary> {
        let title = self.title.as_deref().map(str::trim).filter(|title| !title.is_empty())?;
        // Games and packs of them; an expansion needs its game and is found on its page.
        if self.product_type.as_deref().is_some_and(|kind| kind == "dlc" || kind == "extras") {
            return None;
        }
        let mut app = AppSummary::new(AppId::new(SourceKind::Gog, self.key()?), title.to_string());
        let first = |names: &Option<Vec<String>>| names.as_ref().and_then(|all| all.first().cloned());
        app.author = first(&self.developers).or_else(|| first(&self.publishers)).unwrap_or_default();
        app.price = self.price.as_ref().and_then(|price| price.now.as_deref()).and_then(price_text);
        app.icon_url = self.cover_vertical.as_deref().and_then(own_picture);
        app.homepage = "https://www.gog.com/".to_string();
        app.category = Some("Games".to_string());
        if let Some(rating) = stars(self.reviews_rating.unwrap_or(0.0) / 10.0, self.reviews_count.unwrap_or(0)) {
            app.rating = Some(rating);
            app.popularity = Some(rating.count as u64);
        }
        Some(app)
    }
}

impl Product {
    fn about(&self) -> Option<String> {
        let description = self.description.as_ref()?;
        let html = description.full.as_deref().or(description.lead.as_deref())?;
        let text = super::flathub::strip_html(html);
        (!text.trim().is_empty()).then(|| text.trim().to_string())
    }

    fn lead(&self) -> String {
        let html = self.description.as_ref().and_then(|d| d.lead.as_deref()).unwrap_or("");
        let text = super::flathub::strip_html(html);
        text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect()
    }

    fn systems(&self) -> Vec<Os> {
        let on = self.content_system_compatibility.as_ref();
        let mut systems = Vec::new();
        if on.is_some_and(|s| s.windows) {
            systems.push(Os::Windows);
        }
        if on.is_some_and(|s| s.osx) {
            systems.push(Os::MacOs);
        }
        if on.is_some_and(|s| s.linux) {
            systems.push(Os::Linux);
        }
        systems
    }
}

impl Gog {
    pub fn new(http: Arc<dyn Http>, platform: &Platform) -> Gog {
        let (catalog, api, reviews) =
            ("https://catalog.gog.com/v1", "https://api.gog.com", "https://reviews.gog.com/v1");
        Gog::with_apis(http, platform, catalog, api, reviews)
    }

    pub fn with_apis(http: Arc<dyn Http>, platform: &Platform, catalog: &str, api: &str, reviews: &str) -> Gog {
        let tidy = |address: &str| address.trim_end_matches('/').to_string();
        Gog {
            http,
            os: platform.os,
            country: "US".to_string(),
            catalog: tidy(catalog),
            api: tidy(api),
            reviews: tidy(reviews),
        }
    }

    /// Ask for the prices of this country (two letters, as in `CA`); anything that is
    /// not a country code leaves it as it was.
    pub fn in_country(mut self, code: &str) -> Self {
        if let Some(country) = crate::region::country(code) {
            self.country = country;
        }
        self
    }

    /// Is a game the catalog lists made for this system? One that does not say is kept.
    fn made_for_here(&self, listed: &Listed) -> bool {
        let named = match self.os {
            Os::Windows => "windows",
            Os::MacOs => "osx",
            _ => "linux",
        };
        listed.operating_systems.as_ref().is_none_or(|all| all.is_empty() || all.iter().any(|os| os == named))
    }

    fn lookup(&self, key: &str) -> Result<Product> {
        if !crate::model::valid_key(SourceKind::Gog, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a GOG product number")));
        }
        let url = format!("{}/products/{key}?expand=description,screenshots,downloads", self.api);
        let nothing = || Error::NotFound(format!("GOG has nothing under number {key}."));
        let text = match self.http.get(&url) {
            Ok(text) => text,
            Err(Error::NotFound(_)) | Err(Error::Status { .. }) => return Err(nothing()),
            Err(e) => return Err(e),
        };
        let product: Product = serde_json::from_str(&text)?;
        (product.id.to_string() == key).then_some(product).ok_or_else(nothing)
    }
}

impl Source for Gog {
    fn kind(&self) -> SourceKind {
        SourceKind::Gog
    }

    /// GOG sells for all three; each game says which of them it runs on.
    fn available_on(&self, platform: &Platform) -> bool {
        matches!(platform.os, Os::Windows | Os::MacOs | Os::Linux)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        if query.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/catalog?limit={}&query=like:{}&order=desc:score&productType=in:game,pack&countryCode={}&locale=en-US&currencyCode={}",
            self.catalog,
            limit.clamp(1, 48),
            url_encode(query.trim()),
            self.country,
            crate::region::currency_of(&self.country)
        );
        let found: Found = serde_json::from_str(&self.http.get(&url)?)?;
        // Only what is made for this system, as with Steam: a Windows game is not
        // offered where nothing here would run it.
        Ok(found
            .products
            .iter()
            .filter(|listed| self.made_for_here(listed))
            .filter_map(Listed::summary)
            .take(limit.max(1))
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        let product = self.lookup(key)?;
        let mut app = AppSummary::new(
            AppId::new(SourceKind::Gog, key),
            product.title.clone().unwrap_or_default().trim().to_string(),
        );
        app.summary = product.lead();
        app.category = Some("Games".to_string());
        let slug = product.slug.as_deref().filter(|slug| slug_ok(slug));
        app.homepage = slug.map(store_page).unwrap_or_else(|| "https://www.gog.com/".to_string());
        let images = product.images.as_ref();
        app.icon_url = images
            .and_then(|i| i.icon.as_deref().and_then(own_picture))
            .or_else(|| images.and_then(|i| i.logo2x.as_deref().and_then(own_picture)));
        app.version = product
            .downloads
            .as_ref()
            .and_then(|d| d.installers.iter().find_map(|i| i.version.clone()))
            .filter(|version| !version.trim().is_empty());
        // Who made it, the price and what people think of it are kept elsewhere; a
        // page without them is still a page.
        #[derive(Deserialize)]
        struct Game {
            #[serde(rename = "_embedded")]
            embedded: Makers,
        }
        #[derive(Deserialize)]
        struct Makers {
            #[serde(default)]
            developers: Vec<Named>,
            #[serde(default)]
            publisher: Option<Named>,
        }
        #[derive(Deserialize)]
        struct Named {
            #[serde(default)]
            name: String,
        }
        let game = self.http.get(&format!("{}/v2/games/{key}", self.api));
        if let Some(makers) = game.ok().and_then(|text| serde_json::from_str::<Game>(&text).ok()) {
            let developer = makers.embedded.developers.into_iter().map(|d| d.name).find(|name| !name.trim().is_empty());
            app.author = developer.or_else(|| makers.embedded.publisher.map(|p| p.name)).unwrap_or_default();
        }
        #[derive(Deserialize)]
        struct Prices {
            #[serde(rename = "_embedded")]
            embedded: Embedded,
        }
        #[derive(Deserialize)]
        struct Embedded {
            #[serde(default)]
            prices: Vec<Price>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Price {
            #[serde(default)]
            final_price: String,
        }
        let prices = self.http.get(&format!("{}/products/{key}/prices?countryCode={}", self.api, self.country));
        app.price = prices
            .ok()
            .and_then(|text| serde_json::from_str::<Prices>(&text).ok())
            .and_then(|prices| prices.embedded.prices.first().and_then(|price| cents_text(&price.final_price)));
        let average = self.http.get(&format!("{}/products/{key}/averageRating", self.reviews));
        if let Some(average) = average.ok().and_then(|text| serde_json::from_str::<Average>(&text).ok()) {
            if let Some(rating) = stars(average.value, average.count) {
                app.rating = Some(rating);
                app.popularity = Some(rating.count as u64);
            }
        }
        Ok(app)
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some("GOG")
    }

    fn about(&self, key: &str) -> Result<Option<String>> {
        Ok(self.lookup(key)?.about())
    }

    fn pictures(&self, key: &str) -> Vec<String> {
        let Ok(product) = self.lookup(key) else { return Vec::new() };
        product
            .screenshots
            .unwrap_or_default()
            .into_iter()
            .filter_map(|shot| shot.formatter_template_url)
            // A template: "..._{formatter}.png". This form is 1600 wide, a JPEG.
            .filter(|template| template.contains("_{formatter}."))
            .filter_map(|template| {
                let base = template.split("_{formatter}.").next().unwrap_or("");
                own_picture(&format!("{base}_ggvgl_2x.jpg"))
            })
            .take(crate::screenshots::MAX_PICTURES)
            .collect()
    }

    fn reviews(&self, key: &str) -> Vec<Review> {
        #[derive(Deserialize)]
        struct Page {
            #[serde(rename = "_embedded")]
            embedded: Items,
        }
        #[derive(Deserialize)]
        struct Items {
            #[serde(default)]
            items: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            content: Option<Content>,
            reviewer: Option<Reviewer>,
            date: Option<String>,
            rating: Option<Stars>,
        }
        #[derive(Deserialize)]
        struct Content {
            title: Option<String>,
            description: Option<String>,
        }
        #[derive(Deserialize)]
        struct Reviewer {
            username: Option<String>,
        }
        #[derive(Deserialize)]
        struct Stars {
            value: Option<f32>,
        }
        if !crate::model::valid_key(SourceKind::Gog, key) {
            return Vec::new();
        }
        let url = format!("{}/products/{key}/reviews?language=in:en-US&limit=20&order=desc:votes", self.reviews);
        let Some(page) = self.http.get(&url).ok().and_then(|text| serde_json::from_str::<Page>(&text).ok()) else {
            return Vec::new();
        };
        page.embedded
            .items
            .into_iter()
            .filter_map(|item| {
                let content = item.content?;
                let text = content.description.unwrap_or_default().trim().to_string();
                let stars = item.rating.and_then(|r| r.value).unwrap_or(0.0).round().clamp(0.0, 5.0) as u8;
                (!text.is_empty() && stars >= 1).then(|| Review {
                    stars,
                    title: content.title.unwrap_or_default().trim().to_string(),
                    text,
                    author: item.reviewer.and_then(|r| r.username).unwrap_or_default(),
                    date: item.date.unwrap_or_default().chars().take(10).collect(),
                    version: String::new(),
                })
            })
            .collect()
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let product = self.lookup(key)?;
        let slug = product
            .slug
            .clone()
            .filter(|slug| slug_ok(slug))
            .ok_or_else(|| Error::NotFound(format!("GOG has no page for number {key}.")))?;
        let installers = product.downloads.as_ref().map(|d| d.installers.as_slice()).unwrap_or(&[]);
        // One entry per system the game is made for: this device is offered its own.
        let assets = product
            .systems()
            .into_iter()
            .map(|os| {
                let named = match os {
                    Os::Windows => "windows",
                    Os::MacOs => "mac",
                    _ => "linux",
                };
                let of_os = || installers.iter().filter(|i| i.os == named);
                let size = of_os().find(|i| i.language == "en").or_else(|| of_os().next()).map_or(0, |i| i.total_size);
                Asset {
                    name: "Get it on GOG".to_string(),
                    size,
                    url: store_page(&slug),
                    digest: None,
                    info: Some(AssetInfo {
                        format: Format::StoreApp,
                        os: TargetOs::Known(os),
                        arch: TargetArch::Unknown,
                        libc: None,
                        tv: false,
                        debug: false,
                    }),
                }
            })
            .collect();
        Ok(vec![Release {
            version: installers
                .iter()
                .find_map(|i| i.version.clone())
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "latest".to_string()),
            published: product.release_date.clone().unwrap_or_default().chars().take(10).collect(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_are_read_in_both_the_ways_gog_writes_them() {
        assert_eq!(price_text("$14.99").as_deref(), Some("$14.99"));
        assert_eq!(price_text("$0.00").as_deref(), Some("Free"));
        assert_eq!(price_text(" "), None);
        assert_eq!(cents_text("1499 USD").as_deref(), Some("$14.99"));
        assert_eq!(cents_text("905 USD").as_deref(), Some("$9.05"));
        assert_eq!(cents_text("0 USD").as_deref(), Some("Free"));
        assert_eq!(cents_text("1499 PLN").as_deref(), Some("14.99 PLN"));
        assert_eq!(cents_text("1699 CAD").as_deref(), Some("$16.99"));
        assert_eq!(cents_text("1399 EUR").as_deref(), Some("€13.99"));
        assert_eq!(cents_text("1499 dollars"), None);
        assert_eq!(cents_text("free"), None);
    }

    #[test]
    fn only_gogs_own_pictures_and_pages_are_used() {
        assert_eq!(
            own_picture("//images-1.gog-statics.com/a.png").as_deref(),
            Some("https://images-1.gog-statics.com/a.png")
        );
        assert!(own_picture("https://images.gog-statics.com/a_b.jpg").is_some());
        assert!(own_picture("https://images.gog-statics.com.example.org/a.jpg").is_none());
        assert!(own_picture("http://images.gog.com/a.jpg").is_none());
        assert!(slug_ok("the_witcher_3_wild_hunt_game") && !slug_ok("../x") && !slug_ok("A") && !slug_ok(""));
    }
}
