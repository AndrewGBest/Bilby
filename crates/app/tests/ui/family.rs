//! Adult content, Family mode and the PIN.

use super::*;

#[test]
fn adult_things_have_a_covered_section_and_ask_before_opening() {
    let s = start("linux-tv", Shell::Tv);
    let site = adult_site();
    let id = format!("web:{}", site.host);
    s.app.search(&s.ui, &site.name);

    // In the results it is last, covered: nothing of it shows but that it is there.
    let card = s.store().get_results().iter().find(|c| c.id == id.as_str()).expect("it is found, not hidden");
    assert!(card.adult);
    assert_eq!((card.name.as_str(), card.summary.as_str(), card.stat.as_str()), ("Adult", "Select to view", ""));
    assert!(!card.summary.contains(site.name.as_str()) && !card.author.contains(site.host.as_str()));
    let last = s.store().get_results().iter().last().unwrap();
    assert!(last.adult, "after everything else");
    // On the TV it has a row of its own, named plainly, and is in no other row.
    let rows: Vec<(String, Vec<String>)> = s
        .store()
        .get_tv_rows()
        .iter()
        .map(|row| (row.title.to_string(), row.apps.iter().map(|c| c.id.to_string()).collect()))
        .collect();
    assert_eq!(rows.last().unwrap().0, "Adult");
    assert!(rows.iter().filter(|(title, _)| title != "Adult").all(|(_, ids)| !ids.contains(&id)), "{rows:?}");
    // Nothing for adults is on the home page unless asked for.
    assert!(!s.store().get_adult_on_home());

    // Choosing it asks first. No is the highlighted answer; it changes nothing.
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 1);
    assert!(s.store().get_gate_text().contains("18 or over") && !s.store().get_details_open());
    s.press(Key::Return);
    assert_eq!(s.store().get_gate(), 0);
    assert!(!s.store().get_details_open() && !s.saved().settings.adult_confirmed);

    // Yes opens it, as itself, and the question is not asked again.
    s.app.open(&s.ui, &id);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open() && s.saved().settings.adult_confirmed);
    assert_eq!(s.store().get_app().name, site.name.as_str());
    s.store().set_details_open(false);
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 0);
    assert!(s.store().get_details_open());
    // The tile in the list stays covered all the same.
    assert!(s.store().get_results().iter().any(|c| c.id == id.as_str() && c.name == "Adult"));

    // For those who want no cover at all: shown as itself, among everything else.
    s.store().set_details_open(false);
    s.store().invoke_set_adult_uncovered(true);
    assert!(s.saved().settings.adult_uncovered && s.store().get_gate() == 0, "already confirmed an adult");
    s.app.search(&s.ui, &site.name);
    let card = s.store().get_results().iter().find(|c| c.id == id.as_str()).unwrap();
    assert_eq!(card.name, site.name.as_str());
    assert!(!card.adult && !s.store().get_tv_rows().iter().any(|row| row.title == "Adult"));
    s.app.open(&s.ui, &id);
    assert!(s.store().get_details_open() && s.store().get_gate() == 0);
    // Family mode still overrides it.
    s.store().set_details_open(false);
    s.store().invoke_set_family_mode(true);
    s.app.search(&s.ui, &site.name);
    assert!(!s.store().get_results().iter().any(|c| c.id == id.as_str()));
}

#[test]
fn a_site_nobody_listed_by_hand_is_still_recognised_as_adult() {
    let s = start("linux-tv", Shell::Tv);
    // A language edition of a listed site: on no hand-written list, typed as an address.
    let typed = format!("fr.{}", adult_site().host);
    s.app.search(&s.ui, &typed);
    let card =
        s.store().get_results().iter().find(|c| c.id == format!("web:{typed}").as_str()).expect("offered as typed");
    assert!(card.adult && card.name == "Adult");
    s.store().invoke_set_family_mode(true);
    s.app.search(&s.ui, &typed);
    assert!(!s.store().get_results().iter().any(|c| c.id.contains(typed.as_str())), "and gone in Family mode");
}

#[test]
fn family_mode_takes_adult_things_out_altogether() {
    let s = start("linux-tv", Shell::Tv);
    let site = adult_site();
    let id = format!("web:{}", site.host);
    s.store().invoke_set_family_mode(true);
    assert!(s.store().get_family_mode() && s.saved().settings.family_mode, "anyone may switch it on");

    for query in [site.name.as_str(), site.host.as_str()] {
        s.app.search(&s.ui, query);
        assert!(!s.store().get_results().iter().any(|c| c.adult || c.id == id.as_str()), "{query}: not even covered");
        assert!(!s.store().get_tv_rows().iter().any(|row| row.title == "Adult"));
        assert!(!s.store().get_search_note().to_lowercase().contains("adult"), "and no word of it");
    }
    // By its link: refused, with no question to say yes to.
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 0);
    assert!(!s.store().get_details_open());
    assert!(s.store().get_toast().contains("Family mode"), "{}", s.store().get_toast());
    // Adult things on the home page cannot be asked for while it is on.
    assert!(!s.store().get_adult_on_home());
}

