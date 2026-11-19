//! The graphical store. One Slint window, three interfaces (desktop, mobile, TV), all
//! rendering the same `Store` state that this file fills in from `ustore-core`.
//!
//! Threading: the UI thread never waits on the network. Every catalog call runs on a
//! worker thread and posts plain data back with `invoke_from_event_loop`.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use ustore_core::catalog::SearchOutcome;
use ustore_core::index::Section as CoreSection;
use ustore_core::install::{self, Dirs};
use ustore_core::model::{
    human_count, human_size, AppDetails, AppId, AppSummary, InstallOption, SourceKind,
};
use ustore_core::platform::{FormFactor, PRESETS};
use ustore_core::{Catalog, Platform};

slint::include_modules!();

const USAGE: &str = "\
untitledstore - app store for GitHub Releases and the AUR, filtered to your machine

OPTIONS:
    --ui <desktop|mobile|tv>   Force an interface (default: detected; desktop and mobile
                               also switch with the window width)
    --as <platform>            Preview the store as another machine (ustore --list-platforms)
    --demo                     Built-in fictional catalog, no network
    --open <id>                Start on an app's page (github:owner/repo, aur:package)
    --search <words>           Start with a search
    --size <WxH>               Initial window size
    --screenshot <file.png>    Render one frame to a PNG and exit
    --script <steps>           With --screenshot: replay input first. Steps are separated by
                               spaces: up down left right ok back click:X,Y text:WORD
    --settle <ms>              With --screenshot: wait after the script (default 600)

ENVIRONMENT:
    GITHUB_TOKEN               Raises GitHub's API limit from 60 to 5,000 requests per hour
";

#[derive(Default)]
pub struct Options {
    pub demo: bool,
    pub platform: Option<Platform>,
    pub shell: Option<Shell>,
    pub open: Option<String>,
    pub search: Option<String>,
    pub page: Option<i32>,
    pub size: Option<(u32, u32)>,
    pub screenshot: Option<PathBuf>,
    /// Input to replay before the screenshot: `down,right,ok,back,click:640,300,...`.
    pub script: Vec<String>,
    /// Milliseconds to wait after the script before the screenshot is taken.
    pub settle_ms: u64,
    /// Overrides where downloads and the cache go (Android passes its app directories).
    pub dirs: Option<Dirs>,
}

impl Options {
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut o = Options::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"))
            };
            match arg.as_str() {
                "--demo" => o.demo = true,
                "--ui" => {
                    o.shell = Some(match value("--ui")?.as_str() {
                        "desktop" => Shell::Desktop,
                        "mobile" => Shell::Mobile,
                        "tv" => Shell::Tv,
                        other => {
                            return Err(format!(
                                "unknown interface \"{other}\" (desktop, mobile, tv)"
                            ))
                        }
                    })
                }
                "--as" => {
                    let name = value("--as")?;
                    o.platform = Some(Platform::preset(&name).ok_or_else(|| {
                        format!("unknown platform \"{name}\". Known: {}", PRESETS.join(", "))
                    })?);
                }
                "--open" => o.open = Some(value("--open")?),
                "--search" => o.search = Some(value("--search")?),
                "--page" => o.page = value("--page")?.parse().ok(),
                "--size" => {
                    let v = value("--size")?;
                    let parsed = v
                        .split_once('x')
                        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                    o.size = Some(
                        parsed
                            .ok_or_else(|| format!("--size expects WIDTHxHEIGHT, got \"{v}\""))?,
                    );
                }
                "--screenshot" => o.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--script" => {
                    o.script = value("--script")?.split(' ').map(str::to_string).collect()
                }
                "--settle" => {
                    o.settle_ms = value("--settle")?
                        .parse()
                        .map_err(|_| "--settle expects milliseconds".to_string())?
                }
                "-h" | "--help" => return Err(USAGE.to_string()),
                other => return Err(format!("unknown option \"{other}\"\n\n{USAGE}")),
            }
        }
        Ok(o)
    }
}

