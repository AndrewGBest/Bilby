//! End-to-end tests of the three interfaces.
//!
//! Each test builds the real window on Slint's headless testing backend, with the demo
//! catalog and throw-away directories, and drives it the way a person would: clicking
//! elements found by their accessible label, or sending key presses. Catalog work runs
//! inline (`Options::inline`), so every assertion sees the finished result.

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

struct Session {
    ui: AppWindow,
    app: Arc<App>,
    dirs: Dirs,
    state_file: PathBuf,
    /// Files the store handed to "the system installer" (recorded, never launched).
    opened: Arc<Mutex<Vec<PathBuf>>>,
    /// Programs the store started with arguments (Longboard, Flatpak): which, and with what.
    web_opened: Arc<Mutex<Vec<(PathBuf, String)>>>,
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
        cache: root.join("cache"),
        config: root.join("config"),
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
    let web_opened: Arc<Mutex<Vec<(PathBuf, String)>>> = Arc::default();
    let web_record = web_opened.clone();
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
        launcher: Some(Arc::new(move |path: &std::path::Path| {
            launched.lock().unwrap().push(path.to_path_buf());
            Ok(())
        })),
        runner: Some(Arc::new(move |program: &std::path::Path, args: &[String]| {
            web_record.lock().unwrap().push((program.to_path_buf(), args.join(" ")));
            // A Longboard asked to hand back a Steam library: the person agrees.
            if let Some(to) = args.iter().find_map(|a| a.strip_prefix("--hand-back-to=")) {
                std::fs::write(to, r#"{"steamid":"76561198000000001","owned":[570,440]}"#).unwrap();
            }
            Ok(())
        })),
        // The store's sign-in window: the person signs in and Steam answers.
        signin: Some(Arc::new(|session: &std::path::Path| {
            std::fs::create_dir_all(session)?;
            std::fs::write(session.join("cookies"), b"stand-in")?;
            Ok(Some(r#"{"steamid":"76561198000000001","owned":[620,440,10]}"#.to_string()))
        })),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let (w, h) = match shell {
        Shell::Mobile => (412.0, 892.0),
        Shell::Tv => (1920.0, 1080.0),
        // Tall, so a whole page is in view: the test backend only finds what is visible,
        // and how much fits in a normal window differs between systems.
        Shell::Desktop => (1280.0, 2400.0),
    };
    ui.window().set_size(slint::LogicalSize::new(w, h));
    ui.show().unwrap();
    Session { ui, app, dirs, state_file, opened, web_opened }
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

#[test]
fn first_run_explains_the_store_once() {
    let s = start_with("linux-x64-debian", Shell::Desktop, None);
    assert!(s.store().get_show_welcome());
    assert!(s.has("Welcome to Bilby"));
    assert!(!s.has("View Tidewave"), "the welcome takes the spotlight's place");
    s.click("Got it");
    assert!(!s.store().get_show_welcome());
    assert!(!s.has("Welcome to Bilby"));
    assert!(s.has("View Tidewave"));
    assert!(s.saved().settings.welcomed);

    let saved = std::fs::read_to_string(&s.state_file).unwrap();
    let again = start_with("linux-x64-debian", Shell::Desktop, Some(&saved));
    assert!(!again.has("Welcome to Bilby"), "it does not come back");

    let phone = start_with("android-arm64", Shell::Mobile, None);
    assert!(phone.has("Welcome to Bilby"));
    phone.click("Got it");
    assert!(!phone.has("Welcome to Bilby"));
}

// ---- discover ---------------------------------------------------------------------

#[test]
fn discover_shows_only_what_runs_on_this_machine() {
    let s = start("linux-x64-debian", Shell::Desktop);
    assert!(s.has("Tidewave") && s.has("Lumen") && s.has("Relay"));
    // Windows-only, macOS-only and Android-only apps are not there at all.
    for absent in ["Winspect", "Barley", "Fern Launcher", "Atlas"] {
        assert!(!s.has(absent), "{absent} must not be listed on Linux");
    }
    // No AUR on Debian.
    assert!(!s.section_names().iter().any(|(title, _)| title == "Arch User Repository"));

    let arch = start("linux-x64-arch", Shell::Desktop);
    assert!(arch.section_names().iter().any(|(title, _)| title == "Arch User Repository"));

    let mac = start("macos-arm64", Shell::Desktop);
    assert!(mac.has("Barley") && !mac.has("Winspect"));

    let win = start("windows-x64", Shell::Desktop);
    assert!(win.has("Winspect") && !win.has("Barley"));
}

#[test]
fn category_chips_filter_discover() {
    let s = start("linux-x64-debian", Shell::Desktop);
    assert!(s.has("Tidewave") && s.has("Lumen"));
    s.click("Developer tools");
    assert_eq!(s.store().get_category(), "Developer tools");
    assert!(s.has("Lumen"));
    // Tidewave is a Media app; with the spotlight hidden too, it is gone from the page.
    assert!(!s.has("Tidewave") && !s.has("View Tidewave"));
    s.click("All");
    assert!(s.has("Tidewave"));
}

// ---- details and install ----------------------------------------------------------

#[test]
fn install_an_appimage_end_to_end() {
    let s = start("linux-x64-fedora", Shell::Desktop);
    s.click("Tidewave");
    assert!(s.store().get_details_open());
    assert!(!s.store().get_details_loading());
    assert_eq!(s.option_labels(), ["RPM package · x86_64", "AppImage · x86_64", "Portable archive · x86_64"]);
    assert_eq!(s.store().get_hidden().row_count(), 13);
    assert_eq!(s.store().get_installed_version(), "");

    // The page leads with the best file; the others are one press away.
    assert!(!s.has("Install AppImage · x86_64"));
    s.click("Other versions and files");
    s.click("Install AppImage · x86_64");
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert!(s.store().get_install_message().starts_with("Checksum verified."));
    let file = s.dirs.applications.join("Tidewave-3.4.0-x86_64.AppImage");
    assert!(file.is_file(), "the AppImage is in the Applications folder");
    assert!(s.opened.lock().unwrap().is_empty(), "an AppImage needs no system installer");
    assert_eq!(s.store().get_toast(), "Tidewave is in your library");
    assert_eq!(s.store().get_installed_version(), "v3.4.0");

    // It is in the library, on screen and on disk.
    let saved = s.saved();
    assert_eq!(saved.library.len(), 1);
    assert_eq!(saved.library[0].id, "github:tidewave/tidewave");
    assert_eq!(saved.library[0].name, "Tidewave");
    assert_eq!(saved.library[0].version, "v3.4.0");
    assert!(saved.library[0].store_managed);
    // And in the desktop's app menu, pointing at the file.
    let entry = s.dirs.menu.join("bilby-github-tidewave-tidewave.desktop");
    let text = std::fs::read_to_string(&entry).expect("a menu entry was written");
    assert!(text.contains("Name=Tidewave\n") && text.contains("Tidewave-3.4.0-x86_64.AppImage"), "{text}");
    assert!(s.store().get_install_message().ends_with("It is in your app menu."));

    // The button that installed it now opens it, as in any store.
    assert!(s.store().get_can_open());
    s.click("Open Tidewave");
    assert_eq!(s.opened.lock().unwrap().last(), Some(&file), "the AppImage itself is started");
    assert_eq!(s.store().get_toast(), "Opening Tidewave…");

    // Back on Discover the tile now says so.
    s.click("Back");
    assert!(!s.store().get_details_open());
    let media = s.store().get_sections().row_data(0).unwrap();
    assert!(media.apps.row_data(0).unwrap().installed);

    // Remove it again from the library page: asks first, then deletes the file.
    s.click("Library");
    assert_eq!(s.store().get_page(), 2);
    s.click("Remove Tidewave");
    assert!(file.is_file(), "one click only asks for confirmation");
    s.click("Confirm removing Tidewave");
    assert!(!file.exists(), "the store deletes the AppImage it placed");
    assert!(!entry.exists(), "and takes it out of the app menu");
    assert!(s.saved().library.is_empty());
    assert_eq!(s.store().get_toast(), "Tidewave uninstalled");
    assert!(s.has("Browse apps"), "the empty library offers a way forward");
}

#[test]
fn packages_are_recorded_but_never_deleted_by_the_store() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.click("Lumen");
    s.click("Install Debian package · x86_64");
    assert_eq!(s.store().get_install_state(), 2);
    let saved = s.saved();
    assert!(!saved.library[0].store_managed);
    let downloaded = PathBuf::from(saved.library[0].path.clone().unwrap());
    assert!(downloaded.is_file());
    assert_eq!(*s.opened.lock().unwrap(), vec![downloaded.clone()], "the .deb was handed to the system, once");
    assert!(s.store().get_install_message().contains("Opened"));
    s.app.remove_from_library(&s.ui, "github:lumen-labs/lumen");
    assert!(downloaded.is_file(), "a package handed to the system is not ours to delete");
    assert_eq!(s.store().get_toast(), "Lumen removed from your library");
}

#[test]
fn each_device_is_offered_its_own_file_first() {
    let cases = [
        ("windows-x64", "Installer (.msi) · x86_64"),
        ("windows-arm64", "Installer (.msi) · arm64"),
        ("macos-arm64", "Disk image (.dmg) · universal"),
        ("linux-x64-debian", "Debian package · x86_64"),
        ("linux-x64-arch", "AppImage · x86_64"),
        ("android-arm64", "APK · arm64"),
        ("ios", "iOS app (.ipa)"),
    ];
    for (preset, expected) in cases {
        let s = start(preset, Shell::Desktop);
        s.app.open(&s.ui, "github:tidewave/tidewave");
        assert_eq!(s.option_labels()[0], expected, "{preset}");
    }
    let tv = start("android-tv", Shell::Tv);
    tv.app.open(&tv.ui, "github:tidewave/tidewave");
    assert_eq!(tv.option_labels()[0], "APK · arm64 · TV");
}

#[test]
fn app_without_a_build_for_this_machine_says_so() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.open(&s.ui, "github:winspect/winspect");
    assert!(s.store().get_unavailable().contains("Linux"));
    assert_eq!(s.store().get_options().row_count(), 0);
    assert_eq!(s.store().get_hidden().row_count(), 3);
    s.click("Other versions and files");
    assert!(s.has("Files that won't work on this device"));
    // They can still be had: saved to the downloads folder, not installed.
    s.activate("Files that won't work on this device");
    s.activate("Download anyway: WinspectSetup-0.77-x64.exe");
    assert!(s.store().get_toast().starts_with("Checksum verified. Saved to "), "{}", s.store().get_toast());
    assert!(s.dirs.downloads.join("WinspectSetup-0.77-x64.exe").is_file());
    assert!(s.saved().library.is_empty(), "a file for another device is not an installed app");
}

#[test]
fn the_app_page_warns_about_what_matters() {
    // A current, stable release: nothing to warn about.
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert_eq!(s.store().get_warning(), "");
    assert!(!s.store().get_release_age().is_empty());

    // The AUR's own "out of date" flag is passed on.
    let arch = start("linux-x64-arch", Shell::Desktop);
    arch.app.open(&arch.ui, "aur:quillfeed");
    assert!(arch.store().get_warning().contains("out of date"), "{}", arch.store().get_warning());

    // An abandoned project says so; so does a pre-release.
    arch.app.open(&arch.ui, "github:oldtimer/relic");
    assert!(arch.store().get_warning().contains("years old"), "{}", arch.store().get_warning());
    assert!(arch.store().get_release_age().ends_with("years ago"));
    arch.app.open(&arch.ui, "github:edge-labs/nightjar");
    assert!(arch.store().get_warning().contains("pre-release"), "{}", arch.store().get_warning());
    assert_eq!(arch.store().get_app().version, "v0.9.0-beta.3");
}

#[test]
fn an_older_version_can_be_chosen_and_installed() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.open(&s.ui, "github:kestrel-sh/kestrel");
    let versions: Vec<String> = s.store().get_versions().iter().map(|v| v.to_string()).collect();
    assert_eq!(versions, ["v1.21.0", "v1.20.0"]);
    assert_eq!(s.store().get_selected_version(), 0);
    assert_eq!(s.store().get_app().version, "v1.21.0");
    assert_eq!(s.store().get_options().row_count(), 2);

    assert!(!s.has("Version v1.20.0"), "older versions are not in the way");
    s.click("Other versions and files");
    s.click("Version v1.20.0");
    assert_eq!(s.store().get_selected_version(), 1);
    assert_eq!(s.store().get_app().version, "v1.20.0");
    assert_eq!(s.store().get_options().row_count(), 1, "the older release had one Linux x86_64 file");
    assert_eq!(s.store().get_release_date(), "2026-09-14");
    assert!(s.store().get_warning().contains("older version"), "{}", s.store().get_warning());

    s.app.install(&s.ui, 0);
    assert_eq!(s.store().get_install_state(), 2);
    assert_eq!(s.saved().library[0].version, "v1.20.0");
    assert_eq!(s.store().get_installed_version(), "v1.20.0");
    // The library knows a newer one exists, without another request.
    assert_eq!(s.store().get_updates_count(), 1);
    assert_eq!(s.store().get_library().row_data(0).unwrap().status_text, "Update available: v1.21.0");

    // Back to the newest: the warning goes away.
    s.click("Version v1.21.0");
    assert_eq!(s.store().get_warning(), "");
    assert_eq!(s.store().get_options().row_count(), 2);

    // An app with a single usable release shows no picker.
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert_eq!(s.store().get_versions().row_count(), 1);
    assert!(!s.has("Version v3.4.0"));
    // Out-of-range selections are ignored.
    s.store().invoke_select_version(7);
    assert_eq!(s.store().get_selected_version(), 0);
}

#[test]
fn failed_lookups_can_be_retried() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.open(&s.ui, "github:nobody/nothing");
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_details_error(), "GitHub has no project called “nobody/nothing”.");
    assert_eq!(s.store().get_app().source, "GitHub", "the page still says where it looked");
    assert!(s.has("Try again"));
    s.click("Try again");
    assert!(!s.store().get_details_error().is_empty(), "still failing, still explained");
    assert!(!s.store().get_details_loading());

    s.app.open(&s.ui, "not an id");
    assert!(s.store().get_details_error().contains("not an app id"));
}

