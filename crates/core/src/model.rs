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
        AppId { source, key: key.into() }
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

impl AppId {
    /// Understand what people paste: a repository URL, `owner/repo`, an AUR package page,
    /// or an id. Returns `None` for anything that looks like ordinary search words.
    pub fn from_user_input(input: &str) -> Option<AppId> {
        let input = input.trim();
        if input.contains(char::is_whitespace) || input.is_empty() {
            return None;
        }
        if let Some(id) = AppId::parse(input) {
            return valid_key(id.source, &id.key).then_some(id);
        }
        let stripped = input.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
        if let Some(rest) = stripped.strip_prefix("aur.archlinux.org/packages/") {
            let name = rest.split(['/', '?', '#']).next().unwrap_or("");
            return valid_key(SourceKind::Aur, name).then(|| AppId::new(SourceKind::Aur, name));
        }
        let path = stripped.strip_prefix("github.com/").unwrap_or(stripped);
        // Without the host, only a bare `owner/repo` counts; `a/b/c` is not a repository.
        let had_host = path.len() != stripped.len();
        let mut parts = path.split(['?', '#']).next().unwrap_or("").split('/').filter(|p| !p.is_empty());
        let (owner, repo, extra) = (parts.next()?, parts.next()?, parts.next());
        if extra.is_some() && !had_host {
            return None;
        }
        let key = format!("{owner}/{}", repo.trim_end_matches(".git"));
        valid_key(SourceKind::GitHub, &key).then(|| AppId::new(SourceKind::GitHub, key))
    }
}

/// Keys end up in URLs, so they are checked against what the source allows in names.
pub fn valid_key(source: SourceKind, key: &str) -> bool {
    let ok = |part: &str, extra: &[char]| {
        !part.is_empty()
            && !part.starts_with('.')
            && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') || extra.contains(&c))
    };
    match source {
        SourceKind::GitHub => matches!(key.split_once('/'), Some((owner, repo)) if ok(owner, &[]) && ok(repo, &[])),
        // Arch package names may also contain `+` and `@`.
        SourceKind::Aur => ok(key, &['+', '@']),
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
    fn pasted_links_are_understood() {
        let gh = |s: &str| AppId::from_user_input(s).map(|id| id.to_string());
        assert_eq!(gh("https://github.com/sharkdp/bat").as_deref(), Some("github:sharkdp/bat"));
        assert_eq!(gh("https://github.com/sharkdp/bat/releases/tag/v0.24.0").as_deref(), Some("github:sharkdp/bat"));
        assert_eq!(gh("github.com/sharkdp/bat.git").as_deref(), Some("github:sharkdp/bat"));
        assert_eq!(gh("  sharkdp/bat ").as_deref(), Some("github:sharkdp/bat"));
        assert_eq!(gh("github:sharkdp/bat").as_deref(), Some("github:sharkdp/bat"));
        assert_eq!(gh("https://aur.archlinux.org/packages/paru-bin").as_deref(), Some("aur:paru-bin"));
        assert_eq!(gh("aur:yay").as_deref(), Some("aur:yay"));
        // Ordinary searches are left alone.
        for words in ["bat", "text editor", "a/b/c", "", "https://example.com", "../etc", "github:../x", "c++/thing?x"]
        {
            assert_eq!(gh(words), None, "{words:?}");
        }
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
