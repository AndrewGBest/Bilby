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
//! Written reviews are deliberately not fetched. See `docs/DESIGN.md`.

use crate::error::Result;
use crate::http::Http;
use crate::model::Rating;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub const ODRS_RATINGS: &str = "https://odrs.gnome.org/1.0/reviews/api/ratings";

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
            let rating = Rating { stars: weighted as f32 / count as f32, count };
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
}
