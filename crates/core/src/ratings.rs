//! Star ratings, from the Open Desktop Ratings Service (ODRS).
//!
//! ODRS is the service GNOME Software and KDE Discover get their ratings from. It
//! publishes one file with the star counts of every app it knows, keyed by the app's
//! AppStream id. That file is fetched at most once a day and kept on disk. Asking for
//! it tells ODRS nothing about which apps are looked at: everyone gets the same file.
//!
//! The shape was read from the service's source
//! (<https://github.com/GNOME/odrs-web>, `odrs/views_api.py`); the live service could
//! not be reached from where this was written.
//!
//! Written reviews come from the same service, but one app at a time: opening the page
//! of an app the service knows asks it for that app's reviews. Nothing identifying is
//! sent beyond what any web request carries: no user id, a generic system name, and no
//! version. Writing one is in [`crate::reviewing`].

use crate::error::Result;
use crate::http::Http;
use crate::model::Rating;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub const ODRS_RATINGS: &str = "https://odrs.gnome.org/1.0/reviews/api/ratings";
pub const ODRS_FETCH: &str = "https://odrs.gnome.org/1.0/reviews/api/fetch";

/// How many reviews an app's page asks for.
pub const REVIEWS_SHOWN: usize = 8;

/// What one person wrote about an app.
#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    /// 1 to 5.
    pub stars: u8,
    pub title: String,
    pub text: String,
    pub author: String,
    /// `YYYY-MM-DD`, or empty when the service gave no date.
    pub date: String,
    /// The app's version the review is about, or empty.
    pub version: String,
}

#[derive(Deserialize)]
struct RawReview {
    /// 0 to 100.
    rating: Option<serde_json::Value>,
    summary: Option<String>,
    description: Option<String>,
    user_display: Option<String>,
    date_created: Option<serde_json::Value>,
    version: Option<String>,
}

fn as_number(value: &Option<serde_json::Value>) -> Option<f64> {
    match value.as_ref()? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// One line of text from whatever a stranger typed: no control characters, bounded.
fn tidy(text: Option<String>, limit: usize) -> String {
    let text = text.unwrap_or_default();
    let clean: String = text.chars().map(|c| if c == '\n' || !c.is_control() { c } else { ' ' }).collect();
    let clean = clean.trim();
    match clean.char_indices().nth(limit) {
        Some((at, _)) => format!("{}…", clean[..at].trim_end()),
        None => clean.to_string(),
    }
}

/// Parse the service's answer: an array of reviews, best first.
pub fn parse_reviews(json: &str) -> Result<Vec<Review>> {
    let raw: Vec<RawReview> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .filter_map(|r| {
            let rating = as_number(&r.rating)?;
            let (title, text) = (tidy(r.summary, 120), tidy(r.description, 1200));
            // A row with nothing written is a placeholder, not a review.
            if rating <= 0.0 || (title.is_empty() && text.is_empty()) {
                return None;
            }
            Some(Review {
                stars: ((rating / 20.0).round() as u8).clamp(1, 5),
                title,
                text,
                author: tidy(r.user_display, 40),
                date: as_number(&r.date_created)
                    .filter(|secs| *secs > 0.0)
                    .map(|secs| crate::source::aur::date_from_unix(secs as i64))
                    .unwrap_or_default(),
                version: tidy(r.version, 30).trim_start_matches("unknown").to_string(),
            })
        })
        .take(REVIEWS_SHOWN)
        .collect())
}

/// Ask the service for an app's reviews. One request, naming the app and nothing else.
pub fn fetch_reviews(http: &dyn Http, appstream_id: &str) -> Result<Vec<Review>> {
    let id = appstream_id.trim();
    let body = serde_json::json!({
        "app_id": id,
        // Some apps are filed under the older spelling of their id.
        "compat_ids": [format!("{}.desktop", id.trim_end_matches(".desktop"))],
        "locale": "en",
        "distro": "unknown",
        "version": "unknown",
        "limit": REVIEWS_SHOWN,
        // The service refuses a request without a 40-character id for the person asking
        // (other stores send a hash of the user and machine). Everyone sends the same
        // zeros here, so it identifies nobody.
        "user_hash": ANONYMOUS,
    });
    parse_reviews(&http.post_json(ODRS_FETCH, &body.to_string())?)
}

/// Sent in place of an id for the user: the same for every copy of the store.
const ANONYMOUS: &str = "0000000000000000000000000000000000000000";

/// How long a downloaded ratings file is used before asking again.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Fewer ratings than this say too little to show a number for.
const ENOUGH_TO_SHOW: u32 = 5;

#[derive(Deserialize)]
struct Stars {
    #[serde(default)]
    star1: u32,
    #[serde(default)]
    star2: u32,
    #[serde(default)]
    star3: u32,
    #[serde(default)]
    star4: u32,
    #[serde(default)]
    star5: u32,
}

#[derive(Debug, Default)]
pub struct Ratings {
    /// By AppStream id, lower-cased and without a trailing `.desktop`.
    by_id: HashMap<String, Rating>,
}

fn normal(id: &str) -> String {
    id.trim().to_lowercase().trim_end_matches(".desktop").to_string()
}

