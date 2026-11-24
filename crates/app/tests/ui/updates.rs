//! Updates to apps and to Bilby itself.

use super::*;

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
fn apps_can_be_updated_automatically_and_what_needs_the_person_waits() {
    // Tidewave's AppImage goes in by itself; Kestrel's archive is only fetched for
    // the person to unpack, so it waits for them.
    let library = r#""library":[
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"},
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.0.0"}]"#;
    // Kestrel's version, then Tidewave's, wherever the library lists them.
    let versions = |s: &Session| -> Vec<String> {
        let mut apps = s.saved().library;
        apps.sort_by(|a, b| a.name.cmp(&b.name));
        apps.into_iter().map(|a| a.version).collect()
    };
    let s =
        start_with("linux-x64-arch", Shell::Desktop, Some(&format!(r#"{{"settings":{{"welcomed":true}},{library}}}"#)));
    s.store().set_page(3);
    assert!(!s.store().get_auto_update(), "off unless switched on");
    s.app.check_updates(&s.ui, false);
    assert_eq!(versions(&s), ["v1.20.0", "v3.0.0"], "found, and left for the person");
    assert_eq!(s.store().get_updates_count(), 2);

    // Switched on: what is waiting is installed at once.
    s.activate("Update apps automatically");
    assert!(s.saved().settings.auto_update && s.store().get_auto_update());
    assert_eq!(versions(&s), ["v1.20.0", "v3.4.0"]);
    assert_eq!(s.store().get_updates_count(), 1, "Kestrel still waits in the Library");
    assert_eq!(s.store().get_updating_id(), "");
    assert_eq!(s.store().get_install_state(), 0);
    // What it had to leave is offered: Kestrel, and the system's six, which want a
    // password. Later is later: it is not asked again while the window stays open.
    assert_eq!((s.store().get_gate(), s.store().get_gate_title().as_str()), (1, "Updates are waiting for you"));
    assert!(
        s.store().get_gate_text().starts_with("7 updates waiting for you to finish"),
        "{}",
        s.store().get_gate_text()
    );
    assert_eq!((s.store().get_gate_yes().as_str(), s.store().get_gate_no().as_str()), ("Finish now", "Later"));
    s.store().invoke_gate_answer(false);
    assert_eq!(s.store().get_gate(), 0);
    assert_eq!(versions(&s), ["v1.20.0", "v3.4.0"]);
    s.app.check_everything(&s.ui);
    assert_eq!(s.store().get_gate(), 0, "asked once each time the window is opened");
    assert!(s.notified.lock().unwrap().is_empty(), "and nothing is notified with the window open");

    // Closed to the system tray: a notification says what is waiting, once.
    s.app.window_closed_to_tray();
    s.app.check_everything(&s.ui);
    s.app.check_everything(&s.ui);
    let said = s.notified.lock().unwrap().clone();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("7 updates waiting for you to finish"));
    assert_eq!(s.store().get_gate(), 0, "nothing is asked of a window nobody sees");
    // Opened again, it is asked; "Finish now" does the rest, system and all.
    s.app.window_opened(&s.ui);
    assert_eq!(s.store().get_gate(), 1);
    s.store().invoke_gate_answer(true);
    assert_eq!(versions(&s), ["v1.21.0", "v3.4.0"]);
    assert_eq!(s.store().get_system_headline(), "Up to date");
    assert_eq!(s.store().get_gate(), 0);
    s.app.window_opened(&s.ui);
    assert_eq!(s.store().get_gate(), 0, "nothing is left to offer");

    // Staying in the tray is the default, and can be switched off.
    assert!(s.store().get_close_to_tray() && !s.saved().settings.quit_on_close);
    s.store().invoke_set_close_to_tray(false);
    assert!(!s.store().get_close_to_tray() && s.saved().settings.quit_on_close);

    // Kept, and with a PIN on installs it still asks nothing: it was agreed to in
    // Settings, which the PIN guards.
    let seed = format!(
        r#"{{"settings":{{"welcomed":true,"auto_update":true,"pin_hash":"00","pin_salt":"00","pin_for_installs":true}},{library}}}"#
    );
    let s = start_with("linux-x64-arch", Shell::Desktop, Some(&seed));
    assert!(s.store().get_auto_update());
    s.app.check_everything(&s.ui);
    assert_eq!(versions(&s), ["v1.20.0", "v3.4.0"]);
    assert_eq!(s.store().get_gate_title(), "Updates are waiting for you", "no PIN is asked for; the rest is offered");
    s.store().invoke_gate_answer(false);
    assert_eq!(s.store().get_gate(), 0);
    // Switching it off needs no PIN; switching it on does.
    s.store().invoke_set_auto_update(false);
    assert!(!s.saved().settings.auto_update && s.store().get_gate() == 0);
    s.store().invoke_set_auto_update(true);
    assert!(!s.saved().settings.auto_update && s.store().get_gate() == 2);
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

#[test]
fn an_app_that_was_given_the_stores_name_gets_its_own_back() {
    // An earlier version could note an app updated from the Library under the name
    // of whichever page was open: Longboard, listed as "Bilby".
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:AndrewGBest/Longboard","name":"Bilby","version":"v0.3.0"},
        {"id":"github:lumen-labs/lumen","name":"Lumen","version":"v0.19.2"}]}"#;
    let s = start_as_store_with("linux-x64-debian", "github:kestrel-sh/kestrel", seed);
    let names: Vec<(String, String)> = s.saved().library.iter().map(|a| (a.id.clone(), a.name.clone())).collect();
    assert!(names.contains(&("github:AndrewGBest/Longboard".into(), "Longboard".into())), "{names:?}");
    assert!(names.contains(&("github:lumen-labs/lumen".into(), "Lumen".into())));
    assert_eq!(names.iter().filter(|(_, name)| name == "Bilby").count(), 1, "the store itself, and only it");
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

// Placing an AppImage and pointing a menu entry at it is how Linux does it.
#[cfg(target_os = "linux")]
#[test]
fn updating_bilby_itself_repoints_its_menu_entry_and_offers_to_restart() {
    let start_as = |store_app: &str| {
        init_backend();
        bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
        let dirs = dirs_in(&temp_root());
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
        let started: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let record = started.clone();
        // The copy that is running, somewhere the person put it.
        let running = dirs.downloads.join("bilby-old.AppImage");
        std::fs::create_dir_all(&dirs.downloads).unwrap();
        std::fs::write(&running, b"old").unwrap();
        let options = Options {
            demo: true,
            inline: true,
            platform: Platform::preset("linux-x64-arch"),
            shell: Some(Shell::Desktop),
            dirs: Some(dirs.clone()),
            opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
            launcher: Some(Arc::new(move |path: &std::path::Path| {
                record.lock().unwrap().push(path.to_path_buf());
                Ok(())
            })),
            store_app: Some(store_app.to_string()),
            self_program: Some(running.clone()),
            ..Default::default()
        };
        let (ui, app) = build(&options).unwrap();
        (ui, app, dirs, started, running)
    };
    // A demo project standing in for Bilby whose file for this system is an AppImage.
    let stand_in = ["github:tidewave/tidewave", "github:lumen-labs/lumen", "github:kestrel-sh/kestrel"]
        .into_iter()
        .find(|id| {
            let (ui, app, ..) = start_as(id);
            app.open(&ui, id);
            ui.global::<Store>().get_options().row_data(0).is_some_and(|o| o.label.contains("AppImage"))
        })
        .expect("a demo app that comes as an AppImage");
    let (ui, app, dirs, started, running) = start_as(stand_in);
    let store = ui.global::<Store>();
    let own_entry = dirs.menu.join("bilby.desktop");
    assert!(
        std::fs::read_to_string(&own_entry).unwrap().contains("bilby-old.AppImage"),
        "the menu starts the running copy"
    );

    // Its own page knows it is installed, and there is nothing to "open": it is open.
    app.open(&ui, stand_in);
    assert!(store.get_in_library() && !store.get_can_open());

    // Updating it places the new file and points Bilby's own menu entry at it.
    store.invoke_install(0);
    assert_eq!(store.get_install_state(), 2, "{}", store.get_install_message());
    assert!(store.get_install_message().contains("restart into it"), "{}", store.get_install_message());
    let entry = std::fs::read_to_string(&own_entry).unwrap();
    assert!(!entry.contains("bilby-old.AppImage"), "the menu no longer starts the old copy: {entry}");
    assert!(entry.contains(&dirs.applications.display().to_string()), "it starts the new one: {entry}");
    let entries = std::fs::read_dir(&dirs.menu)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".desktop"))
        .count();
    assert_eq!(entries, 1, "one entry for Bilby, not a second beside it");
    assert!(running.exists(), "the copy the person put there themselves is left alone");

    // It asks at once whether to restart into it; Later leaves things as they are.
    assert_eq!(store.get_gate(), 1);
    assert_eq!((store.get_gate_no().as_str(), store.get_gate_yes().as_str()), ("Later", "Restart Bilby now"));
    store.invoke_gate_answer(false);
    assert!(store.get_gate() == 0 && started.lock().unwrap().is_empty());

    // Open now means: restart into the new copy.
    assert!(store.get_can_open());
    store.invoke_launch();
    let launched = started.lock().unwrap().clone();
    assert_eq!(launched.len(), 1);
    assert!(launched[0].starts_with(&dirs.applications), "{launched:?}");

    // An older copy started later, from a shortcut that still points at it, finds the
    // newer one the store placed and hands over to it. The newer one does not.
    let state = State::load(&dirs.config.join("demo-state.json"));
    let newer = bilby::newer_copy_in(&state, stand_in, &dirs, "0.0.1", Some(&running));
    assert_eq!(newer.as_ref(), Some(&launched[0]));
    assert_eq!(bilby::newer_copy_in(&state, stand_in, &dirs, "0.0.1", Some(&launched[0])), None, "not to itself");
    assert_eq!(bilby::newer_copy_in(&state, stand_in, &dirs, "999.0.0", Some(&running)), None, "not to an older one");
    std::fs::remove_file(&launched[0]).unwrap();
    assert_eq!(bilby::newer_copy_in(&state, stand_in, &dirs, "0.0.1", Some(&running)), None, "not to one that is gone");
}

