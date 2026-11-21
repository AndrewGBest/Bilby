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
use crate::source::system::{Backend, SystemRepos};
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
    /// Sources the person switched off in Settings: not searched, not on the home page.
    /// An app from one can still be opened by its link.
    pub disabled: Vec<SourceKind>,
    /// Family mode and whether adult things may be on the home page. See `parental`.
    pub parental: crate::parental::Policy,
    /// Star ratings, once loaded, and the catalog that links packages to rated ids.
    /// Shared, so a rebuilt catalog keeps what an earlier one fetched.
    ratings: Arc<std::sync::RwLock<Option<Rated>>>,
    /// Where the distribution's AppStream catalog is (empty in the demo and tests).
    pub appstream_dirs: Vec<std::path::PathBuf>,
}

struct Rated {
    ratings: crate::ratings::Ratings,
    components: crate::appstream::Components,
}

/// One line for the search page: which source failed and why, without stuttering
/// ("GitHub: GitHub's request limit ...").
fn describe_source_error(source: SourceKind, error: &Error) -> String {
    match error {
        Error::RateLimited { .. } => error.to_string(),
        other => format!("{} search failed: {other}", source.label()),
    }
}

/// How many times the files of an app's recent releases were downloaded, when its host
/// counts. `None` when it does not, which is different from nobody having downloaded it.
fn downloads_of(releases: &[Release]) -> Option<u64> {
    let counted: Vec<u64> = releases.iter().filter_map(|r| r.downloads).collect();
    (!counted.is_empty()).then(|| counted.iter().sum())
}

/// Add a failed source to what the user is told, once per distinct problem: being
/// offline fails every source the same way, and one line says it.
fn report(errors: &mut Vec<String>, offline: &mut bool, source: SourceKind, error: &Error) {
    let network = matches!(error, Error::Network(_));
    let message = describe_source_error(source, error);
    if !(network && *offline) && !errors.contains(&message) {
        errors.push(message);
    }
    *offline |= network;
}

