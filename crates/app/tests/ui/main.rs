//! End-to-end tests of the three interfaces.
//!
//! Each test builds the real window on Slint's headless testing backend, with the demo
//! catalog and throw-away directories, and drives it the way a person would: clicking
//! elements found by their accessible label, or sending key presses. Catalog work runs
//! inline (`Options::inline`), so every assertion sees the finished result.
//!
//! The tests are in the modules below, one per part of the store. What they share (a
//! `Session` holding the window, and ways to press and find things in it) is here.

use bilby::{build, App, AppCard, AppWindow, Options, Shell, SourceChoice, Store, Theme, VersionChoice};
use bilby_core::install::Dirs;
use bilby_core::model::{AppId, AppSummary};
use bilby_core::state::State;
use bilby_core::Platform;
use i_slint_backend_testing::{ElementHandle, ElementRoot};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

mod accounts;
mod details;
mod device;
mod discover;
mod family;
mod flatpak;
mod fuse;
mod layouts;
mod library;
mod outside;
mod reviews;
mod search;
mod settings;
mod tv;
mod updates;
mod web_apps;

struct Session {
    ui: AppWindow,
    app: Arc<App>,
    dirs: Dirs,
    state_file: PathBuf,
    /// Files the store handed to "the system installer" (recorded, never launched).
    opened: Arc<Mutex<Vec<PathBuf>>>,
    /// Programs the store started with arguments (Longboard, Flatpak): which, and with what.
    web_opened: Arc<Mutex<Vec<(PathBuf, String)>>>,
    /// What the store put in the desktop's notifications.
    notified: Arc<Mutex<Vec<String>>>,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_root() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("bilby-ui-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn dirs_in(root: &std::path::Path) -> Dirs {
    Dirs {
        downloads: root.join("downloads"),
        applications: root.join("Applications"),
        menu: root.join("menu"),
        flatpak: root.join("flatpak-program"),
        elevate: root.join("elevate-program"),
        terminal: None,
        flatpak_roots: vec![root.join("flatpak")],
        programs: Vec::new(),
        cache: root.join("cache"),
        config: root.join("config"),
        desktop: Some(root.join("Desktop")),
    }
}

thread_local! {
    static BACKEND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The testing backend is per thread and may be set up only once on each.
fn init_backend() {
    if !BACKEND.replace(true) {
        i_slint_backend_testing::init_no_event_loop();
    }
}

/// Start the store as `preset` with the given interface. `seed` is written as the
/// saved state first, to test start-up with an existing library.
fn start_with(preset: &str, shell: Shell, seed: Option<&str>) -> Session {
    start_on(preset, shell, seed, None)
}

/// The same, on a made-up machine of the test's own.
fn start_on(preset: &str, shell: Shell, seed: Option<&str>, host: Option<Arc<dyn bilby_core::os::Host>>) -> Session {
    init_backend();
    bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
    let root = temp_root();
    let dirs = dirs_in(&root);
    let state_file = dirs.config.join("demo-state.json");
    if let Some(json) = seed {
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(&state_file, json).unwrap();
    }
    // A stand-in for the password prompt: it notes what it was asked to run and succeeds.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dirs.elevate.parent().unwrap()).unwrap();
        let log = dirs.elevate.with_extension("log");
        std::fs::write(&dirs.elevate, format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display())).unwrap();
        std::fs::set_permissions(&dirs.elevate, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let record = opened.clone();
    let launched = opened.clone();
    let browsed = opened.clone();
    let web_opened: Arc<Mutex<Vec<(PathBuf, String)>>> = Arc::default();
    let web_record = web_opened.clone();
    let notified: Arc<Mutex<Vec<String>>> = Arc::default();
    let said = notified.clone();
    let options = Options {
        demo: true,
        inline: true,
        platform: Some(Platform::preset(preset).unwrap()),
        shell: Some(shell),
        dirs: Some(dirs.clone()),
        opener: Some(Arc::new(move |path: &std::path::Path| {
            record.lock().unwrap().push(path.to_path_buf());
            Ok(())
        })),
        // No real browser is started: what would have been opened is noted.
        browser: Some(Arc::new(move |url: &str| {
            // As the real one does, it refuses anything that is not a plain https link.
            if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(bilby_core::Error::Unsupported(format!("Refusing to open \"{url}\": not an https link")));
            }
            browsed.lock().unwrap().push(PathBuf::from(url));
            Ok(())
        })),
        launcher: Some(Arc::new(move |path: &std::path::Path| {
            launched.lock().unwrap().push(path.to_path_buf());
            Ok(())
        })),
        notifier: Some(Arc::new(move |_title: &str, body: &str| said.lock().unwrap().push(body.to_string()))),
        runner: Some(Arc::new(move |program: &std::path::Path, args: &[String]| {
            web_record.lock().unwrap().push((program.to_path_buf(), args.join(" ")));
            // A Longboard asked to hand back a Steam library: the person agrees.
            if let Some(to) = args.iter().find_map(|a| a.strip_prefix("--hand-back-to=")) {
                std::fs::write(to, r#"{"steamid":"76561198000000001","owned":[570,440]}"#).unwrap();
            }
            Ok(())
        })),
        // The store's sign-in window: the person signs in and Steam answers.
        signin: Some(Arc::new(|site: &str, session: &std::path::Path| {
            std::fs::create_dir_all(session)?;
            std::fs::write(session.join("cookies"), b"stand-in")?;
            Ok(Some(match site {
                // GOG hands back a one-time code, which the store then asks GOG about.
                "gog" => r#"{"code":"one-time-code-1"}"#.to_string(),
                _ => r#"{"steamid":"76561198000000001","owned":[620,440,10]}"#.to_string(),
            }))
        })),
        host,
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let (w, h) = match shell {
        Shell::Mobile => (412.0, 892.0),
        Shell::Tv => (1920.0, 1080.0),
        // Tall, so a whole page is in view: the test backend only finds what is visible,
        // and how much fits in a normal window differs between systems.
        Shell::Desktop => (1280.0, 3000.0),
    };
    ui.window().set_size(slint::LogicalSize::new(w, h));
    ui.show().unwrap();
    Session { ui, app, dirs, state_file, opened, web_opened, notified }
}

/// A store that has been opened before (the first-run welcome is out of the way).
fn start(preset: &str, shell: Shell) -> Session {
    start_with(preset, shell, Some(r#"{"settings":{"welcomed":true}}"#))
}

impl Session {
    fn store(&self) -> Store<'_> {
        self.ui.global::<Store>()
    }

    /// Elements with this accessible label, controls before plain text (a `Text` is
    /// labelled by its own content, so a heading can share a button's label).
    fn labelled(&self, label: &str) -> Vec<ElementHandle> {
        let mut found: Vec<ElementHandle> = ElementHandle::find_by_accessible_label(&self.ui, label).collect();
        // The settings show one section at a time. A person looking for a setting
        // looks through the sections; so does this, staying on the one that has it.
        if found.is_empty() && self.store().get_page() == 3 {
            let showing = self.store().get_settings_section();
            for section in 0..=6 {
                self.store().set_settings_section(section);
                found = ElementHandle::find_by_accessible_label(&self.ui, label).collect();
                if !found.is_empty() {
                    break;
                }
            }
            if found.is_empty() {
                self.store().set_settings_section(showing);
            }
        }
        found.sort_by_key(|e| {
            e.accessible_role() == Some(slint::private_unstable_api::re_exports::AccessibleRole::Text)
        });
        found
    }

    fn has(&self, label: &str) -> bool {
        !self.labelled(label).is_empty()
    }

    /// Press the element with this accessible label: a real pointer press at its centre
    /// when it is in view. When the page has scrolled it out of view (how much fits
    /// differs between systems), a person would scroll to it first; the test activates
    /// it directly instead, which exercises the same handler.
    fn click(&self, label: &str) {
        let found = self.labelled(label);
        let element = found.first().unwrap_or_else(|| panic!("nothing on screen is labelled {label:?}"));
        let window = self.ui.window().size().to_logical(self.ui.window().scale_factor());
        let (position, size) = (element.absolute_position(), element.size());
        let centre = (position.x + size.width / 2.0, position.y + size.height / 2.0);
        let in_view = centre.0 >= 0.0 && centre.0 <= window.width && centre.1 >= 0.0 && centre.1 <= window.height;
        if in_view {
            element.mock_single_click(PointerEventButton::Left);
        } else {
            element.invoke_accessible_default_action();
        }
    }

    /// Trigger the element's default accessible action, as a screen reader would.
    fn activate(&self, label: &str) {
        let found = self.labelled(label);
        found
            .first()
            .unwrap_or_else(|| panic!("nothing on screen is labelled {label:?}"))
            .invoke_accessible_default_action();
    }

    fn press(&self, key: impl Into<SharedString>) {
        let text: SharedString = key.into();
        self.ui.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.ui.window().dispatch_event(WindowEvent::KeyReleased { text });
    }

    fn saved(&self) -> State {
        State::load(&self.state_file)
    }

    fn section_names(&self) -> Vec<(String, Vec<String>)> {
        self.store()
            .get_sections()
            .iter()
            .map(|s| (s.title.to_string(), s.apps.iter().map(|a| a.name.to_string()).collect()))
            .collect()
    }

    fn option_labels(&self) -> Vec<String> {
        self.store().get_options().iter().map(|o| o.label.to_string()).collect()
    }
}

// ---- first run --------------------------------------------------------------------

// ---- discover ---------------------------------------------------------------------

// ---- details and install ----------------------------------------------------------

// ---- library and updates ----------------------------------------------------------

const OLD_KESTREL: &str = r#"{"library":[{"id":"github:kestrel-sh/kestrel","name":"Kestrel","author":"kestrel-sh",
  "summary":"A friendly prompt","version":"v1.20.0","label":"Portable archive · x86_64","file":"k.tar.gz","installed_at":1700000000}]}"#;

// ---- search -----------------------------------------------------------------------

// ---- settings ---------------------------------------------------------------------

/// A store whose "own repository" is the given demo project.
fn start_as_store(preset: &str, store_app: &str) -> Session {
    start_as_store_with(preset, store_app, r#"{"settings":{"welcomed":true}}"#)
}

fn start_as_store_with(preset: &str, store_app: &str, seed: &str) -> Session {
    init_backend();
    let root = temp_root();
    let dirs = dirs_in(&root);
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), seed).unwrap();
    let options = Options {
        demo: true,
        inline: true,
        platform: Some(Platform::preset(preset).unwrap()),
        shell: Some(Shell::Desktop),
        dirs: Some(dirs.clone()),
        opener: Some(Arc::new(|_| Ok(()))),
        store_app: Some(store_app.to_string()),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 2400.0));
    ui.show().unwrap();
    let state_file = dirs.config.join("demo-state.json");
    Session { ui, app, dirs, state_file, opened: Arc::default(), web_opened: Arc::default(), notified: Arc::default() }
}

