//! Plain data shared by sources, the catalog and the UIs.

use crate::asset::AssetInfo;
use crate::compat::Verdict;
use crate::install::InstallPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    GitHub,
    Aur,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::GitHub => "GitHub",
            SourceKind::Aur => "AUR",
        }
    }
    pub fn prefix(self) -> &'static str {
        match self {
            SourceKind::GitHub => "github",
            SourceKind::Aur => "aur",
        }
    }
}

/// `github:owner/repo` or `aur:package`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppId {
    pub source: SourceKind,
    pub key: String,
}

impl AppId {
    pub fn new(source: SourceKind, key: impl Into<String>) -> Self {
        AppId {
            source,
            key: key.into(),
        }
    }

    pub fn parse(s: &str) -> Option<AppId> {
        let (prefix, key) = s.split_once(':')?;
        let source = match prefix {
            "github" => SourceKind::GitHub,
            "aur" => SourceKind::Aur,
            _ => return None,
        };
        (!key.is_empty()).then(|| AppId::new(source, key))
    }
}

impl std::fmt::Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.source.prefix(), self.key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppSummary {
    pub id: AppId,
    pub name: String,
    pub author: String,
    pub summary: String,
    /// Stars on GitHub, votes on the AUR.
    pub popularity: Option<u64>,
    pub homepage: String,
    pub license: Option<String>,
    pub category: Option<String>,
    /// Version if the source's listing already knows it (the AUR does, GitHub search does not).
    pub version: Option<String>,
    /// Short label for the file that would be installed, e.g. "AppImage · x86_64".
    /// Filled in once releases were checked against this machine.
    pub install_label: Option<String>,
    /// Flagged by the source (AUR "out of date").
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: String,
    /// `sha256:<hex>` when the source publishes one.
    pub digest: Option<String>,
    /// Set by sources that know exactly what the asset is (the AUR); otherwise the
    /// file name is classified.
    pub info: Option<AssetInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub published: String,
    pub prerelease: bool,
    pub notes: String,
    pub assets: Vec<Asset>,
}

/// A file from a release that works on this machine, and how to install it.
#[derive(Debug, Clone)]
pub struct InstallOption {
    pub asset: Asset,
    pub info: AssetInfo,
    pub verdict: Verdict,
    pub plan: InstallPlan,
}

impl InstallOption {
    pub fn label(&self) -> String {
        self.info.label()
    }
}

/// A file that was filtered out, with the reason shown to the user on request.
#[derive(Debug, Clone, PartialEq)]
pub struct HiddenAsset {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedRelease {
    pub version: String,
    pub published: String,
    pub prerelease: bool,
    pub notes: String,
    /// Best first.
    pub options: Vec<InstallOption>,
    pub hidden: Vec<HiddenAsset>,
}

#[derive(Debug, Clone)]
pub struct AppDetails {
    pub summary: AppSummary,
    /// The newest release that has something for this machine. `None` if no release does.
    pub release: Option<ResolvedRelease>,
    /// Why `release` is `None`, in words.
    pub unavailable_reason: Option<String>,
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if bytes == 0 {
        return String::new();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn human_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.0}k", n as f64 / 1000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let id = AppId::parse("github:sharkdp/bat").unwrap();
        assert_eq!(id.source, SourceKind::GitHub);
        assert_eq!(id.to_string(), "github:sharkdp/bat");
        assert_eq!(AppId::parse("aur:paru").unwrap().key, "paru");
        assert!(AppId::parse("nope").is_none());
        assert!(AppId::parse("npm:left-pad").is_none());
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "");
        assert_eq!(human_size(900), "900 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(150 * 1024 * 1024), "150 MB");
        assert_eq!(human_count(950), "950");
        assert_eq!(human_count(48_200), "48k");
    }
}
