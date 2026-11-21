//! The Arch User Repository, through the aurweb RPC interface (version 5).
//! <https://wiki.archlinux.org/title/Aurweb_RPC_interface>
//!
//! AUR packages are build recipes (PKGBUILDs), not binaries: "installing" one means
//! building it on the user's machine. They are modelled as a single release with a
//! single [`Format::AurBuild`] asset, so the rest of the store treats them like anything
//! else, and the compatibility engine keeps them off non-Arch systems.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{LinuxFamily, Os, Platform};
use serde::Deserialize;
use std::sync::Arc;

const RPC: &str = "https://aur.archlinux.org/rpc/v5";

pub struct Aur {
    http: Arc<dyn Http>,
    rpc: String,
}

impl Aur {
    pub fn new(http: Arc<dyn Http>) -> Aur {
        Aur::with_rpc(http, RPC)
    }

    /// Talk to a different aurweb instance (a mirror, a test server).
    pub fn with_rpc(http: Arc<dyn Http>, rpc: &str) -> Aur {
        Aur { http, rpc: rpc.trim_end_matches('/').to_string() }
    }

    fn info(&self, name: &str) -> Result<Package> {
        if !crate::model::valid_key(SourceKind::Aur, name) {
            return Err(Error::NotFound(format!("\"{name}\" is not an AUR package name")));
        }
        let url = format!("{}/info?arg[]={}", self.rpc, url_encode(name));
        let resp: RpcResponse = serde_json::from_str(&self.http.get(&url)?)?;
        resp.into_results()?
            .into_iter()
            .next()
            .ok_or_else(|| Error::NotFound(format!("The AUR has no package called “{name}”.")))
    }
}

#[derive(Deserialize)]
struct RpcResponse {
    #[serde(default)]
    results: Vec<Package>,
    #[serde(rename = "type")]
    kind: String,
    error: Option<String>,
}

impl RpcResponse {
    fn into_results(self) -> Result<Vec<Package>> {
        if self.kind == "error" {
            return Err(Error::Parse(self.error.unwrap_or_else(|| "AUR returned an error".to_string())));
        }
        Ok(self.results)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Package {
    name: String,
    package_base: Option<String>,
    version: String,
    description: Option<String>,
    #[serde(rename = "URL")]
    url: Option<String>,
    #[serde(default)]
    num_votes: u64,
    /// Votes weighted by how recent they are: each vote fades a little every day.
    #[serde(default)]
    popularity: f32,
    maintainer: Option<String>,
    out_of_date: Option<i64>,
    #[serde(default)]
    last_modified: i64,
    #[serde(rename = "URLPath")]
    url_path: Option<String>,
    #[serde(default)]
    license: Vec<String>,
}

impl Package {
    fn summary(&self) -> AppSummary {
        AppSummary {
            id: AppId::new(SourceKind::Aur, self.name.clone()),
            name: self.name.clone(),
            author: self.maintainer.clone().unwrap_or_else(|| "orphaned".to_string()),
            summary: self.description.clone().unwrap_or_default(),
            popularity: Some(self.num_votes),
            homepage: format!("https://aur.archlinux.org/packages/{}", self.name),
            license: self.license.first().cloned(),
            category: None,
            version: Some(self.version.clone()),
            install_label: None,
            warning: match (self.out_of_date, &self.maintainer) {
                (Some(_), _) => Some("Flagged out of date on the AUR".to_string()),
                (None, None) => Some("Nobody maintains this AUR package at the moment.".to_string()),
                _ => None,
            },
            origin: None,
            installed_version: None,
            rating: None,
            icon_url: None,
            downloads: None,
            verified: false,
            // The best-known packages sit at a few dozen; most are under one.
            momentum: Some(((self.popularity.max(0.0) + 1.0).ln() / 31.0_f32.ln()).clamp(0.0, 1.0)),
            price: None,
            adult: false,
            restricted: false,
        }
    }
}

/// Days-since-epoch to `YYYY-MM-DD` (civil-from-days, Howard Hinnant's algorithm).
pub(crate) fn date_from_unix(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

impl Source for Aur {
    fn kind(&self) -> SourceKind {
        SourceKind::Aur
    }

    fn available_on(&self, platform: &Platform) -> bool {
        platform.os == Os::Linux && platform.linux.as_ref().is_some_and(|l| l.family == LinuxFamily::Arch)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        // The RPC rejects queries shorter than two characters.
        if query.trim().chars().count() < 2 {
            return Ok(vec![]);
        }
        let url = format!("{}/search/{}?by=name-desc", self.rpc, url_encode(query.trim()));
        let resp: RpcResponse = serde_json::from_str(&self.http.get(&url)?)?;
        let mut packages = resp.into_results()?;
        // The RPC has no ranking; most-voted first is what the AUR website defaults to.
        packages.sort_by_key(|p| std::cmp::Reverse(p.num_votes));
        Ok(packages.iter().take(limit).map(Package::summary).collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        Ok(self.info(key)?.summary())
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        // Every AUR package builds on every Arch system (as far as the RPC can tell),
        // and the search result already has the version.
        Some(Format::AurBuild.label())
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let pkg = self.info(key)?;
        let base = pkg.package_base.clone().unwrap_or_else(|| pkg.name.clone());
        let snapshot = pkg.url_path.clone().unwrap_or_else(|| format!("/cgit/aur.git/snapshot/{base}.tar.gz"));
        let mut notes = String::new();
        if let Some(upstream) = &pkg.url {
            notes.push_str(&format!("Upstream: {upstream}\n"));
        }
        notes.push_str("Built on your machine from a community-maintained recipe (PKGBUILD).");
        Ok(vec![Release {
            version: pkg.version.clone(),
            published: date_from_unix(pkg.last_modified),
            prerelease: false,
            downloads: None,
            notes,
            assets: vec![Asset {
                name: format!("{base} (PKGBUILD)"),
                size: 0,
                url: format!("https://aur.archlinux.org{snapshot}"),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::AurBuild,
                    os: TargetOs::Known(Os::Linux),
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
    fn dates() {
        assert_eq!(date_from_unix(0), "");
        assert_eq!(date_from_unix(86_400), "1970-01-02");
        assert_eq!(date_from_unix(1_700_000_000), "2023-11-14");
        assert_eq!(date_from_unix(1_709_164_800), "2024-02-29");
    }
}
