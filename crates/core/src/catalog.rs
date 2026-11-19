//! The store's brain: combines sources, the index and the compatibility engine into the
//! three questions a UI asks: what should I show, what matches this search, and what
//! exactly can this machine install from this app?

use crate::asset::classify;
use crate::compat::evaluate;
use crate::error::{Error, Result};
use crate::http::Http;
use crate::index::{Index, Section};
use crate::install;
use crate::model::{AppDetails, AppId, AppSummary, HiddenAsset, InstallOption, Release, ResolvedRelease, SourceKind};
use crate::platform::Platform;
use crate::source::{aur::Aur, github::GitHub, Source};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct SearchOutcome {
    /// Only apps with something installable on this machine.
    pub apps: Vec<AppSummary>,
    /// How many matches were dropped because nothing in their releases fits this machine.
    pub hidden: usize,
    /// One message per source that failed; the other sources' results are still shown.
    pub errors: Vec<String>,
}

pub struct Catalog {
    pub platform: Platform,
    sources: Vec<Arc<dyn Source>>,
    index: Index,
    http: Arc<dyn Http>,
    /// How many search hits per source get their releases checked. Each check is one
    /// API request, so this is the knob that trades completeness for rate limit.
    pub check_limit: usize,
    /// Treat pre-releases like stable releases (a user setting).
    pub include_prereleases: bool,
}

/// Result of checking one library entry for a newer release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    Available {
        version: String,
        label: String,
    },
    /// The check itself failed (offline, rate limited, project gone).
    Unknown(String),
}

impl Catalog {
    pub fn new(platform: Platform, http: Arc<dyn Http>, index: Index) -> Catalog {
        let sources: Vec<Arc<dyn Source>> = vec![Arc::new(GitHub::new(http.clone())), Arc::new(Aur::new(http.clone()))];
        Catalog { platform, sources, index, http, check_limit: 8, include_prereleases: false }
    }

    /// Assemble a catalog from explicit sources (other hosts, tests, future stores).
    pub fn with_sources(
        platform: Platform,
        http: Arc<dyn Http>,
        sources: Vec<Arc<dyn Source>>,
        index: Index,
    ) -> Catalog {
        Catalog { platform, sources, index, http, check_limit: 8, include_prereleases: false }
    }

    /// Talks to the real GitHub and AUR, caching responses under `cache_dir`.
    #[cfg(feature = "net")]
    pub fn live(platform: Platform, github_token: Option<String>, cache_dir: std::path::PathBuf) -> Catalog {
        use crate::http::{CachedHttp, NetHttp};
        use std::time::Duration;
        let has_token = github_token.as_deref().is_some_and(|t| !t.trim().is_empty());
        let http = CachedHttp::new(NetHttp::new(github_token), cache_dir.join("http"), Duration::from_secs(30 * 60));
        let mut catalog = Catalog::new(platform, Arc::new(http), Index::bundled());
        if has_token {
            catalog.check_limit = 20;
        }
        catalog
    }

    /// Self-contained catalog of fictional apps: no network, used for demos and tests.
    pub fn demo(platform: Platform) -> Catalog {
        Catalog::new(platform, Arc::new(crate::demo::DemoHttp), crate::demo::index())
    }

    pub fn http(&self) -> Arc<dyn Http> {
        self.http.clone()
    }

    fn source(&self, kind: SourceKind) -> Result<&Arc<dyn Source>> {
        self.sources.iter().find(|s| s.kind() == kind).filter(|s| s.available_on(&self.platform)).ok_or_else(|| {
            Error::Unsupported(format!("{} is not available on {}", kind.label(), self.platform.os_name))
        })
    }

    /// Sources that make sense on this machine.
    pub fn active_sources(&self) -> Vec<SourceKind> {
        self.sources.iter().filter(|s| s.available_on(&self.platform)).map(|s| s.kind()).collect()
    }

    /// The Discover page. Costs no network requests.
    pub fn featured(&self) -> Vec<Section> {
        let active = self.active_sources();
        let mut sections = self.index.sections(&self.platform);
        for s in &mut sections {
            s.apps.retain(|a| active.contains(&a.id.source));
        }
        sections.retain(|s| !s.apps.is_empty());
        sections
    }

