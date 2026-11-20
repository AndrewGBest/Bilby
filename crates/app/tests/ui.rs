//! End-to-end tests of the three interfaces.
//!
//! Each test builds the real window on Slint's headless testing backend, with the demo
//! catalog and throw-away directories, and drives it the way a person would: clicking
//! elements found by their accessible label, or sending key presses. Catalog work runs
//! inline (`Options::inline`), so every assertion sees the finished result.

use bilby::{build, App, AppWindow, Options, Shell, Store, Theme};
use bilby_core::install::Dirs;
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
    let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let record = opened.clone();
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
    Session { ui, app, dirs, state_file, opened }
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
    s.click("More options and details");
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
    s.click("More options and details");
    assert!(s.has("Files that won't work on this device"));
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
    s.click("More options and details");
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
    assert_eq!(option.verb, "Build from AUR");
    assert_eq!(option.command, "paru -S lumen-bin");
    assert!(!option.verified);
    s.app.install(&s.ui, 0);
    assert_eq!(s.store().get_install_state(), 2);
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
    assert!(mac.store().get_search_note().contains("1 more matched but have no file for this device"));
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
    assert_eq!(s.store().get_results().row_count(), 1);

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
    assert_eq!(option.verb, "Install with pacman");
    assert_eq!(option.command, "sudo pacman -S swiftkernel");
    assert!(s.store().get_notes().contains("“cachyos-v3” repository"));
    // The store shows the command; pacman does the installing, so nothing is recorded.
    s.app.install(&s.ui, 0);
    assert!(s.store().get_install_message().contains("sudo pacman -S swiftkernel"));
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
    assert_eq!(debian.store().get_options().row_data(0).unwrap().command, "sudo apt install harbor");
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
    assert!(s.has("Forget token"));
    s.click("Forget token");
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
    Session { ui, app, dirs, state_file, opened: Arc::default() }
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
    s.click("View the store's update");
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
    assert_eq!(s.store().get_results().row_count(), 1);
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
    assert!(s.store().get_app().id.starts_with("github:"), "focus landed on {:?}", s.store().get_app().id);
    assert!(s.store().get_details_error().is_empty());
}

#[test]
fn tv_puts_the_library_first() {
    let seed = r#"{"library":[{"id":"github:marquee-tv/marquee","name":"Marquee","version":"v20.0"}]}"#;
    let s = start_with("android-tv", Shell::Tv, Some(seed));
    assert_eq!(s.store().get_tv_rows().row_data(0).unwrap().title, "Your library");
    s.app.check_updates(&s.ui, false);
    let row = s.store().get_tv_rows().row_data(0).unwrap();
    assert_eq!(row.title, "Your library  ·  1 update");
    assert!(row.apps.row_data(0).unwrap().has_update);
    assert_eq!(s.store().get_toast(), "", "a start-up check stays quiet");
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
