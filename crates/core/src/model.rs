//! Plain data shared by sources, the catalog and the UIs.

use crate::asset::AssetInfo;
use crate::compat::Verdict;
use crate::install::InstallPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    GitHub,
    /// Flathub, the Flatpak app repository.
    Flathub,
    /// Codeberg (a Forgejo instance).
    Codeberg,
    Aur,
    /// Repositories configured for pacman on this machine (Arch, CachyOS, ...).
    Pacman,
    /// Repositories configured for apt on this machine (Debian, Ubuntu, PPAs, ...).
    Apt,
    /// Repositories configured for dnf on this machine (Fedora and its relatives).
    Dnf,
    /// Repositories configured for zypper on this machine (openSUSE).
    Zypper,
    /// Websites opened like apps on a Linux TV.
    Web,
    /// GitLab.com projects that publish files with their releases.
    GitLab,
    /// Homebrew casks: Mac apps, with the address and checksum of each download.
    Homebrew,
    /// The Windows Package Manager's community repository.
    Winget,
    /// The Snap Store.
    Snap,
    /// F-Droid, the catalog of free Android apps.
    FDroid,
    /// Steam. The store hands over to the Steam app, which installs and runs things.
    Steam,
    /// The Mac App Store, handed over to in the same way.
    MacAppStore,
}