#[test]
fn aur_packages_show_a_command_and_are_not_tracked() {
    let s = start("linux-x64-arch", Shell::Desktop);
    s.app.open(&s.ui, "aur:lumen-bin");
    let option = s.store().get_options().row_data(0).unwrap();
    assert_eq!(option.verb, "Build and install");
    assert_eq!(option.command, "paru -S lumen-bin");
    assert!(!option.verified);
    // Building asks questions, so it happens in a terminal. This machine has none
    // (the test says so), and then the command is what can be offered.
    s.app.install(&s.ui, 0);
    assert_eq!(s.store().get_install_state(), 2);
    assert!(s.store().get_install_message().contains("No terminal was found"));
    assert!(s.store().get_install_message().contains("paru -S lumen-bin"));
    assert!(s.saved().library.is_empty(), "the store did not install it, so it does not claim to");
}

// ---- library and updates ----------------------------------------------------------

const OLD_KESTREL: &str = r#"{"library":[{"id":"github:kestrel-sh/kestrel","name":"Kestrel","author":"kestrel-sh",
  "summary":"A friendly prompt","version":"v1.20.0","label":"Portable archive · x86_64","file":"k.tar.gz","installed_at":1700000000}]}"#;

#[test]
fn updates_are_found_and_applied() {
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(OLD_KESTREL));
    assert_eq!(s.store().get_library().row_count(), 1);
    assert_eq!(s.store().get_updates_count(), 0, "nothing is claimed before checking");

    s.click("Library");
    s.click("Check for updates");
    assert_eq!(s.store().get_updates_count(), 1);
    let row = s.store().get_library().row_data(0).unwrap();
    assert_eq!(row.status, 3);
    assert_eq!(row.status_text, "Update available: v1.21.0");
    assert_eq!(s.store().get_toast(), "1 update available");
    assert_eq!(s.store().get_last_checked(), "just now");

    // One click updates in place: no detour through the app page.
    s.click("Update Kestrel");
    assert!(!s.store().get_details_open());
    assert_eq!(s.saved().library[0].version, "v1.21.0");
    assert_eq!(s.store().get_updates_count(), 0);
    assert_eq!(s.store().get_library().row_data(0).unwrap().status, 2);
    assert_eq!(s.store().get_toast(), "Kestrel updated to v1.21.0");
    assert_eq!(s.store().get_updating_id(), "");
    assert_eq!(s.store().get_install_state(), 0);
    assert!(s.dirs.downloads.join("kestrel-x86_64-unknown-linux-gnu.tar.gz").is_file());

    // The app page is still one click away, and agrees.
    s.click("Details of Kestrel");
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_installed_version(), "v1.21.0");
    assert_eq!(s.store().get_app().version, "v1.21.0");
}

#[test]
fn update_all_works_through_the_queue_and_survives_a_failure() {
    // Two real updates with a broken entry between them.
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"},
        {"id":"github:winspect/winspect","name":"Winspect","version":"v0.1"},
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.0.0"},
        {"id":"github:lumen-labs/lumen","name":"Lumen","version":"v0.19.2"}]}"#;
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(seed));
    s.store().set_page(2);
    assert!(!s.has("Update all"), "nothing to update before a check");
    s.app.check_updates(&s.ui, false);
    assert_eq!(s.store().get_updates_count(), 2);
    assert!(s.has("Update all"));

    s.click("Update all");
    let versions: Vec<(String, String)> =
        s.saved().library.iter().map(|a| (a.name.clone(), a.version.clone())).collect();
    assert!(versions.contains(&("Kestrel".into(), "v1.21.0".into())), "{versions:?}");
    assert!(versions.contains(&("Tidewave".into(), "v3.4.0".into())), "{versions:?}");
    assert!(versions.contains(&("Lumen".into(), "v0.19.2".into())), "up-to-date apps are left alone");
    assert!(versions.contains(&("Winspect".into(), "v0.1".into())));
    assert_eq!(s.store().get_updates_count(), 0);
    assert_eq!(s.store().get_updating_id(), "");
    assert!(!s.has("Update all"), "the button goes away when there is nothing left");
}

#[test]
fn updating_an_app_that_lost_its_build_says_so() {
    // Winspect has nothing for Linux; pretend it was in the library anyway.
    let seed = r#"{"settings":{"welcomed":true},"library":[{"id":"github:winspect/winspect","name":"Winspect","version":"v0.1"}]}"#;
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(seed));
    s.app.update_app(&s.ui, "github:winspect/winspect");
    assert_eq!(s.store().get_toast(), "Winspect no longer has a file for this device");
    assert!(s.store().get_toast_is_error());
    assert_eq!(s.store().get_updating_id(), "");
    assert_eq!(s.saved().library[0].version, "v0.1", "the library is untouched");
    // Unknown ids and apps outside the library are ignored.
    s.app.update_app(&s.ui, "github:tidewave/tidewave");
    s.app.update_app(&s.ui, "nonsense");
    assert_eq!(s.store().get_updating_id(), "");
}

#[test]
fn update_check_reports_apps_it_could_not_check() {
    let seed = r#"{"library":[{"id":"github:gone/gone","name":"Gone","version":"v1"}]}"#;
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(seed));
    s.app.check_updates(&s.ui, true);
    let row = s.store().get_library().row_data(0).unwrap();
    assert_eq!(row.status, 4);
    assert!(row.status_text.starts_with("Couldn't check"));
    assert_eq!(s.store().get_toast(), "Couldn't check 1 app");
    assert!(s.store().get_toast_is_error());
}

#[test]
fn a_damaged_state_file_does_not_stop_the_app() {
    let s = start_with("linux-x64-debian", Shell::Desktop, Some("{{{ not json"));
    assert_eq!(s.store().get_library().row_count(), 0);
    assert!(s.has("Tidewave"), "the store still starts and shows apps");
    let set_aside = s.state_file.with_file_name("demo-state.json.corrupt");
    assert_eq!(std::fs::read_to_string(set_aside).unwrap(), "{{{ not json");
}

// ---- search -----------------------------------------------------------------------

#[test]
fn search_lists_only_installable_apps_and_says_what_it_left_out() {
    let mac = start("macos-arm64", Shell::Desktop);
    mac.app.search(&mac.ui, "monitor");
    assert_eq!(mac.store().get_page(), 1);
    assert!(mac.store().get_searched() && !mac.store().get_searching());
    let names: Vec<String> = mac.store().get_results().iter().map(|a| a.id.to_string()).collect();
    assert_eq!(names, ["github:barley/barley"]);
    assert!(mac.store().get_search_note().contains("1 more matched but do not work on this device"));
    assert_eq!(mac.store().get_results().row_data(0).unwrap().badge, "Disk image (.dmg)");

    let phone = start("android-arm64", Shell::Mobile);
    phone.app.search(&phone.ui, "monitor");
    assert_eq!(phone.store().get_results().row_count(), 0);
    assert!(phone.store().get_search_note().contains("2 more matched"));
    assert!(phone.has("No apps for this device") || phone.store().get_results().row_count() == 0);
}

#[test]
fn pasting_a_link_opens_the_app_and_searches_are_remembered() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.search(&s.ui, "https://github.com/lumen-labs/lumen/releases");
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_app().id, "github:lumen-labs/lumen");
    assert_eq!(s.option_labels()[0], "Debian package · x86_64");
    assert!(s.saved().recent_searches.is_empty(), "a link is not a search");

    s.app.search(&s.ui, "editor");
    assert!(!s.store().get_details_open(), "searching leaves the app page");
    s.app.search(&s.ui, "backup");
    assert_eq!(s.saved().recent_searches, ["backup", "editor"]);
    assert_eq!(s.store().get_recent().row_count(), 2);

    // An AUR link on a system without the AUR is treated as plain text, not opened.
    s.app.search(&s.ui, "aur:lumen-bin");
    assert!(!s.store().get_details_open());

    s.app.search(&s.ui, "x");
    assert_eq!(s.store().get_search_note(), "Type at least two characters.");
}

#[test]
fn typing_suggests_featured_apps_without_searching() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.typing(&s.ui, "lum");
    assert_eq!(s.store().get_page(), 1, "typing brings up the search page");
    let names: Vec<String> = s.store().get_suggestions().iter().map(|a| a.name.to_string()).collect();
    assert_eq!(names, ["Lumen"]);
    assert!(!s.store().get_searched() && !s.store().get_searching(), "nothing was sent anywhere");
    assert!(s.saved().recent_searches.is_empty(), "keystrokes are not history");
    assert!(s.has("Lumen"));

    // A suggestion opens like any other tile.
    s.click("Lumen");
    assert_eq!(s.store().get_app().id, "github:lumen-labs/lumen");
    s.click("Back");

    // Enter runs the real search and replaces the suggestions.
    s.app.search(&s.ui, "lum");
    assert_eq!(s.store().get_suggestions().row_count(), 0);
    assert_eq!(s.store().get_results().row_data(0).unwrap().id, "github:lumen-labs/lumen");

    // Typing again clears the old results so the two never mix.
    s.app.typing(&s.ui, "tide");
    assert_eq!(s.store().get_results().row_count(), 0);
    assert_eq!(s.store().get_suggestions().row_data(0).unwrap().name, "Tidewave");
    s.app.typing(&s.ui, "");
    assert_eq!(s.store().get_suggestions().row_count(), 0);
}

#[test]
fn system_repositories_are_searched_first_and_installed_packages_marked() {
    // An Arch-style machine with CachyOS repositories configured.
    let s = start("linux-x64-arch", Shell::Desktop);
    let facts: Vec<(String, String)> =
        s.store().get_facts().iter().map(|f| (f.name.to_string(), f.value.to_string())).collect();
    assert!(facts.contains(&("System repositories".into(), "cachyos-v3, extra, core".into())), "{facts:?}");

    s.app.search(&s.ui, "kernel");
    let hit = s.store().get_results().row_data(0).unwrap();
    assert_eq!(hit.id, "pacman:swiftkernel");
    assert_eq!(hit.source, "cachyos-v3", "the card names the repository");
    assert_eq!(hit.badge, "Repository package");
    assert_eq!(hit.version, "6.17.4-2");
    assert!(!hit.installed);

    s.app.open(&s.ui, "pacman:swiftkernel");
    assert!(s.store().get_is_package());
    let option = s.store().get_options().row_data(0).unwrap();
    assert_eq!(option.label, "Repository package · x86_64");
    assert_eq!(option.verb, "Install");
    assert_eq!(option.command, "sudo pacman -S --noconfirm --needed swiftkernel");
    assert!(s.store().get_notes().contains("“cachyos-v3” repository"));
    // One press: pacman installs it behind the system's password prompt, and keeps
    // track of it itself, so nothing goes in the store's library.
    s.app.install(&s.ui, 0);
    #[cfg(unix)]
    assert_eq!(s.store().get_install_message(), "Installed.");
    assert!(s.saved().library.is_empty());

    // A package pacman already has, with a newer version in the repository.
    s.app.search(&s.ui, "containers");
    let harbor = s.store().get_results().row_data(0).unwrap();
    assert_eq!((harbor.id.as_str(), harbor.installed), ("pacman:harbor", true));
    s.app.open(&s.ui, "pacman:harbor");
    assert_eq!(s.store().get_installed_version(), "5.2.0-1");
    assert_eq!(s.store().get_app().version, "5.3.1-1");

    // The same machine type without pacman has no such source; apt takes its place.
    let debian = start("linux-x64-debian", Shell::Desktop);
    debian.app.search(&debian.ui, "containers");
    let hit = debian.store().get_results().row_data(0).unwrap();
    assert_eq!(hit.id, "apt:harbor");
    assert_eq!(hit.source, "noble-updates/universe");
    debian.app.open(&debian.ui, "apt:harbor");
    assert_eq!(debian.store().get_options().row_data(0).unwrap().command, "sudo apt-get install -y harbor");
    debian.app.open(&debian.ui, "pacman:harbor");
    assert!(!debian.store().get_details_error().is_empty(), "pacman packages are not offered on a Debian system");

    // And nowhere else.
    let windows = start("windows-x64", Shell::Desktop);
    windows.app.search(&windows.ui, "containers");
    assert_eq!(windows.store().get_results().row_count(), 0);
}

#[test]
fn aur_results_appear_only_on_arch() {
    let arch = start("linux-x64-arch", Shell::Desktop);
    arch.app.search(&arch.ui, "lumen");
    let sources: Vec<String> = arch.store().get_results().iter().map(|a| a.source.to_string()).collect();
    assert_eq!(sources, ["GitHub", "AUR", "AUR"]);

    let debian = start("linux-x64-debian", Shell::Desktop);
    debian.app.search(&debian.ui, "lumen");
    let sources: Vec<String> = debian.store().get_results().iter().map(|a| a.source.to_string()).collect();
    assert_eq!(sources, ["GitHub"]);
}

// ---- settings ---------------------------------------------------------------------

#[test]
fn settings_apply_immediately_and_survive_a_restart() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.click("Settings");
    assert_eq!(s.store().get_page(), 3);

    s.click("Light");
    assert_eq!(s.ui.global::<Theme>().get_mode(), 1);
    assert_eq!(s.saved().settings.theme, "light");

    assert!(!s.store().get_include_prereleases());
    s.activate("Include pre-releases");
    assert!(s.store().get_include_prereleases());
    assert!(s.saved().settings.include_prereleases);

    assert!(!s.store().get_has_token());
    s.store().invoke_save_token("  ghp_example  ".into());
    assert!(s.store().get_has_token());
    assert_eq!(s.saved().settings.github_token, "ghp_example");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&s.state_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the file holding the token is private");
    }
    assert!(s.has("Sign out of GitHub"));
    s.click("Sign out of GitHub");
    assert!(!s.store().get_has_token());
    assert_eq!(s.saved().settings.github_token, "");

    // A second launch on the same files comes back the same way.
    let saved = std::fs::read_to_string(&s.state_file).unwrap();
    let again = start_with("linux-x64-debian", Shell::Desktop, Some(&saved));
    assert_eq!(again.store().get_theme_mode(), 1);
    assert!(again.store().get_include_prereleases());
}