/// Where the store itself is published. Its own releases are found, filtered and
/// installed exactly like any other app's.
pub const STORE_REPOSITORY: &str = "AndrewGBest/bilby";

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
        let sources: Vec<Arc<dyn Source>> = vec![
            Arc::new(GitHub::new(http.clone())),
            Arc::new(crate::source::flathub::Flathub::new(http.clone(), Vec::new())),
            Arc::new(crate::source::forgejo::Forgejo::codeberg(http.clone())),
            Arc::new(Aur::new(http.clone())),
            Arc::new(crate::source::web::Websites),
        ];
        Catalog {
            platform,
            sources,
            index,
            http,
            check_limit: 8,
            include_prereleases: false,
            disabled: Vec::new(),
            parental: crate::parental::Policy::default(),
            ratings: Arc::default(),
            appstream_dirs: Vec::new(),
        }
    }

    /// Tell the catalog where Flatpak keeps installed apps, so Flathub apps that are
    /// already on the device are marked as such.
    pub fn use_flatpak_roots(&mut self, roots: Vec<std::path::PathBuf>) {
        let flathub = Arc::new(crate::source::flathub::Flathub::new(self.http.clone(), roots));
        if let Some(slot) = self.sources.iter_mut().find(|s| s.kind() == SourceKind::Flathub) {
            *slot = flathub;
        }
    }

    /// Assemble a catalog from explicit sources (other hosts, tests, future stores).
    pub fn with_sources(
        platform: Platform,
        http: Arc<dyn Http>,
        sources: Vec<Arc<dyn Source>>,
        index: Index,
    ) -> Catalog {
        Catalog {
            platform,
            sources,
            index,
            http,
            check_limit: 8,
            include_prereleases: false,
            disabled: Vec::new(),
            parental: crate::parental::Policy::default(),
            ratings: Arc::default(),
            appstream_dirs: Vec::new(),
        }
    }

    /// Talks to the real GitHub and AUR, caching responses under `cache_dir`.
    #[cfg(feature = "net")]
    pub fn live(platform: Platform, github_token: Option<String>, cache_dir: std::path::PathBuf) -> Catalog {
        use crate::http::{CachedHttp, NetHttp};
        use std::time::Duration;
        let has_token = github_token.as_deref().is_some_and(|t| !t.trim().is_empty());
        let http = CachedHttp::new(NetHttp::new(github_token), cache_dir.join("http"), Duration::from_secs(30 * 60));
        let mut catalog = Catalog::new(platform.clone(), Arc::new(http), Index::bundled());
        // The repositories this machine's own package manager is set up for come first:
        // they are maintained, signed and kept up to date by the distribution.
        for backend in
            [Backend::system_pacman(), Backend::system_apt(), Backend::system_dnf(), Backend::system_zypper()]
        {
            catalog.sources.insert(0, Arc::new(SystemRepos::new(backend, platform.clone())));
        }
        // More places apps are published. They are asked only by the live catalog; the
        // demo has no stand-in for them.
        let shared = catalog.http.clone();
        catalog.sources.push(Arc::new(crate::source::gitlab::GitLab::new(shared.clone())));
        catalog.sources.push(Arc::new(crate::source::homebrew::Homebrew::new(shared.clone(), &platform)));
        catalog.sources.push(Arc::new(crate::source::snap::SnapStore::new(shared.clone())));
        catalog.sources.push(Arc::new(crate::source::fdroid::FDroid::new(shared)));
        catalog.sources.push(Arc::new(crate::source::winget::Winget::new()));
        // Other stores, handed over to: they install and run what they sell.
        let shared = catalog.http.clone();
        catalog.sources.push(Arc::new(crate::source::steam::Steam::new(shared.clone(), &platform)));
        catalog.sources.push(Arc::new(crate::source::appstore::MacAppStore::new(shared)));
        if has_token {
            catalog.check_limit = 20;
        }
        catalog.use_flatpak_roots(crate::install::Dirs::for_current_user().flatpak_roots);
        catalog.appstream_dirs = crate::appstream::system_dirs();
        catalog
    }

    /// Self-contained catalog of fictional apps: no network, used for demos and tests.
    pub fn demo(platform: Platform) -> Catalog {
        let mut catalog = Catalog::new(platform.clone(), Arc::new(crate::demo::DemoHttp), crate::demo::index());
        // Fictional system repositories, so the demo never reads the real machine's.
        for backend in crate::demo::system_repos() {
            catalog.sources.insert(0, Arc::new(SystemRepos::new(backend, platform.clone())));
        }
        catalog
    }

    /// GitHub's remaining request allowance, once known.
    pub fn quota(&self) -> Option<crate::http::Quota> {
        self.http.quota()
    }

    pub fn http(&self) -> Arc<dyn Http> {
        self.http.clone()
    }

    fn source(&self, kind: SourceKind) -> Result<&Arc<dyn Source>> {
        self.sources.iter().find(|s| s.kind() == kind).filter(|s| s.available_on(&self.platform)).ok_or_else(|| {
            Error::Unsupported(format!("{} is not available on {}", kind.label(), self.platform.os_name))
        })
    }

    /// The names of the system repositories being read (`cachyos-v3`, `extra`, ...).
    /// Reads the package databases if that has not happened yet.
    pub fn system_repositories(&self) -> Vec<String> {
        self.sources.iter().filter(|s| self.uses(s.as_ref())).flat_map(|s| s.origins()).collect()
    }

    /// Sources that make sense on this machine.
    /// A source that applies to this device and has not been switched off.
    fn uses(&self, source: &dyn Source) -> bool {
        source.available_on(&self.platform) && !self.disabled.contains(&source.kind())
    }

    /// Every source that applies to this device, switched off or not.
    pub fn available_sources(&self) -> Vec<SourceKind> {
        self.sources.iter().filter(|s| s.available_on(&self.platform)).map(|s| s.kind()).collect()
    }

    pub fn active_sources(&self) -> Vec<SourceKind> {
        self.sources.iter().filter(|s| self.uses(s.as_ref())).map(|s| s.kind()).collect()
    }

    /// The Discover page. Costs no network requests.
    pub fn featured(&self) -> Vec<Section> {
        let active = self.active_sources();
        let mut sections = self.index.sections(&self.platform);
        for s in &mut sections {
            s.apps.retain(|a| active.contains(&a.id.source));
        }
        sections.retain(|s| !s.apps.is_empty());
        // On a Linux TV, websites sit beside the apps.
        if active.contains(&SourceKind::Web) {
            sections.push(Section { title: "Web apps".to_string(), apps: crate::source::web::featured() });
        }
        sections
    }

    /// Lists the sources themselves keep, such as what is popular on Flathub. Unlike
    /// [`Catalog::featured`] this uses the network, so call it off the UI thread.
    /// Names of the packages the system's own package manager has installed.
    pub fn installed_packages(&self) -> Vec<String> {
        self.sources.iter().flat_map(|source| source.installed_names()).collect()
    }

    pub fn collections(&self, limit: usize) -> Vec<Section> {
        let mut sections = Vec::new();
        for source in &self.sources {
            if !self.uses(source.as_ref()) {
                continue;
            }
            for (title, mut apps) in source.collections(limit) {
                // The home page carries adult things only where the person asked for that.
                if self.parental.family_mode || !self.parental.adult_on_home {
                    apps.retain(|app| !app.adult);
                }
                for app in &mut apps {
                    app.rating = app.rating.or_else(|| self.rating_for(&app.id));
                }
                sections.push(Section { title, apps });
            }
        }
        sections
    }

    /// Featured apps matching what has been typed so far. Instant and free: it only looks
    /// at the bundled index, so it can run on every keystroke.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<AppSummary> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.iter().map(String::len).sum::<usize>() < 2 {
            return Vec::new();
        }
        let mut scored: Vec<(u8, AppSummary)> = Vec::new();
        for app in self.featured().into_iter().flat_map(|s| s.apps) {
            if scored.iter().any(|(_, seen)| seen.id == app.id) {
                continue;
            }
            // Match on word starts ("ed" finds "editor", not "media" or "self-hosted"):
            // substring matches feel random when only two letters have been typed.
            let name = app.name.to_lowercase();
            let rest =
                format!("{} {} {}", app.author, app.summary, app.category.clone().unwrap_or_default()).to_lowercase();
            let starts =
                |text: &str, w: &str| text.split(|c: char| !c.is_alphanumeric()).any(|word| word.starts_with(w));
            if !words.iter().all(|w| starts(&name, w) || starts(&rest, w)) {
                continue;
            }
            // The app's own name first, then matches in its description.
            let rank = if name.starts_with(&words[0]) {
                0
            } else if words.iter().any(|w| starts(&name, w)) {
                1
            } else {
                2
            };
            scored.push((rank, app));
        }
        scored.sort_by_key(|(rank, _)| *rank);
        scored.into_iter().take(limit).map(|(_, app)| app).collect()
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
                hidden.push(HiddenAsset {
                    name: asset.name.clone(),
                    reason: verdict.note.unwrap_or_default(),
                    asset: asset.clone(),
                });
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

    /// The project's README as Markdown, or `None` when there is none to show.
    pub fn about(&self, id: &AppId) -> Result<Option<String>> {
        self.source(id.source)?.about(&id.key)
    }

    /// Fetch star ratings (at most once a day; see `ratings`) and read the system's
    /// catalog that links packages to rated apps. Returns whether ratings are now known.
    pub fn load_ratings(&self, cache_dir: &std::path::Path) -> bool {
        if self.ratings.read().unwrap().is_some() {
            return true;
        }
        let Ok(ratings) = crate::ratings::Ratings::load(self.http.as_ref(), cache_dir) else { return false };
        let components = crate::appstream::Components::load(&self.appstream_dirs);
        *self.ratings.write().unwrap() = Some(Rated { ratings, components });
        true
    }

    /// Forget the ratings (the person turned them off).
    pub fn drop_ratings(&self) {
        *self.ratings.write().unwrap() = None;
    }

    /// Keep using the ratings another catalog already loaded.
    pub fn share_ratings_with(&mut self, other: &Catalog) {
        self.ratings = other.ratings.clone();
    }

    /// The star rating of an app, when ratings are loaded and the app can be matched to
    /// a rated id: directly for Flathub, through the system's catalog for packages and
    /// GitHub projects.
    pub fn rating_for(&self, id: &AppId) -> Option<crate::model::Rating> {
        self.rated_as(id).map(|(_, rating)| rating)
    }

    /// The id the ratings service knows the app by, and its rating. `None` when
    /// ratings are off, or the service does not know the app.
    fn rated_as(&self, id: &AppId) -> Option<(String, crate::model::Rating)> {
        let guard = self.ratings.read().unwrap();
        let rated = guard.as_ref()?;
        let appstream_id = match id.source {
            SourceKind::Flathub => id.key.as_str(),
            SourceKind::Pacman | SourceKind::Apt | SourceKind::Dnf | SourceKind::Zypper => {
                rated.components.id_of_package(&id.key)?
            }
            SourceKind::GitHub => rated.components.id_of_repo(&id.key)?,
            SourceKind::Codeberg
            | SourceKind::Aur
            | SourceKind::Web
            | SourceKind::GitLab
            | SourceKind::Homebrew
            | SourceKind::Winget
            | SourceKind::Snap
            | SourceKind::FDroid
            | SourceKind::Steam
            | SourceKind::MacAppStore => return None,
        };
        Some((appstream_id.to_string(), rated.ratings.get(appstream_id)?))
    }

    /// What people wrote about the app. Asked for only when the ratings service is
    /// known to have the app; anything else returns nothing without a request.
    pub fn reviews(&self, id: &AppId) -> Vec<crate::ratings::Review> {
        match self.rated_as(id) {
            Some((appstream_id, _)) => {
                crate::ratings::fetch_reviews(self.http.as_ref(), &appstream_id).unwrap_or_default()
            }
            None => Vec::new(),
        }
    }

    /// Screenshots the source publishes for the app (Flathub does; release pages do not).
    pub fn pictures(&self, id: &AppId) -> Vec<String> {
        self.source(id.source).map(|s| s.pictures(&id.key)).unwrap_or_default()
    }

    /// The app as its own source describes it, without its releases: a quick way to
    /// ask whether a source has something by that name.
    pub fn source_summary(&self, id: &AppId) -> Result<AppSummary> {
        self.source(id.source)?.summary(&id.key)
    }

    pub fn details(&self, id: &AppId) -> Result<AppDetails> {
        let source = self.source(id.source)?;
        // "Not found" in a person's terms, not as the URL that was asked for.
        let named = |e: Error| match e {
            Error::NotFound(_) => {
                let what = if id.source.is_forge() { "project" } else { "package" };
                Error::NotFound(format!("{} has no {what} called “{}”.", id.source.label(), id.key))
            }
            other => other,
        };
        let mut summary = source.summary(&id.key).map_err(named)?;
        if summary.adult && self.parental.family_mode {
            return Err(Error::Unsupported("This is not available while Family mode is on.".into()));
        }
        let releases = source.releases(&id.key).map_err(named)?;
        let (release, unavailable_reason) = self.pick_release(&releases);
        if let Some(r) = &release {
            summary.version = Some(r.version.clone());
            summary.install_label = r.options.first().map(|o| o.label());
        }
        summary.downloads = downloads_of(&releases).or(summary.downloads);
        summary.rating = summary.rating.or_else(|| self.rating_for(&summary.id));
        let versions: Vec<ResolvedRelease> =
            releases.iter().map(|r| self.resolve_release(r)).filter(|r| !r.options.is_empty()).collect();
        Ok(AppDetails { summary, release, unavailable_reason, versions })
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
                app.downloads = downloads_of(&releases).or(app.downloads);
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
        let mut offline = false;
        for source in self.sources.iter().filter(|s| self.uses(s.as_ref())) {
            let hits = match source.search(query, self.check_limit) {
                Ok(hits) => hits,
                Err(e) => {
                    report(&mut outcome.errors, &mut offline, source.kind(), &e);
                    continue;
                }
            };
            if let Some(label) = source.search_hits_are_final() {
                let finished = hits.into_iter().take(self.check_limit).map(|mut app| {
                    app.install_label = Some(label.to_string());
                    app
                });
                per_source.push(finished.collect());
                continue;
            }
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
                report(&mut outcome.errors, &mut offline, source.kind(), &e);
            }
            per_source.push(kept);
        }
        // One list, best match first: how close the name is, how many people use it,
        // and what else is known. See `rank`.
        let featured: Vec<AppId> = self.featured().into_iter().flat_map(|s| s.apps).map(|a| a.id).collect();
        // Apps kept on record as ways to use the service that was named.
        let family = crate::related::family_named(crate::related::families(), query);
        let mut hits = Vec::new();
        for list in per_source {
            let of = list.len();
            for (position, mut app) in list.into_iter().enumerate() {
                app.rating = app.rating.or_else(|| self.rating_for(&app.id));
                let related =
                    family.is_some_and(|f| f.members.iter().any(|m| m.id.eq_ignore_ascii_case(&app.id.to_string())));
                let context = crate::rank::Context { position, of, featured: featured.contains(&app.id), related };
                hits.push((app, context));
            }
        }
        // Family mode: nothing for adults, and no word that anything was left out.
        if self.parental.family_mode {
            hits.retain(|(app, _)| !app.adult);
        }
        outcome.apps = crate::rank::sort(query, hits);
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
    fn a_missing_project_is_named_not_shown_as_a_url() {
        let error = demo("linux-x64-debian").details(&id("github:nobody/nothing")).unwrap_err();
        assert_eq!(error.to_string(), "GitHub has no project called “nobody/nothing”.");
        let error = demo("linux-x64-arch").details(&id("aur:no-such-package")).unwrap_err();
        assert_eq!(error.to_string(), "AUR has no package called “no-such-package”.");
    }

    #[test]
    fn the_stores_own_release_files_resolve_on_every_desktop() {
        use crate::model::{Asset, Release};
        // The names .github/workflows/release.yml produces.
        let names = [
            "bilby-v0.2.0-x86_64-unknown-linux-gnu.tar.gz",
            "bilby-v0.2.0-x86_64-pc-windows-msvc.zip",
            "bilby-v0.2.0-aarch64-apple-darwin.tar.gz",
            "SHA256SUMS",
        ];
        let release = Release {
            version: "v0.2.0".into(),
            published: String::new(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets: names
                .iter()
                .map(|n| Asset { name: n.to_string(), size: 1, url: String::new(), digest: None, info: None })
                .collect(),
        };
        let offered = |preset: &str| -> Vec<String> {
            demo(preset).resolve_release(&release).options.into_iter().map(|o| o.asset.name).collect()
        };
        assert_eq!(offered("linux-x64-debian"), [names[0]]);
        assert_eq!(offered("linux-x64-arch"), [names[0]]);
        assert_eq!(offered("windows-x64"), [names[1]]);
        assert_eq!(offered("windows-arm64"), [names[1]], "through emulation");
        assert_eq!(offered("macos-arm64"), [names[2]]);
        assert!(offered("macos-x64").is_empty(), "no Intel Mac build is published");
        assert!(offered("linux-arm64-debian").is_empty());
        assert!(offered("android-arm64").is_empty());
    }

    #[test]
    fn older_versions_with_a_build_are_listed() {
        let versions = |preset: &str| -> Vec<String> {
            let d = demo(preset).details(&id("github:kestrel-sh/kestrel")).unwrap();
            d.versions.into_iter().map(|r| r.version).collect()
        };
        assert_eq!(versions("linux-x64-debian"), ["v1.21.0", "v1.20.0"]);
        // The newest release has nothing for macOS, so it is not offered there at all.
        assert_eq!(versions("macos-arm64"), ["v1.20.0"]);
        assert!(demo("linux-x64-debian").details(&id("github:winspect/winspect")).unwrap().versions.is_empty());
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
    fn suggestions_come_from_the_index_and_respect_the_device() {
        let names =
            |preset: &str, q: &str| -> Vec<String> { demo(preset).suggest(q, 5).into_iter().map(|a| a.name).collect() };
        assert_eq!(names("linux-x64-debian", "lu"), ["Lumen"]);
        assert_eq!(names("linux-x64-debian", "LUMEN"), ["Lumen"], "case does not matter");
        assert_eq!(names("linux-x64-debian", "editor"), ["Lumen"], "summaries are searched too");
        assert_eq!(names("linux-x64-debian", "media"), ["Tidewave", "Marquee", "Couchplay"], "and categories");
        assert_eq!(names("linux-x64-debian", "music player"), ["Tidewave"], "every word must match");
        assert_eq!(names("linux-x64-debian", "ed"), ["Lumen"], "words match from their start: editor, not media");
        assert_eq!(names("android-arm64", "launcher"), ["Fern Launcher"], "any word of the name counts");
        assert!(names("linux-x64-debian", "l").is_empty(), "one letter is too little");
        assert!(names("linux-x64-debian", "").is_empty());
        // Device filtering applies: a Windows-only app is not suggested on Linux.
        assert!(names("linux-x64-debian", "winspect").is_empty());
        assert_eq!(names("windows-x64", "winspect"), ["Winspect"]);
        // Name matches come before summary matches.
        assert_eq!(names("android-arm64", "re")[0], "Relay");
        // On a TV the same app sits in two rows; it is suggested once.
        assert_eq!(names("android-tv", "tide"), ["Tidewave"]);
        assert_eq!(demo("linux-x64-debian").suggest("a", 2).len(), 0);
        assert!(demo("linux-x64-debian").suggest("an", 2).len() <= 2, "the limit holds");
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
            downloads: None,
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