// ---- core data -> UI data ---------------------------------------------------------

const TINTS: [u32; 10] = [
    0x5b8def, 0x8e6cf0, 0xe0679a, 0xe8804a, 0xd9a521, 0x4fb477, 0x2fa7a0, 0x3d9bd8, 0x7a86a8,
    0xc65fd0,
];

fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => "?".to_string(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
    };
    let mut chars = letters.chars();
    match (chars.next(), chars.next()) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_lowercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".to_string(),
    }
}

fn card(app: &AppSummary) -> AppCard {
    let hash = app
        .id
        .key
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    let popularity = match (app.popularity, app.id.source) {
        (Some(n), SourceKind::GitHub) => format!("★ {}", human_count(n)),
        (Some(n), SourceKind::Aur) => format!("{} votes", human_count(n)),
        (None, _) => String::new(),
    };
    AppCard {
        id: app.id.to_string().into(),
        name: app.name.clone().into(),
        author: app.author.clone().into(),
        summary: app.summary.clone().into(),
        source: app.id.source.label().into(),
        badge: app.install_label.clone().unwrap_or_default().into(),
        version: app.version.clone().unwrap_or_default().into(),
        popularity: popularity.into(),
        initials: initials(&app.name).into(),
        tint: slint::Color::from_argb_encoded(0xff00_0000 | TINTS[hash as usize % TINTS.len()]),
    }
}

fn cards(apps: &[AppSummary]) -> ModelRc<AppCard> {
    ModelRc::new(VecModel::from(apps.iter().map(card).collect::<Vec<_>>()))
}

fn sections(list: &[CoreSection]) -> Vec<Section> {
    list.iter()
        .map(|s| Section {
            title: s.title.clone().into(),
            apps: cards(&s.apps),
        })
        .collect()
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
        verb: o.plan.verb.clone().into(),
        steps: o.plan.steps.join("\n").into(),
        command: o.plan.command.clone().unwrap_or_default().into(),
    }
}

// ---- application state ------------------------------------------------------------

struct App {
    ui: slint::Weak<AppWindow>,
    catalog: Arc<Catalog>,
    dirs: Dirs,
    featured: Vec<CoreSection>,
    /// Bumped per request so a slow, superseded answer is dropped instead of shown.
    search_generation: AtomicU64,
    details_generation: AtomicU64,
    /// Install options of the app currently on the details page.
    options: Mutex<Vec<InstallOption>>,
}

impl App {
    fn show_search(&self, ui: &AppWindow, query: &str, outcome: &SearchOutcome) {
        let store = ui.global::<Store>();
        store.set_searching(false);
        store.set_searched(true);
        store.set_results(cards(&outcome.apps));
        let mut note = if outcome.apps.is_empty() {
            format!(
                "Nothing installable on {} matches “{query}”.",
                self.catalog.platform.title()
            )
        } else {
            format!(
                "{} for “{query}” on {}.",
                plural(outcome.apps.len(), "app"),
                self.catalog.platform.title()
            )
        };
        if outcome.hidden > 0 {
            note.push_str(&format!(
                " {} more matched but have no file for this device.",
                outcome.hidden
            ));
        }
        for e in &outcome.errors {
            note.push_str(&format!(" {e}"));
        }
        store.set_search_note(note.into());
        // TV shows results as the first row of the same grid.
        let mut rows = Vec::new();
        if !outcome.apps.is_empty() {
            rows.push(Section {
                title: format!("Results for “{query}”").into(),
                apps: cards(&outcome.apps),
            });
        }
        rows.extend(sections(&self.featured));
        store.set_tv_rows(ModelRc::new(VecModel::from(rows)));
    }

