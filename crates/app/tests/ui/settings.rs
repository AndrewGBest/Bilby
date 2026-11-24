//! Settings, and that they are remembered.

use super::*;

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
    // (The GitHub card is far down the Settings page, so it is pressed through its action.)
    s.store().invoke_save_token("".into());
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
fn the_country_for_prices_can_be_chosen_and_is_remembered() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // The system's own to begin with, named; then the countries on offer.
    let choices = s.store().get_countries();
    assert!(choices.row_count() > 40);
    assert!(choices.row_data(0).unwrap().on && choices.row_data(0).unwrap().id.is_empty());
    assert_eq!(s.store().get_country_name(), choices.row_data(0).unwrap().label);
    s.store().invoke_set_country("CA".into());
    assert_eq!(s.saved().settings.country, "CA");
    assert_eq!(s.store().get_country_name(), "Canada");
    let on: Vec<String> = s.store().get_countries().iter().filter(|c| c.on).map(|c| c.id.to_string()).collect();
    assert_eq!(on, ["CA"]);
    // Not a country on offer: nothing changes.
    s.store().invoke_set_country("ZZ".into());
    s.store().invoke_set_country("../x".into());
    assert_eq!(s.saved().settings.country, "CA");
    // A remote steps through them, both ways and round the ends; "" is the system's own.
    s.store().invoke_step_country(true);
    assert_eq!(s.saved().settings.country, "MX");
    s.store().invoke_step_country(false);
    s.store().invoke_step_country(false);
    assert_eq!(s.saved().settings.country, "US");
    s.store().invoke_step_country(false);
    assert_eq!(s.saved().settings.country, "");
    s.store().invoke_step_country(false);
    assert_eq!(s.saved().settings.country, "NZ");
    s.store().invoke_set_country("".into());
    assert!(s.store().get_countries().row_data(0).unwrap().on);
}

#[test]
fn settings_come_in_sections_and_windows_apps_have_choices_of_their_own() {
    // On a desktop: one section at a time. General first; a setting elsewhere is in its section.
    let s = start("linux-x64-debian", Shell::Desktop);
    s.click("Settings");
    assert_eq!(s.store().get_settings_section(), 0);
    assert!(!ElementHandle::find_by_accessible_label(&s.ui, "Include pre-releases").any(|_| true));
    s.click("Updates");
    assert_eq!(s.store().get_settings_section(), 5);
    assert!(ElementHandle::find_by_accessible_label(&s.ui, "Include pre-releases").any(|_| true));
    // The Windows section is only offered where Windows apps can run; not on this stand-in.
    assert!(!s.has("Windows apps"));

    // How Windows apps run is kept where their launchers read it, and shown back.
    assert_eq!((s.store().get_windows_proton(), s.store().get_windows_overlay()), (0, false));
    s.store().invoke_set_windows_proton(1);
    s.store().invoke_set_windows_overlay(true);
    assert_eq!((s.store().get_windows_proton(), s.store().get_windows_overlay()), (1, true));
    let kept = std::fs::read_to_string(bilby_core::winrun::general_choices(&s.dirs)).unwrap();
    assert!(kept.contains("export PROTONPATH=GE-Proton") && kept.contains("export MANGOHUD=1"), "{kept}");
    s.store().invoke_set_windows_proton(0);
    s.store().invoke_set_windows_overlay(false);
    assert!(!bilby_core::winrun::general_choices(&s.dirs).exists(), "nothing chosen leaves no file");
    // An app that is not installed has no choices to change.
    s.store().invoke_set_windows_app("github:nobody/nothing".into(), 2, 1);
    assert_eq!(s.store().get_windows_installed().row_count(), 0);

    // On a TV: the sections are a list on the left. Left goes to it, Down along it,
    // Right into the section pointed at.
    let tv = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    for key in [Key::UpArrow, Key::UpArrow, Key::RightArrow, Key::RightArrow, Key::RightArrow, Key::Return] {
        tv.press(key);
    }
    assert_eq!(tv.store().get_page(), 3);
    assert!(tv.has("Language and country"));
    // Left, Down to Family, Right: the first setting of that section is the one OK changes.
    tv.press(Key::LeftArrow);
    tv.press(Key::DownArrow);
    tv.press(Key::RightArrow);
    assert!(!tv.saved().settings.family_mode);
    tv.press(Key::Return);
    assert!(tv.saved().settings.family_mode, "Family's first setting is family mode");
    // In the list of sections, OK does not change a setting.
    tv.press(Key::LeftArrow);
    tv.press(Key::Return);
    assert!(tv.saved().settings.family_mode);
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
    tv.press(Key::RightArrow);
    tv.press(Key::Return);
    for _ in 0..4 {
        tv.press(Key::DownArrow);
    }
    assert!(tv.has("Recommend the apps everyone knows, on Home and in searches"));
    tv.press(Key::Return);
    assert!(tv.saved().settings.hide_essentials, "OK on the row switches it off");
}

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