// ---- keyboard, mobile, TV ---------------------------------------------------------

/// Every control on screen ends inside the window: nothing is pushed off the right edge.
fn assert_fits(s: &Session, what: &str) {
    let width = s.ui.window().size().to_logical(s.ui.window().scale_factor()).width;
    let root = s.ui.root_element();
    for role in [slint::private_unstable_api::re_exports::AccessibleRole::Button] {
        for element in root.query_descendants().match_accessible_role(role).find_all() {
            let right = element.absolute_position().x + element.size().width;
            let label = element.accessible_label().unwrap_or_default();
            // The row of filters above the lists scrolls sideways: its chips may run
            // past the edge, and how many fit depends on the font.
            if s.store().get_categories().iter().any(|name| name == label) {
                continue;
            }
            assert!(right <= width + 0.5, "{what}: \"{label}\" ends at {right}, the window is {width} wide");
        }
    }
}

/// A stand-in for the `flatpak` program: it does what the real one would do to the
/// folders the store looks at, and writes down how it was called.
#[cfg(unix)]
fn fake_flatpak(s: &Session) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let log = s.dirs.cache.join("flatpak-calls.txt");
    let apps = s.dirs.flatpak_roots[0].join("app");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1\" in\n  install) for last; do :; done; mkdir -p '{apps}'/\"$last\" ;;\nesac\nexit 0\n",
        log = log.display(),
        apps = apps.display(),
    );
    std::fs::create_dir_all(s.dirs.flatpak.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&s.dirs.cache).unwrap();
    std::fs::write(&s.dirs.flatpak, script).unwrap();
    std::fs::set_permissions(&s.dirs.flatpak, std::fs::Permissions::from_mode(0o755)).unwrap();
    log
}