#[test]
fn start_up_update_checks_can_be_turned_off_and_the_downloads_folder_opened() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(s.store().get_check_updates_on_start(), "on by default");
    s.activate("Check for updates when the store opens");
    assert!(!s.store().get_check_updates_on_start());
    assert!(s.saved().settings.skip_update_check_on_start);
    s.activate("Check for updates when the store opens");
    assert!(!s.saved().settings.skip_update_check_on_start);

    s.store().invoke_open_downloads();
    assert_eq!(*s.opened.lock().unwrap(), vec![s.dirs.downloads.clone()]);
    assert!(s.dirs.downloads.is_dir(), "the folder exists by the time it is opened");
}

#[test]
fn the_library_lists_waiting_updates_first() {
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.4.0"},
        {"id":"github:lumen-labs/lumen","name":"Lumen","version":"v0.19.2"},
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(seed));
    let names =
        |s: &Session| -> Vec<String> { s.store().get_library().iter().map(|r| r.card.name.to_string()).collect() };
    assert_eq!(names(&s), ["Tidewave", "Lumen", "Kestrel"], "saved order before anything is known");
    s.app.check_updates(&s.ui, false);
    assert_eq!(names(&s), ["Kestrel", "Tidewave", "Lumen"], "the app with an update moves to the top");
}

/// A store whose "own repository" is the given demo project.
fn start_as_store(preset: &str, store_app: &str) -> Session {
    init_backend();
    let root = temp_root();
    let dirs = dirs_in(&root);
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
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
    Session { ui, app, dirs, state_file, opened: Arc::default(), web_opened: Arc::default() }
}

#[test]
fn the_store_finds_its_own_updates_like_any_other_apps() {
    // Kestrel v1.21.0 stands in for a newer release of the store (which is 0.x).
    let s = start_as_store("linux-x64-debian", "github:kestrel-sh/kestrel");
    s.store().set_page(3);
    assert_eq!(s.store().get_store_update(), "", "nothing is claimed before checking");
    assert!(!s.has("View the store's update"));
    s.app.check_store_update(&s.ui);
    assert_eq!(s.store().get_store_update(), "v1.21.0");
    assert_eq!(s.store().get_toast(), "", "it does not interrupt");

    // The update is an ordinary app page: the file for this device, checksum and all.
    // (The About card is far down the page, so the button is pressed through its action.)
    s.store().invoke_view_store_update();
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_app().id, "github:kestrel-sh/kestrel");
    assert_eq!(s.option_labels()[0], "Portable archive · x86_64");

    // A newer release with nothing for this device is not an update for this device.
    let no_build = start_as_store("linux-x64-debian", "github:winspect/winspect");
    no_build.app.check_store_update(&no_build.ui);
    assert_eq!(no_build.store().get_store_update(), "");
    // A check that fails stays silent rather than alarming.
    let gone = start_as_store("linux-x64-debian", "github:nobody/nothing");
    gone.app.check_store_update(&gone.ui);
    assert_eq!(gone.store().get_store_update(), "");
    assert_eq!(gone.store().get_toast(), "");
    // The demo has no such project unless a test names one.
    let demo = start("linux-x64-debian", Shell::Desktop);
    demo.app.check_store_update(&demo.ui);
    assert_eq!(demo.store().get_store_update(), "");
}

#[test]
fn clearing_the_cache_only_touches_the_cache() {
    let s = start("linux-x64-debian", Shell::Desktop);
    let cached = s.dirs.cache.join("http").join("x.json");
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, "{}").unwrap();
    std::fs::create_dir_all(&s.dirs.downloads).unwrap();
    let keep = s.dirs.downloads.join("keep.bin");
    std::fs::write(&keep, "x").unwrap();
    s.store().invoke_clear_cache();
    assert!(!cached.exists());
    assert!(keep.is_file());
    assert_eq!(s.store().get_toast(), "Cached catalog data cleared");
}

#[test]
fn unsafe_links_are_refused() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().invoke_open_url("file:///etc/passwd".into());
    assert!(s.store().get_toast_is_error());
    assert!(s.store().get_toast().contains("not an https link"));
}

// ---- keyboard, mobile, TV ---------------------------------------------------------

#[test]
fn escape_leaves_the_app_page_on_desktop() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.click("Tidewave");
    assert!(s.store().get_details_open());
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
}

#[test]
fn desktop_works_without_a_mouse() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // Tab walks the sidebar: Discover, Search, Library, Settings.
    s.press(Key::Tab);
    s.press(Key::Tab);
    s.press(Key::Tab);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 2, "third Tab stop is Library");
    s.press(Key::Tab);
    s.press(" ");
    assert_eq!(s.store().get_page(), 3, "Space activates like Enter");

    // Keep tabbing from the sidebar into the page until an app opens with Enter.
    let s = start("linux-x64-debian", Shell::Desktop);
    let mut opened = false;
    for _ in 0..12 {
        s.press(Key::Tab);
        if s.labelled("View Tidewave").is_empty() {
            break;
        }
    }
    for _ in 0..12 {
        s.press(Key::Return);
        if s.store().get_details_open() {
            opened = true;
            break;
        }
        s.press(Key::Tab);
    }
    assert!(opened, "an app can be opened with Tab and Enter alone");
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
}

#[test]
fn mobile_has_bottom_navigation_and_back() {
    let s = start("android-arm64", Shell::Mobile);
    assert_eq!(s.ui.get_effective(), Shell::Mobile);
    assert!(s.has("Fern Launcher"), "Android-only apps are listed on a phone");
    assert!(!s.has("Lumen"), "desktop-only apps are not");
    s.click("Tidewave");
    assert!(s.store().get_details_open());
    assert_eq!(s.option_labels()[0], "APK · arm64");
    assert!(!s.option_labels().iter().any(|l| l.contains("TV")), "TV builds are hidden on a phone");
    s.click("Back");
    assert!(!s.store().get_details_open());
    // Android's Back button does the same.
    s.click("Tidewave");
    s.press(Key::Back);
    assert!(!s.store().get_details_open());
    s.click("Library");
    assert_eq!(s.store().get_page(), 2);
    s.click("Settings");
    assert_eq!(s.store().get_page(), 3);
    s.click("Discover");
    assert_eq!(s.store().get_page(), 0);
}

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

#[test]
fn everything_fits_on_the_narrowest_phone() {
    let seed = r#"{"settings":{"welcomed":true},"recent_searches":["editor","backup"],"library":[
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.3.0","label":"APK · arm64","installed_at":1794000000},
        {"id":"github:relay-app/relay","name":"Relay","version":"v1.15.0","label":"APK · arm64","installed_at":1790000000}]}"#;
    let s = start_with("android-arm64", Shell::Mobile, Some(seed));
    s.ui.window().set_size(slint::LogicalSize::new(320.0, 640.0));
    s.app.check_updates(&s.ui, true);
    assert_eq!(s.store().get_updates_count(), 2, "so that Update all is on the page being measured");
    assert_fits(&s, "discover");
    for (page, name) in [(1, "search"), (2, "library"), (3, "settings")] {
        s.store().set_page(page);
        assert_fits(&s, name);
    }
    s.app.search(&s.ui, "play");
    assert_fits(&s, "search results");
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert_fits(&s, "app page");
    s.app.open(&s.ui, "github:nobody/nothing");
    assert_fits(&s, "app page with an error");

    // And the first-run welcome.
    let fresh = start_with("android-arm64", Shell::Mobile, None);
    fresh.ui.window().set_size(slint::LogicalSize::new(320.0, 640.0));
    assert_fits(&fresh, "welcome");
}

#[test]
fn every_button_has_a_name_a_screen_reader_can_say() {
    use slint::private_unstable_api::re_exports::AccessibleRole;
    let seed = r#"{"recent_searches":["editor"],"library":[{"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    for (preset, shell) in [("linux-x64-arch", Shell::Desktop), ("android-arm64", Shell::Mobile)] {
        let s = start_with(preset, shell, Some(seed));
        s.app.check_updates(&s.ui, false);
        let check = |what: &str| {
            for role in [AccessibleRole::Button, AccessibleRole::Tab, AccessibleRole::Switch] {
                for element in s.ui.root_element().query_descendants().match_accessible_role(role).find_all() {
                    let label = element.accessible_label().unwrap_or_default();
                    assert!(!label.trim().is_empty(), "{preset} {what}: a control has no accessible label");
                }
            }
        };
        check("welcome and discover");
        for page in 1..=3 {
            s.store().set_page(page);
            check(&format!("page {page}"));
        }
        s.app.search(&s.ui, "lumen");
        check("search results");
        s.app.open(&s.ui, "github:tidewave/tidewave");
        check("app page");
        s.app.open(&s.ui, "github:nobody/nothing");
        check("app page with an error");
    }
}

#[test]
fn slash_and_ctrl_k_start_a_search_from_the_keyboard() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // "/" moves the focus to the search field; what is typed next lands in it.
    s.press("/");
    s.press("l");
    s.press("u");
    assert_eq!(s.store().get_query(), "lu");
    assert_eq!(s.store().get_page(), 1, "typing brings up suggestions");
    assert_eq!(s.store().get_suggestions().row_data(0).unwrap().name, "Lumen");
    // Enter searches.
    s.press(Key::Return);
    assert!(s.store().get_searched());
    assert_eq!(
        s.store().get_results().row_data(0).unwrap().id,
        "github:lumen-labs/lumen",
        "the app first, web apps after"
    );
    assert_eq!(s.saved().recent_searches, ["lu"]);
    // Escape leaves the field; Ctrl+K comes back to it and selects the text, so typing replaces it.
    s.press(Key::Escape);
    s.ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    s.press("k");
    s.ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    s.press("t");
    assert_eq!(s.store().get_query(), "t");
}

#[test]
fn discover_points_at_waiting_updates() {
    let seed = r#"{"settings":{"welcomed":true},"library":[{"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(seed));
    assert!(!s.has("Show updates"), "nothing is claimed before a check");
    s.app.check_updates(&s.ui, false);
    assert!(s.has("Show updates"));
    s.click("Show updates");
    assert_eq!(s.store().get_page(), 2);
}

#[test]
fn a_settings_file_that_cannot_be_written_is_reported_not_fatal() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // Replace the config directory with a plain file: saving must now fail.
    let config = s.state_file.parent().unwrap().to_path_buf();
    std::fs::remove_dir_all(&config).unwrap();
    std::fs::write(&config, "in the way").unwrap();
    s.store().invoke_set_prereleases(true);
    assert!(s.store().get_toast_is_error());
    assert!(s.store().get_toast().starts_with("Couldn't save your settings"));
    assert!(s.store().get_include_prereleases(), "the choice still applies for this session");
    assert!(s.has("Tidewave") || s.store().get_page() == 0, "and the app carries on");
}

#[test]
fn window_width_picks_desktop_or_mobile() {
    init_backend();
    let root = temp_root();
    let options = Options {
        demo: true,
        inline: true,
        platform: Some(Platform::preset("linux-x64-debian").unwrap()),
        dirs: Some(dirs_in(&root)),
        ..Default::default()
    };
    let (ui, _app) = build(&options).unwrap();
    ui.show().unwrap();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    assert_eq!(ui.get_effective(), Shell::Desktop);
    ui.window().set_size(slint::LogicalSize::new(420.0, 800.0));
    assert_eq!(ui.get_effective(), Shell::Mobile);
    ui.window().set_size(slint::LogicalSize::new(900.0, 800.0));
    assert_eq!(ui.get_effective(), Shell::Desktop);
}

#[test]
fn tv_is_driven_entirely_by_the_remote() {
    let s = start("android-tv", Shell::Tv);
    assert_eq!(s.ui.get_effective(), Shell::Tv);
    let rows = s.store().get_tv_rows();
    assert_eq!(rows.row_data(0).unwrap().title, "Made for TV");
    let second = rows.row_data(0).unwrap().apps.row_data(1).unwrap();

    // Right, OK: open the second app in the first row.
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_app().id, second.id);

    // OK installs the focused (best) option; APK hand-off is not implemented, and the
    // interface says so instead of pretending.
    s.press(Key::Return);
    assert_eq!(s.store().get_install_state(), 3);
    assert!(s.store().get_install_message().contains("not implemented"));

    // Back returns to the grid; Down, OK opens the first app of the next row.
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
    s.press(Key::LeftArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    let first_of_next = rows.row_data(1).unwrap().apps.row_data(0).unwrap();
    assert_eq!(s.store().get_app().id, first_of_next.id);
}

#[test]
fn tv_focus_survives_rows_changing_under_it() {
    let s = start("android-tv", Shell::Tv);
    // Move to the last row and its last tile, then make the grid shorter by searching
    // for something with a single result row in front.
    for _ in 0..6 {
        s.press(Key::DownArrow);
    }
    for _ in 0..6 {
        s.press(Key::RightArrow);
    }
    s.app.search(&s.ui, "play");
    // Whatever is focused now must be a real app: OK opens it.
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    let landed = s.store().get_app().id;
    assert!(landed.starts_with("github:") || landed.starts_with("web:"), "focus landed on {landed:?}");
    assert!(s.store().get_details_error().is_empty());
}