impl SourceKind {
    /// A code host whose projects publish release files (as opposed to a package repository).
    pub fn is_forge(self) -> bool {
        matches!(self, SourceKind::GitHub | SourceKind::Codeberg | SourceKind::GitLab)
    }
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::GitHub => "GitHub",
            SourceKind::Flathub => "Flathub",
            SourceKind::Codeberg => "Codeberg",
            SourceKind::Aur => "AUR",
            SourceKind::Pacman => "pacman repositories",
            SourceKind::Apt => "apt repositories",
            SourceKind::Dnf => "dnf repositories",
            SourceKind::Zypper => "zypper repositories",
            SourceKind::Web => "Web apps",
            SourceKind::GitLab => "GitLab",
            SourceKind::Homebrew => "Homebrew",
            SourceKind::Winget => "winget",
            SourceKind::Snap => "Snap Store",
            SourceKind::FDroid => "F-Droid",
            SourceKind::Steam => "Steam",
            SourceKind::MacAppStore => "Mac App Store",
        }
    }
    pub fn prefix(self) -> &'static str {
        match self {
            SourceKind::GitHub => "github",
            SourceKind::Flathub => "flathub",
            SourceKind::Codeberg => "codeberg",
            SourceKind::Aur => "aur",
            SourceKind::Pacman => "pacman",
            SourceKind::Apt => "apt",
            SourceKind::Dnf => "dnf",
            SourceKind::Zypper => "zypper",
            SourceKind::Web => "web",
            SourceKind::GitLab => "gitlab",
            SourceKind::Homebrew => "brew",
            SourceKind::Winget => "winget",
            SourceKind::Snap => "snap",
            SourceKind::FDroid => "fdroid",
            SourceKind::Steam => "steam",
            SourceKind::MacAppStore => "mas",
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
            "flathub" => SourceKind::Flathub,
            "codeberg" => SourceKind::Codeberg,
            "aur" => SourceKind::Aur,
            "pacman" => SourceKind::Pacman,
            "apt" => SourceKind::Apt,
            "dnf" => SourceKind::Dnf,
            "zypper" => SourceKind::Zypper,
            "web" => SourceKind::Web,
            "gitlab" => SourceKind::GitLab,
            "brew" => SourceKind::Homebrew,
            "winget" => SourceKind::Winget,
            "snap" => SourceKind::Snap,
            "fdroid" => SourceKind::FDroid,
            "steam" => SourceKind::Steam,
            "mas" => SourceKind::MacAppStore,
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
        if let Some(rest) = stripped.strip_prefix("flathub.org/") {
            // flathub.org/apps/org.example.App, with an optional language in front.
            let name = rest.split(['?', '#']).next().unwrap_or("").split('/').rfind(|p| !p.is_empty()).unwrap_or("");
            return valid_key(SourceKind::Flathub, name).then(|| AppId::new(SourceKind::Flathub, name));
        }
        if let Some(rest) = stripped.strip_prefix("gitlab.com/") {
            // gitlab.com/group/subgroup/project, up to the "/-/" that starts a page inside it.
            let path = rest.split(['?', '#']).next().unwrap_or("").split("/-/").next().unwrap_or("");
            let key = path.trim_matches('/').trim_end_matches(".git").to_string();
            return valid_key(SourceKind::GitLab, &key).then(|| AppId::new(SourceKind::GitLab, key));
        }
        let last = |rest: &str| -> String {
            rest.split(['?', '#']).next().unwrap_or("").split('/').rfind(|p| !p.is_empty()).unwrap_or("").to_string()
        };
        if let Some(rest) = stripped.strip_prefix("store.steampowered.com/app/") {
            let id = rest.split(['/', '?', '#']).next().unwrap_or("");
            return valid_key(SourceKind::Steam, id).then(|| AppId::new(SourceKind::Steam, id));
        }
        if let Some(rest) = stripped.strip_prefix("apps.apple.com/") {
            // apps.apple.com/us/app/some-name/id123456789
            let id =
                rest.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("").trim_start_matches("id");
            return valid_key(SourceKind::MacAppStore, id).then(|| AppId::new(SourceKind::MacAppStore, id));
        }
        if let Some(rest) = stripped.strip_prefix("f-droid.org/") {
            let name = last(rest);
            return valid_key(SourceKind::FDroid, &name).then(|| AppId::new(SourceKind::FDroid, name));
        }
        if let Some(rest) = stripped.strip_prefix("snapcraft.io/") {
            let name = last(rest);
            return valid_key(SourceKind::Snap, &name).then(|| AppId::new(SourceKind::Snap, name));
        }
        if let Some(rest) = stripped.strip_prefix("formulae.brew.sh/cask/") {
            let name = last(rest);
            return valid_key(SourceKind::Homebrew, &name).then(|| AppId::new(SourceKind::Homebrew, name));
        }
        if let Some(rest) = stripped.strip_prefix("codeberg.org/") {
            let mut parts = rest.split(['?', '#']).next().unwrap_or("").split('/').filter(|p| !p.is_empty());
            let key = format!("{}/{}", parts.next()?, parts.next()?.trim_end_matches(".git"));
            return valid_key(SourceKind::Codeberg, &key).then(|| AppId::new(SourceKind::Codeberg, key));
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
        SourceKind::GitHub | SourceKind::Codeberg => {
            matches!(key.split_once('/'), Some((owner, repo)) if ok(owner, &[]) && ok(repo, &[]))
        }
        // group/project, or group/subgroup/.../project.
        SourceKind::GitLab => {
            let parts: Vec<&str> = key.split('/').collect();
            (2..=6).contains(&parts.len()) && parts.iter().all(|part| ok(part, &[]))
        }
        // Arch and Debian package names may also contain `+` and `@`; so may cask names.
        SourceKind::Aur
        | SourceKind::Pacman
        | SourceKind::Apt
        | SourceKind::Dnf
        | SourceKind::Zypper
        | SourceKind::Homebrew => ok(key, &['+', '@']),
        SourceKind::Snap => key.len() <= 40 && ok(key, &[]),
        // A number the store gave the app.
        SourceKind::Steam | SourceKind::MacAppStore => {
            (1..=12).contains(&key.len()) && key.chars().all(|c| c.is_ascii_digit())
        }
        // Publisher.Package, as winget writes them.
        SourceKind::Winget => key.len() <= 128 && key.contains('.') && ok(key, &['+']),
        // An Android package name: at least two dotted parts.
        SourceKind::FDroid => {
            key.len() <= 255 && key.split('.').count() >= 2 && key.split('.').all(|part| ok(part, &[]))
        }
        // A reverse-DNS application id: at least three dotted parts.
        SourceKind::Flathub => {
            key.len() <= 255 && key.split('.').count() >= 3 && key.split('.').all(|part| ok(part, &[]))
        }
        // A host name: labels of letters, digits and hyphens, with at least one dot.
        SourceKind::Web => {
            key.len() <= 253
                && key.contains('.')
                && key.split('.').all(|label| {
                    !label.is_empty()
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                })
        }
    }
}

