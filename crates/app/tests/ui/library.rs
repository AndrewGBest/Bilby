//! The Library: what is installed, favourites, and taking things off.

use super::*;

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
fn the_desktop_library_lists_apps_the_store_did_not_install_and_opens_their_pages() {
    let s = start("linux-x64-arch", Shell::Desktop);
    let flatpak = s.dirs.flatpak_roots[0].join("app/org.example.Paint/current/active/export/share/applications");
    std::fs::create_dir_all(&flatpak).unwrap();
    std::fs::write(flatpak.join("org.example.Paint.desktop"), "[Desktop Entry]\nName=Paint Box\n").unwrap();
    s.app.scan_installed(&s.ui);
    s.store().set_page(2);
    let listed: Vec<(String, String)> =
        s.store().get_also_installed().iter().map(|card| (card.id.to_string(), card.source.to_string())).collect();
    assert!(listed.contains(&("flathub:org.example.Paint".to_string(), "Flathub".to_string())), "{listed:?}");
    assert!(listed.iter().any(|(id, _)| id.starts_with("pacman:")), "the system's own packages too: {listed:?}");
    assert!(s.has("Also installed here") && s.has("Paint Box"));

    // Its page is where it is started, updated or removed, like any other app's.
    s.activate("Paint Box");
    assert_eq!(s.store().get_app().id, "flathub:org.example.Paint");
    assert!(s.store().get_can_uninstall() && !s.store().get_in_library());

    // Something the store installed itself is in its own list, not in this one.
    let package = s.store().get_also_installed().iter().find(|card| card.id.starts_with("pacman:")).unwrap();
    s.app.open(&s.ui, package.id.as_str());
    assert!(s.store().get_can_uninstall(), "a system package is removed with the system's own tools");
}

#[test]
#[cfg(unix)]
fn apps_the_store_did_not_install_can_be_uninstalled() {
    // A package pacman installed: the page offers to uninstall it, asked twice.
    let s = start("linux-x64-arch", Shell::Desktop);
    s.app.open(&s.ui, "pacman:harbor");
    let name = s.store().get_app().name.to_string();
    assert!(s.store().get_can_uninstall() && !s.store().get_in_library());
    let log = s.dirs.elevate.with_extension("log");
    s.click(&format!("Uninstall {name}"));
    assert!(!log.exists(), "nothing happens until it is confirmed");
    s.click(&format!("Confirm uninstalling {name}"));
    let asked = std::fs::read_to_string(&log).unwrap();
    assert_eq!(asked.trim(), "pacman -R --noconfirm harbor", "pacman, with the system's password");
    assert_eq!(s.store().get_toast(), format!("{name} uninstalled"));
    assert_eq!(s.store().get_installed_version(), "");
    // What is not on the device, and what no tool here looks after, has no such button.
    s.app.open(&s.ui, "pacman:swiftkernel");
    assert!(!s.store().get_can_uninstall());
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert!(!s.store().get_can_uninstall());

    // A Flatpak from somewhere else, on a TV: its page has Uninstall, and Flatpak is asked.
    let s = start("linux-tv", Shell::Tv);
    let flatpak = s.dirs.flatpak_roots[0].join("app/org.example.Paint/current/active/export/share/applications");
    std::fs::create_dir_all(&flatpak).unwrap();
    std::fs::write(flatpak.join("org.example.Paint.desktop"), "[Desktop Entry]\nName=Paint Box\n").unwrap();
    s.app.scan_installed(&s.ui);
    s.app.open(&s.ui, "flathub:org.example.Paint");
    assert!(s.store().get_can_uninstall() && !s.store().get_in_library());
    s.app.remove_from_library(&s.ui, "flathub:org.example.Paint");
    let ran = s.web_opened.lock().unwrap().clone();
    assert!(ran.iter().any(|(_, args)| args == "uninstall --noninteractive --assumeyes org.example.Paint"), "{ran:?}");
    assert!(s.store().get_toast().starts_with("Uninstalling"));
    let left: Vec<String> = s.store().get_also_installed().iter().map(|card| card.name.to_string()).collect();
    assert!(!left.contains(&"Paint Box".to_string()), "and it leaves the library's list: {left:?}");
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
    // With it, what the system's own package manager installed.
    assert_eq!(also.1, ["Paint Box", "harbor"]);
    // The same app is marked installed wherever the store shows it.
    assert!(s.app.card(&AppSummary::new(AppId::parse("flathub:org.example.Paint").unwrap(), "Paint Box")).installed);

    // The device's card opens its page; the store's own card opens the store's.
    s.store().set_page(2);
    s.store().invoke_open("bilby:this-device".into());
    assert_eq!(s.store().get_page(), 4);
    assert!(!s.store().get_details_open());

    // The store is in its own library, and asking for a newer one always answers.
    let own = start_as_store("linux-x64-debian", "github:kestrel-sh/kestrel");
    let own_id = "github:kestrel-sh/kestrel";
    let running = format!("v{}", env!("CARGO_PKG_VERSION"));
    let noted = own.saved().library.into_iter().find(|a| a.id == own_id).expect("the store is in its own library");
    assert_eq!((noted.name.as_str(), noted.version.as_str()), ("Bilby", running.as_str()));
    assert!(own.store().get_library().iter().any(|row| row.card.id == own_id), "listed on the Library page");
    assert!(own.store().get_tv_library_rows().iter().any(|row| row.apps.iter().any(|c| c.id == own_id)));
    // It is marked installed wherever it is shown, and cannot be removed from its own library.
    own.app.open(&own.ui, own_id);
    assert!(own.store().get_in_library());
    own.store().invoke_remove_from_library(own_id.into());
    assert!(own.saved().library.iter().any(|a| a.id == own_id));
    assert!(own.store().get_toast().contains("Bilby itself"), "{}", own.store().get_toast());
    own.store().invoke_check_store_update();
    assert!(own.store().get_toast().contains("Bilby"), "{}", own.store().get_toast());
}
