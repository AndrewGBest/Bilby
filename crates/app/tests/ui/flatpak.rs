//! Flatpak apps: from Flathub, from a bundle, and ones something else installed.

use super::*;

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
    assert_eq!(hit.icon.size().width, 44, "Flathub's icon for the app is shown, at the size of a tile");

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
        "install --assumeyes --or-update flathub io.quillpad.Quillpad"
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

/// A Flatpak bundle from a developer's own release page: Flatpak takes the app out of
/// it, so the page must offer Open afterwards, nothing may be left in the downloads
/// folder, and installing it again must go through.
#[test]
#[cfg(unix)]
fn a_flatpak_bundle_is_opened_installed_again_and_removed() {
    use std::os::unix::fs::PermissionsExt;
    let s = start("linux-x64-debian", Shell::Tv);
    // A stand-in for Flatpak that says what the real one says, and does what it does.
    let log = s.dirs.cache.join("flatpak-calls.txt");
    let apps = s.dirs.flatpak_roots[0].join("app");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{log}'\nif [ \"$1\" = install ]; then echo 'Installing app/org.driftnotes.Drift/x86_64/stable'; mkdir -p '{apps}/org.driftnotes.Drift'; fi\nexit 0\n",
        log = log.display(),
        apps = apps.display(),
    );
    std::fs::create_dir_all(s.dirs.flatpak.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&s.dirs.cache).unwrap();
    std::fs::write(&s.dirs.flatpak, script).unwrap();
    std::fs::set_permissions(&s.dirs.flatpak, std::fs::Permissions::from_mode(0o755)).unwrap();

    s.app.open(&s.ui, "github:driftnotes/drift");
    let bundle = s.option_labels().iter().position(|l| l.contains("Flatpak")).expect("the bundle is offered") as i32;
    assert!(!s.store().get_can_open());
    for round in 0..2 {
        s.app.install(&s.ui, bundle);
        assert_eq!(s.store().get_install_state(), 2, "round {round}: {}", s.store().get_install_message());
    }
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls.lines().count(), 2);
    assert!(
        calls.lines().all(|l| l.starts_with("install --user --noninteractive --assumeyes --reinstall ")),
        "{calls}"
    );

    // The bundle was a parcel: it is not in the downloads folder, nor kept anywhere.
    assert!(!s.dirs.downloads.join("Drift-2.2.0.flatpak").exists());
    assert_eq!(std::fs::read_dir(s.dirs.cache.join("bundles")).unwrap().count(), 0);

    // The library knows which Flatpak it became, so the page opens it instead of
    // offering to install it again.
    assert_eq!(s.saved().library[0].flatpak, "org.driftnotes.Drift");
    assert!(s.store().get_can_open());
    assert!(!s.has("Install again"));
    s.app.launch(&s.ui);
    assert_eq!(s.web_opened.lock().unwrap().last().unwrap().1, "run org.driftnotes.Drift");

    // Removed outside the store: the page goes back to offering it.
    std::fs::remove_dir_all(apps.join("org.driftnotes.Drift")).unwrap();
    s.app.open(&s.ui, "github:driftnotes/drift");
    assert!(!s.store().get_can_open());
    std::fs::create_dir_all(apps.join("org.driftnotes.Drift")).unwrap();

    // Removing it from the library has Flatpak uninstall it.
    s.app.remove_from_library(&s.ui, "github:driftnotes/drift");
    assert_eq!(
        s.web_opened.lock().unwrap().last().unwrap().1,
        "uninstall --noninteractive --assumeyes org.driftnotes.Drift"
    );
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