#[test]
fn a_list_on_the_persons_own_server_can_be_added_and_its_apps_installed() {
    let s = start("linux-x64-debian", Shell::Desktop);
    let address = bilby_core::demo::OWN_SOURCE;
    assert_eq!(s.store().get_own_sources().row_count(), 0);

    // An address that is not such a list is refused, with the reason, and nothing is kept.
    s.store().invoke_add_own_source("https://workshop.example/not-a-list".into());
    assert!(s.store().get_toast().starts_with("Not added."), "{}", s.store().get_toast());
    s.store().invoke_add_own_source("ftp://workshop.example/apps.json".into());
    assert!(s.store().get_toast().contains("not a web address"), "{}", s.store().get_toast());
    assert!(s.saved().settings.own_sources.is_empty());

    // The real one is fetched, named by what it calls itself, and remembered.
    s.store().invoke_add_own_source(address.into());
    assert!(s.store().get_toast().contains("Added “Workshop apps”: 1 app"), "{}", s.store().get_toast());
    let kept = s.saved().settings.own_sources;
    assert_eq!((kept[0].name.as_str(), kept[0].url.as_str()), ("Workshop apps", address));
    assert_eq!(s.store().get_own_sources().row_data(0).unwrap().label, "Workshop apps");
    s.store().invoke_add_own_source(address.into());
    assert_eq!(s.saved().settings.own_sources.len(), 1, "not twice");

    // Its apps are searched like any others, and it is a source like any other.
    s.app.search(&s.ui, "gauge");
    let card = s.store().get_results().iter().find(|c| c.id == "own:workshop-apps/gauge").expect("found by searching");
    assert_eq!((card.name.as_str(), card.source.as_str()), ("Gauge", "Workshop apps"));
    assert!(s.store().get_sources().iter().any(|source| source.id == "own"), "and can be switched off with the rest");

    // The file for this device is picked from its name, and installs.
    s.app.open(&s.ui, "own:workshop-apps/gauge");
    assert!(s.store().get_details_error().is_empty(), "{}", s.store().get_details_error());
    assert_eq!(s.option_labels()[0], "Debian package · x86_64");
    s.store().invoke_install(0);
    assert!(s.saved().library.iter().any(|a| a.id == "own:workshop-apps/gauge"));

    // Removing the source takes its apps out of searches; what was installed stays.
    s.store().set_details_open(false);
    s.store().invoke_remove_own_source(address.into());
    assert!(s.saved().settings.own_sources.is_empty());
    s.app.search(&s.ui, "gauge");
    assert!(!s.store().get_results().iter().any(|c| c.id.starts_with("own:")));
    assert!(s.saved().library.iter().any(|a| a.id == "own:workshop-apps/gauge"));
}

#[test]
fn moving_about_makes_quiet_sounds_that_can_be_switched_off() {
    use bilby::sound::Sound;
    init_backend();
    let dirs = dirs_in(&temp_root());
    std::fs::create_dir_all(&dirs.config).unwrap();
    let state_file = dirs.config.join("demo-state.json");
    std::fs::write(&state_file, r#"{"settings":{"welcomed":true}}"#).unwrap();
    let heard: Arc<Mutex<Vec<Sound>>> = Arc::default();
    let record = heard.clone();
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-tv"),
        shell: Some(Shell::Tv),
        dirs: Some(dirs),
        heard: Some(Arc::new(move |sound| record.lock().unwrap().push(sound))),
        ..Default::default()
    };
    let (ui, _app) = build(&options).unwrap();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 720.0));
    let press = |key: Key| {
        let text: SharedString = key.into();
        ui.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        ui.window().dispatch_event(WindowEvent::KeyReleased { text });
    };
    let store = ui.global::<Store>();
    assert!(store.get_sounds(), "on to begin with");

    // The remote: a tick for each move, a pop for OK, a lower note for Back.
    press(Key::RightArrow);
    press(Key::DownArrow);
    press(Key::Return);
    press(Key::Escape);
    assert_eq!(*heard.lock().unwrap(), [Sound::Move, Sound::Move, Sound::Select, Sound::Back]);

    // Switched off, there is silence, and it is remembered.
    store.invoke_set_sounds(false);
    assert!(State::load(&state_file).settings.mute_sounds && !store.get_sounds());
    heard.lock().unwrap().clear();
    press(Key::RightArrow);
    press(Key::Return);
    press(Key::Escape);
    assert!(heard.lock().unwrap().is_empty());
    // Switching them back on is itself heard.
    store.invoke_set_sounds(true);
    assert_eq!(*heard.lock().unwrap(), [Sound::Toggle]);
}

#[test]
fn the_kind_of_app_offered_first_can_be_chosen_in_settings() {
    let s = start("linux-x64-arch", Shell::Desktop);
    let store = s.store();
    assert!(store.get_prefer_applies() && store.get_prefer_kind() == 0);
    // An app that comes more than one way on this system.
    let (id, first) = ["github:tidewave/tidewave", "github:lumen-labs/lumen", "github:kestrel-sh/kestrel"]
        .into_iter()
        .find_map(|id| {
            s.app.open(&s.ui, id);
            let labels: Vec<String> = store.get_options().iter().map(|o| o.label.to_string()).collect();
            (labels.len() > 1 && labels.iter().skip(1).any(|l| l.contains("AppImage"))).then(|| (id, labels[0].clone()))
        })
        .expect("a demo app whose AppImage is not the first choice on Arch");
    assert!(!first.contains("AppImage"));
    // Asked for, the AppImage leads; it is remembered; Automatic puts things back.
    store.invoke_set_prefer_kind(2);
    assert_eq!(State::load(&s.state_file).settings.prefer_kind, "appimage");
    s.app.open(&s.ui, id);
    assert!(store.get_options().row_data(0).unwrap().label.contains("AppImage"));
    store.invoke_set_prefer_kind(0);
    s.app.open(&s.ui, id);
    assert_eq!(store.get_options().row_data(0).unwrap().label, first.as_str());
}