#[test]
fn the_tv_has_a_library_page_and_a_home_that_is_the_store() {
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:marquee-tv/marquee","name":"Marquee","version":"v20.0"},
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.4.0"}]}"#;
    let s = start_with("android-tv", Shell::Tv, Some(seed));
    // Home is the store: what is installed is not what it leads with.
    let home: Vec<String> = s.store().get_tv_rows().iter().map(|row| row.title.to_string()).collect();
    assert_eq!(home[0], "Made for TV", "{home:?}");
    assert!(!home.iter().any(|t| t == "Developer tools"), "nobody writes code from the sofa: {home:?}");

    s.app.check_updates(&s.ui, false);
    assert_eq!(s.store().get_toast(), "", "a start-up check stays quiet");
    let library: Vec<String> = s.store().get_tv_library_rows().iter().map(|row| row.title.to_string()).collect();
    assert_eq!(
        library,
        ["This TV", "Updates available", "Installed with Bilby"],
        "the device and the store itself come first"
    );
    assert!(s.has("Library  ·  1 update"), "the top bar says an update is waiting");

    // With the remote: Up to search, Up to the tabs, Right to Library, OK.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 2);
    assert!(s.has("Check for updates"));
    // The library's buttons have the focus; Down goes to the device's own row, Down
    // again to the apps, OK opens the first.
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_app().id, "github:marquee-tv/marquee", "the app with an update comes first");

    // Back closes the app, Back again goes up to the bar, and Back from there goes Home.
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
    assert_eq!(s.store().get_page(), 2);
    s.press(Key::Escape);
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 0);
}

#[test]
fn the_home_page_adds_the_lists_a_source_keeps() {
    // On Linux with Flatpak, what is popular on Flathub follows the bundled apps.
    let s = start("linux-x64-debian", Shell::Desktop);
    let titles: Vec<String> = s.store().get_sections().iter().map(|section| section.title.to_string()).collect();
    assert!(titles.iter().any(|t| t == "Popular on Flathub"), "{titles:?}");
    assert!(titles.iter().position(|t| t == "Popular on Flathub") > titles.iter().position(|t| t == "Developer tools"));
    let popular = s.store().get_sections().iter().find(|section| section.title == "Popular on Flathub").unwrap();
    assert_eq!(popular.apps.row_data(0).unwrap().id, "flathub:org.reelstudio.Reel", "most installed first");

    // Where Flathub is not a source, there are no such lists and nothing is asked for.
    let windows = start("windows-x64", Shell::Desktop);
    assert!(windows.store().get_sections().iter().all(|section| !section.title.contains("Flathub")));
}

#[test]
fn favourites_are_kept_in_the_library_without_installing() {
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(r#"{"settings":{"welcomed":true}}"#));
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert!(!s.store().get_is_favourite());
    s.click("Add to favourites");
    assert!(s.store().get_is_favourite());
    assert_eq!(s.saved().favourites[0].id, "github:lumen-labs/lumen");
    assert_eq!(s.saved().favourites[0].name, "Lumen");
    assert!(s.saved().library.is_empty(), "a favourite is not an install");

    // The library page lists it even though nothing is installed, and it opens from there.
    s.store().set_details_open(false);
    s.store().set_page(2);
    assert!(s.has("Favourites"));
    assert!(!s.has("Your library is empty"));
    s.activate("Lumen");
    assert!(s.store().get_details_open());
    assert!(s.store().get_is_favourite(), "the page remembers");
    s.click("Remove from favourites");
    assert!(s.saved().favourites.is_empty());
    assert!(!s.store().get_is_favourite());
}

#[test]
fn an_update_can_be_skipped_until_the_next_one() {
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"},
        {"id":"github:lumen-labs/lumen","name":"Lumen","version":"v0.19.2"}]}"#;
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(seed));
    s.store().set_page(2);
    s.app.check_updates(&s.ui, false);
    assert_eq!(s.store().get_updates_count(), 1);
    s.click("Skip this update of Kestrel");
    assert_eq!(s.store().get_updates_count(), 0);
    assert!(s.store().get_toast().contains("Skipping Kestrel v1.21.0"));
    assert_eq!(s.saved().library.iter().find(|a| a.name == "Kestrel").unwrap().skipped_version, "v1.21.0");
    let row = s.store().get_library().iter().find(|r| r.card.name == "Kestrel").unwrap();
    assert_eq!(row.status_text, "Up to date  ·  skipping v1.21.0");
    assert!(!s.has("Skip this update of Kestrel"));

    // Checking again does not bring the same version back.
    s.app.check_updates(&s.ui, false);
    assert_eq!(s.store().get_updates_count(), 0);
}

#[test]
fn an_app_page_says_what_the_project_is() {
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(r#"{"settings":{"welcomed":true}}"#));
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    let about = s.store().get_about().to_string();
    assert!(about.starts_with("Fast, keyboard-driven text editor"), "{about}");
    assert!(about.contains("•  Works offline") && !about.contains("badge") && !about.contains('<'), "{about}");
    assert!(s.has("About"));
    // A package from a repository has no README to show, and the page says nothing rather
    // than keeping the previous app's text.
    s.app.open(&s.ui, "apt:harbor");
    assert_eq!(s.store().get_about(), "");
}

#[test]
fn an_app_can_be_installed_straight_from_a_list() {
    let s = start("linux-x64-fedora", Shell::Desktop);
    s.activate("Install Kestrel now");
    assert!(s.store().get_details_open(), "its page opens so the install can be watched");
    assert_eq!(s.store().get_app().id, "github:kestrel-sh/kestrel");
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert_eq!(s.saved().library[0].id, "github:kestrel-sh/kestrel");

    // Once installed, the list offers nothing more to do for it.
    s.store().set_details_open(false);
    assert!(!s.has("Install Kestrel now"));
    // Opening a page the ordinary way never installs anything by itself.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_install_state(), 0);
    assert_eq!(s.saved().library.len(), 1);
}

#[test]
fn a_repository_package_installs_with_one_press() {
    let s = start("linux-x64-arch", Shell::Desktop);
    s.app.quick_install(&s.ui, "pacman:swiftkernel");
    assert!(s.store().get_details_open());
    assert!(s.saved().library.is_empty(), "pacman keeps track of it, not the store's library");
    #[cfg(unix)]
    {
        assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
        let asked = std::fs::read_to_string(s.dirs.elevate.with_extension("log")).unwrap();
        assert_eq!(asked.trim(), "pacman -S --noconfirm --needed swiftkernel", "run through the password prompt");
        assert_eq!(s.store().get_installed_version(), "6.17.4-2");
        assert_eq!(s.store().get_toast(), "swiftkernel is installed");
    }
}

#[test]
fn an_app_page_shows_pictures_from_the_readme() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_screenshots().row_count(), 3, "the logo and the badge are not screenshots");
    assert!(s.has("Picture 1 of Lumen"));
    let first = s.store().get_screenshots().row_data(0).unwrap();
    assert_eq!((first.size().width, first.size().height), (800, 500));
    assert!(s.dirs.cache.join("screenshots").read_dir().unwrap().count() >= 3, "they are kept for next time");

    // A package has no README, and the last app's pictures do not linger.
    s.app.open(&s.ui, "apt:harbor");
    assert_eq!(s.store().get_screenshots().row_count(), 0);
}

#[test]
fn the_tv_app_page_has_one_main_button_and_the_rest_a_step_away() {
    let s = start("android-tv", Shell::Tv);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    let id = s.store().get_app().id.to_string();
    assert_eq!(id, "github:marquee-tv/marquee");
    assert_eq!(s.store().get_screenshots().row_count(), 3, "the page has pictures to show");

    // Right moves to Favourite; OK stars the app without leaving the page.
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.store().get_is_favourite());
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_install_state(), 0, "nothing was installed by starring");

    // Right again is Versions; OK opens the list, Down picks the second file, OK takes it.
    s.press(Key::RightArrow);
    s.press(Key::Return);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_install_index(), 1, "the second file, not the recommended one");

    // Back closes the list first, and only then the page.
    s.press(Key::Escape);
    assert!(s.store().get_details_open());
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());

    // Opening another app starts on its main button again: OK goes straight to installing.
    s.press(Key::LeftArrow);
    s.press(Key::Return);
    assert_ne!(s.store().get_app().id, id);
    s.press(Key::Return);
    assert_eq!(s.store().get_install_index(), 0);
}

#[test]
fn on_a_linux_tv_a_website_is_added_and_opened_like_an_app() {
    let s = start("linux-tv", Shell::Tv);
    let rows = s.store().get_tv_rows();
    let websites = rows.iter().find(|r| r.title == "Web apps").expect("a Web apps row on a Linux TV");
    assert!(websites.apps.iter().any(|a| a.id == "web:lichess.org"));

    // Without Longboard the page says what is needed, and adding fails with the same advice.
    s.app.open(&s.ui, "web:lichess.org");
    assert_eq!(s.store().get_app().name, "Lichess");
    assert!(s.store().get_warning().contains("Longboard"), "{}", s.store().get_warning());
    s.press(Key::Return);
    assert_eq!(s.store().get_install_state(), 3);
    assert!(s.store().get_install_message().contains("Install Longboard"));
    assert!(s.saved().library.is_empty());

    // With Longboard on the device (here: a stand-in file where the store puts AppImages).
    std::fs::create_dir_all(&s.dirs.applications).unwrap();
    let longboard = s.dirs.applications.join("Longboard-x86_64.AppImage");
    std::fs::write(&longboard, b"stand-in").unwrap();
    s.app.open(&s.ui, "web:lichess.org");
    assert_eq!(s.store().get_warning(), "");
    s.press(Key::Return);
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert_eq!(s.saved().library[0].id, "web:lichess.org");
    let entry = std::fs::read_to_string(s.dirs.menu.join("bilby-web-lichess-org.desktop")).unwrap();
    assert!(entry.contains("Name=Lichess\n"), "{entry}");
    assert!(entry.contains("Longboard-x86_64.AppImage\" \"--url=https://lichess.org/\"\n"), "{entry}");

    // The same button now opens it: Longboard is started on the site's address.
    assert!(s.store().get_can_open());
    s.press(Key::Return);
    assert_eq!(
        s.web_opened.lock().unwrap().as_slice(),
        [(longboard.clone(), "--url=https://lichess.org/".to_string())]
    );

    // Removing it takes the entry out of the TV's app list.
    s.app.remove_from_library(&s.ui, "web:lichess.org");
    assert!(!s.dirs.menu.join("bilby-web-lichess-org.desktop").exists());
    assert_eq!(s.store().get_toast(), "Lichess removed from this TV's app list");
}

#[test]
fn web_apps_are_offered_on_every_kind_of_device() {
    for (preset, shell) in
        [("linux-x64-debian", Shell::Desktop), ("android-tv", Shell::Tv), ("windows-x64", Shell::Desktop)]
    {
        let s = start(preset, shell);
        assert!(s.store().get_sections().iter().any(|r| r.title == "Web apps"), "{preset}");
        s.app.open(&s.ui, "web:lichess.org");
        assert!(s.store().get_details_error().is_empty(), "{preset}: {}", s.store().get_details_error());
        assert_eq!(s.store().get_options().row_count(), 1, "{preset}: one thing to add");
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

#[test]
#[cfg(unix)]
fn a_flathub_app_is_found_installed_opened_and_removed() {
    let s = start("linux-x64-debian", Shell::Desktop);
    let log = fake_flatpak(&s);

    // Search finds it beside the GitHub projects, with its own icon address.
    s.app.search(&s.ui, "notes");
    let hit =
        s.store().get_results().iter().find(|c| c.id == "flathub:io.quillpad.Quillpad").expect("a Flathub result");
    assert_eq!(hit.source, "Flathub");
    assert_eq!(hit.icon.size().width, 128, "Flathub's icon for the app is shown");

    // Its page has Flathub's pictures and description, and one button.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_app().name, "Quillpad");
    assert_eq!(s.store().get_screenshots().row_count(), 2);
    assert!(s.store().get_about().contains("•  Works without an account"), "{}", s.store().get_about());
    assert!(!s.store().get_can_open());

    // Install: Flatpak is asked to fetch it from Flathub.
    s.click("Install Flatpak");
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "install --noninteractive --assumeyes --or-update flathub io.quillpad.Quillpad"
    );
    assert_eq!(s.saved().library[0].id, "flathub:io.quillpad.Quillpad");

    // Now the button opens it, through Flatpak.
    assert!(s.store().get_can_open());
    s.click("Open Quillpad");
    assert_eq!(s.web_opened.lock().unwrap().last().unwrap().1, "run io.quillpad.Quillpad");

    // Removing it from the library has Flatpak uninstall it.
    s.app.remove_from_library(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(
        s.web_opened.lock().unwrap().last().unwrap().1,
        "uninstall --noninteractive --assumeyes io.quillpad.Quillpad"
    );
    assert_eq!(s.store().get_toast(), "Uninstalling Quillpad…");
}

#[test]
fn a_flatpak_installed_by_something_else_is_recognised() {
    let s = start("linux-x64-debian", Shell::Desktop);
    std::fs::create_dir_all(s.dirs.flatpak_roots[0].join("app").join("org.sundial.Sundial")).unwrap();
    s.app.open(&s.ui, "flathub:org.sundial.Sundial");
    assert_eq!(s.store().get_installed_version(), "installed");
    assert!(s.store().get_can_open(), "it can be opened even though this store did not install it");
    assert!(s.has("Open Sundial"));
    assert!(s.saved().library.is_empty());
}

#[test]
fn flathub_is_only_offered_where_flatpak_is() {
    let windows = start("windows-x64", Shell::Desktop);
    windows.app.search(&windows.ui, "notes");
    assert!(!windows.store().get_results().iter().any(|c| c.id.starts_with("flathub:")));
    windows.app.open(&windows.ui, "flathub:io.quillpad.Quillpad");
    assert!(!windows.store().get_details_error().is_empty());
}

#[test]
fn star_ratings_are_shown_where_known_and_can_be_turned_off() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // What the store does when it opens.
    s.app.load_ratings(&s.ui);
    assert!(s.dirs.cache.join("ratings.json").is_file(), "one file with every app's rating");

    // In search results and on the app's page, for an app the ratings service knows.
    s.app.search(&s.ui, "notes");
    let rated = s.store().get_results().iter().find(|c| c.id == "flathub:io.quillpad.Quillpad").unwrap();
    assert_eq!((rated.rating.as_str(), rated.rating_count.as_str()), ("4.6", "821 ratings"));
    let unrated = s.store().get_results().iter().find(|c| c.id == "github:driftnotes/drift").unwrap();
    assert_eq!(unrated.rating, "", "no rating is invented for an app the service does not know");
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_app().rating, "4.6");
    assert!(s.has("★ 4.6 821 ratings"));

    // Settings: off means gone from the screen straight away, and remembered.
    s.store().invoke_set_ratings(false);
    assert!(s.saved().settings.hide_ratings);
    assert!(!s.store().get_show_ratings());
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().all(|c| c.rating.is_empty()));

    // And back on.
    s.store().invoke_set_ratings(true);
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().any(|c| c.rating == "4.6"));
}