impl std::fmt::Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.source.prefix(), self.key)
    }
}

impl AppSummary {
    /// An app known only by id and name; the source fills in what else it has.
    pub fn new(id: AppId, name: impl Into<String>) -> AppSummary {
        AppSummary {
            id,
            name: name.into(),
            author: String::new(),
            summary: String::new(),
            popularity: None,
            homepage: String::new(),
            license: None,
            category: None,
            version: None,
            install_label: None,
            warning: None,
            origin: None,
            installed_version: None,
            rating: None,
            icon_url: None,
            downloads: None,
            verified: false,
            momentum: None,
            price: None,
        }
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
    /// Where exactly it comes from, when the source has several places: the repository
    /// name (`cachyos-v3`, `extra`, `noble-updates/main`).
    pub origin: Option<String>,
    /// The version the system's package manager already has installed, if any.
    pub installed_version: Option<String>,
    /// Star rating from people who use it, where a ratings service knows the app.
    pub rating: Option<Rating>,
    /// Address of the app's own icon, when the source gives one.
    pub icon_url: Option<String>,
    /// How many times its recent releases' files were downloaded, where the host counts.
    pub downloads: Option<u64>,
    /// The source has confirmed that the app is published by its own developer.
    pub verified: bool,
    /// Recent interest on a 0..1 scale, where the source measures it (the AUR's
    /// "popularity", which fades when people stop voting).
    pub momentum: Option<f32>,
    /// What it costs where it is sold, as that store words it: "Free", "$9.99". `None`
    /// for sources where everything is free of charge.
    pub price: Option<String>,
}

/// An average star rating and how many people gave one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rating {
    /// 0.0 to 5.0.
    pub stars: f32,
    pub count: u32,
    /// How many gave one star, two, ... five.
    pub spread: [u32; 5],
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
    /// How many times this release's files were downloaded, where the host counts.
    pub downloads: Option<u64>,
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
    /// The file itself, so it can still be downloaded by someone who knows better.
    pub asset: Asset,
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
    /// Every recent release that has something for this machine, newest first, so an
    /// older version can be chosen. Includes `release`.
    pub versions: Vec<ResolvedRelease>,
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

/// Days since the Unix epoch for a `YYYY-MM-DD` date (Howard Hinnant's days-from-civil).
fn days_from_date(date: &str) -> Option<i64> {
    let mut parts = date.get(..10)?.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// "today", "3 days ago", "5 months ago", "2 years ago" for a `YYYY-MM-DD` date.
/// Empty when the date cannot be read or lies in the future.
pub fn age_label(date: &str, now_unix: u64) -> String {
    let Some(then) = days_from_date(date) else { return String::new() };
    let days = (now_unix / 86_400) as i64 - then;
    let plural = |n: i64, unit: &str| if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") };
    match days {
        d if d < 0 => String::new(),
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        d if d < 60 => plural(d, "day"),
        d if d < 730 => plural(d / 30, "month"),
        d => plural(d / 365, "year"),
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
        assert_eq!(gh("https://codeberg.org/forgejo/forgejo").as_deref(), Some("codeberg:forgejo/forgejo"));
        assert_eq!(gh("codeberg.org/fern/notes/releases").as_deref(), Some("codeberg:fern/notes"));
        assert_eq!(gh("codeberg:fern/notes").as_deref(), Some("codeberg:fern/notes"));
        assert_eq!(gh("https://codeberg.org/fern"), None);
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
    fn ages() {
        let now = 1_700_000_000; // 2023-11-14
        assert_eq!(age_label("2023-11-14", now), "today");
        assert_eq!(age_label("2023-11-13T23:00:00Z", now), "yesterday");
        assert_eq!(age_label("2023-11-04", now), "10 days ago");
        assert_eq!(age_label("2023-08-14", now), "3 months ago");
        assert_eq!(age_label("2020-11-14", now), "3 years ago");
        assert_eq!(age_label("2024-01-01", now), "", "future dates say nothing");
        assert_eq!(age_label("", now), "");
        assert_eq!(age_label("not-a-date", now), "");
        assert_eq!(age_label("2023-13-40", now), "");
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
