//! Apps on the device that the store did not install: their updates.

use super::*;
use bilby_core::os::FakeHost;

/// The made-up machine of `preset`, kept so the test can read what was run on it.
fn machine(preset: &str, change: impl FnOnce(FakeHost) -> FakeHost) -> Arc<FakeHost> {
    Arc::new(change(bilby_core::os::demo_host(&Platform::preset(preset).unwrap())))
}

fn start_there(preset: &str, shell: Shell, host: &Arc<FakeHost>) -> Session {
    let s = start_on(preset, shell, Some(r#"{"settings":{"welcomed":true}}"#), Some(host.clone()));
    looked(&s);
    s
}

/// What is on the device and what is waiting for it have both been looked up. (The
/// real app redraws its lists a moment after the look for updates, on a timer that
/// does not run here.)
fn looked(s: &Session) {
    s.app.scan_installed(&s.ui);
    s.app.check_system(&s.ui, false);
    s.app.refresh(&s.ui);
}

fn also_installed(s: &Session) -> Vec<(String, bool, String)> {
    s.store().get_also_installed().iter().map(|c| (c.id.to_string(), c.has_update, c.summary.to_string())).collect()
}

#[test]
fn an_app_the_system_installed_is_updated_by_itself_from_the_library() {
    // apt has a newer harbor waiting, among the system's other updates.
    let host = machine("linux-x64-debian", |host| host);
    let s = start_there("linux-x64-debian", Shell::Desktop, &host);
    let listed = also_installed(&s);
    assert_eq!(
        listed[0],
        ("apt:harbor".to_string(), true, "5.0.3+ds1-2ubuntu1 → 5.3.1+ds1-1".to_string()),
        "first in the list, with what changes: {listed:?}"
    );

    // Its row has the button; apt is asked for that one package, behind the password.
    s.store().set_page(2);
    s.activate("Update harbor now");
    assert_eq!(
        *host.streamed.lock().unwrap(),
        ["pkexec env DEBIAN_FRONTEND=noninteractive apt-get -y -o Dpkg::Options::=--force-confold install --only-upgrade harbor"]
    );
    assert_eq!(s.store().get_toast(), "harbor updated");
    assert!(!s.store().get_details_open(), "the Library stays where it was");
    assert!(also_installed(&s).iter().all(|(_, waiting, _)| !waiting), "and nothing is waiting for it now");
    assert!(!s.has("Update harbor now"));
    assert!(s.store().get_system_log().contains("Updating through apt"));
}

#[test]
fn its_page_offers_the_update_and_says_when_it_did_not_go_in() {
    // The password prompt is closed: nothing changes, and that is said.
    let host = machine("linux-x64-debian", |host| FakeHost { install_code: 126, ..host });
    let s = start_there("linux-x64-debian", Shell::Desktop, &host);
    s.app.open(&s.ui, "apt:harbor");
    assert!(s.store().get_outside_update());
    assert_eq!(s.store().get_outside_change(), "5.0.3+ds1-2ubuntu1 → 5.3.1+ds1-1");
    assert!(s.has("Update available  ·  5.0.3+ds1-2ubuntu1 → 5.3.1+ds1-1"));
    s.click("Update harbor");
    assert_eq!(host.streamed.lock().unwrap().len(), 1);
    let said = s.store().get_toast().to_string();
    assert!(said.starts_with("harbor was not updated.") && said.contains("password prompt was closed"), "{said}");
    assert!(s.store().get_toast_is_error());

    // An app that is not on the device, or has nothing waiting, has no such offer.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert!(!s.store().get_outside_update());
}

#[test]
fn where_packages_only_move_together_the_whole_system_is_offered() {
    // pacman: bringing one package forward alone can break it or what uses it.
    let host = machine("linux-x64-arch", |host| host);
    let s = start_there("linux-x64-arch", Shell::Desktop, &host);
    let harbor = also_installed(&s).into_iter().find(|(id, _, _)| id == "pacman:harbor").unwrap();
    assert_eq!((harbor.1, harbor.2.as_str()), (true, "5.2.0-1 → 5.3.1-1"));
    s.store().set_page(2);
    s.activate("Update harbor now");
    assert!(host.streamed.lock().unwrap().is_empty(), "nothing is run before the person agrees");
    assert_eq!(s.store().get_gate_title(), "Update the whole system?");
    assert!(s.store().get_gate_text().contains("harbor is updated together with everything else pacman looks after"));

    // "Later" changes nothing; "Update everything" is the system's own update.
    s.store().invoke_gate_answer(false);
    assert!(host.streamed.lock().unwrap().is_empty());
    s.activate("Update harbor now");
    s.store().invoke_gate_answer(true);
    assert_eq!(host.streamed.lock().unwrap()[0], "pkexec pacman -Syu --noconfirm");
    assert!(also_installed(&s).iter().all(|(_, waiting, _)| !waiting));
}

#[test]
fn a_flatpak_from_somewhere_else_is_updated_from_its_page_on_a_tv() {
    let listing = "Looking for updates…\n\n 1.\t   \tio.quillpad.Quillpad\tstable\tu\tflathub\t< 2.1 MB\n\nProceed with these changes to the user installation? [Y/n]: n\n";
    let host = machine("linux-tv", |host| host.program("flatpak").command("flatpak update --user", 1, listing));
    let s = start_on("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true}}"#), Some(host.clone()));
    let flatpak = s.dirs.flatpak_roots[0].join("app/io.quillpad.Quillpad/current/active/export/share/applications");
    std::fs::create_dir_all(&flatpak).unwrap();
    std::fs::write(flatpak.join("io.quillpad.Quillpad.desktop"), "[Desktop Entry]\nName=Quillpad\n").unwrap();
    looked(&s);

    // The Library's row of updates has it, whoever installed it.
    let rows: Vec<(String, Vec<String>)> = s
        .store()
        .get_tv_library_rows()
        .iter()
        .map(|row| (row.title.to_string(), row.apps.iter().map(|c| c.name.to_string()).collect()))
        .collect();
    let waiting = rows.iter().find(|(title, _)| title == "Updates available").expect("a row of updates");
    assert!(waiting.1.contains(&"Quillpad".to_string()), "{rows:?}");
    let also = rows.iter().find(|(title, _)| title == "Also installed here").map(|(_, apps)| apps.clone());
    assert!(!also.unwrap_or_default().contains(&"Quillpad".to_string()), "and is listed once");

    // On its page the first button is Update, and Flatpak is asked for that app alone.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert!(s.store().get_outside_update());
    assert!(s.has("Update"), "the main button");
    s.press(Key::Return);
    assert_eq!(*host.streamed.lock().unwrap(), ["flatpak update -y --noninteractive io.quillpad.Quillpad"]);
    assert_eq!(s.store().get_toast(), "Quillpad updated");
    assert!(!s.store().get_outside_update(), "the page no longer offers it");
}

#[test]
fn an_app_in_the_store_s_own_library_is_not_updated_this_way() {
    // The store installed harbor itself: its own look for updates is what counts.
    let host = machine("linux-x64-debian", |host| host);
    let seed = r#"{"settings":{"welcomed":true},"library":[{"id":"apt:harbor","name":"harbor","version":"5.0.3+ds1-2ubuntu1"}]}"#;
    let s = start_on("linux-x64-debian", Shell::Desktop, Some(seed), Some(host.clone()));
    looked(&s);
    assert!(also_installed(&s).iter().all(|(id, _, _)| id != "apt:harbor"));
    s.app.open(&s.ui, "apt:harbor");
    assert!(!s.store().get_outside_update());
}