#[test]
fn each_app_shows_the_signs_of_use_its_source_has_and_no_others() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.load_ratings(&s.ui);

    // Flathub: a score with its breakdown, installs in total and a month, a confirmed developer.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    let app = s.store().get_app();
    let spread: Vec<f32> = app.rating_spread.iter().collect();
    assert_eq!(spread.len(), 5, "five stars first, one star last");
    assert!((spread.iter().sum::<f32>() - 1.0).abs() < 0.001 && spread[0] > spread[4], "{spread:?}");
    assert!(app.verified);
    assert_eq!((app.downloads.as_str(), app.downloads_note.as_str()), ("1.3M downloads", "in total"));
    assert_eq!(app.popularity, "41k installs a month");
    assert!(s.has("4.6 out of 5, from 821 ratings") && s.has("Quillpad contributors  ✓ verified"));

    // GitHub: stars and how often its recent files were downloaded; no rating, nobody confirmed.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    let app = s.store().get_app();
    assert!(
        app.popularity.ends_with("stars") && app.downloads.ends_with("downloads"),
        "{} {}",
        app.popularity,
        app.downloads
    );
    assert_eq!(app.downloads_note, "of recent versions");
    assert_eq!((app.rating.as_str(), app.rating_spread.row_count(), app.verified), ("", 0, false));
    assert!(!s.has("4.6 out of 5, from 821 ratings") && s.has("lumen-labs"));

    // A tile has room for one figure: the rating where there is one, else downloads or stars.
    s.app.search(&s.ui, "notes");
    let results = s.store().get_results();
    assert!(
        results.iter().all(|c| !c.rating.is_empty() || !c.headline.is_empty()),
        "every demo app has some sign of use"
    );
}

#[test]
fn with_ratings_turned_off_nothing_is_fetched() {
    let s =
        start_with("linux-x64-debian", Shell::Desktop, Some(r#"{"settings":{"welcomed":true,"hide_ratings":true}}"#));
    s.app.load_ratings(&s.ui);
    assert!(!s.dirs.cache.join("ratings.json").exists());
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().all(|c| c.rating.is_empty()));
}

#[test]
fn written_reviews_appear_only_for_apps_that_have_them() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.load_ratings(&s.ui);

    // An app the ratings service knows: its reviews are on its page.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    let reviews = s.store().get_reviews();
    assert_eq!(reviews.row_count(), 3);
    let first = reviews.row_data(0).unwrap();
    assert_eq!((first.stars.as_str(), first.title.as_str()), ("★★★★★", "Exactly what I wanted"));
    assert_eq!(first.meta, "Marta  ·  2026-11-01  ·  version 4.2.0");
    assert_eq!(reviews.row_data(2).unwrap().stars, "★★☆☆☆");
    assert!(s.has("Ratings and reviews"));

    // An app it does not know: no section, and the last app's reviews do not linger.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_reviews().row_count(), 0);
    assert!(!s.has("Ratings and reviews"));

    // With ratings and reviews turned off, nothing is asked for and nothing is shown.
    s.store().invoke_set_ratings(false);
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_reviews().row_count(), 0);
}

// ---- the operating system's own updates ----

#[test]
fn the_system_s_updates_are_in_the_library_and_on_their_own_page() {
    let s = start("linux-x64-arch", Shell::Desktop);
    assert!(s.store().get_system_supported());
    assert_eq!(s.store().get_system_name(), "Arch Linux");
    assert_eq!((s.store().get_system_state(), s.store().get_system_headline().as_str()), (0, "Not checked yet"));

    s.app.check_system(&s.ui, false);
    assert_eq!((s.store().get_system_state(), s.store().get_system_headline().as_str()), (3, "6 updates available"));
    let first = s.store().get_system_updates().row_data(0).unwrap();
    assert_eq!((first.name.as_str(), first.change.as_str()), ("linux", "6.11.5.arch1-1 → 6.11.6.arch1-1"));

    // The library leads with the system, even with no apps in it yet.
    s.store().set_page(2);
    assert!(s.has("Update system") && s.has("6 updates available"));
    s.click("System details");
    assert_eq!(s.store().get_page(), 4);
    assert!(s.has("linux: 6.11.5.arch1-1 → 6.11.6.arch1-1"), "what is waiting is listed");
    assert!(s.has("Version: Rolling release: always the newest") && s.has("Memory: 16 GB"));

    // Installing them: the made-up machine has none left afterwards.
    s.click("Update now");
    assert_eq!((s.store().get_system_state(), s.store().get_system_headline().as_str()), (2, "Up to date"));
    assert_eq!(s.store().get_system_updates().row_count(), 0);
    assert_eq!(s.store().get_toast(), "The system is up to date.");
    assert!(s.store().get_system_log().contains("Updating through pacman"));
    assert!(!s.has("Update now"), "nothing left to press");
    s.store().set_page(2);
    assert!(!s.has("Update system") && s.has("Up to date"));
}

#[test]
fn systems_that_update_themselves_get_a_way_to_their_own_updater_instead() {
    let s = start("windows-x64", Shell::Desktop);
    assert!(!s.store().get_system_supported());
    s.app.check_system(&s.ui, false);
    assert_eq!(s.store().get_system_state(), 0, "nothing is checked where nothing can be");
    s.store().set_page(2);
    assert!(!s.has("System details"), "no system card in the library");
    // Still reachable from Settings, with the details and a button to Windows Update.
    s.store().set_page(3);
    s.click("System details and updates");
    assert_eq!(s.store().get_page(), 4);
    assert!(s.has("Version: 10.0.26100.2314") && !s.has("Check again"));
    s.click("Open Windows Update");
    assert_eq!(s.opened.lock().unwrap().last().unwrap().to_str(), Some("ms-settings:windowsupdate"));

    let phone = start("android-arm64", Shell::Mobile);
    assert!(!phone.store().get_system_supported() && phone.store().get_system_handoff().is_empty());
    assert!(phone.store().get_system_note().contains("Settings"));
}

#[test]
fn a_tv_remote_reaches_the_system_page_and_updates_from_it() {
    let s = start("linux-tv", Shell::Tv);
    s.app.check_system(&s.ui, false);
    assert_eq!(s.store().get_system_state(), 3);
    // Up to search, Up to the tabs, Right along them to the device's, OK.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 4);
    assert!(s.has("Update now") && s.has("Check again"));
    // The first button is the update.
    s.press(Key::Return);
    assert_eq!(s.store().get_system_headline(), "Up to date");
    assert!(!s.has("Update now"));
    // Back returns to the apps.
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 0);
}

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

#[test]
fn the_layout_chosen_in_settings_is_remembered_and_switches_at_once() {
    // A desktop computer on which the TV layout was chosen earlier starts in it.
    let seed = r#"{"settings":{"welcomed":true,"interface":"tv"}}"#;
    let (ui, app, state_file) = start_unforced("linux-x64-debian", seed, false);
    let store = ui.global::<Store>();
    assert_eq!(ui.get_effective(), Shell::Tv);
    assert_eq!((store.get_interface_mode(), store.get_device_name().as_str()), (2, "This TV"));
    let websites = |store: &Store| store.get_tv_rows().iter().any(|row| row.title == "Web apps");
    assert!(websites(&store), "web apps are offered on a TV");

    // Back to the desktop layout: at once, and saved.
    app.set_interface(&ui, 1);
    assert_eq!(ui.get_effective(), Shell::Desktop);
    assert_eq!(store.get_device_name(), "This Linux computer");
    assert_eq!(State::load(&state_file).settings.interface, "desktop");
    assert!(websites(&store), "and on a computer, to open in its own browser");

    // "Automatic" on a device that is a TV is the TV layout; "Desktop" there is honoured.
    let (tv, tv_app, _) = start_unforced("android-tv", r#"{"settings":{"welcomed":true}}"#, false);
    assert_eq!(tv.get_effective(), Shell::Tv);
    tv_app.set_interface(&tv, 1);
    assert_eq!(tv.get_effective(), Shell::Desktop);
    tv_app.set_interface(&tv, 0);
    assert_eq!(tv.get_effective(), Shell::Tv);
}

#[test]
fn a_controller_on_an_ordinary_desktop_gets_one_question() {
    let seed = r#"{"settings":{"welcomed":true}}"#;
    let (ui, _app, state_file) = start_unforced("linux-x64-arch", seed, true);
    let store = ui.global::<Store>();
    assert!(store.get_ask_tv());
    assert_eq!(ui.get_effective(), Shell::Desktop, "nothing changes until it is answered");
    store.invoke_answer_tv(false);
    assert!(!store.get_ask_tv());
    let saved = State::load(&state_file).settings;
    assert!(saved.asked_about_tv && saved.interface.is_empty());

    // It is not asked again, with or without the controller.
    let again = std::fs::read_to_string(&state_file).unwrap();
    let (later, _, _) = start_unforced("linux-x64-arch", &again, true);
    assert!(!later.global::<Store>().get_ask_tv());

    // Saying yes switches to the TV layout and remembers it.
    let (yes, _app, yes_file) = start_unforced("linux-x64-arch", seed, true);
    yes.global::<Store>().invoke_answer_tv(true);
    assert_eq!(yes.get_effective(), Shell::Tv);
    assert_eq!(State::load(&yes_file).settings.interface, "tv");

    // No controller, or a device already known to be a TV: no question.
    assert!(!start_unforced("linux-x64-arch", seed, false).0.global::<Store>().get_ask_tv());
    assert!(!start_unforced("android-tv", seed, true).0.global::<Store>().get_ask_tv());
}

#[test]
fn the_tv_layout_has_a_way_back_that_a_remote_can_reach() {
    let (ui, _app, state_file) =
        start_unforced("linux-x64-debian", r#"{"settings":{"welcomed":true,"interface":"tv"}}"#, false);
    let press = |key: slint::platform::Key| {
        let text: SharedString = key.into();
        ui.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        ui.window().dispatch_event(WindowEvent::KeyReleased { text });
    };
    // Up to search, Up to the tabs, Right along them to "This TV", OK; then right to the last button.
    press(Key::UpArrow);
    press(Key::UpArrow);
    press(Key::RightArrow);
    press(Key::RightArrow);
    press(Key::RightArrow);
    press(Key::Return);
    assert_eq!(ui.global::<Store>().get_page(), 4);
    press(Key::RightArrow);
    press(Key::RightArrow);
    press(Key::Return);
    assert_eq!(ui.get_effective(), Shell::Desktop);
    assert_eq!(State::load(&state_file).settings.interface, "desktop");
}

#[test]
fn an_appimage_adds_itself_to_the_app_menu_and_can_be_told_not_to() {
    init_backend();
    let dirs = dirs_in(&temp_root());
    let options = Options {
        demo: true,
        inline: true,
        platform: Some(Platform::preset("linux-x64-debian").unwrap()),
        dirs: Some(dirs.clone()),
        self_program: Some(PathBuf::from("/home/someone/bilby.AppImage")),
        ..Default::default()
    };
    let (ui, _app) = build(&options).unwrap();
    let entry = bilby_core::install::self_entry_path(&dirs);
    assert!(std::fs::read_to_string(&entry).unwrap().contains("Exec=\"/home/someone/bilby.AppImage\""));
    let store = ui.global::<Store>();
    assert!(store.get_can_register() && store.get_in_menu());

    // Turning it off in Settings removes the entry, and it stays off.
    store.invoke_set_in_menu(false);
    assert!(!entry.exists() && !store.get_in_menu());
    let (again, _app) = build(&options).unwrap();
    assert!(!entry.exists() && !again.global::<Store>().get_in_menu(), "the next start respects the choice");
    again.global::<Store>().invoke_set_in_menu(true);
    assert!(entry.exists());

    // Started any other way (from source, a package), there is nothing to add and no setting.
    let plain = start("linux-x64-debian", Shell::Desktop);
    assert!(!plain.store().get_can_register());
    assert!(!bilby_core::install::self_entry_path(&plain.dirs).exists());
}

// ---- getting around: every page has a way back, and a remote reaches everything ----

#[test]
fn the_device_page_goes_back_to_where_it_was_opened_from() {
    let s = start("linux-x64-arch", Shell::Desktop);
    // From the library's system card.
    s.store().set_page(2);
    s.click("System details");
    assert_eq!(s.store().get_page(), 4);
    s.click("Back");
    assert_eq!(s.store().get_page(), 2);
    // From Settings, by button and by Escape.
    s.store().set_page(3);
    s.click("System details and updates");
    assert_eq!(s.store().get_page(), 4);
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 3);
    // An app's page still closes with the same arrow.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    s.click("Back");
    assert!(!s.store().get_details_open());

    // On a phone the device page has its own arrow in the header.
    let phone = start("linux-x64-arch", Shell::Mobile);
    phone.store().set_page(3);
    phone.click("System details and updates");
    phone.click("Back");
    assert_eq!(phone.store().get_page(), 3);
}

#[test]
fn a_tv_remote_reaches_settings_and_can_remove_an_app() {
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.4.0"}]}"#;
    let s = start_with("linux-tv", Shell::Tv, Some(seed));

    // Settings have a tab of their own: Up past search to the tabs, Right to it, OK, and
    // the first setting is highlighted.
    assert!(s.has("Settings"));
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 3);
    assert!(s.store().get_show_ratings());
    s.press(Key::Return);
    assert!(!s.store().get_show_ratings(), "OK switches the focused setting");
    assert!(s.saved().settings.hide_ratings);
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.saved().settings.include_prereleases);
    // Up climbs back through the settings to the buttons, then to the top bar.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::LeftArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 2, "Library, from the top bar");

    // Past the device's own row to the installed app: its page offers Remove, which asks twice.
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open() && s.store().get_in_library());
    assert!(s.has("Install again"), "an installed app is not offered as if it were new");
    for _ in 0..3 {
        s.press(Key::RightArrow);
    }
    s.press(Key::Return);
    assert!(s.has("Confirm") && s.saved().library.len() == 1, "the first OK only asks");
    s.press(Key::Return);
    assert!(s.saved().library.is_empty());
    assert!(!s.store().get_in_library() && !s.has("Remove"));
}

