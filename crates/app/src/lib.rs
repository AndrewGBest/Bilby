//! The graphical store. One Slint window, three interfaces (desktop, mobile, TV), all
//! rendering the same `Store` state that this file fills in from `ustore-core`.
//!
//! Threading: the UI thread never waits on the network. Every catalog call goes through
//! [`App::job`], which runs the work on a worker thread and hands the result back to the
//! UI thread. Under test the same code runs inline, so tests exercise the real handlers.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use ustore_core::catalog::{SearchOutcome, UpdateStatus};
use ustore_core::index::Section as CoreSection;
use ustore_core::install::{self, Action, Dirs};
use ustore_core::model::{
    age_label, human_count, human_size, AppDetails, AppId, AppSummary, InstallOption, SourceKind,
};
use ustore_core::platform::{FormFactor, PRESETS};
use ustore_core::state::{now_unix, InstalledApp, Settings, State};
use ustore_core::{Catalog, Error, Platform};

slint::include_modules!();

const USAGE: &str = "\
untitledstore - app store for GitHub Releases and the AUR, filtered to your machine

OPTIONS:
    --ui <desktop|mobile|tv>   Force an interface (default: detected; desktop and mobile
                               also switch with the window width)
    --as <platform>            Preview the store as another machine (ustore --list-platforms)
    --demo                     Built-in fictional catalog, no network
    --open <id or link>        Start on an app's page (github:owner/repo, aur:package, URL)
    --search <words>           Start with a search
    --page <name>              Start on discover, search, library or settings
    --theme <system|light|dark>  Override the saved appearance for this run
    --size <WxH>               Initial window size
    --state <file>             Use this file for settings and library instead of the default
    --screenshot <file.png>    Render one frame to a PNG and exit
    --script <steps>           With --screenshot: replay input first. Steps are separated by
                               spaces: up down left right ok back tab click:X,Y text:WORD
    --settle <ms>              With --screenshot: wait after the script (default 600)

ENVIRONMENT:
    GITHUB_TOKEN               Raises GitHub's API limit from 60 to 5,000 requests per hour
                               (a token saved in Settings does the same)
";

#[derive(Default)]
pub struct Options {
    pub demo: bool,
    pub platform: Option<Platform>,
    pub shell: Option<Shell>,
    pub open: Option<String>,
    pub search: Option<String>,
    pub page: Option<i32>,
    pub theme: Option<String>,
    pub size: Option<(u32, u32)>,
    pub screenshot: Option<PathBuf>,
    /// Input to replay before the screenshot: `down right ok back click:640,300 ...`.
    pub script: Vec<String>,
    /// Milliseconds to wait after the script before the screenshot is taken.
    pub settle_ms: u64,
    /// Overrides where downloads, cache and settings go (Android passes its app directories).
    pub dirs: Option<Dirs>,
    pub state_file: Option<PathBuf>,
    /// Run catalog work on the calling thread instead of workers. For tests.
    pub inline: bool,
    /// How downloaded files are handed to the system. Defaults to the desktop's own
    /// handler; tests record the call instead of launching installers.
    pub opener: Option<Opener>,
}

pub type Opener = Arc<dyn Fn(&std::path::Path) -> ustore_core::Result<()> + Send + Sync>;

impl Options {
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut o = Options::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"));
            match arg.as_str() {
                "--demo" => o.demo = true,
                "--ui" => {
                    o.shell = Some(match value("--ui")?.as_str() {
                        "desktop" => Shell::Desktop,
                        "mobile" => Shell::Mobile,
                        "tv" => Shell::Tv,
                        other => return Err(format!("unknown interface \"{other}\" (desktop, mobile, tv)")),
                    })
                }
                "--as" => {
                    let name = value("--as")?;
                    o.platform = Some(
                        Platform::preset(&name)
                            .ok_or_else(|| format!("unknown platform \"{name}\". Known: {}", PRESETS.join(", ")))?,
                    );
                }
                "--open" => o.open = Some(value("--open")?),
                "--search" => o.search = Some(value("--search")?),
                "--page" => {
                    o.page = Some(match value("--page")?.as_str() {
                        "discover" => 0,
                        "search" => 1,
                        "library" => 2,
                        "settings" => 3,
                        other => return Err(format!("unknown page \"{other}\" (discover, search, library, settings)")),
                    })
                }
                "--theme" => {
                    let v = value("--theme")?;
                    if !["system", "light", "dark"].contains(&v.as_str()) {
                        return Err(format!("unknown theme \"{v}\" (system, light, dark)"));
                    }
                    o.theme = Some(v);
                }
                "--size" => {
                    let v = value("--size")?;
                    let parsed = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                    o.size = Some(parsed.ok_or_else(|| format!("--size expects WIDTHxHEIGHT, got \"{v}\""))?);
                }
                "--state" => o.state_file = Some(PathBuf::from(value("--state")?)),
                "--screenshot" => o.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--script" => o.script = value("--script")?.split(' ').map(str::to_string).collect(),
                "--settle" => {
                    o.settle_ms = value("--settle")?.parse().map_err(|_| "--settle expects milliseconds".to_string())?
                }
                "-h" | "--help" => return Err(USAGE.to_string()),
                other => return Err(format!("unknown option \"{other}\"\n\n{USAGE}")),
            }
        }
        Ok(o)
    }
}

