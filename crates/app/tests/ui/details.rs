//! An app's page, and installing from it.

use super::*;

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

#[test]
fn a_download_asked_for_while_another_is_going_waits_its_turn() {
    let s = start("linux-x64-arch", Shell::Desktop);
    let names = |s: &Session| -> Vec<String> { s.saved().library.iter().map(|a| a.name.clone()).collect() };
    // Something else is downloading.
    s.app.hold_download(&s.ui, Some(("github:kestrel-sh/kestrel".into(), "Kestrel".into())));
    s.app.open(&s.ui, "github:tidewave/tidewave");
    s.app.install(&s.ui, 0);
    assert!(s.store().get_queued() && s.store().get_install_state() == 0);
    assert_eq!(s.store().get_toast(), "Tidewave is queued. It starts when Kestrel has finished.");
    assert!(names(&s).is_empty(), "nothing has started");
    // Asked again, it leaves the queue.
    s.app.install(&s.ui, 0);
    assert!(!s.store().get_queued());
    assert_eq!(s.store().get_toast(), "Tidewave is no longer queued");
    // Two wait, and the person moves on to some other page; its own page remembers.
    s.app.install(&s.ui, 0);
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert!(!s.store().get_queued());
    s.app.install(&s.ui, 0);
    assert!(s.store().get_queued());
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert!(s.store().get_queued(), "still waiting when its page is opened again");
    s.app.open(&s.ui, "pacman:harbor");
    // The first finishes: the others follow, in the order they were asked for.
    s.app.hold_download(&s.ui, None);
    let mut installed = names(&s);
    installed.sort();
    assert_eq!(installed, ["Lumen", "Tidewave"]);
    assert_eq!(
        (s.store().get_queued(), s.store().get_install_state()),
        (false, 0),
        "the page that is open is left alone"
    );
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert!(!s.store().get_queued() && !s.store().get_installed_version().is_empty());

    // An update asked for from the Library while something downloads waits as well.
    let s = start_with("linux-x64-debian", Shell::Desktop, Some(OLD_KESTREL));
    s.app.check_updates(&s.ui, false);
    s.app.hold_download(&s.ui, Some(("github:tidewave/tidewave".into(), "Tidewave".into())));
    s.app.update_app(&s.ui, "github:kestrel-sh/kestrel");
    assert_eq!(s.saved().library[0].version, "v1.20.0");
    s.app.hold_download(&s.ui, None);
    assert_eq!(s.saved().library[0].version, "v1.21.0");
}

#[test]
fn unsafe_links_are_refused() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().invoke_open_url("file:///etc/passwd".into());
    assert!(s.store().get_toast_is_error());
    assert!(s.store().get_toast().contains("not an https link"));
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
    assert!(s.has("Picture 1 of 3"));
    let first = s.store().get_screenshots().row_data(0).unwrap();
    assert_eq!((first.size().width, first.size().height), (800, 500));
    assert!(s.dirs.cache.join("screenshots").read_dir().unwrap().count() >= 3, "they are kept for next time");

    // A package has no README, and the last app's pictures do not linger.
    s.app.open(&s.ui, "apt:harbor");
    assert_eq!(s.store().get_screenshots().row_count(), 0);
}

#[test]
fn an_apps_page_says_where_it_comes_from_and_links_to_its_page_there() {
    let s = start("linux-x64-arch", Shell::Desktop);
    // From a project's releases: the repository is a press away.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_source_page(), "https://github.com/lumen-labs/lumen");
    assert!(s.has("Comes from GitHub") || s.has("Installing from GitHub"), "the source is named by the button");
    s.activate("See it on GitHub");
    assert_eq!(s.opened.lock().unwrap().last().unwrap(), &PathBuf::from("https://github.com/lumen-labs/lumen"));

    // From a store: its listing there, not the project's own website.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_source_page(), "https://flathub.org/apps/io.quillpad.Quillpad");
    assert!(s.has("See it on Flathub"));

    // A phone's page says it too.
    let phone = start("android-arm64", Shell::Mobile);
    phone.app.open(&phone.ui, "github:lumen-labs/lumen");
    assert!(phone.has("See it on GitHub"));
}