    fn show_details(&self, ui: &AppWindow, details: &AppDetails) {
        let store = ui.global::<Store>();
        store.set_details_loading(false);
        // The list's display name ("Tidewave") reads better than the repository's ("tidewave").
        let mut app_card = card(&details.summary);
        let shown = store.get_app();
        if shown.id == app_card.id && !shown.name.is_empty() && shown.name != shown.id {
            app_card.name = shown.name;
            app_card.initials = shown.initials;
        }
        store.set_app(app_card);
        store.set_license(details.summary.license.clone().unwrap_or_default().into());
        store.set_warning(details.summary.warning.clone().unwrap_or_default().into());
        store.set_unavailable(
            details
                .unavailable_reason
                .clone()
                .unwrap_or_default()
                .into(),
        );
        let (options, hidden, notes, date) = match &details.release {
            Some(r) => (
                r.options.clone(),
                r.hidden.clone(),
                plain_notes(&r.notes),
                r.published.clone(),
            ),
            None => (vec![], vec![], String::new(), String::new()),
        };
        store.set_options(ModelRc::new(VecModel::from(
            options.iter().map(option_row).collect::<Vec<_>>(),
        )));
        store.set_hidden(ModelRc::new(VecModel::from(
            hidden
                .iter()
                .map(|h| HiddenRow {
                    file: h.name.clone().into(),
                    reason: h.reason.clone().into(),
                })
                .collect::<Vec<_>>(),
        )));
        store.set_notes(notes.into());
        store.set_release_date(date.into());
        *self.options.lock().unwrap() = options;
    }

    fn begin_details(&self, ui: &AppWindow, id: &str) -> Option<AppId> {
        let store = ui.global::<Store>();
        store.set_details_open(true);
        store.set_details_error(SharedString::new());
        store.set_unavailable(SharedString::new());
        store.set_warning(SharedString::new());
        store.set_notes(SharedString::new());
        store.set_release_date(SharedString::new());
        store.set_license(SharedString::new());
        store.set_options(ModelRc::default());
        store.set_hidden(ModelRc::default());
        store.set_install_state(0);
        self.options.lock().unwrap().clear();
        // Show what the list already knew while the releases load.
        let known = find_card(&store, id);
        let parsed = AppId::parse(id);
        store.set_app(known.unwrap_or_else(|| AppCard {
            id: id.into(),
            name: id.into(),
            ..Default::default()
        }));
        match parsed {
            Some(app_id) => {
                store.set_details_loading(true);
                Some(app_id)
            }
            None => {
                store.set_details_loading(false);
                store.set_details_error(format!("“{id}” is not an app id.").into());
                None
            }
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn find_card(store: &Store, id: &str) -> Option<AppCard> {
    let in_sections = store
        .get_sections()
        .iter()
        .flat_map(|s| s.apps.iter().collect::<Vec<_>>())
        .find(|c| c.id == id);
    in_sections.or_else(|| store.get_results().iter().find(|c| c.id == id))
}

fn shell_for(form_factor: FormFactor) -> Shell {
    match form_factor {
        FormFactor::Desktop => Shell::Desktop,
        FormFactor::Mobile => Shell::Mobile,
        FormFactor::Tv => Shell::Tv,
    }
}

pub fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let mut platform = options.platform.clone().unwrap_or_else(Platform::detect);
    // Forcing the TV interface also means "prefer TV builds", and vice versa.
    match options.shell {
        Some(Shell::Tv) => platform.form_factor = FormFactor::Tv,
        Some(_) if platform.form_factor == FormFactor::Tv => {
            platform.form_factor = FormFactor::Desktop
        }
        _ => {}
    }
    let dirs = options.dirs.clone().unwrap_or_else(Dirs::for_current_user);
    let catalog = Arc::new(if options.demo {
        Catalog::demo(platform.clone())
    } else {
        Catalog::live(
            platform.clone(),
            std::env::var("GITHUB_TOKEN").ok(),
            dirs.cache.clone(),
        )
    });

    let ui = AppWindow::new()?;
    let store = ui.global::<Store>();
    let shell = options
        .shell
        .unwrap_or_else(|| shell_for(platform.form_factor));
    store.set_shell(shell);
    // A forced interface stays put; otherwise desktop and mobile follow the window width.
    store.set_adaptive(options.shell.is_none());
    store.set_demo(options.demo);
    store.set_platform_title(platform.title().into());
    store.set_facts(ModelRc::new(VecModel::from(
        platform
            .facts()
            .into_iter()
            .map(|(name, value)| Fact {
                name: name.into(),
                value: value.into(),
            })
            .collect::<Vec<_>>(),
    )));

    let featured = catalog.featured();
    store.set_sections(ModelRc::new(VecModel::from(sections(&featured))));
    store.set_tv_rows(ModelRc::new(VecModel::from(sections(&featured))));

    let app = Arc::new(App {
        ui: ui.as_weak(),
        catalog,
        dirs,
        featured,
        search_generation: AtomicU64::new(0),
        details_generation: AtomicU64::new(0),
        options: Mutex::new(Vec::new()),
    });

    store.on_search({
        let app = app.clone();
        move |query| {
            let Some(ui) = app.ui.upgrade() else { return };
            let query = query.trim().to_string();
            if query.is_empty() {
                return;
            }
            let store = ui.global::<Store>();
            store.set_details_open(false);
            store.set_page(1);
            store.set_searching(true);
            store.set_search_note(SharedString::new());
            let generation = app.search_generation.fetch_add(1, Ordering::SeqCst) + 1;
            let app = app.clone();
            std::thread::spawn(move || {
                let outcome = app.catalog.search(&query);
                let weak = app.ui.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if app.search_generation.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    if let Some(ui) = weak.upgrade() {
                        app.show_search(&ui, &query, &outcome);
                    }
                });
            });
        }
    });