    /// Sort a release's files into "works here" (best first) and "hidden, because ...".
    pub fn resolve_release(&self, release: &Release) -> ResolvedRelease {
        let mut options = Vec::new();
        let mut hidden = Vec::new();
        for asset in &release.assets {
            let Some(info) = asset.info.clone().or_else(|| classify(&asset.name)) else {
                // Checksums, signatures, sources: not worth a line in the UI.
                continue;
            };
            let verdict = evaluate(&info, &self.platform);
            if verdict.usable() {
                let plan = install::plan(asset, &info, &self.platform);
                options.push(InstallOption { asset: asset.clone(), info, verdict, plan });
            } else {
                hidden.push(HiddenAsset { name: asset.name.clone(), reason: verdict.note.unwrap_or_default() });
            }
        }
        options.sort_by(|a, b| b.verdict.score.cmp(&a.verdict.score).then_with(|| a.asset.name.cmp(&b.asset.name)));
        ResolvedRelease {
            version: release.version.clone(),
            published: release.published.clone(),
            prerelease: release.prerelease,
            notes: release.notes.clone(),
            options,
            hidden,
        }
    }

    /// The newest release with something for this machine: stable releases first, then
    /// pre-releases. If nothing fits, the newest release is returned with no options so
    /// the UI can explain why.
    pub fn pick_release(&self, releases: &[Release]) -> (Option<ResolvedRelease>, Option<String>) {
        if releases.is_empty() {
            return (None, Some("This project has not published any releases.".to_string()));
        }
        // Stable first, then pre-releases; or simply newest first when the user opted in.
        let passes: &[Option<bool>] = if self.include_prereleases { &[None] } else { &[Some(false), Some(true)] };
        for want in passes {
            for release in releases.iter().filter(|r| want.is_none_or(|w| r.prerelease == w)) {
                let resolved = self.resolve_release(release);
                if !resolved.options.is_empty() {
                    return (Some(resolved), None);
                }
            }
        }
        let newest = self.resolve_release(&releases[0]);
        let reason = format!("None of the files in recent releases work on {}.", self.platform.title());
        (Some(newest), Some(reason))
    }

    pub fn details(&self, id: &AppId) -> Result<AppDetails> {
        let source = self.source(id.source)?;
        let mut summary = source.summary(&id.key)?;
        let (release, unavailable_reason) = self.pick_release(&source.releases(&id.key)?);
        if let Some(r) = &release {
            summary.version = Some(r.version.clone());
            summary.install_label = r.options.first().map(|o| o.label());
        }
        Ok(AppDetails { summary, release, unavailable_reason })
    }

    /// Is there a newer release than `installed_version` with a file for this machine?
    /// One request per app.
    pub fn check_update(&self, id: &AppId, installed_version: &str) -> UpdateStatus {
        let releases = match self.source(id.source).and_then(|s| s.releases(&id.key)) {
            Ok(releases) => releases,
            Err(e) => return UpdateStatus::Unknown(e.to_string()),
        };
        match self.pick_release(&releases) {
            (Some(release), None) if crate::version::is_newer(&release.version, installed_version) => {
                let label = release.options.first().map(|o| o.label()).unwrap_or_default();
                UpdateStatus::Available { version: release.version, label }
            }
            _ => UpdateStatus::UpToDate,
        }
    }

    /// Check many apps at once, in parallel. The result is in the same order as `apps`.
    pub fn check_updates(&self, apps: &[(AppId, String)]) -> Vec<UpdateStatus> {
        std::thread::scope(|scope| {
            let handles: Vec<_> =
                apps.iter().map(|(id, version)| scope.spawn(move || self.check_update(id, version))).collect();
            handles.into_iter().map(|h| h.join().expect("update check panicked")).collect()
        })
    }

    /// Check one search hit against this machine. `Ok(None)` means "nothing for you here".
    fn check(&self, source: &dyn Source, mut app: AppSummary) -> Result<Option<AppSummary>> {
        let releases = source.releases(&app.id.key)?;
        let (release, reason) = self.pick_release(&releases);
        match (release, reason) {
            (Some(r), None) => {
                app.version = Some(r.version.clone());
                app.install_label = r.options.first().map(|o| o.label());
                Ok(Some(app))
            }
            _ => Ok(None),
        }
    }