#[test]
fn a_tv_remote_reads_reviews_and_picks_an_older_version() {
    // Without motion: the page is where it is going at once, as the test's clock stands still.
    let s = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    s.app.load_ratings(&s.ui);

    // An app with reviews: they are a row on its page, and OK on one reads it in full.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_reviews().row_count(), 3);
    // Down to the foot of the page and one back up is the row of reviews; OK reads the first.
    for _ in 0..5 {
        s.press(Key::DownArrow);
    }
    s.press(Key::UpArrow);
    assert!(s.has("Review: Exactly what I wanted"), "the reviews are on the page");
    s.press(Key::Return);
    assert!(s.has("Review 1: Exactly what I wanted"));
    s.press(Key::DownArrow);
    assert!(s.has("Review 2: Good, with one wish") && !s.has("Review 1: Exactly what I wanted"));
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    assert!(s.has("Review 3: Lost my window layout"), "it stops at the last one");
    // Back closes the reviews, not the page.
    s.press(Key::Escape);
    assert!(s.store().get_details_open() && !s.has("Review 3: Lost my window layout"));

    // Versions: a list to pick from. Left leaves the list instead of changing version.
    s.app.open(&s.ui, "github:kestrel-sh/kestrel");
    s.press(Key::RightArrow);
    s.press(Key::LeftArrow);
    assert!(s.has("☆  Favourite"), "the favourites button says what it is");
    let versions = s.store().get_versions().row_count();
    assert!(versions > 1, "the demo app has older releases");
    // Main button, Favourite, Versions: OK opens the list, on its "Change version" row.
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.has("Change version"));
    assert_eq!(s.store().get_selected_version(), 0);
    let newest = s.store().get_app().version;
    // OK opens the releases; Down and OK takes the next one, and the files follow it.
    s.press(Key::Return);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_selected_version(), 1, "an older version was picked from the list");
    assert_ne!(s.store().get_app().version, newest);
    assert!(s.has("Change version"), "and the list of releases closed again");
    // Left and Right no longer change the version; Left leaves the list for the buttons.
    s.press(Key::RightArrow);
    assert_eq!(s.store().get_selected_version(), 1);
    s.press(Key::LeftArrow);
    assert_eq!(s.store().get_selected_version(), 1);
    assert!(!s.has("Change version") && s.store().get_details_open());
}

#[test]
fn motion_can_be_turned_down_and_the_choice_is_kept() {
    let s = start("linux-x64-debian", Shell::Desktop);
    assert!(!s.ui.global::<Theme>().get_reduce_motion(), "things move unless asked not to");
    s.store().invoke_set_reduce_motion(true);
    assert!(s.ui.global::<Theme>().get_reduce_motion());
    assert!(s.saved().settings.reduce_motion);
    assert_eq!(s.ui.global::<Theme>().get_smooth(), 0, "with it on, nothing takes time");

    // A download shows how far along it is on the button that started it.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert!(s.store().get_options().row_count() > 0);
    s.store().invoke_install(0);
    assert_eq!(s.store().get_install_state(), 2, "the demo download finishes at once");
}

// ---- signing in to GitHub with a code ----

#[test]
fn signing_in_to_github_shows_a_code_and_keeps_the_token_it_is_given() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(!s.store().get_has_token());
    // The demo's GitHub says yes the first time it is asked.
    s.click("Sign in with GitHub");
    assert!(s.store().get_has_token());
    assert_eq!(s.saved().settings.github_token, "demo-token");
    assert_eq!((s.store().get_signin_state(), s.store().get_signin_code().as_str()), (0, ""));
    assert_eq!(s.store().get_toast(), "Signed in to GitHub");
    assert!(!s.has("Sign in with GitHub"), "nothing to sign in to twice");

    s.click("Sign out of GitHub");
    assert!(!s.store().get_has_token() && s.saved().settings.github_token.is_empty());
    assert!(s.has("Sign in with GitHub"));

    // On a TV it is under Accounts on the Settings tab, reached with the remote.
    let tv = start("linux-tv", Shell::Tv);
    tv.press(Key::UpArrow);
    tv.press(Key::UpArrow);
    tv.press(Key::RightArrow);
    tv.press(Key::RightArrow);
    tv.press(Key::Return);
    for _ in 0..12 {
        tv.press(Key::DownArrow);
    }
    tv.press(Key::Return);
    assert!(tv.store().get_has_token(), "OK on the GitHub row signs in");
    tv.press(Key::Return);
    assert!(!tv.store().get_has_token(), "and OK again signs out");
}

// ---- choosing where apps come from ----

#[test]
fn sources_can_be_switched_off_and_results_filtered_by_source() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // A search that several sources answer.
    s.app.search(&s.ui, "notes");
    let sources: Vec<(String, i32)> =
        s.store().get_result_sources().iter().map(|c| (c.id.to_string(), c.count)).collect();
    assert!(sources.len() >= 2 && sources.iter().any(|(id, _)| id == "flathub"), "{sources:?}");
    let all = s.store().get_results().row_count();

    // Narrow to one source, and back.
    s.store().invoke_filter_results("flathub".into());
    assert_eq!(s.store().get_source_filter(), "flathub");
    assert!(s.store().get_results().iter().all(|c| c.id.starts_with("flathub:")));
    assert!(s.store().get_results().row_count() < all);
    s.store().invoke_filter_results("".into());
    assert_eq!(s.store().get_results().row_count(), all);

    // Switched off in Settings: gone from searches and the home page, and remembered.
    s.store().set_page(3);
    assert!(s.store().get_sources().iter().any(|c| c.id == "flathub" && c.on));
    s.store().invoke_set_source("flathub".into(), false);
    assert_eq!(s.saved().settings.disabled_sources, ["flathub"]);
    assert!(s.store().get_sources().iter().any(|c| c.id == "flathub" && !c.on), "still listed, to switch back on");
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().all(|c| !c.id.starts_with("flathub:")));
    assert!(s.store().get_sections().iter().all(|section| !section.title.contains("Flathub")));
    // A link to an app from it still opens: the person asked for that app.
    s.app.search(&s.ui, "flathub:io.quillpad.Quillpad");
    assert!(s.store().get_details_open());

    s.store().invoke_set_source("flathub".into(), true);
    assert!(s.saved().settings.disabled_sources.is_empty());
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().any(|c| c.id.starts_with("flathub:")));
}

// ---- one app, several sources ----

#[test]
fn an_app_found_at_several_sources_is_one_result_with_its_sources_by_the_button() {
    // On an Arch-based system the demo has Relay both in pacman's repositories and on GitHub.
    let s = start("linux-x64-arch", Shell::Desktop);
    s.app.search(&s.ui, "relay");
    let results: Vec<AppCard> = s.store().get_results().iter().collect();
    let harbors: Vec<&AppCard> = results.iter().filter(|c| c.name.to_lowercase() == "relay").collect();
    assert_eq!(
        harbors.len(),
        1,
        "one tile, not one per source: {:?}",
        results.iter().map(|c| c.id.to_string()).collect::<Vec<_>>()
    );
    assert!(harbors[0].more_sources >= 1);
    let lead = harbors[0].id.to_string();

    // Its page says which source the button uses and offers the others beside it.
    s.app.open(&s.ui, &lead);
    let sources: Vec<SourceChoice> = s.store().get_app_sources().iter().collect();
    assert!(sources.len() >= 2);
    assert_eq!(sources.iter().filter(|c| c.on).count(), 1);
    assert_eq!(sources.iter().find(|c| c.on).unwrap().id, lead.as_str());
    let other = sources.iter().find(|c| !c.on).unwrap().clone();
    assert!(s.has(&format!("Switch to {}", other.label)));

    // Choosing another source shows the app as that source has it, with the choice kept.
    s.click(&format!("Switch to {}", other.label));
    assert_eq!(s.store().get_app().id, other.id);
    let now: Vec<SourceChoice> = s.store().get_app_sources().iter().collect();
    assert_eq!(now.iter().find(|c| c.on).unwrap().id, other.id);
    assert_eq!(now.len(), sources.len());

    // Filtering by source shows each entry as that source has it.
    s.store().set_details_open(false);
    s.store().invoke_filter_results("github".into());
    assert!(s.store().get_results().iter().all(|c| c.id.starts_with("github:") && c.more_sources == 0));

    // An app only one source has offers no choice.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_app_sources().row_count(), 0);
}

// ---- apps that are versions of one service ----

#[test]
fn a_service_with_several_apps_is_one_result_and_its_page_offers_the_versions() {
    // A Linux TV can use YouTube's own site (in Longboard) and the desktop apps on Flathub and GitHub.
    let s = start("linux-tv", Shell::Tv);
    s.app.search(&s.ui, "youtube");
    let first = s.store().get_results().row_data(0).unwrap();
    assert_eq!(first.name, "YouTube", "the service is one tile, whatever else matched");
    assert_eq!(first.id, "web:youtube.com", "its own site leads where that can be used");
    assert!(first.headline.ends_with("versions"), "{}", first.headline);
    let tiles = s.store().get_results().iter().filter(|c| c.name == "YouTube").count();
    assert_eq!(tiles, 1);

    s.app.open(&s.ui, "web:youtube.com");
    assert_eq!(s.store().get_app_family_name(), "YouTube");
    // One line per app, however many sources have it, each saying what it is and whose.
    let versions: Vec<VersionChoice> = s.store().get_app_family().iter().collect();
    let names: Vec<&str> = versions.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["YouTube website", "VacuumTube", "FreeTube"], "only what this device can use, each once");
    assert_eq!(versions.iter().filter(|v| v.on).count(), 1);
    assert!(versions[0].on && versions[0].official);
    let vacuum = &versions[1];
    assert_eq!(
        (vacuum.id.as_str(), vacuum.official),
        ("flathub:rocks.shy.VacuumTube", false),
        "whose it is, is always said"
    );
    assert_eq!(vacuum.note, "YouTube's TV interface, as an app");

    // Choosing one shows it, with its other sources beside the button.
    s.app.open(&s.ui, "flathub:rocks.shy.VacuumTube");
    let now: Vec<VersionChoice> = s.store().get_app_family().iter().collect();
    assert_eq!(now.iter().find(|v| v.on).unwrap().name, "VacuumTube");
    let sources: Vec<(String, bool)> =
        s.store().get_app_sources().iter().map(|c| (c.label.to_string(), c.on)).collect();
    assert_eq!(sources, [("Flathub".to_string(), true), ("GitHub".to_string(), false)]);
    s.app.open(&s.ui, "github:shy1132/VacuumTube");
    let now: Vec<VersionChoice> = s.store().get_app_family().iter().collect();
    assert_eq!(
        now.iter().find(|v| v.on).map(|v| v.id.as_str()),
        Some("github:shy1132/VacuumTube"),
        "the list follows the source in use"
    );

    // On a phone the same family leads with the website and offers the Android apps.
    let phone = start("android-arm64", Shell::Mobile);
    phone.app.search(&phone.ui, "YouTube");
    let first = phone.store().get_results().row_data(0).unwrap();
    assert_eq!((first.name.as_str(), first.id.as_str()), ("YouTube", "web:youtube.com"));
    phone.app.open(&phone.ui, "web:youtube.com");
    let on_phone: Vec<String> = phone.store().get_app_family().iter().map(|v| v.name.to_string()).collect();
    assert!(on_phone.contains(&"NewPipe".to_string()) && !on_phone.contains(&"VacuumTube".to_string()), "{on_phone:?}");

    // An app that belongs to no family shows no such choice.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_app_family().row_count(), 0);
}

#[test]
fn a_tv_remote_can_switch_version_from_the_versions_list() {
    let s = start("linux-tv", Shell::Tv);
    s.app.open(&s.ui, "web:youtube.com");
    let choices: Vec<SourceChoice> = s.store().get_tv_choices().iter().collect();
    assert!(choices.len() >= 2 && choices.iter().all(|c| c.label.starts_with("Switch to ")), "{}", choices.len());
    let first = choices[0].clone();
    // Main button, star, then Versions: OK opens the list with the choices on top.
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.has(first.label.as_str()));
    s.press(Key::Return);
    assert_eq!(s.store().get_app().id, first.id, "OK on a version opens it");
    assert!(s.store().get_tv_choices().iter().any(|c| c.id == "web:youtube.com"), "and the way back is offered");
}

// ---- signing in to Steam ----