    store.on_open({
        let app = app.clone();
        move |id| {
            let Some(ui) = app.ui.upgrade() else { return };
            let generation = app.details_generation.fetch_add(1, Ordering::SeqCst) + 1;
            let Some(app_id) = app.begin_details(&ui, &id) else {
                return;
            };
            let app = app.clone();
            std::thread::spawn(move || {
                let result = app.catalog.details(&app_id).map_err(|e| e.to_string());
                let weak = app.ui.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if app.details_generation.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    let Some(ui) = weak.upgrade() else { return };
                    match result {
                        Ok(details) => app.show_details(&ui, &details),
                        Err(message) => {
                            let store = ui.global::<Store>();
                            store.set_details_loading(false);
                            store.set_details_error(message.into());
                        }
                    }
                });
            });
        }
    });

    store.on_close_details({
        let app = app.clone();
        move || {
            if let Some(ui) = app.ui.upgrade() {
                // Invalidate any details request still in flight.
                app.details_generation.fetch_add(1, Ordering::SeqCst);
                ui.global::<Store>().set_details_open(false);
            }
        }
    });

    store.on_install({
        let app = app.clone();
        move |index| {
            let Some(ui) = app.ui.upgrade() else { return };
            let store = ui.global::<Store>();
            if store.get_install_state() == 1 {
                return;
            }
            let Some(option) = app.options.lock().unwrap().get(index as usize).cloned() else {
                return;
            };
            store.set_install_index(index);
            store.set_install_state(1);
            store.set_install_progress(0.0);
            store.set_install_message(SharedString::new());
            let app = app.clone();
            std::thread::spawn(move || {
                let weak = app.ui.clone();
                let mut last_percent = 0u64;
                let mut progress = |done: u64, total: Option<u64>| {
                    let Some(total) = total.filter(|t| *t > 0) else {
                        return;
                    };
                    let percent = done * 100 / total;
                    // One UI update per percent is plenty.
                    if percent != last_percent {
                        last_percent = percent;
                        let weak = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.global::<Store>()
                                    .set_install_progress(percent as f32 / 100.0);
                            }
                        });
                    }
                };
                let http = app.catalog.http();
                let result = install::execute(
                    &option.plan,
                    &option.asset,
                    http.as_ref(),
                    &app.dirs,
                    &install::open_with_system,
                    &mut progress,
                );
                let weak = app.ui.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let store = ui.global::<Store>();
                    match result {
                        Ok(outcome) => {
                            store.set_install_state(2);
                            store.set_install_message(outcome.message.into());
                        }
                        Err(e) => {
                            store.set_install_state(3);
                            store.set_install_message(e.to_string().into());
                        }
                    }
                });
            });
        }
    });

    if let Some((w, h)) = options.size {
        ui.window()
            .set_size(slint::LogicalSize::new(w as f32, h as f32));
    } else if shell == Shell::Tv && options.screenshot.is_none() {
        ui.window().set_fullscreen(true);
    }
    if let Some(page) = options.page {
        store.set_page(page);
    }

    if let Some(query) = &options.search {
        store.set_query(query.as_str().into());
        store.invoke_search(query.as_str().into());
    }
    if let Some(id) = &options.open {
        store.invoke_open(id.as_str().into());
    }

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
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(250),
        move || {
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
                    other => {
                        if let Some((x, y)) =
                            other.strip_prefix("click:").and_then(|p| p.split_once(','))
                        {
                            let position = slint::LogicalPosition::new(
                                x.parse().unwrap_or(0.0),
                                y.parse().unwrap_or(0.0),
                            );
                            window.dispatch_event(WindowEvent::PointerMoved { position });
                            window.dispatch_event(WindowEvent::PointerPressed {
                                position,
                                button: PointerEventButton::Left,
                            });
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
                        let result = ui
                            .window()
                            .take_snapshot()
                            .map_err(|e| e.to_string())
                            .and_then(|pixels| {
                                let file =
                                    std::fs::File::create(&path).map_err(|e| e.to_string())?;
                                let mut encoder = png::Encoder::new(
                                    std::io::BufWriter::new(file),
                                    pixels.width(),
                                    pixels.height(),
                                );
                                encoder.set_color(png::ColorType::Rgba);
                                encoder.set_depth(png::BitDepth::Eight);
                                let mut writer =
                                    encoder.write_header().map_err(|e| e.to_string())?;
                                writer
                                    .write_image_data(pixels.as_bytes())
                                    .map_err(|e| e.to_string())
                            });
                        if let Err(e) = result {
                            eprintln!("screenshot failed: {e}");
                        }
                        let _ = slint::quit_event_loop();
                    });
                }
            }
        },
    );
}