// ---- core data -> UI data ---------------------------------------------------------

const TINTS: [u32; 10] =
    [0x5b8def, 0x8e6cf0, 0xe0679a, 0xe8804a, 0xd9a521, 0x4fb477, 0x2fa7a0, 0x3d9bd8, 0x7a86a8, 0xc65fd0];

fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let letters: String = match words.as_slice() {
        [] => "?".to_string(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first.chars().take(1).chain(second.chars().take(1)).collect(),
    };
    let mut chars = letters.chars();
    match (chars.next(), chars.next()) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_lowercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".to_string(),
    }
}

fn tint_for(id: &str) -> slint::Color {
    let key = id.split_once(':').map(|(_, key)| key).unwrap_or(id);
    let hash = key.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    slint::Color::from_argb_encoded(0xff00_0000 | TINTS[hash as usize % TINTS.len()])
}

/// Release notes are Markdown; show them as readable plain text.
fn plain_notes(markdown: &str) -> String {
    let mut out = String::new();
    for line in markdown.lines() {
        let line = line.trim_end();
        let line = line.trim_start_matches('#').trim_start();
        let line = match line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            Some(item) => format!("•  {item}"),
            None => line.to_string(),
        };
        out.push_str(&line.replace("**", "").replace('`', ""));
        out.push('\n');
        if out.len() > 2400 {
            out.push('…');
            break;
        }
    }
    out.trim().to_string()
}