    pub fn search(&self, query: &str) -> SearchOutcome {
        let query = query.trim();
        let mut outcome = SearchOutcome::default();
        if query.is_empty() {
            return outcome;
        }
        let mut per_source: Vec<Vec<AppSummary>> = Vec::new();
        for source in self.sources.iter().filter(|s| s.available_on(&self.platform)) {
            let hits = match source.search(query, self.check_limit) {
                Ok(hits) => hits,
                Err(e) => {
                    outcome.errors.push(format!("{}: {e}", source.kind().label()));
                    continue;
                }
            };
            // Each hit is checked in parallel: one request each, bounded by check_limit.
            let checked: Vec<Result<Option<AppSummary>>> = std::thread::scope(|scope| {
                let handles: Vec<_> = hits
                    .into_iter()
                    .take(self.check_limit)
                    .map(|app| scope.spawn(move || self.check(source.as_ref(), app)))
                    .collect();
                handles.into_iter().map(|h| h.join().expect("check thread panicked")).collect()
            });
            let mut kept = Vec::new();
            let mut first_error = None;
            for result in checked {
                match result {
                    Ok(Some(app)) => kept.push(app),
                    Ok(None) => outcome.hidden += 1,
                    // A repository without releases answers 404-free with [], so errors
                    // here are real (rate limit, network). Report once per source.
                    Err(e) => first_error = first_error.or(Some(e)),
                }
            }
            if let Some(e) = first_error {
                outcome.errors.push(format!("{}: {e}", source.kind().label()));
            }
            per_source.push(kept);
        }
        // Interleave so no source buries the other; each list is already ranked by its source.
        let longest = per_source.iter().map(Vec::len).max().unwrap_or(0);
        for i in 0..longest {
            for list in &per_source {
                if let Some(app) = list.get(i) {
                    outcome.apps.push(app.clone());
                }
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::Format;
    use crate::compat::Level;

    fn demo(preset: &str) -> Catalog {
        Catalog::demo(Platform::preset(preset).unwrap())
    }
    fn id(s: &str) -> AppId {
        AppId::parse(s).unwrap()
    }
    fn best(preset: &str, app: &str) -> String {
        let d = demo(preset).details(&id(app)).unwrap();
        d.release.unwrap().options.first().map(|o| o.asset.name.clone()).unwrap_or_default()
    }

    #[test]
    fn details_offer_the_right_file_per_machine() {
        let app = "github:tidewave/tidewave";
        assert_eq!(best("windows-x64", app), "Tidewave-3.4.0-x64.msi");
        assert_eq!(best("linux-x64-debian", app), "tidewave_3.4.0_amd64.deb");
        assert_eq!(best("linux-x64-fedora", app), "tidewave-3.4.0-1.x86_64.rpm");
        assert_eq!(best("macos-arm64", app), "Tidewave-3.4.0-universal.dmg");
        assert_eq!(best("android-arm64", app), "tidewave-3.4.0-arm64-v8a.apk");
        assert_eq!(best("android-tv", app), "tidewave-3.4.0-androidtv-arm64-v8a.apk");
        assert_eq!(best("ios", app), "Tidewave-3.4.0.ipa");
    }

    #[test]
    fn every_listed_option_is_usable_and_hidden_ones_say_why() {
        for preset in crate::platform::PRESETS {
            let d = demo(preset).details(&id("github:tidewave/tidewave")).unwrap();
            let r = d.release.unwrap();
            assert!(!r.options.is_empty(), "{preset}");
            assert!(r.options.iter().all(|o| o.verdict.level != Level::Incompatible), "{preset}");
            assert!(!r.hidden.is_empty(), "{preset}");
            assert!(r.hidden.iter().all(|h| !h.reason.is_empty()), "{preset}");
            // Checksums never show up on either list.
            assert!(r.hidden.iter().all(|h| h.name != "SHA256SUMS"));
        }
    }

    #[test]
    fn app_without_a_build_for_this_machine_explains_itself() {
        let d = demo("linux-x64-debian").details(&id("github:winspect/winspect")).unwrap();
        assert!(d.unavailable_reason.unwrap().contains("Linux"));
        assert!(d.release.unwrap().options.is_empty());
    }

    #[test]
    fn falls_back_to_an_older_release_that_has_a_build() {
        // kestrel's newest release has no macOS files yet; the previous one does.
        let d = demo("macos-arm64").details(&id("github:kestrel-sh/kestrel")).unwrap();
        let r = d.release.unwrap();
        assert_eq!(r.version, "v1.20.0");
        assert!(d.unavailable_reason.is_none());
        let d = demo("linux-x64-debian").details(&id("github:kestrel-sh/kestrel")).unwrap();
        assert_eq!(d.release.unwrap().version, "v1.21.0");
    }

    #[test]
    fn update_checks() {
        let c = demo("linux-x64-debian");
        let app = id("github:kestrel-sh/kestrel");
        assert_eq!(c.check_update(&app, "v1.21.0"), UpdateStatus::UpToDate);
        assert_eq!(c.check_update(&app, "v9.0.0"), UpdateStatus::UpToDate, "never offer a downgrade");
        assert_eq!(
            c.check_update(&app, "v1.20.0"),
            UpdateStatus::Available { version: "v1.21.0".into(), label: "Portable archive · x86_64".into() }
        );
        // On macOS the newest release has no files, so v1.20.0 is still the latest that works.
        assert_eq!(demo("macos-arm64").check_update(&app, "v1.20.0"), UpdateStatus::UpToDate);
        assert!(matches!(c.check_update(&id("github:nobody/nothing"), "v1"), UpdateStatus::Unknown(_)));

        let many =
            c.check_updates(&[(app.clone(), "v1.20.0".into()), (id("github:tidewave/tidewave"), "v3.4.0".into())]);
        assert!(matches!(many[0], UpdateStatus::Available { .. }));
        assert_eq!(many[1], UpdateStatus::UpToDate);
    }

    #[test]
    fn prereleases_are_opt_in() {
        use crate::model::{Asset, Release};
        let release = |version: &str, prerelease: bool| Release {
            version: version.to_string(),
            published: String::new(),
            prerelease,
            notes: String::new(),
            assets: vec![Asset {
                name: "app-x86_64.AppImage".into(),
                size: 1,
                url: String::new(),
                digest: None,
                info: None,
            }],
        };
        let releases = [release("v2.0.0-beta.1", true), release("v1.9.0", false)];
        let mut c = demo("linux-x64-debian");
        assert_eq!(c.pick_release(&releases).0.unwrap().version, "v1.9.0");
        c.include_prereleases = true;
        assert_eq!(c.pick_release(&releases).0.unwrap().version, "v2.0.0-beta.1");
        // A project that only ever ships pre-releases is still installable by default.
        c.include_prereleases = false;
        assert_eq!(c.pick_release(&releases[..1]).0.unwrap().version, "v2.0.0-beta.1");
    }

    #[test]
    fn search_only_returns_installable_apps() {
        let on = |preset: &str, q: &str| -> Vec<String> {
            demo(preset).search(q).apps.into_iter().map(|a| a.id.to_string()).collect()
        };
        // "monitor" matches a macOS-only and a Windows-only app.
        assert_eq!(on("macos-arm64", "monitor"), vec!["github:barley/barley"]);
        assert_eq!(on("windows-x64", "monitor"), vec!["github:winspect/winspect"]);
        assert!(on("android-arm64", "monitor").is_empty());
        assert_eq!(demo("android-arm64").search("monitor").hidden, 2);
    }

    #[test]
    fn aur_only_appears_on_arch() {
        let sources = |preset: &str| -> Vec<SourceKind> {
            demo(preset).search("lumen").apps.into_iter().map(|a| a.id.source).collect()
        };
        assert_eq!(sources("linux-x64-arch"), vec![SourceKind::GitHub, SourceKind::Aur, SourceKind::Aur]);
        assert_eq!(sources("linux-x64-debian"), vec![SourceKind::GitHub]);
        assert_eq!(sources("windows-x64"), vec![SourceKind::GitHub]);
        assert!(demo("linux-x64-debian").details(&id("aur:lumen-bin")).is_err());

        let d = demo("linux-x64-arch").details(&id("aur:lumen-bin")).unwrap();
        let r = d.release.unwrap();
        assert_eq!(r.options[0].info.format, Format::AurBuild);
        assert_eq!(r.options[0].plan.command.as_deref(), Some("paru -S lumen-bin"));
    }

    #[test]
    fn featured_is_filtered_and_tv_gets_its_own_row() {
        let titles = |preset: &str| -> Vec<String> { demo(preset).featured().into_iter().map(|s| s.title).collect() };
        assert_eq!(titles("android-tv")[0], "Made for TV");
        assert!(!titles("windows-x64").contains(&"Made for TV".to_string()));
        assert!(titles("linux-x64-arch").contains(&"Arch User Repository".to_string()));
        assert!(!titles("linux-x64-debian").contains(&"Arch User Repository".to_string()));
        for preset in crate::platform::PRESETS {
            let catalog = demo(preset);
            // Everything Discover lists must really have a file for this machine.
            for section in catalog.featured() {
                for app in section.apps {
                    let d = catalog.details(&app.id).unwrap();
                    assert!(d.unavailable_reason.is_none(), "{preset}: {} listed but not installable", app.id);
                }
            }
        }
    }
}
