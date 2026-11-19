//! End-to-end tests of the three interfaces.
//!
//! Each test builds the real window on Slint's headless testing backend, with the demo
//! catalog and throw-away directories, and drives it the way a person would: clicking
//! elements found by their accessible label, or sending key presses. Catalog work runs
//! inline (`Options::inline`), so every assertion sees the finished result.

use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use untitledstore::{build, App, AppWindow, Options, Shell, Store, Theme};
use ustore_core::install::Dirs;
use ustore_core::state::State;
use ustore_core::Platform;

struct Session {
    ui: AppWindow,
    app: Arc<App>,
    dirs: Dirs,
    state_file: PathBuf,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_root() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("ustore-ui-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn dirs_in(root: &std::path::Path) -> Dirs {
    Dirs {
        downloads: root.join("downloads"),
        applications: root.join("Applications"),
        cache: root.join("cache"),
        config: root.join("config"),
    }
}

/// Start the store as `preset` with the given interface. `seed` is written as the
/// saved state first, to test start-up with an existing library.
thread_local! {
    static BACKEND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The testing backend is per thread and may be set up only once on each.
fn init_backend() {
    if !BACKEND.replace(true) {
        i_slint_backend_testing::init_no_event_loop();
    }
}

fn start_with(preset: &str, shell: Shell, seed: Option<&str>) -> Session {
    init_backend();
    ustore_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
    let root = temp_root();
    let dirs = dirs_in(&root);
    let state_file = dirs.config.join("demo-state.json");
    if let Some(json) = seed {
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(&state_file, json).unwrap();
    }
    let options = Options {
        demo: true,
        inline: true,
        platform: Some(Platform::preset(preset).unwrap()),
        shell: Some(shell),
        dirs: Some(dirs.clone()),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let (w, h) = match shell {
        Shell::Mobile => (412.0, 892.0),
        Shell::Tv => (1920.0, 1080.0),
        Shell::Desktop => (1280.0, 800.0),
    };
    ui.window().set_size(slint::LogicalSize::new(w, h));
    ui.show().unwrap();
    Session { ui, app, dirs, state_file }
}

fn start(preset: &str, shell: Shell) -> Session {
    start_with(preset, shell, None)
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

    /// Click the element with this accessible label, with a real pointer press at its centre.
    fn click(&self, label: &str) {
        let found = self.labelled(label);
        let element = found.first().unwrap_or_else(|| panic!("nothing on screen is labelled {label:?}"));
        element.mock_single_click(PointerEventButton::Left);
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

    s.click("Install AppImage · x86_64");
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert!(s.store().get_install_message().starts_with("Checksum verified."));
    let file = s.dirs.applications.join("Tidewave-3.4.0-x86_64.AppImage");
    assert!(file.is_file(), "the AppImage is in the Applications folder");
    assert_eq!(s.store().get_toast(), "Tidewave is in your library");
    assert_eq!(s.store().get_installed_version(), "v3.4.0");

    // It is in the library, on screen and on disk.
    let saved = s.saved();
    assert_eq!(saved.library.len(), 1);
    assert_eq!(saved.library[0].id, "github:tidewave/tidewave");
    assert_eq!(saved.library[0].name, "Tidewave");
    assert_eq!(saved.library[0].version, "v3.4.0");
    assert!(saved.library[0].store_managed);

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
    assert!(s.has("Files that won't work on this device"));
}

#[test]
fn failed_lookups_can_be_retried() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.open(&s.ui, "github:nobody/nothing");
    assert!(s.store().get_details_open());
    assert!(!s.store().get_details_error().is_empty());
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

    s.click("Update Kestrel");
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_installed_version(), "v1.20.0");
    assert_eq!(s.store().get_app().version, "v1.21.0");
    assert!(s.has("Update Portable archive · x86_64") || s.has("Download Portable archive · x86_64"));
    s.app.install(&s.ui, 0);
    assert_eq!(s.saved().library[0].version, "v1.21.0");
    assert_eq!(s.store().get_updates_count(), 0);
    assert_eq!(s.store().get_library().row_data(0).unwrap().status, 2);
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