fn option_row(o: &InstallOption) -> OptionRow {
    OptionRow {
        file: o.asset.name.clone().into(),
        label: o.label().into(),
        size: human_size(o.asset.size).into(),
        note: o.verdict.note.clone().unwrap_or_default().into(),
        native: o.verdict.level == ustore_core::compat::Level::Native,
        verified: o.asset.digest.is_some() && o.plan.action != Action::ShowCommand,
        verb: o.plan.verb.clone().into(),
        steps: ModelRc::new(VecModel::from(o.plan.steps.iter().map(SharedString::from).collect::<Vec<_>>())),
        command: o.plan.command.clone().unwrap_or_default().into(),
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn theme_mode(name: &str) -> i32 {
    match name {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    }
}

fn shell_for(form_factor: FormFactor) -> Shell {
    match form_factor {
        FormFactor::Desktop => Shell::Desktop,
        FormFactor::Mobile => Shell::Mobile,
        FormFactor::Tv => Shell::Tv,
    }
}

fn make_catalog(platform: &Platform, demo: bool, settings: &Settings, dirs: &Dirs) -> Arc<Catalog> {
    let mut catalog = if demo {
        Catalog::demo(platform.clone())
    } else {
        // A token typed into Settings wins over the environment.
        let token = Some(settings.github_token.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("GITHUB_TOKEN").ok());
        Catalog::live(platform.clone(), token, dirs.cache.clone())
    };
    catalog.include_prereleases = settings.include_prereleases;
    Arc::new(catalog)
}

// ---- application state ------------------------------------------------------------

/// The app currently on the details page.
#[derive(Default)]
struct Current {
    id: String,
    details: Option<AppDetails>,
}

pub struct App {
    ui: slint::Weak<AppWindow>,
    catalog: RwLock<Arc<Catalog>>,
    platform: Platform,
    demo: bool,
    inline: bool,
    dirs: Dirs,
    state_path: PathBuf,
    state: Mutex<State>,
    results: Mutex<(String, Vec<AppSummary>)>,
    updates: Mutex<HashMap<String, UpdateStatus>>,
    last_checked: AtomicU64,
    current: Mutex<Current>,
    /// Bumped per request so a slow, superseded answer is dropped instead of shown.
    search_generation: AtomicU64,
    details_generation: AtomicU64,
    toast_generation: Arc<AtomicU64>,
    cancel_install: Arc<AtomicBool>,
    opener: Opener,
}

impl App {
    fn catalog(&self) -> Arc<Catalog> {
        self.catalog.read().unwrap().clone()
    }

    /// The settings and library as they are right now.
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    /// Change the saved state and write it to disk. A failed write is reported, not fatal.
    fn update_state(&self, ui: &AppWindow, change: impl FnOnce(&mut State)) {
        let result = {
            let mut state = self.state.lock().unwrap();
            change(&mut state);
            state.save(&self.state_path)
        };
        if let Err(e) = result {
            self.toast(ui, &format!("Couldn't save your settings: {e}"), true);
        }
    }

    /// Run `work` off the UI thread, then `done` on it. Inline under test.
    fn job<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&App) -> T + Send + 'static,
        done: impl FnOnce(&Arc<App>, &AppWindow, T) + Send + 'static,
    ) {
        if self.inline {
            let result = work(self);
            if let Some(ui) = self.ui.upgrade() {
                done(self, &ui, result);
                self.refresh_quota(&ui);
            }
            return;
        }
        let app = self.clone();
        std::thread::spawn(move || {
            let result = work(&app);
            let weak = app.ui.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    done(&app, &ui, result);
                    app.refresh_quota(&ui);
                }
            });
        });
    }

    fn toast(&self, ui: &AppWindow, message: &str, is_error: bool) {
        let store = ui.global::<Store>();
        store.set_toast(message.into());
        store.set_toast_is_error(is_error);
        let generation = self.toast_generation.fetch_add(1, Ordering::SeqCst) + 1;
        if self.inline {
            return;
        }
        // Errors stay longer. The timer only clears the toast it was started for.
        let latest = self.toast_generation.clone();
        let weak = self.ui.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(if is_error { 7 } else { 4 }), move || {
            if latest.load(Ordering::SeqCst) == generation {
                if let Some(ui) = weak.upgrade() {
                    ui.global::<Store>().set_toast(SharedString::new());
                }
            }
        });
    }

    // ---- cards ----

    fn card(&self, app: &AppSummary) -> AppCard {
        let id = app.id.to_string();
        let popularity = match (app.popularity, app.id.source) {
            (Some(n), SourceKind::GitHub) => format!("★ {}", human_count(n)),
            (Some(n), SourceKind::Aur) => format!("{} votes", human_count(n)),
            (None, _) => String::new(),
        };
        let installed = self.state.lock().unwrap().installed(&id).is_some();
        let has_update = matches!(self.updates.lock().unwrap().get(&id), Some(UpdateStatus::Available { .. }));
        AppCard {
            tint: tint_for(&id),
            id: id.into(),
            name: app.name.clone().into(),
            author: app.author.clone().into(),
            summary: app.summary.clone().into(),
            source: app.id.source.label().into(),
            badge: app.install_label.clone().unwrap_or_default().into(),
            version: app.version.clone().unwrap_or_default().into(),
            popularity: popularity.into(),
            initials: initials(&app.name).into(),
            installed,
            has_update: installed && has_update,
        }
    }

    fn cards(&self, apps: &[AppSummary]) -> ModelRc<AppCard> {
        ModelRc::new(VecModel::from(apps.iter().map(|a| self.card(a)).collect::<Vec<_>>()))
    }

    fn library_card(&self, app: &InstalledApp) -> AppCard {
        let update = self.updates.lock().unwrap().get(&app.id).cloned();
        let (version, has_update) = match update {
            Some(UpdateStatus::Available { version, .. }) => (format!("{} → {version}", app.version), true),
            _ => (app.version.clone(), false),
        };
        AppCard {
            id: app.id.as_str().into(),
            name: app.name.as_str().into(),
            author: app.author.as_str().into(),
            summary: app.summary.as_str().into(),
            source: AppId::parse(&app.id).map(|id| id.source.label()).unwrap_or("").into(),
            badge: app.label.as_str().into(),
            version: version.into(),
            popularity: SharedString::new(),
            initials: initials(&app.name).into(),
            tint: tint_for(&app.id),
            installed: true,
            has_update,
        }
    }

    // ---- pages ----

    /// Discover, the TV rows and the library all depend on the same data; redraw them
    /// together so "Installed" and "Update" markers never disagree between pages.
    pub fn refresh(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let featured = self.catalog().featured();
        let to_sections = |list: &[CoreSection]| -> Vec<Section> {
            list.iter().map(|s| Section { title: s.title.clone().into(), apps: self.cards(&s.apps) }).collect()
        };
        store.set_sections(ModelRc::new(VecModel::from(to_sections(&featured))));
        store.set_categories(ModelRc::new(VecModel::from(
            featured.iter().map(|s| SharedString::from(&s.title)).collect::<Vec<_>>(),
        )));
        if !featured.iter().any(|s| s.title == store.get_category().as_str()) {
            store.set_category(SharedString::new());
        }
        store.set_spotlight(featured.first().and_then(|s| s.apps.first()).map(|a| self.card(a)).unwrap_or_default());

        let state = self.state();
        let updates = self.updates.lock().unwrap().clone();
        let now = now_unix();
        let rows: Vec<LibraryRow> = state
            .library
            .iter()
            .map(|app| {
                let date = age_label(&unix_date(app.installed_at), now);
                let mut detail = app.version.clone();
                if !app.label.is_empty() {
                    detail.push_str(&format!("  ·  {}", app.label));
                }
                if !date.is_empty() {
                    detail.push_str(&format!("  ·  installed {date}"));
                }
                let (status, status_text) = match updates.get(&app.id) {
                    None => (0, String::new()),
                    Some(UpdateStatus::UpToDate) => (2, "Up to date".to_string()),
                    Some(UpdateStatus::Available { version, .. }) => (3, format!("Update available: {version}")),
                    Some(UpdateStatus::Unknown(why)) => (4, format!("Couldn't check: {why}")),
                };
                LibraryRow {
                    card: self.library_card(app),
                    detail: detail.into(),
                    status,
                    status_text: status_text.into(),
                    store_managed: app.store_managed,
                }
            })
            .collect();
        let update_count = rows.iter().filter(|r| r.status == 3).count();
        store.set_updates_count(update_count as i32);
        store.set_library(ModelRc::new(VecModel::from(rows)));
        let checked = self.last_checked.load(Ordering::SeqCst);
        store.set_last_checked(
            match now.saturating_sub(checked) {
                _ if checked == 0 => String::new(),
                s if s < 90 => "just now".to_string(),
                s if s < 3600 => format!("{} minutes ago", s / 60),
                s => format!("{} hours ago", s / 3600),
            }
            .into(),
        );
        store.set_recent(ModelRc::new(VecModel::from(
            state.recent_searches.iter().map(SharedString::from).collect::<Vec<_>>(),
        )));

        // Search results keep their markers in sync too.
        let (query, results) = self.results.lock().unwrap().clone();
        store.set_results(self.cards(&results));

        // TV: library first (updates are the reason to open a store), then results.
        let mut tv = Vec::new();
        if !state.library.is_empty() {
            let title = if update_count > 0 {
                format!("Your library  ·  {}", plural(update_count, "update"))
            } else {
                "Your library".to_string()
            };
            let cards: Vec<AppCard> = state.library.iter().map(|a| self.library_card(a)).collect();
            tv.push(Section { title: title.into(), apps: ModelRc::new(VecModel::from(cards)) });
        }
        if !results.is_empty() {
            tv.push(Section { title: format!("Results for “{query}”").into(), apps: self.cards(&results) });
        }
        tv.extend(to_sections(&featured));
        store.set_tv_rows(ModelRc::new(VecModel::from(tv)));
    }

    fn refresh_settings(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let settings = self.state().settings;
        store.set_theme_mode(theme_mode(&settings.theme));
        store.set_include_prereleases(settings.include_prereleases);
        store.set_has_token(!settings.github_token.trim().is_empty());
        store.set_downloads_dir(self.dirs.downloads.display().to_string().into());
        store.set_show_welcome(!settings.welcomed);
        self.refresh_quota(ui);
    }

    /// Show how much of GitHub's hourly allowance is left, once GitHub has told us.
    fn refresh_quota(&self, ui: &AppWindow) {
        let text = self.catalog().quota().map(|q| q.describe(now_unix())).unwrap_or_default();
        ui.global::<Store>().set_quota_text(text.into());
    }

    fn rebuild_catalog(&self) {
        let settings = self.state().settings;
        *self.catalog.write().unwrap() = make_catalog(&self.platform, self.demo, &settings, &self.dirs);
    }

    // ---- search ----

    /// While typing: instant matches from the bundled catalog. No network, no history.
    pub fn typing(&self, ui: &AppWindow, text: &str) {
        let store = ui.global::<Store>();
        let suggestions = self.catalog().suggest(text, 6);
        if !suggestions.is_empty() && !store.get_details_open() {
            store.set_page(1);
        }
        // Old results belong to the previous query; typing starts a new one.
        if store.get_searched() {
            store.set_searched(false);
            store.set_search_note(SharedString::new());
            store.set_search_error(SharedString::new());
            *self.results.lock().unwrap() = (String::new(), Vec::new());
            store.set_results(ModelRc::default());
        }
        store.set_suggestions(self.cards(&suggestions));
    }

    pub fn search(self: &Arc<Self>, ui: &AppWindow, query: &str) {
        let query = query.trim().to_string();
        if query.is_empty() {
            return;
        }
        // A pasted link or owner/repo goes straight to that app.
        if let Some(id) = AppId::from_user_input(&query) {
            if id.source != SourceKind::Aur || self.catalog().active_sources().contains(&SourceKind::Aur) {
                self.open(ui, &id.to_string());
                return;
            }
        }
        let store = ui.global::<Store>();
        if query.chars().count() < 2 {
            store.set_page(1);
            store.set_search_note("Type at least two characters.".into());
            return;
        }
        self.close_details(ui);
        store.set_page(1);
        store.set_searching(true);
        store.set_search_note(SharedString::new());
        store.set_search_error(SharedString::new());
        self.update_state(ui, |state| state.add_search(&query));
        let generation = self.search_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let work_query = query.clone();
        self.job(
            move |app| app.catalog().search(&work_query),
            move |app, ui, outcome| {
                if app.search_generation.load(Ordering::SeqCst) == generation {
                    app.show_search(ui, &query, &outcome);
                }
            },
        );
    }

    fn show_search(&self, ui: &AppWindow, query: &str, outcome: &SearchOutcome) {
        let store = ui.global::<Store>();
        store.set_searching(false);
        store.set_searched(true);
        // The real results replace the as-you-type suggestions.
        store.set_suggestions(ModelRc::default());
        let mut note = if outcome.apps.is_empty() {
            String::new()
        } else {
            format!("{} for “{query}” on {}.", plural(outcome.apps.len(), "app"), self.platform.title())
        };
        if outcome.hidden > 0 {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(&format!("{} more matched but have no file for this device.", outcome.hidden));
        }
        store.set_search_note(note.into());
        store.set_search_error(outcome.errors.join("\n").into());
        *self.results.lock().unwrap() = (query.to_string(), outcome.apps.clone());
        self.refresh(ui);
    }

    // ---- details ----

    pub fn open(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        let store = ui.global::<Store>();
        let generation = self.details_generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Show what the list already knew while the releases load.
        let known = find_card(&store, id);
        store.set_details_open(true);
        store.set_details_error(SharedString::new());
        store.set_unavailable(SharedString::new());
        store.set_warning(SharedString::new());
        store.set_notes(SharedString::new());
        store.set_release_date(SharedString::new());
        store.set_release_age(SharedString::new());
        store.set_license(SharedString::new());
        store.set_homepage(SharedString::new());
        store.set_options(ModelRc::default());
        store.set_hidden(ModelRc::default());
        store.set_install_state(0);
        store.set_installed_version(
            self.state.lock().unwrap().installed(id).map(|a| a.version.clone()).unwrap_or_default().into(),
        );
        *self.current.lock().unwrap() = Current { id: id.to_string(), details: None };
        let mut card = known.unwrap_or_else(|| {
            let name = id.rsplit(['/', ':']).next().unwrap_or(id);
            AppCard {
                id: id.into(),
                name: name.into(),
                initials: initials(name).into(),
                tint: tint_for(id),
                ..Default::default()
            }
        });
        // Library cards carry "old → new" as their version; the page shows the release's.
        card.version = SharedString::new();
        store.set_app(card);
        let Some(app_id) = AppId::parse(id) else {
            store.set_details_loading(false);
            store.set_details_error(format!("“{id}” is not an app id.").into());
            return;
        };
        store.set_details_loading(true);
        self.job(
            move |app| app.catalog().details(&app_id).map_err(|e| e.to_string()),
            move |app, ui, result| {
                if app.details_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                match result {
                    Ok(details) => app.show_details(ui, details),
                    Err(message) => {
                        let store = ui.global::<Store>();
                        store.set_details_loading(false);
                        store.set_details_error(message.into());
                    }
                }
            },
        );
    }

    fn show_details(&self, ui: &AppWindow, details: AppDetails) {
        let store = ui.global::<Store>();
        store.set_details_loading(false);
        // The list's display name ("Tidewave") reads better than the repository's ("tidewave").
        let mut card = self.card(&details.summary);
        let shown = store.get_app();
        if shown.id == card.id && !shown.name.is_empty() && !shown.author.is_empty() {
            card.name = shown.name;
            card.initials = shown.initials;
        }
        store.set_app(card);
        store.set_license(details.summary.license.clone().unwrap_or_default().into());
        store.set_warning(details.summary.warning.clone().unwrap_or_default().into());
        store.set_homepage(details.summary.homepage.as_str().into());
        store.set_unavailable(details.unavailable_reason.clone().unwrap_or_default().into());
        let (options, hidden, notes, date) = match &details.release {
            Some(r) => (r.options.clone(), r.hidden.clone(), plain_notes(&r.notes), r.published.clone()),
            None => (vec![], vec![], String::new(), String::new()),
        };
        store.set_options(ModelRc::new(VecModel::from(options.iter().map(option_row).collect::<Vec<_>>())));
        store.set_hidden(ModelRc::new(VecModel::from(
            hidden
                .iter()
                .map(|h| HiddenRow { file: h.name.clone().into(), reason: h.reason.clone().into() })
                .collect::<Vec<_>>(),
        )));
        store.set_notes(notes.into());
        store.set_release_age(age_label(&date, now_unix()).into());
        store.set_release_date(date.into());
        self.current.lock().unwrap().details = Some(details);
    }

    pub fn close_details(&self, ui: &AppWindow) {
        // Invalidate any details request still in flight.
        self.details_generation.fetch_add(1, Ordering::SeqCst);
        ui.global::<Store>().set_details_open(false);
    }

    // ---- install ----

    pub fn install(self: &Arc<Self>, ui: &AppWindow, index: i32) {
        let store = ui.global::<Store>();
        if store.get_install_state() == 1 {
            return;
        }
        let (details, option) = {
            let current = self.current.lock().unwrap();
            let Some(details) = current.details.clone() else { return };
            let Some(option) = details.release.as_ref().and_then(|r| r.options.get(index.max(0) as usize)).cloned()
            else {
                return;
            };
            (details, option)
        };
        store.set_install_index(index);
        store.set_install_state(1);
        store.set_install_progress(0.0);
        store.set_install_message(SharedString::new());
        self.cancel_install.store(false, Ordering::SeqCst);

        let cancel = self.cancel_install.clone();
        let inline = self.inline;
        let weak = self.ui.clone();
        let work_option = option.clone();
        self.job(
            move |app| {
                let mut last_percent = 0u64;
                let mut progress = |done: u64, total: Option<u64>| {
                    if let Some(total) = total.filter(|t| *t > 0) {
                        let percent = done * 100 / total;
                        // One UI update per percent is plenty.
                        if percent != last_percent {
                            last_percent = percent;
                            let set =
                                move |ui: AppWindow| ui.global::<Store>().set_install_progress(percent as f32 / 100.0);
                            if inline {
                                if let Some(ui) = weak.upgrade() {
                                    set(ui);
                                }
                            } else {
                                let weak = weak.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(ui) = weak.upgrade() {
                                        set(ui);
                                    }
                                });
                            }
                        }
                    }
                    !cancel.load(Ordering::SeqCst)
                };
                let http = app.catalog().http();
                install::execute(
                    &work_option.plan,
                    &work_option.asset,
                    http.as_ref(),
                    &app.dirs,
                    app.opener.as_ref(),
                    &mut progress,
                )
            },
            move |app, ui, result| {
                let store = ui.global::<Store>();
                match result {
                    Ok(outcome) => {
                        store.set_install_state(2);
                        store.set_install_message(outcome.message.as_str().into());
                        if option.plan.action == Action::ShowCommand {
                            return;
                        }
                        let version = details.release.as_ref().map(|r| r.version.clone()).unwrap_or_default();
                        let name = store.get_app().name.to_string();
                        let mut entry = InstalledApp::new(
                            &details.summary,
                            &version,
                            &option.label(),
                            &option.asset.name,
                            now_unix(),
                        );
                        entry.name = name.clone();
                        entry.path = outcome.file.as_ref().map(|p| p.display().to_string());
                        entry.store_managed = outcome.store_managed;
                        let id = entry.id.clone();
                        app.update_state(ui, |state| state.record_install(entry));
                        app.updates.lock().unwrap().insert(id, UpdateStatus::UpToDate);
                        store.set_installed_version(version.into());
                        app.refresh(ui);
                        app.toast(ui, &format!("{name} is in your library"), false);
                    }
                    Err(Error::Cancelled) => {
                        store.set_install_state(0);
                        app.toast(ui, "Download cancelled", false);
                    }
                    Err(e) => {
                        store.set_install_state(3);
                        store.set_install_message(e.to_string().into());
                    }
                }
            },
        );
    }

    // ---- library ----

    pub fn check_updates(self: &Arc<Self>, ui: &AppWindow, announce: bool) {
        let store = ui.global::<Store>();
        let apps: Vec<(AppId, String)> =
            self.state().library.iter().filter_map(|a| Some((AppId::parse(&a.id)?, a.version.clone()))).collect();
        if apps.is_empty() || store.get_checking_updates() {
            return;
        }
        store.set_checking_updates(true);
        let work_apps = apps.clone();
        self.job(
            move |app| app.catalog().check_updates(&work_apps),
            move |app, ui, statuses| {
                let available = statuses.iter().filter(|s| matches!(s, UpdateStatus::Available { .. })).count();
                let failed = statuses.iter().filter(|s| matches!(s, UpdateStatus::Unknown(_))).count();
                {
                    let mut updates = app.updates.lock().unwrap();
                    for ((id, _), status) in apps.iter().zip(statuses) {
                        updates.insert(id.to_string(), status);
                    }
                }
                app.last_checked.store(now_unix(), Ordering::SeqCst);
                ui.global::<Store>().set_checking_updates(false);
                app.refresh(ui);
                if announce {
                    let message = match (available, failed) {
                        (0, 0) => "Everything is up to date".to_string(),
                        (0, f) => format!("Couldn't check {}", plural(f, "app")),
                        (n, _) => format!("{} available", plural(n, "update")),
                    };
                    app.toast(ui, &message, available == 0 && failed > 0);
                }
            },
        );
    }

    pub fn remove_from_library(&self, ui: &AppWindow, id: &str) {
        let mut removed = None;
        self.update_state(ui, |state| removed = state.remove(id));
        let Some(entry) = removed else { return };
        self.updates.lock().unwrap().remove(id);
        let mut message = format!("{} removed from your library", entry.name);
        // Only files the store itself placed, and only inside its own Applications folder.
        if let (true, Some(path)) = (entry.store_managed, entry.path.as_deref()) {
            let path = PathBuf::from(path);
            if path.starts_with(&self.dirs.applications) && std::fs::remove_file(&path).is_ok() {
                message = format!("{} uninstalled", entry.name);
            }
        }
        let store = ui.global::<Store>();
        if store.get_app().id == id {
            store.set_installed_version(SharedString::new());
        }
        self.refresh(ui);
        self.toast(ui, &message, false);
    }
}