/// Android entry point (NativeActivity / GameActivity via `android-activity`).
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android_app: slint::android::AndroidApp) {
    let internal = android_app
        .internal_data_path()
        .unwrap_or_else(std::env::temp_dir);
    let downloads = android_app
        .external_data_path()
        .unwrap_or_else(|| internal.clone());
    slint::android::init(android_app).expect("Slint Android backend failed to start");
    let dirs = Dirs {
        downloads: downloads.join("downloads"),
        applications: internal.join("apps"),
        cache: internal.join("cache"),
    };
    if let Err(e) = run(Options {
        dirs: Some(dirs),
        ..Default::default()
    }) {
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
        assert_eq!(
            plain_notes("## New\n\n- **Fast** `startup`\n* Fixes"),
            "New\n\n•  Fast startup\n•  Fixes"
        );
    }

    #[test]
    fn options_parse() {
        let o = Options::from_args(
            [
                "--demo",
                "--ui",
                "tv",
                "--size",
                "1920x1080",
                "--as",
                "android-tv",
            ]
            .map(String::from)
            .into_iter(),
        )
        .unwrap();
        assert!(o.demo);
        assert_eq!(o.shell, Some(Shell::Tv));
        assert_eq!(o.size, Some((1920, 1080)));
        assert!(Options::from_args(["--ui", "watch"].map(String::from).into_iter()).is_err());
    }
}