impl Ratings {
    pub fn parse(json: &str) -> Result<Ratings> {
        let raw: HashMap<String, Stars> = serde_json::from_str(json)?;
        let mut by_id = HashMap::new();
        for (id, s) in raw {
            let counts = [s.star1, s.star2, s.star3, s.star4, s.star5];
            let count: u32 = counts.iter().sum();
            if count < ENOUGH_TO_SHOW {
                continue;
            }
            let weighted: u32 = counts.iter().enumerate().map(|(i, n)| (i as u32 + 1) * n).sum();
            let rating = Rating { stars: weighted as f32 / count as f32, count, spread: counts };
            // An app can be listed under both spellings of its id; keep the busier one.
            let slot = by_id.entry(normal(&id)).or_insert(rating);
            if rating.count > slot.count {
                *slot = rating;
            }
        }
        Ok(Ratings { by_id })
    }

    pub fn get(&self, appstream_id: &str) -> Option<Rating> {
        self.by_id.get(&normal(appstream_id)).copied()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The ratings, from `dir/ratings.json` while it is fresh, otherwise downloaded. If
    /// the download fails, yesterday's file is better than nothing.
    pub fn load(http: &dyn Http, dir: &Path) -> Result<Ratings> {
        let path = dir.join("ratings.json");
        let read = || std::fs::read_to_string(&path).ok().and_then(|text| Ratings::parse(&text).ok());
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < MAX_AGE);
        if fresh {
            if let Some(ratings) = read() {
                return Ok(ratings);
            }
        }
        std::fs::create_dir_all(dir)?;
        let download = dir.join("ratings.download");
        // The whole file is a few megabytes at most; refuse anything absurd.
        let fetched = http.download(ODRS_RATINGS, &download, &mut |done, _| done < 64 * 1024 * 1024);
        let parsed = fetched.and_then(|_| Ratings::parse(&std::fs::read_to_string(&download)?));
        match parsed {
            Ok(ratings) => {
                std::fs::rename(&download, &path)?;
                Ok(ratings)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&download);
                read().ok_or(e)
            }
        }
    }
}

/// "4.6" for a rating; one decimal is all the precision there is.
pub fn stars_text(rating: Rating) -> String {
    format!("{:.1}", rating.stars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_are_averaged_and_thin_evidence_is_not_shown() {
        let ratings = Ratings::parse(
            r#"{
            "org.example.Good": {"star0": 0, "star1": 1, "star2": 0, "star3": 1, "star4": 8, "star5": 30, "total": 40},
            "org.example.Few.desktop": {"star0": 0, "star1": 0, "star2": 0, "star3": 0, "star4": 0, "star5": 3, "total": 3},
            "Org.Example.Mixed.desktop": {"star1": 5, "star5": 5, "total": 10, "something_new": 1}
        }"#,
        )
        .unwrap();
        let good = ratings.get("org.example.Good").unwrap();
        assert_eq!(good.count, 40);
        assert!((good.stars - 4.65).abs() < 0.01, "{}", good.stars);
        assert_eq!(stars_text(good), "4.7");
        assert!(ratings.get("org.example.Few").is_none(), "three ratings are not a rating");
        assert_eq!(ratings.get("org.example.mixed").unwrap().stars, 3.0, "ids match whatever their case or suffix");
        assert!(ratings.get("org.example.Unknown").is_none());
        assert_eq!(ratings.len(), 2);
        assert!(Ratings::parse("[]").is_err());
        assert!(Ratings::parse("not json").is_err());
    }

    #[test]
    fn reviews_are_read_cleaned_and_placeholders_dropped() {
        let json = r#"[
          {"app_id":"org.example.Good","rating":100,"summary":"Does the job","description":"Simple and quick.\nNo account needed.",
           "user_display":"Ana","date_created":1793750400,"version":"4.2.0","karma_up":3,"score":90,"user_hash":"abc"},
          {"rating":"40","summary":"  Crashed\u0007 once  ","description":"","user_display":null,"date_created":"0","version":"unknown"},
          {"rating":0,"summary":"","description":"","user_skey":"x"},
          {"summary":"no rating at all","description":"x"}
        ]"#;
        let reviews = parse_reviews(json).unwrap();
        assert_eq!(reviews.len(), 2, "the placeholder and the unrated row are not reviews");
        assert_eq!(
            reviews[0],
            Review {
                stars: 5,
                title: "Does the job".into(),
                text: "Simple and quick.\nNo account needed.".into(),
                author: "Ana".into(),
                date: "2026-11-04".into(),
                version: "4.2.0".into(),
            }
        );
        assert_eq!((reviews[1].stars, reviews[1].title.as_str(), reviews[1].author.as_str()), (2, "Crashed  once", ""));
        assert_eq!((reviews[1].date.as_str(), reviews[1].version.as_str()), ("", ""));
        let long = format!(r#"[{{"rating":80,"summary":"s","description":"{}"}}]"#, "word ".repeat(600));
        assert!(parse_reviews(&long).unwrap()[0].text.chars().count() <= 1201, "an essay is cut");
        assert!(parse_reviews("{}").is_err());
        assert_eq!(parse_reviews("[]").unwrap(), []);
    }
}