#[test]
fn signing_in_to_steam_in_the_stores_window_records_what_the_account_owns() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(s.has("Sign in to Steam with a QR code"), "both ways are offered");
    assert!(s.has("Sign in to Steam in a browser") && !s.store().get_steam_signed_in());
    s.click("Sign in to Steam in a browser");
    assert!(s.store().get_steam_signed_in());
    assert_eq!(s.store().get_steam_summary(), "3 games and tools");
    assert!(s.store().get_toast().contains("3 games and tools"), "{}", s.store().get_toast());
    let saved = s.saved().steam.expect("kept for next time");
    assert_eq!((saved.steamid.as_str(), saved.owned.as_slice()), ("76561198000000001", &[10, 440, 620][..]));
    assert!(s.app.owns(&AppId::parse("steam:440").unwrap()) && !s.app.owns(&AppId::parse("steam:570").unwrap()));
    assert!(!s.app.owns(&AppId::parse("flathub:440").unwrap()), "only Steam's own ids");
    // Only the list is kept by the store: nothing that could act as the account.
    let on_disk = std::fs::read_to_string(&s.state_file).unwrap();
    assert!(!on_disk.contains("token") || on_disk.contains("github_token"), "{on_disk}");

    // Signing out forgets the list and the window's own data.
    assert!(s.dirs.config.join("steam-session").join("cookies").exists());
    s.click("Sign out of Steam");
    assert!(!s.store().get_steam_signed_in() && s.saved().steam.is_none());
    assert!(!s.dirs.config.join("steam-session").exists());
    assert!(!s.app.owns(&AppId::parse("steam:440").unwrap()));
}

#[test]
fn signing_in_to_steam_with_a_code_shows_it_and_keeps_only_the_list() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    // The demo's Steam swaps the code once and approves the second time it is asked.
    s.click("Sign in to Steam with a QR code");
    assert!(s.store().get_steam_signed_in() && s.store().get_steam_state() == 0);
    assert_eq!(s.saved().steam.unwrap().owned, [440, 620]);
    let on_disk = std::fs::read_to_string(&s.state_file).unwrap();
    assert!(!on_disk.contains("demo.") && !on_disk.contains("refresh"), "no key from Steam is kept: {on_disk}");
    assert_eq!(s.store().get_steam_qr().size().width, 0, "the code is taken down afterwards");
    assert!(!s.dirs.config.join("steam-session").exists(), "and no browser was involved");

    // A TV offers the same two ways, the code first.
    let tv = start("linux-tv", Shell::Tv);
    tv.store().invoke_steam_signin_qr();
    assert!(tv.store().get_steam_signed_in());
}

#[test]
fn on_a_tv_steam_sign_in_happens_in_longboard_which_hands_the_list_back() {
    let s = start("linux-tv", Shell::Tv);
    // Without Longboard there is nowhere to sign in: say so, and what to do.
    s.store().invoke_steam_signin();
    assert!(s.store().get_toast().contains("Install Longboard"), "{}", s.store().get_toast());
    assert!(!s.store().get_steam_signed_in() && s.store().get_steam_state() == 0);

    std::fs::create_dir_all(&s.dirs.applications).unwrap();
    let longboard = s.dirs.applications.join("Longboard-x86_64.AppImage");
    std::fs::write(&longboard, b"stand-in").unwrap();
    s.store().invoke_steam_signin();
    let (program, args) = s.web_opened.lock().unwrap().last().cloned().expect("Longboard was started");
    assert_eq!(program, longboard);
    assert!(args.starts_with("--hand-back=steam-library --hand-back-to="), "{args}");
    assert!(args.ends_with("--url=https://store.steampowered.com/login/?redir=&redir_ssl=1"), "{args}");
    assert!(s.store().get_steam_signed_in());
    assert_eq!(s.saved().steam.unwrap().owned, [440, 570]);
    assert!(!s.dirs.cache.join("hand-back").join("steam.json").exists(), "the answer is removed once read");
}

#[test]
fn a_tv_app_page_is_sections_to_move_down_through_and_open() {
    // Without motion, so the page is where it is going at once: the test's clock stands still.
    let s = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    s.press(Key::RightArrow);
    s.press(Key::LeftArrow);
    assert!(s.has("Description"), "the description is a thing to select, below the buttons");

    // OK on the description opens it to read; Back closes it and the page stays.
    s.activate("Description");
    assert!(!s.has("Description") || s.store().get_details_open());
    s.press(Key::DownArrow);
    s.press(Key::Escape);
    assert!(s.store().get_details_open(), "Back closes the description, not the page");
    s.press(Key::Escape);
    assert!(!s.store().get_details_open(), "and Back again closes the page");
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    s.press(Key::RightArrow);
    s.press(Key::LeftArrow);

    // Further down are the app's own details, not the device's.
    let from = format!("From: {}", s.store().get_app().source);
    let mut seen = s.has(&from);
    for _ in 0..6 {
        s.press(Key::DownArrow);
        seen |= s.has(&from);
    }
    assert!(seen, "{from}");
    assert!(!s.has("C library: glibc"));
    // And Up comes all the way back to the buttons, which still work.
    for _ in 0..8 {
        s.press(Key::UpArrow);
    }
    s.press(Key::Return);
    assert!(
        s.store().get_install_state() != 0 || s.store().get_in_library() || s.store().get_can_open(),
        "OK presses the main button again"
    );
}

#[test]
fn links_in_a_description_are_found_for_the_remote() {
    let text = "Docs at https://example.org/docs. Chat (https://chat.example.org/room), again https://example.org/docs and http://old.example.net; not a link: https://nodot or ftp://x.y";
    assert_eq!(
        bilby::links_in(text),
        ["https://example.org/docs", "https://chat.example.org/room", "http://old.example.net"]
    );
    assert!(bilby::links_in("nothing here").is_empty());
}

#[test]
fn the_tv_has_a_settings_tab_with_every_setting_reachable_by_remote() {
    // Without motion, so the list is where it is going at once: the test's clock stands still.
    let s = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 3);
    // Every setting comes into view on the way down the list.
    let mut seen: Vec<&str> = Vec::new();
    let wanted = [
        "Ratings and reviews",
        "Use the desktop layout, for a mouse and keyboard",
        "Family mode: nothing for adults is shown anywhere",
        "Set a PIN to guard these settings and adult content",
        "Sign in to Steam with a QR code, to see what you own",
        "Or sign in to Steam in a browser (Longboard)",
    ];
    for _ in 0..14 {
        seen.extend(wanted.iter().filter(|label| s.has(label)));
        s.press(Key::DownArrow);
    }
    for label in wanted {
        assert!(seen.contains(&label), "{label}");
    }
    for _ in 0..20 {
        s.press(Key::UpArrow);
    }
    s.press(Key::DownArrow);
    // Sources can be switched off from the sofa too: Down to the first of them, OK.
    let sources: Vec<SourceChoice> = s.store().get_sources().iter().collect();
    assert!(sources.len() >= 2 && sources.iter().all(|c| c.on));
    for _ in 0..15 {
        s.press(Key::DownArrow);
    }
    s.press(Key::Return);
    assert!(!s.store().get_sources().row_data(0).unwrap().on, "OK on a source switches it off");
    assert_eq!(s.saved().settings.disabled_sources, [sources[0].id.to_string()]);
    // Down stops at the last row; Up from the first goes back to the tabs; This TV is the next tab along.
    for _ in 0..40 {
        s.press(Key::DownArrow);
    }
    s.press(Key::Return);
    assert!(!s.store().get_sources().iter().last().unwrap().on);
    for _ in 0..60 {
        s.press(Key::UpArrow);
    }
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 4);
}

#[test]
fn the_library_shows_the_device_the_store_and_what_was_installed_some_other_way() {
    let s = start("linux-tv", Shell::Tv);
    // A Flatpak and a Steam game that Bilby had no part in.
    let flatpak = s.dirs.flatpak_roots[0].join("app/org.example.Paint/current/active/export/share/applications");
    std::fs::create_dir_all(&flatpak).unwrap();
    std::fs::write(flatpak.join("org.example.Paint.desktop"), "[Desktop Entry]\nName=Paint Box\n").unwrap();
    s.app.scan_installed(&s.ui);
    let rows: Vec<(String, Vec<String>)> = s
        .store()
        .get_tv_library_rows()
        .iter()
        .map(|row| (row.title.to_string(), row.apps.iter().map(|c| c.name.to_string()).collect()))
        .collect();
    let also =
        rows.iter().find(|(title, _)| title == "Also installed here").expect("a row for what came from elsewhere");
    assert_eq!(also.1, ["Paint Box"]);
    // The same app is marked installed wherever the store shows it.
    assert!(s.app.card(&AppSummary::new(AppId::parse("flathub:org.example.Paint").unwrap(), "Paint Box")).installed);

    // The device's card opens its page; the store's own card opens the store's.
    s.store().set_page(2);
    s.store().invoke_open("bilby:this-device".into());
    assert_eq!(s.store().get_page(), 4);
    assert!(!s.store().get_details_open());

    // The store is in its own library, and asking for a newer one always answers.
    let own = start_as_store("linux-x64-debian", "github:kestrel-sh/kestrel");
    let names: Vec<String> =
        own.store().get_tv_library_rows().row_data(0).unwrap().apps.iter().map(|c| c.name.to_string()).collect();
    assert_eq!(names.last().map(String::as_str), Some("Bilby"), "{names:?}");
    own.store().invoke_check_store_update();
    assert!(own.store().get_toast().contains("Bilby"), "{}", own.store().get_toast());
}

#[test]
fn down_from_a_tab_goes_into_the_page_showing_and_only_ok_changes_page() {
    let s = start("linux-tv", Shell::Tv);
    // Home: up to search, up to the tabs, right to Library, then Down.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::DownArrow);
    assert_eq!(s.store().get_page(), 0, "Down from the Library tab does not open the Library");
    // It went to search, the first thing on Home; Down again is the apps.
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open(), "and the apps are below that");
}

// ---- what an app needs before it will run ----

#[test]
fn an_appimage_on_a_system_without_fuse_says_so_and_offers_to_install_it() {
    use bilby_core::needs::Fuse;
    let start_with_fuse = |fuse: Fuse, preset: &str| {
        init_backend();
        let dirs = dirs_in(&temp_root());
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
        let options = Options {
            demo: true,
            inline: true,
            platform: Platform::preset(preset),
            shell: Some(Shell::Desktop),
            dirs: Some(dirs),
            fuse: Some(fuse),
            ..Default::default()
        };
        build(&options).unwrap()
    };
    // A system and a demo app where the best file is an AppImage.
    let without = Fuse { library: false, mounter: true };
    let (preset, id) = ["linux-x64-arch", "linux-x64-fedora", "linux-x64-debian"]
        .into_iter()
        .flat_map(|preset| ["github:tidewave/tidewave", "github:lumen-labs/lumen"].map(|id| (preset, id)))
        .find(|(preset, id)| {
            let (ui, app) = start_with_fuse(without, preset);
            app.open(&ui, id);
            ui.global::<Store>().get_options().row_data(0).is_some_and(|o| o.label.contains("AppImage"))
        })
        .expect("the demo has an app that comes as an AppImage somewhere");
    let (ui, app) = start_with_fuse(without, preset);
    app.open(&ui, id);
    let store = ui.global::<Store>();
    assert!(store.get_need_note().contains("FUSE 2"), "said before anything is downloaded: {}", store.get_need_note());
    // The demo's repositories have no such package, so no button is offered: only advice.
    if store.get_helper_id().is_empty() {
        assert!(store.get_need_note().contains("libfuse2 or fuse2"), "{}", store.get_need_note());
    } else {
        assert!(store.get_helper_label().contains("FUSE"), "{}", store.get_helper_label());
    }

    // With FUSE in place nothing is said.
    let (ui, app) = start_with_fuse(Fuse { library: true, mounter: true }, preset);
    app.open(&ui, id);
    assert_eq!(ui.global::<Store>().get_need_note(), "");
    assert_eq!(ui.global::<Store>().get_helper_id(), "");
}

// ---- adult content, Family mode and the PIN ----

fn adult_site() -> &'static bilby_core::source::web::Site {
    bilby_core::source::web::sites().iter().find(|site| site.adult).unwrap()
}

#[test]
fn adult_things_have_a_covered_section_and_ask_before_opening() {
    let s = start("linux-tv", Shell::Tv);
    let site = adult_site();
    let id = format!("web:{}", site.host);
    s.app.search(&s.ui, &site.name);

    // In the results it is last, covered: nothing of it shows but that it is there.
    let card = s.store().get_results().iter().find(|c| c.id == id.as_str()).expect("it is found, not hidden");
    assert!(card.adult);
    assert_eq!((card.name.as_str(), card.summary.as_str(), card.stat.as_str()), ("Adult", "Select to view", ""));
    assert!(!card.summary.contains(site.name.as_str()) && !card.author.contains(site.host.as_str()));
    let last = s.store().get_results().iter().last().unwrap();
    assert!(last.adult, "after everything else");
    // On the TV it has a row of its own, named plainly, and is in no other row.
    let rows: Vec<(String, Vec<String>)> = s
        .store()
        .get_tv_rows()
        .iter()
        .map(|row| (row.title.to_string(), row.apps.iter().map(|c| c.id.to_string()).collect()))
        .collect();
    assert_eq!(rows.last().unwrap().0, "Adult");
    assert!(rows.iter().filter(|(title, _)| title != "Adult").all(|(_, ids)| !ids.contains(&id)), "{rows:?}");
    // Nothing for adults is on the home page unless asked for.
    assert!(!s.store().get_adult_on_home());

    // Choosing it asks first. No is the highlighted answer; it changes nothing.
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 1);
    assert!(s.store().get_gate_text().contains("18 or over") && !s.store().get_details_open());
    s.press(Key::Return);
    assert_eq!(s.store().get_gate(), 0);
    assert!(!s.store().get_details_open() && !s.saved().settings.adult_confirmed);

    // Yes opens it, as itself, and the question is not asked again.
    s.app.open(&s.ui, &id);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open() && s.saved().settings.adult_confirmed);
    assert_eq!(s.store().get_app().name, site.name.as_str());
    s.store().set_details_open(false);
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 0);
    assert!(s.store().get_details_open());
    // The tile in the list stays covered all the same.
    assert!(s.store().get_results().iter().any(|c| c.id == id.as_str() && c.name == "Adult"));

    // For those who want no cover at all: shown as itself, among everything else.
    s.store().set_details_open(false);
    s.store().invoke_set_adult_uncovered(true);
    assert!(s.saved().settings.adult_uncovered && s.store().get_gate() == 0, "already confirmed an adult");
    s.app.search(&s.ui, &site.name);
    let card = s.store().get_results().iter().find(|c| c.id == id.as_str()).unwrap();
    assert_eq!(card.name, site.name.as_str());
    assert!(!card.adult && !s.store().get_tv_rows().iter().any(|row| row.title == "Adult"));
    s.app.open(&s.ui, &id);
    assert!(s.store().get_details_open() && s.store().get_gate() == 0);
    // Family mode still overrides it.
    s.store().set_details_open(false);
    s.store().invoke_set_family_mode(true);
    s.app.search(&s.ui, &site.name);
    assert!(!s.store().get_results().iter().any(|c| c.id == id.as_str()));
}