fn unix_date(secs: u64) -> String {
    // Reuse the AUR module's civil-date conversion through a tiny local copy.
    let z = (secs / 86_400) as i64 + 719_468;
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

fn find_card(store: &Store, id: &str) -> Option<AppCard> {
    let in_sections = store.get_sections().iter().flat_map(|s| s.apps.iter().collect::<Vec<_>>()).find(|c| c.id == id);
    in_sections
        .or_else(|| store.get_results().iter().find(|c| c.id == id))
        .or_else(|| store.get_suggestions().iter().find(|c| c.id == id))
        .or_else(|| store.get_library().iter().map(|r| r.card).find(|c| c.id == id))
}

/// Create the window and wire it to the catalog, without starting the event loop.
pub fn build(options: &Options) -> Result<(AppWindow, Arc<App>), Box<dyn std::error::Error>> {
    let mut platform = options.platform.clone().unwrap_or_else(Platform::detect);
    // Forcing the TV interface also means "prefer TV builds", and vice versa.
    match options.shell {
        Some(Shell::Tv) => platform.form_factor = FormFactor::Tv,
        Some(_) if platform.form_factor == FormFactor::Tv => platform.form_factor = FormFactor::Desktop,
        _ => {}
    }
    let dirs = options.dirs.clone().unwrap_or_else(Dirs::for_current_user);
    // The demo catalog gets its own library so fictional apps never mix with real ones.
    let state_path = options
        .state_file
        .clone()
        .unwrap_or_else(|| dirs.config.join(if options.demo { "demo-state.json" } else { "state.json" }));
    let state = State::load(&state_path);
    let catalog = make_catalog(&platform, options.demo, &state.settings, &dirs);

    let ui = AppWindow::new()?;
    let store = ui.global::<Store>();
    let shell = options.shell.unwrap_or_else(|| shell_for(platform.form_factor));
    store.set_shell(shell);
    // A forced interface stays put; otherwise desktop and mobile follow the window width.
    store.set_adaptive(options.shell.is_none());
    store.set_demo(options.demo);
    store.set_app_version(env!("CARGO_PKG_VERSION").into());
    store.set_platform_title(platform.title().into());
    let mut facts = platform.facts();
    let sources: Vec<_> = catalog.active_sources().iter().map(|s| s.label()).collect();
    facts.push(("Sources".to_string(), sources.join(", ")));
    store.set_facts(ModelRc::new(VecModel::from(
        facts.into_iter().map(|(name, value)| Fact { name: name.into(), value: value.into() }).collect::<Vec<_>>(),
    )));

    let app = Arc::new(App {
        ui: ui.as_weak(),
        catalog: RwLock::new(catalog),
        platform,
        demo: options.demo,
        inline: options.inline,
        dirs,
        state_path,
        state: Mutex::new(state),
        results: Mutex::new((String::new(), Vec::new())),
        updates: Mutex::new(HashMap::new()),
        last_checked: AtomicU64::new(0),
        current: Mutex::new(Current::default()),
        search_generation: AtomicU64::new(0),
        details_generation: AtomicU64::new(0),
        toast_generation: Arc::new(AtomicU64::new(0)),
        cancel_install: Arc::new(AtomicBool::new(false)),
        opener: options.opener.clone().unwrap_or_else(|| Arc::new(install::open_with_system)),
    });

    // The TV interface is designed dark; elsewhere the saved preference applies.
    let theme = options.theme.clone().unwrap_or_else(|| app.state().settings.theme);
    ui.global::<Theme>().set_mode(if shell == Shell::Tv { 2 } else { theme_mode(&theme) });
    app.refresh(&ui);
    app.refresh_settings(&ui);
    if let Some(theme) = &options.theme {
        store.set_theme_mode(theme_mode(theme));
    }

    // Each handler looks the window up again: callbacks must not keep it alive.
    macro_rules! on {
        ($setter:ident, |$app:ident, $ui:ident $(, $arg:ident)*| $body:block) => {
            store.$setter({
                let $app = app.clone();
                move |$($arg),*| {
                    if let Some($ui) = $app.ui.upgrade() {
                        let $ui = &$ui;
                        $body
                    }
                }
            });
        };
    }
    on!(on_search, |app, ui, query| { app.search(ui, &query) });
    on!(on_typing, |app, ui, text| { app.typing(ui, &text) });
    on!(on_open, |app, ui, id| { app.open(ui, &id) });
    on!(on_close_details, |app, ui| { app.close_details(ui) });
    on!(on_retry_details, |app, ui| {
        let id = app.current.lock().unwrap().id.clone();
        if !id.is_empty() {
            app.open(ui, &id);
        }
    });
    on!(on_install, |app, ui, index| { app.install(ui, index) });
    on!(on_cancel_install, |app, ui| {
        let _ = ui;
        app.cancel_install.store(true, Ordering::SeqCst);
    });
    on!(on_check_updates, |app, ui| { app.check_updates(ui, true) });
    on!(on_remove_from_library, |app, ui, id| { app.remove_from_library(ui, &id) });
    on!(on_set_theme, |app, ui, mode| {
        let name = ["system", "light", "dark"].get(mode as usize).copied().unwrap_or("system");
        app.update_state(ui, |state| state.settings.theme = name.to_string());
        if ui.global::<Store>().get_shell() != Shell::Tv {
            ui.global::<Theme>().set_mode(theme_mode(name));
        }
        app.refresh_settings(ui);
    });
    on!(on_set_prereleases, |app, ui, enabled| {
        app.update_state(ui, |state| state.settings.include_prereleases = enabled);
        app.rebuild_catalog();
        app.refresh_settings(ui);
    });
    on!(on_save_token, |app, ui, token| {
        let token = token.trim().to_string();
        let cleared = token.is_empty();
        app.update_state(ui, |state| state.settings.github_token = token);
        app.rebuild_catalog();
        app.refresh_settings(ui);
        app.toast(ui, if cleared { "Token forgotten" } else { "Token saved on this device" }, false);
    });
    on!(on_clear_cache, |app, ui| {
        // Only the store's own cache folder, never a path from outside.
        let cache = app.dirs.cache.join("http");
        let result = if cache.exists() { std::fs::remove_dir_all(&cache) } else { Ok(()) };
        match result {
            Ok(()) => app.toast(ui, "Cached catalog data cleared", false),
            Err(e) => app.toast(ui, &format!("Couldn't clear the cache: {e}"), true),
        }
    });
    on!(on_clear_recent, |app, ui| {
        app.update_state(ui, |state| state.recent_searches.clear());
        app.refresh(ui);
    });
    on!(on_open_url, |app, ui, url| {
        if let Err(e) = install::open_url(&url) {
            app.toast(ui, &e.to_string(), true);
        }
    });
    on!(on_dismiss_welcome, |app, ui| {
        app.update_state(ui, |state| state.settings.welcomed = true);
        app.refresh_settings(ui);
    });
    on!(on_dismiss_toast, |app, ui| {
        let _ = &app;
        ui.global::<Store>().set_toast(SharedString::new());
    });

    if let Some(page) = options.page {
        store.set_page(page);
    }
    Ok((ui, app))
}

pub fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let (ui, app) = build(&options)?;
    let store = ui.global::<Store>();
    if let Some((w, h)) = options.size {
        ui.window().set_size(slint::LogicalSize::new(w as f32, h as f32));
    } else if store.get_shell() == Shell::Tv && options.screenshot.is_none() {
        ui.window().set_fullscreen(true);
    }
    if let Some(query) = &options.search {
        store.set_query(query.as_str().into());
        app.search(&ui, query);
    }
    if let Some(target) = &options.open {
        let id = AppId::from_user_input(target).map(|id| id.to_string()).unwrap_or_else(|| target.clone());
        app.open(&ui, &id);
    }
    // Quietly look for updates at start-up: it is the reason to open the store again.
    app.check_updates(&ui, false);

    if let Some(path) = options.screenshot.clone() {
        screenshot_after_script(&ui, options.script.clone(), options.settle_ms, path);
    }
    ui.run()?;
    Ok(())
}