// ---- the operating system's own updates ----

// ---- which layout: detected, chosen in Settings, or asked once ----

/// Start without forcing a layout, as a person would: the saved choice and the device
/// decide. `controller` is whether a game controller is attached.
fn start_unforced(preset: &str, seed: &str, controller: bool) -> (AppWindow, Arc<App>, PathBuf) {
    init_backend();
    let dirs = dirs_in(&temp_root());
    let state_file = dirs.config.join("demo-state.json");
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(&state_file, seed).unwrap();
    let options = Options {
        demo: true,
        inline: true,
        platform: Some(Platform::preset(preset).unwrap()),
        dirs: Some(dirs),
        controller: Some(controller),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 900.0));
    ui.show().unwrap();
    (ui, app, state_file)
}

// ---- getting around: every page has a way back, and a remote reaches everything ----

// ---- signing in to GitHub with a code ----

// ---- choosing where apps come from ----

// ---- one app, several sources ----

// ---- apps that are versions of one service ----

// ---- signing in to Steam ----

// ---- what an app needs before it will run ----

// ---- adult content, Family mode and the PIN ----

fn adult_site() -> &'static bilby_core::source::web::Site {
    bilby_core::source::web::sites().iter().find(|site| site.adult).unwrap()
}

// ---- where downloads go ----

// ---- the person's own sources ----

// ---- sounds ----

// ---- the store updating itself ----