#[test]
fn a_site_nobody_listed_by_hand_is_still_recognised_as_adult() {
    let s = start("linux-tv", Shell::Tv);
    // A language edition of a listed site: on no hand-written list, typed as an address.
    let typed = format!("fr.{}", adult_site().host);
    s.app.search(&s.ui, &typed);
    let card =
        s.store().get_results().iter().find(|c| c.id == format!("web:{typed}").as_str()).expect("offered as typed");
    assert!(card.adult && card.name == "Adult");
    s.store().invoke_set_family_mode(true);
    s.app.search(&s.ui, &typed);
    assert!(!s.store().get_results().iter().any(|c| c.id.contains(typed.as_str())), "and gone in Family mode");
}

// The entry it writes is Linux's kind; the Windows and macOS shortcuts go to places
// a test has no business writing to.
#[cfg(target_os = "linux")]
#[test]
fn on_a_computer_a_web_app_opens_in_the_persons_own_browser() {
    init_backend();
    let dirs = dirs_in(&temp_root());
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
    let opened: Arc<Mutex<Vec<String>>> = Arc::default();
    let record = opened.clone();
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-x64-debian"),
        shell: Some(Shell::Desktop),
        dirs: Some(dirs.clone()),
        browser: Some(Arc::new(move |url: &str| {
            record.lock().unwrap().push(url.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let store = ui.global::<Store>();

    // Web apps are offered here too: found by name, and on the home page.
    app.search(&ui, "lichess");
    assert!(store.get_results().iter().any(|c| c.id == "web:lichess.org"));
    assert!(store.get_sections().iter().any(|row| row.title == "Web apps"));
    assert!(store.get_sections().iter().any(|row| row.title == "Web apps to watch"));

    // Adding one puts an entry among the apps that opens the address.
    app.open(&ui, "web:lichess.org");
    assert_eq!(store.get_options().row_data(0).unwrap().verb, "Add to my apps");
    store.invoke_install(0);
    assert!(store.get_install_message().contains("opens in your browser"), "{}", store.get_install_message());
    let entry = std::fs::read_dir(&dirs.menu)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().contains("lichess"))
        .expect("a menu entry");
    let text = std::fs::read_to_string(entry.path()).unwrap();
    assert!(
        text.contains("Name=Lichess") && text.contains("xdg-open") && text.contains("\"https://lichess.org/\""),
        "{text}"
    );
    assert!(!text.to_lowercase().contains("longboard"));

    // Open goes to the browser, not to Longboard.
    assert!(store.get_can_open());
    store.invoke_launch();
    assert_eq!(*opened.lock().unwrap(), ["https://lichess.org/"]);
}

#[test]
fn family_mode_takes_adult_things_out_altogether() {
    let s = start("linux-tv", Shell::Tv);
    let site = adult_site();
    let id = format!("web:{}", site.host);
    s.store().invoke_set_family_mode(true);
    assert!(s.store().get_family_mode() && s.saved().settings.family_mode, "anyone may switch it on");

    for query in [site.name.as_str(), site.host.as_str()] {
        s.app.search(&s.ui, query);
        assert!(!s.store().get_results().iter().any(|c| c.adult || c.id == id.as_str()), "{query}: not even covered");
        assert!(!s.store().get_tv_rows().iter().any(|row| row.title == "Adult"));
        assert!(!s.store().get_search_note().to_lowercase().contains("adult"), "and no word of it");
    }
    // By its link: refused, with no question to say yes to.
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 0);
    assert!(!s.store().get_details_open());
    assert!(s.store().get_toast().contains("Family mode"), "{}", s.store().get_toast());
    // Adult things on the home page cannot be asked for while it is on.
    assert!(!s.store().get_adult_on_home());
}

#[test]
fn a_pin_guards_the_settings_adult_content_and_installs() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(s.has("Set a PIN") && !s.store().get_has_pin());

    // Choosing a PIN: four digits, twice. Two that differ start over.
    let enter = |pin: &str| s.store().invoke_gate_pin(pin.into());
    s.click("Set a PIN");
    assert_eq!(s.store().get_gate(), 3);
    enter("2580");
    assert_eq!(s.store().get_gate(), 4);
    enter("2581");
    assert_eq!(s.store().get_gate(), 3, "not the same: choose again");
    enter("2580");
    enter("2580");
    assert_eq!(s.store().get_gate(), 0);
    assert!(s.store().get_has_pin());
    let saved = s.saved().settings;
    assert_eq!(saved.pin_hash.len(), 64);
    assert!(!std::fs::read_to_string(&s.state_file).unwrap().contains("2580"), "the PIN itself is not kept");

    // Family mode goes on without the PIN, and comes off only with it.
    s.store().invoke_set_family_mode(true);
    assert!(s.saved().settings.family_mode && s.store().get_gate() == 0);
    s.store().invoke_set_family_mode(false);
    assert_eq!(s.store().get_gate(), 2);
    assert!(s.saved().settings.family_mode, "still on until the PIN is given");
    enter("0000");
    assert!(s.store().get_gate() == 2 && s.store().get_gate_error().contains("not the PIN"));
    assert!(s.saved().settings.family_mode);
    s.store().invoke_gate_cancel();
    assert!(s.saved().settings.family_mode, "stepping back changes nothing");
    s.store().invoke_set_family_mode(false);
    enter("2580");
    assert!(!s.saved().settings.family_mode && s.store().get_gate() == 0);

    // With a PIN, adult content asks for it rather than whether you are an adult.
    let id = format!("web:{}", adult_site().host);
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 2);
    s.store().invoke_gate_cancel();
    assert!(!s.store().get_details_open());

    // Installs and removals ask for it too, once that is switched on.
    s.store().invoke_set_pin_for_installs(true);
    assert!(s.saved().settings.pin_for_installs, "stricter: no PIN needed to switch it on");
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    s.store().invoke_install(0);
    assert_eq!(s.store().get_gate(), 2);
    assert_eq!(s.store().get_install_state(), 0, "nothing starts until the PIN is given");
    enter("2580");
    assert_ne!(s.store().get_install_state(), 0, "then it goes ahead");
    assert!(s.saved().library.iter().any(|a| a.id == "github:lumen-labs/lumen"));

    // Removing the PIN needs the PIN, and takes the install lock with it.
    s.store().invoke_remove_pin();
    assert_eq!(s.store().get_gate(), 2);
    enter("2580");
    let saved = s.saved().settings;
    assert!(saved.pin_hash.is_empty() && !saved.pin_for_installs && !s.store().get_has_pin());
}

#[test]
fn a_tv_remote_enters_the_pin_digit_by_digit() {
    let s = start("linux-tv", Shell::Tv);
    s.store().invoke_set_pin();
    assert_eq!(s.store().get_gate(), 3);
    // The digits are a row: Right twice is 2, OK enters it; number keys work as well.
    let two = || {
        s.press(Key::Return);
    };
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    for _ in 0..4 {
        two();
    }
    assert_eq!(s.store().get_gate(), 4, "2222 entered once");
    // A new question starts with the highlight back at the first digit.
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    for _ in 0..4 {
        two();
    }
    assert!(s.store().get_has_pin() && s.store().get_gate() == 0);
    // Back steps away from a question without answering it.
    s.store().invoke_remove_pin();
    assert_eq!(s.store().get_gate(), 2);
    s.press(Key::Escape);
    assert!(s.store().get_gate() == 0 && s.store().get_has_pin());
}

#[test]
fn family_mode_also_takes_out_gambling_and_piracy_sites() {
    let s = start("linux-tv", Shell::Tv);
    let betting =
        ["bet365.com", "pokerstars.com"].into_iter().find(|h| bilby_core::source::web::restricted_host(h)).unwrap();
    let id = format!("web:{betting}");
    // Ordinarily it is found like any other site, uncovered: it is not adult content.
    s.app.search(&s.ui, betting);
    let card = s.store().get_results().iter().find(|c| c.id == id.as_str()).expect("found when searched for");
    assert!(!card.adult);
    // It is never on the home page.
    s.app.search(&s.ui, "");
    assert!(!s.store().get_sections().iter().any(|row| row.apps.iter().any(|c| c.id == id.as_str())));
    // In Family mode it is gone, and will not open.
    s.store().invoke_set_family_mode(true);
    s.app.search(&s.ui, betting);
    assert!(!s.store().get_results().iter().any(|c| c.id == id.as_str()));
    s.app.open(&s.ui, &id);
    assert!(
        s.store().get_details_error().contains("Family mode") || !s.store().get_details_open(),
        "{}",
        s.store().get_details_error()
    );
    assert_eq!(s.store().get_options().row_count(), 0);
}

#[test]
fn recommending_the_apps_everyone_knows_can_be_switched_off() {
    let s = start("linux-x64-debian", Shell::Desktop);
    assert!(s.store().get_show_essentials(), "on to begin with");
    s.store().invoke_set_essentials(false);
    assert!(!s.store().get_show_essentials() && s.saved().settings.hide_essentials);
    s.store().invoke_set_essentials(true);
    assert!(s.store().get_show_essentials() && !s.saved().settings.hide_essentials);
    // On a TV it is a row in Settings.
    let tv = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    tv.press(Key::UpArrow);
    tv.press(Key::UpArrow);
    tv.press(Key::RightArrow);
    tv.press(Key::RightArrow);
    tv.press(Key::Return);
    for _ in 0..4 {
        tv.press(Key::DownArrow);
    }
    assert!(tv.has("Recommend the apps everyone knows, on Home and in searches"));
    tv.press(Key::Return);
    assert!(tv.saved().settings.hide_essentials, "OK on the row switches it off");
}

// ---- where downloads go ----

#[test]
fn the_downloads_folder_can_be_chosen_and_each_download_can_ask() {
    init_backend();
    let root = temp_root();
    let dirs = dirs_in(&root);
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
    let state_file = dirs.config.join("demo-state.json");
    // The folder chooser: answers with whatever the test last put here.
    let answer: Arc<Mutex<bilby::Picked>> = Arc::new(Mutex::new(bilby::Picked::Cancelled));
    let asked: Arc<Mutex<usize>> = Arc::default();
    let (reply, count) = (answer.clone(), asked.clone());
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-x64-debian"),
        shell: Some(Shell::Desktop),
        dirs: Some(dirs.clone()),
        opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
        picker: Some(Arc::new(move |_: &std::path::Path| {
            *count.lock().unwrap() += 1;
            reply.lock().unwrap().clone()
        })),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let store = ui.global::<Store>();
    let saved = || State::load(&state_file).settings;

    // To begin with: the usual folder, and no questions.
    assert_eq!(store.get_downloads_dir(), dirs.downloads.display().to_string().as_str());
    assert!(!store.get_downloads_custom() && !store.get_ask_where());

    // Choosing a folder: backing out changes nothing; picking one is remembered.
    store.invoke_choose_downloads();
    assert!(saved().downloads_dir.is_empty());
    let chosen = root.join("my downloads");
    std::fs::create_dir_all(&chosen).unwrap();
    *answer.lock().unwrap() = bilby::Picked::Folder(chosen.clone());
    store.invoke_choose_downloads();
    assert_eq!(saved().downloads_dir, chosen.display().to_string());
    assert!(store.get_downloads_custom() && store.get_downloads_dir() == chosen.display().to_string().as_str());

    // A download goes there.
    app.open(&ui, "github:lumen-labs/lumen");
    store.invoke_install(0);
    assert!(std::fs::read_dir(&chosen).unwrap().count() >= 1, "the file was saved in the chosen folder");
    assert!(!dirs.downloads.exists(), "and not in the usual one");

    // A path can be typed instead; one that cannot be used is refused.
    store.invoke_set_downloads("relative/folder".into());
    assert!(store.get_toast().contains("cannot be used"), "{}", store.get_toast());
    assert_eq!(saved().downloads_dir, chosen.display().to_string());
    store.invoke_set_downloads("".into());
    assert!(saved().downloads_dir.is_empty() && !store.get_downloads_custom());

    // Asking each time: off until switched on; then each download asks first.
    let before = *asked.lock().unwrap();
    store.invoke_set_ask_where(true);
    assert!(saved().ask_where);
    let once = root.join("just this once");
    std::fs::create_dir_all(&once).unwrap();
    *answer.lock().unwrap() = bilby::Picked::Folder(once.clone());
    app.open(&ui, "github:kestrel-sh/kestrel");
    store.invoke_install(0);
    assert_eq!(*asked.lock().unwrap(), before + 1);
    assert!(std::fs::read_dir(&once).unwrap().count() >= 1, "saved where the person said");
    assert!(saved().downloads_dir.is_empty(), "a one-off answer does not change the setting");
    // Backing out of the question cancels the download.
    *answer.lock().unwrap() = bilby::Picked::Cancelled;
    app.open(&ui, "github:tidewave/tidewave");
    store.invoke_install(0);
    assert_eq!(store.get_install_state(), 0, "nothing was started");
}