/// Replays scripted input through the window exactly as real key presses and clicks
/// would arrive, one step every 250 ms, then saves a frame and quits. This is how the
/// interfaces are exercised headlessly (CI, documentation screenshots).
fn screenshot_after_script(ui: &AppWindow, script: Vec<String>, settle_ms: u64, path: PathBuf) {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    let settle = std::time::Duration::from_millis(if settle_ms == 0 { 600 } else { settle_ms });
    let weak = ui.as_weak();
    let mut steps = script.into_iter().filter(|s| !s.is_empty());
    let timer = std::rc::Rc::new(slint::Timer::default());
    let timer_handle = timer.clone();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        let Some(ui) = weak.upgrade() else { return };
        let window = ui.window();
        let key = |text: SharedString| {
            window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            window.dispatch_event(WindowEvent::KeyReleased { text });
        };
        match steps.next() {
            Some(step) => match step.as_str() {
                "up" => key(Key::UpArrow.into()),
                "down" => key(Key::DownArrow.into()),
                "left" => key(Key::LeftArrow.into()),
                "right" => key(Key::RightArrow.into()),
                "ok" => key(Key::Return.into()),
                "back" => key(Key::Escape.into()),
                "tab" => key(Key::Tab.into()),
                "wait" => {}
                other => {
                    if let Some((x, y)) = other.strip_prefix("click:").and_then(|p| p.split_once(',')) {
                        let position = slint::LogicalPosition::new(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                        window.dispatch_event(WindowEvent::PointerMoved { position });
                        window
                            .dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
                        window.dispatch_event(WindowEvent::PointerReleased {
                            position,
                            button: PointerEventButton::Left,
                        });
                    } else if let Some(text) = other.strip_prefix("text:") {
                        for c in text.chars() {
                            key(c.to_string().into());
                        }
                    } else {
                        eprintln!("unknown script step \"{other}\"");
                    }
                }
            },
            None => {
                timer_handle.stop();
                let weak = ui.as_weak();
                let path = path.clone();
                slint::Timer::single_shot(settle, move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let result = ui.window().take_snapshot().map_err(|e| e.to_string()).and_then(|pixels| {
                        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                        let mut encoder =
                            png::Encoder::new(std::io::BufWriter::new(file), pixels.width(), pixels.height());
                        encoder.set_color(png::ColorType::Rgba);
                        encoder.set_depth(png::BitDepth::Eight);
                        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                        writer.write_image_data(pixels.as_bytes()).map_err(|e| e.to_string())
                    });
                    if let Err(e) = result {
                        eprintln!("screenshot failed: {e}");
                    }
                    let _ = slint::quit_event_loop();
                });
            }
        }
    });
}