#[cfg(target_os = "linux")]
#[test]
fn updating_bilby_from_the_library_also_asks_to_restart() {
    init_backend();
    bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
    let dirs = dirs_in(&temp_root());
    std::fs::create_dir_all(&dirs.config).unwrap();
    let started: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let record = started.clone();
    let find = |seed: Option<&str>, store_app: &str| {
        std::fs::write(dirs.config.join("demo-state.json"), seed.unwrap_or(r#"{"settings":{"welcomed":true}}"#))
            .unwrap();
        let record = record.clone();
        let options = Options {
            demo: true,
            inline: true,
            platform: Platform::preset("linux-x64-arch"),
            shell: Some(Shell::Desktop),
            dirs: Some(dirs.clone()),
            opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
            launcher: Some(Arc::new(move |path: &std::path::Path| {
                record.lock().unwrap().push(path.to_path_buf());
                Ok(())
            })),
            store_app: Some(store_app.to_string()),
            ..Default::default()
        };
        build(&options).unwrap()
    };
    let Some(stand_in) =
        ["github:tidewave/tidewave", "github:lumen-labs/lumen", "github:kestrel-sh/kestrel"].into_iter().find(|id| {
            let (ui, app) = find(None, id);
            app.open(&ui, id);
            ui.global::<Store>().get_options().row_data(0).is_some_and(|o| o.label.contains("AppImage"))
        })
    else {
        return;
    };
    let (ui, app) = find(None, stand_in);
    let store = ui.global::<Store>();
    // The Library's own Update, with no app page open.
    app.update_app(&ui, stand_in);
    assert_eq!(store.get_gate(), 1, "the question comes up over the Library");
    assert!(store.get_gate_text().contains("Restart Bilby"), "{}", store.get_gate_text());
    store.invoke_gate_answer(true);
    let launched = started.lock().unwrap().clone();
    assert_eq!(launched.len(), 1, "Restart now starts the new copy");
    assert!(launched[0].starts_with(&dirs.applications));
}