#[test]
fn a_pin_guards_the_settings_adult_content_and_installs() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(s.has("Set a PIN") && !s.store().get_has_pin());

    // Choosing a PIN: four digits, twice. Two that differ start over.
    let enter = |pin: &str| s.store().invoke_gate_pin(pin.into());
    s.click("Set a PIN");
    assert_eq!(s.store().get_gate(), 3);
    enter("2580");
    assert_eq!(s.store().get_gate(), 4);
    enter("2581");
    assert_eq!(s.store().get_gate(), 3, "not the same: choose again");
    enter("2580");
    enter("2580");
    assert_eq!(s.store().get_gate(), 0);
    assert!(s.store().get_has_pin());
    let saved = s.saved().settings;
    assert_eq!(saved.pin_hash.len(), 64);
    assert!(!std::fs::read_to_string(&s.state_file).unwrap().contains("2580"), "the PIN itself is not kept");

    // Family mode goes on without the PIN, and comes off only with it.
    s.store().invoke_set_family_mode(true);
    assert!(s.saved().settings.family_mode && s.store().get_gate() == 0);
    s.store().invoke_set_family_mode(false);
    assert_eq!(s.store().get_gate(), 2);
    assert!(s.saved().settings.family_mode, "still on until the PIN is given");
    enter("0000");
    assert!(s.store().get_gate() == 2 && s.store().get_gate_error().contains("not the PIN"));
    assert!(s.saved().settings.family_mode);
    s.store().invoke_gate_cancel();
    assert!(s.saved().settings.family_mode, "stepping back changes nothing");
    s.store().invoke_set_family_mode(false);
    enter("2580");
    assert!(!s.saved().settings.family_mode && s.store().get_gate() == 0);

    // With a PIN, adult content asks for it rather than whether you are an adult.
    let id = format!("web:{}", adult_site().host);
    s.app.open(&s.ui, &id);
    assert_eq!(s.store().get_gate(), 2);
    s.store().invoke_gate_cancel();
    assert!(!s.store().get_details_open());

    // Installs and removals ask for it too, once that is switched on.
    s.store().invoke_set_pin_for_installs(true);
    assert!(s.saved().settings.pin_for_installs, "stricter: no PIN needed to switch it on");
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    s.store().invoke_install(0);
    assert_eq!(s.store().get_gate(), 2);
    assert_eq!(s.store().get_install_state(), 0, "nothing starts until the PIN is given");
    enter("2580");
    assert_ne!(s.store().get_install_state(), 0, "then it goes ahead");
    assert!(s.saved().library.iter().any(|a| a.id == "github:lumen-labs/lumen"));

    // Removing the PIN needs the PIN, and takes the install lock with it.
    s.store().invoke_remove_pin();
    assert_eq!(s.store().get_gate(), 2);
    enter("2580");
    let saved = s.saved().settings;
    assert!(saved.pin_hash.is_empty() && !saved.pin_for_installs && !s.store().get_has_pin());
}

#[test]
fn family_mode_also_takes_out_gambling_and_piracy_sites() {
    let s = start("linux-tv", Shell::Tv);
    let betting =
        ["bet365.com", "pokerstars.com"].into_iter().find(|h| bilby_core::source::web::restricted_host(h)).unwrap();
    let id = format!("web:{betting}");
    // Ordinarily it is found like any other site, uncovered: it is not adult content.
    s.app.search(&s.ui, betting);
    let card = s.store().get_results().iter().find(|c| c.id == id.as_str()).expect("found when searched for");
    assert!(!card.adult);
    // It is never on the home page.
    s.app.search(&s.ui, "");
    assert!(!s.store().get_sections().iter().any(|row| row.apps.iter().any(|c| c.id == id.as_str())));
    // In Family mode it is gone, and will not open.
    s.store().invoke_set_family_mode(true);
    s.app.search(&s.ui, betting);
    assert!(!s.store().get_results().iter().any(|c| c.id == id.as_str()));
    s.app.open(&s.ui, &id);
    assert!(
        s.store().get_details_error().contains("Family mode") || !s.store().get_details_open(),
        "{}",
        s.store().get_details_error()
    );
    assert_eq!(s.store().get_options().row_count(), 0);
}