/// Android entry point (NativeActivity / GameActivity via `android-activity`).
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android_app: slint::android::AndroidApp) {
    let internal = android_app.internal_data_path().unwrap_or_else(std::env::temp_dir);
    let downloads = android_app.external_data_path().unwrap_or_else(|| internal.clone());
    slint::android::init(android_app).expect("Slint Android backend failed to start");
    let dirs = Dirs {
        downloads: downloads.join("downloads"),
        applications: internal.join("apps"),
        cache: internal.join("cache"),
        config: internal.join("config"),
    };
    if let Err(e) = run(Options { dirs: Some(dirs), ..Default::default() }) {
        eprintln!("untitledstore: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_from_names() {
        assert_eq!(initials("Tidewave"), "Ti");
        assert_eq!(initials("fern-launcher"), "Fl");
        assert_eq!(initials("bat"), "Ba");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn notes_lose_their_markdown() {
        assert_eq!(plain_notes("## New\n\n- **Fast** `startup`\n* Fixes"), "New\n\n•  Fast startup\n•  Fixes");
    }

    #[test]
    fn options_parse() {
        let args = ["--demo", "--ui", "tv", "--size", "1920x1080", "--as", "android-tv", "--page", "library"];
        let o = Options::from_args(args.map(String::from).into_iter()).unwrap();
        assert!(o.demo);
        assert_eq!(o.shell, Some(Shell::Tv));
        assert_eq!(o.size, Some((1920, 1080)));
        assert_eq!(o.page, Some(2));
        assert!(Options::from_args(["--ui", "watch"].map(String::from).into_iter()).is_err());
        assert!(Options::from_args(["--theme", "pink"].map(String::from).into_iter()).is_err());
        assert!(Options::from_args(["--page", "nowhere"].map(String::from).into_iter()).is_err());
    }

    #[test]
    fn dates_and_tints_are_stable() {
        assert_eq!(unix_date(1_700_000_000), "2023-11-14");
        assert_eq!(tint_for("github:a/b"), tint_for("github:a/b"));
    }
}
