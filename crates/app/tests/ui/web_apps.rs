//! Websites added like apps.

use super::*;

/// Longboard is Longboard however it was installed: its Flatpak, or an AppImage that
/// had to be unpacked, is used as it is and nothing is downloaded.
#[test]
fn a_longboard_installed_another_way_is_used_for_websites() {
    // As a Flatpak: started through Flatpak.
    let s = start("linux-tv", Shell::Tv);
    std::fs::create_dir_all(s.dirs.flatpak_roots[0].join("app").join("io.github.andrewgbest.Longboard")).unwrap();
    s.app.open(&s.ui, "web:lichess.org");
    assert_eq!(s.store().get_warning(), "");
    s.press(Key::Return);
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert_eq!(s.saved().library.len(), 1, "no second Longboard was fetched");
    assert!(!s.dirs.applications.exists());
    let entry = std::fs::read_to_string(s.dirs.menu.join("bilby-web-lichess-org.desktop")).unwrap();
    assert!(
        entry
            .contains("flatpak-program\" \"run\" \"io.github.andrewgbest.Longboard\" \"--url=https://lichess.org/\"\n"),
        "{entry}"
    );
    assert!(s.store().get_can_open());
    s.press(Key::Return);
    assert_eq!(
        s.web_opened.lock().unwrap().as_slice(),
        [(s.dirs.flatpak.clone(), "run io.github.andrewgbest.Longboard --url=https://lichess.org/".to_string())]
    );

    // Unpacked into a folder, as on a system without FUSE: its AppRun is the program.
    let s = start("linux-tv", Shell::Tv);
    let folder = s.dirs.applications.join("Longboard-x86_64.AppDir");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("AppRun"), b"stand-in").unwrap();
    s.app.open(&s.ui, "web:lichess.org");
    assert_eq!(s.store().get_warning(), "");
    s.press(Key::Return);
    assert_eq!(s.saved().library.len(), 1);
    s.press(Key::Return);
    assert_eq!(s.web_opened.lock().unwrap().last().unwrap().0, folder.join("AppRun"));
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

// Choosing a browser and writing menu and desktop entries is how Linux does it; on
// Windows and macOS a web app is a shortcut the system opens.
#[test]
#[cfg(target_os = "linux")]
fn a_web_app_asks_for_a_browser_that_can_give_it_a_window_of_its_own() {
    // A computer whose only browser cannot: the page says so and offers one that can.
    let s = start("linux-x64-arch", Shell::Desktop);
    s.app.open(&s.ui, "web:youtube.com");
    assert_eq!(s.store().get_helper_id(), "pacman:chromium");
    assert_eq!(s.store().get_helper_label(), "Install Chromium");
    assert!(s.store().get_need_note().contains("usual browser"), "{}", s.store().get_need_note());

    // Added anyway, it opens as a page in the usual browser. It is in the app menu
    // and has an icon on the desktop, which the desktop will only start if it may run.
    let desktop = s.dirs.desktop.clone().unwrap();
    std::fs::create_dir_all(&desktop).unwrap();
    s.store().invoke_install(0);
    let entry = bilby_core::install::menu_entry_path(&s.dirs, "web:youtube.com");
    let written = std::fs::read_to_string(&entry).unwrap_or_else(|_| panic!("{}", s.store().get_toast()));
    assert!(!written.contains("--app="), "{written}");
    let icon = desktop.join(entry.file_name().unwrap());
    assert_eq!(std::fs::read_to_string(&icon).unwrap(), written, "the same entry, on the desktop");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&icon).unwrap().permissions().mode() & 0o111, 0o111);
    }

    // Once such a browser is on the computer, the app gets its own window without
    // being added again, and the page stops asking.
    let brave = s.dirs.flatpak_roots[0].join("app/com.brave.Browser/current/active/export/share/applications");
    std::fs::create_dir_all(&brave).unwrap();
    std::fs::write(brave.join("com.brave.Browser.desktop"), "[Desktop Entry]\nName=Brave\n").unwrap();
    s.app.scan_installed(&s.ui);
    let written = std::fs::read_to_string(&entry).unwrap();
    assert!(written.contains("com.brave.Browser") && written.contains("--app=https://"), "{written}");
    s.app.open(&s.ui, "web:youtube.com");
    assert_eq!(s.store().get_helper_id(), "");
    assert_eq!(s.store().get_need_note(), "");

    assert!(std::fs::read_to_string(&icon).unwrap().contains("--app=https://"), "the desktop icon follows");

    // Removed from the library, it leaves the menu and the desktop.
    s.app.remove_from_library(&s.ui, "web:youtube.com");
    assert!(!entry.exists() && !icon.exists());

    // A TV opens web apps in Longboard, made for a remote: no other browser is asked for.
    let tv = start("linux-tv", Shell::Tv);
    tv.app.open(&tv.ui, "web:youtube.com");
    assert_ne!(tv.store().get_helper_id(), "pacman:chromium");
    assert!(!tv.store().get_need_note().contains("Chromium"));
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
    // One row is only websites; the rest sit among the apps of their kind.
    let titles: Vec<String> = store.get_sections().iter().map(|row| row.title.to_string()).collect();
    assert_eq!(titles.iter().filter(|title| title.contains("Web apps")).count(), 1, "{titles:?}");
    // Nothing is shown twice in the rows the store composes itself.
    let mut seen = std::collections::HashSet::new();
    for row in store.get_sections().iter().filter(|row| row.title != "Web apps") {
        for app in row.apps.iter() {
            assert!(seen.insert(app.id.to_string()), "{} is in two rows (again in {})", app.name, row.title);
        }
    }

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
