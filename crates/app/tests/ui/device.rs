//! The page about this device and the operating system's own updates.

use super::*;

#[test]
fn the_system_s_updates_are_in_the_library_and_on_their_own_page() {
    let s = start("linux-x64-arch", Shell::Desktop);
    assert!(s.store().get_system_supported());
    assert_eq!(s.store().get_system_name(), "Arch Linux");
    assert_eq!((s.store().get_system_state(), s.store().get_system_headline().as_str()), (0, "Not checked yet"));

    s.app.check_system(&s.ui, false);
    assert_eq!((s.store().get_system_state(), s.store().get_system_headline().as_str()), (3, "7 updates available"));
    let first = s.store().get_system_updates().row_data(0).unwrap();
    assert_eq!((first.name.as_str(), first.change.as_str()), ("linux", "6.11.5.arch1-1 → 6.11.6.arch1-1"));

    // The library leads with the system, even with no apps in it yet.
    s.store().set_page(2);
    assert!(s.has("Update system") && s.has("7 updates available"));
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
fn an_update_still_going_from_before_is_shown_as_going() {
    // The system's updater is in the middle of a change that this Bilby did not start:
    // it was closed and opened again while one was being installed.
    let platform = Platform::preset("linux-x64-fedora").unwrap();
    let busy = r#"{"transaction":["rebase","client","/run/x"],"deployments":[{"booted":true}]}"#;
    let host = bilby_core::os::demo_host(&platform).file("/run/ostree-booted", "").program("rpm-ostree").command(
        "rpm-ostree status --json",
        0,
        busy,
    );
    let s =
        start_on("linux-x64-fedora", Shell::Desktop, Some(r#"{"settings":{"welcomed":true}}"#), Some(Arc::new(host)));
    s.app.check_system(&s.ui, false);
    assert_eq!((s.store().get_system_state(), s.store().get_system_headline().as_str()), (4, "Installing updates…"));
    assert_eq!(s.store().get_system_step(), "Started earlier and still going");
    s.store().set_page(4);
    assert!(!s.has("Update now"), "not offered again while it is being installed");
    s.app.update_system(&s.ui);
    assert_eq!(s.store().get_system_state(), 4);
}

#[test]
fn updates_that_wait_for_a_restart_offer_one_and_ask_first() {
    // A whole-image system with the new image in place: nothing is waiting but a restart.
    let platform = Platform::preset("linux-x64-fedora").unwrap();
    let staged = r#"{"deployments":[{"booted":false,"staged":true},{"booted":true}]}"#;
    let machine = || {
        bilby_core::os::demo_host(&platform)
            .file("/run/ostree-booted", "")
            .program("rpm-ostree")
            .command("rpm-ostree status --json", 0, staged)
            .command("flatpak update --user", 0, "Nothing to do.\n")
            .command("flatpak update --system", 0, "Nothing to do.\n")
    };
    let seed = Some(r#"{"settings":{"welcomed":true}}"#);
    for shell in [Shell::Desktop, Shell::Tv] {
        let s = start_on("linux-x64-fedora", shell, seed, Some(Arc::new(machine())));
        s.app.check_system(&s.ui, false);
        assert_eq!(s.store().get_system_headline(), "Restart to finish updating");
        s.store().set_page(4);
        assert!(s.has("Restart now") && !s.has("Update now"), "{shell:?}");
        // It asks before it does it; "later" changes nothing.
        match shell {
            Shell::Tv => s.press(Key::Return),
            _ => s.click("Restart now"),
        }
        assert_eq!(s.store().get_gate(), 1, "{shell:?}");
        assert_eq!(s.store().get_gate_title(), "Restart to finish updating");
        assert_eq!(
            (s.store().get_gate_yes().as_str(), s.store().get_gate_no().as_str()),
            ("Restart now", "Restart later")
        );
        s.store().invoke_gate_answer(false);
        assert_eq!(s.store().get_gate(), 0);
        assert!(s.has("Restart now"), "still offered");
    }
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
