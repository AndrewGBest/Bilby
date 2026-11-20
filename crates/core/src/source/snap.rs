//! The Snap Store, through its public API.
//! <https://api.snapcraft.io/docs/>
//!
//! Snaps are installed by snapd, which needs administrator rights, so the store shows
//! the command rather than running it, as it does for distribution packages.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{Os, Platform};
use serde::Deserialize;
use std::sync::Arc;

pub const SNAP_API: &str = "https://api.snapcraft.io/v2/snaps";
const FIELDS: &str = "title,summary,description,publisher,version,media,download,license";

pub struct SnapStore {
    http: Arc<dyn Http>,
    api: String,
}

impl SnapStore {
    pub fn new(http: Arc<dyn Http>) -> SnapStore {
        SnapStore::with_api(http, SNAP_API)
    }

    pub fn with_api(http: Arc<dyn Http>, api: &str) -> SnapStore {
        SnapStore { http, api: api.trim_end_matches('/').to_string() }
    }

    fn info(&self, key: &str) -> Result<Info> {
        if !crate::model::valid_key(SourceKind::Snap, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a snap name")));
        }
        Ok(serde_json::from_str(&self.http.get(&format!("{}/info/{key}?fields={FIELDS}", self.api))?)?)
    }
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    results: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    name: String,
    snap: Snap,
    revision: Option<Revision>,
}

#[derive(Deserialize)]
struct Revision {
    version: Option<String>,
}

#[derive(Deserialize)]
struct Snap {
    title: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    license: Option<String>,
    publisher: Option<Publisher>,
    #[serde(default)]
    media: Vec<Media>,
}

#[derive(Deserialize)]
struct Publisher {
    #[serde(rename = "display-name")]
    display_name: Option<String>,
    /// "verified" or "starred" when the Snap Store has confirmed who they are.
    validation: Option<String>,
}

#[derive(Deserialize)]
struct Media {
    #[serde(rename = "type")]
    kind: String,
    url: String,
}

#[derive(Deserialize)]
struct Info {
    name: String,
    snap: Snap,
    #[serde(rename = "channel-map", default)]
    channels: Vec<Channel>,
}

#[derive(Deserialize)]
struct Channel {
    channel: ChannelName,
    version: Option<String>,
    download: Option<Download>,
}

#[derive(Deserialize)]
struct ChannelName {
    architecture: Option<String>,
    risk: Option<String>,
    #[serde(rename = "released-at")]
    released_at: Option<String>,
}

#[derive(Deserialize)]
struct Download {
    size: Option<u64>,
}

fn summary_of(name: &str, snap: &Snap, version: Option<String>) -> AppSummary {
    let mut app =
        AppSummary::new(AppId::new(SourceKind::Snap, name), snap.title.clone().unwrap_or_else(|| name.to_string()));
    app.summary = snap.summary.clone().unwrap_or_default();
    app.license = snap.license.clone().filter(|l| !l.is_empty() && l != "unset");
    app.homepage = format!("https://snapcraft.io/{name}");
    app.version = version;
    if let Some(publisher) = &snap.publisher {
        app.author = publisher.display_name.clone().unwrap_or_default();
        app.verified = matches!(publisher.validation.as_deref(), Some("verified" | "starred"));
    }
    // Only pictures the Snap Store itself serves.
    app.icon_url = snap
        .media
        .iter()
        .find(|m| m.kind == "icon" && m.url.starts_with("https://dashboard.snapcraft.io/") && m.url.ends_with(".png"))
        .map(|m| m.url.clone());
    app
}

impl Source for SnapStore {
    fn kind(&self) -> SourceKind {
        SourceKind::Snap
    }

    /// Wherever snapd is set up.
    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::Linux && platform.linux.as_ref().is_some_and(|l| l.snap)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let url = format!("{}/find?q={}&fields={FIELDS}", self.api, url_encode(query.trim()));
        let found: Found = serde_json::from_str(&self.http.get(&url)?)?;
        Ok(found
            .results
            .into_iter()
            .filter(|hit| crate::model::valid_key(SourceKind::Snap, &hit.name))
            .take(limit)
            .map(|hit| summary_of(&hit.name, &hit.snap, hit.revision.and_then(|r| r.version)))
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        let info = self.info(key)?;
        let stable = info.channels.iter().find(|c| c.channel.risk.as_deref() == Some("stable"));
        Ok(summary_of(&info.name, &info.snap, stable.and_then(|c| c.version.clone())))
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some(Format::SnapStore.label())
    }

    fn about(&self, key: &str) -> Result<Option<String>> {
        Ok(self.info(key)?.snap.description.filter(|d| !d.trim().is_empty()))
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let info = self.info(key)?;
        // The stable channel; a snap with none is offered from its first channel.
        let Some(channel) = info
            .channels
            .iter()
            .find(|c| c.channel.risk.as_deref() == Some("stable"))
            .or_else(|| info.channels.first())
        else {
            return Ok(Vec::new());
        };
        let _ = &channel.channel.architecture;
        Ok(vec![Release {
            version: channel.version.clone().unwrap_or_else(|| "latest".to_string()),
            published: channel.channel.released_at.clone().unwrap_or_default().chars().take(10).collect(),
            prerelease: channel.channel.risk.as_deref().is_some_and(|risk| risk != "stable"),
            downloads: None,
            notes: String::new(),
            assets: vec![Asset {
                name: info.name.clone(),
                size: channel.download.as_ref().and_then(|d| d.size).unwrap_or(0),
                url: format!("https://snapcraft.io/{}", info.name),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::SnapStore,
                    os: TargetOs::Known(Os::Linux),
                    // snapd picks the build for this machine.
                    arch: TargetArch::Unknown,
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }
}