#[test]
fn apps_that_do_not_run_here_can_be_shown_and_are_marked_not_installable() {
    let s = start("android-arm64", Shell::Mobile);
    s.app.search(&s.ui, "monitor");
    let usual = s.store().get_results().row_count();
    assert!(s.store().get_search_note().contains("do not work on this device"));

    // Settings has the switch; with it on, the same search finds them too.
    s.store().invoke_set_show_everything(true);
    assert!(s.saved().settings.show_everything);
    s.app.search(&s.ui, "monitor");
    let cards: Vec<AppCard> = s.store().get_results().iter().collect();
    assert_eq!(cards.len(), usual + 2);
    assert!(!s.store().get_search_note().contains("do not work on this device"));
    let marked: Vec<&AppCard> = cards.iter().filter(|card| card.headline == "Not for this device").collect();
    assert_eq!(marked.len(), 2);
    assert_eq!(cards.last().unwrap().headline, "Not for this device", "they come last");
    assert!(marked.iter().all(|card| card.stat == "Not for this device" && card.rating.is_empty()));
    // No quick Install on those.
    let name = marked[0].name.to_string();
    assert!(!s.has(&format!("Install {name} now")));

    // Off again: as before.
    s.store().invoke_set_show_everything(false);
    s.app.search(&s.ui, "monitor");
    assert_eq!(s.store().get_results().row_count(), usual);
}

#[test]
fn a_picture_on_an_apps_page_opens_large_and_is_put_away_again() {
    for shell in [Shell::Desktop, Shell::Mobile] {
        let s = start("linux-x64-debian", shell);
        s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
        assert_eq!(s.store().get_screenshots().row_count(), 2);
        assert_eq!(s.store().get_picture(), -1, "nothing is large until a picture is pressed");
        assert!(!s.has("Picture 1 of 2, large"));

        s.activate("Picture 1 of 2");
        assert_eq!(s.store().get_picture(), 0);
        assert!(s.has("Picture 1 of 2, large"));
        // The arrows go to the next picture and back, and stop at both ends.
        assert!(!s.has("Previous picture"));
        s.activate("Next picture");
        assert!(s.has("Picture 2 of 2, large") && !s.has("Next picture"));
        s.press(Key::RightArrow);
        assert_eq!(s.store().get_picture(), 1, "there is no third");
        s.press(Key::LeftArrow);
        assert_eq!(s.store().get_picture(), 0);

        // Escape puts the picture away and leaves the app's page open.
        s.press(Key::Escape);
        assert_eq!(s.store().get_picture(), -1);
        assert!(s.store().get_details_open());
        // And then Escape is the page's again.
        s.press(Key::Escape);
        assert!(!s.store().get_details_open());

        // The cross closes it too, and another app's page starts with nothing large.
        s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
        s.activate("Picture 2 of 2");
        s.activate("Close");
        assert_eq!(s.store().get_picture(), -1);
        s.activate("Picture 2 of 2");
        s.app.open(&s.ui, "github:lumen-labs/lumen");
        assert_eq!(s.store().get_picture(), -1);
    }
}

#[test]
fn an_installs_log_is_on_the_apps_page_folded_away_until_asked_for() {
    let s = start("linux-x64-fedora", Shell::Desktop);
    s.click("Tidewave");
    assert_eq!(s.store().get_install_log(), "", "nothing has been installed yet");
    assert!(!s.has("Show the install log"));

    s.click("Install");
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    // It says what was fetched, from where, how it was checked and how it ended.
    let log = s.store().get_install_log().to_string();
    assert!(log.starts_with("tidewave · RPM package · x86_64\ngithub:tidewave/tidewave\nBilby "), "{log}");
    assert!(log.contains("\nFetching https://") && log.contains("SHA-256") && log.contains("\nDone: "), "{log}");
    // Not shown until asked for.
    assert!(!s.store().get_log_open());
    assert!(s.has("Show the install log") && !s.has("Copy the log"));
    s.activate("Show the install log");
    assert!(s.store().get_log_open());
    assert!(s.has("Hide the install log") && s.has("Copy the log"));

    // Another app's page has no log of its own, and this one's is still there on
    // coming back, folded away again.
    s.click("Back");
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_install_log(), "");
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert!(s.store().get_install_log().contains("\nDone: "));
    assert!(!s.store().get_log_open());
}
