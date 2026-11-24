//! The home page.

use super::*;

#[test]
fn first_run_explains_the_store_once() {
    let s = start_with("linux-x64-debian", Shell::Desktop, None);
    assert!(s.store().get_show_welcome());
    assert!(s.has("Welcome to Bilby"));
    assert!(!s.has("View Tidewave"), "the welcome takes the spotlight's place");
    s.click("Got it");
    assert!(!s.store().get_show_welcome());
    assert!(!s.has("Welcome to Bilby"));
    assert!(s.has("View Tidewave"));
    assert!(s.saved().settings.welcomed);

    let saved = std::fs::read_to_string(&s.state_file).unwrap();
    let again = start_with("linux-x64-debian", Shell::Desktop, Some(&saved));
    assert!(!again.has("Welcome to Bilby"), "it does not come back");

    let phone = start_with("android-arm64", Shell::Mobile, None);
    assert!(phone.has("Welcome to Bilby"));
    phone.click("Got it");
    assert!(!phone.has("Welcome to Bilby"));
}

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

#[test]
fn a_category_can_be_chosen_and_browsed_in_full() {
    // On a TV the categories have a page of their own; choosing one gives it the page.
    let s = start("linux-tv", Shell::Tv);
    let rows = |s: &Session| -> Vec<(String, Vec<String>)> {
        let rows = s.store().get_tv_rows();
        rows.iter().map(|row| (row.title.to_string(), row.apps.iter().map(|c| c.id.to_string()).collect())).collect()
    };
    let home = rows(&s);
    assert!(home.iter().all(|(title, _)| title != "Categories"), "the categories are not a row of the home page");
    let kinds: Vec<String> = s.store().get_tv_categories().iter().map(|kind| kind.id.to_string()).collect();
    assert!(kinds.len() > 1 && kinds.iter().all(|id| id.starts_with("category:")), "{kinds:?}");
    // They are kinds of app, not the home page's recommendations or where apps come from.
    let names: Vec<String> = kinds.iter().map(|id| id.trim_start_matches("category:").to_string()).collect();
    for kind in ["Video", "Music", "Games"] {
        assert!(names.iter().any(|name| name == kind), "{kind} in {names:?}");
    }
    for not_a_kind in ["Essentials", "Made for TV", "Web apps", "Developer tools"] {
        assert!(names.iter().all(|name| name != not_a_kind), "{not_a_kind} in {names:?}");
    }
    // The made-up Flathub files one app, under utilities.
    let at = names.iter().position(|name| name == "Utilities").expect("utilities");
    let title = names[at].clone();
    s.store().invoke_open(kinds[at].clone().into());
    assert_eq!(s.store().get_page(), 1);
    assert!(!s.store().get_details_open() && !s.store().get_searching());
    let browsed = rows(&s);
    assert_eq!(browsed[0].0, title, "the category's name heads the page");
    assert!(browsed.iter().all(|(_, apps)| !apps.is_empty()));
    // Back is Home again.
    s.store().invoke_leave_search();
    assert_eq!(s.store().get_page(), 0);
    assert_eq!(rows(&s), home);

    // With the remote: Up to the search bar, Up to the top bar, along to Categories.
    let first = s.store().get_tv_categories().row_data(0).unwrap();
    // A category's tile is a button with its name; a row's heading is only text.
    let tiles = |s: &Session| {
        let button = Some(slint::private_unstable_api::re_exports::AccessibleRole::Button);
        s.labelled(&first.name).iter().filter(|e| e.accessible_role() == button).count()
    };
    assert_eq!(tiles(&s), 0);
    for key in [Key::UpArrow, Key::UpArrow, Key::RightArrow, Key::RightArrow, Key::Return] {
        s.press(key);
    }
    assert_eq!(tiles(&s), 1, "the page of categories is up");
    assert_eq!(s.store().get_page(), 0);
    // Four to a row: Down and Right to the one wanted, and OK opens it in full.
    for _ in 0..at / 4 {
        s.press(Key::DownArrow);
    }
    for _ in 0..at % 4 {
        s.press(Key::RightArrow);
    }
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 1);
    assert_eq!(rows(&s)[0].0, title);
    // Back returns to the categories, with the same one still pointed at.
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 0);
    assert_eq!(tiles(&s), 1);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 1);
    assert_eq!(rows(&s)[0].0, title);
    // Back twice is Home, where OK opens an app.
    s.press(Key::Escape);
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 0);
    assert_eq!(tiles(&s), 0, "the page of categories is put away");
    s.press(Key::Return);
    assert!(s.store().get_details_open());

    // On a desktop the chips do it: the section then holds the category in full.
    let s = start("linux-x64-debian", Shell::Desktop);
    let name = s.store().get_categories().row_data(0).unwrap();
    s.store().invoke_browse_category(name.clone());
    let section = s.store().get_sections().iter().find(|section| section.title == name).unwrap();
    assert!(section.apps.row_count() > 0);
    s.store().invoke_browse_category("".into());
    assert!(s.store().get_sections().row_count() > 1);
}

#[test]
fn a_damaged_state_file_does_not_stop_the_app() {
    let s = start_with("linux-x64-debian", Shell::Desktop, Some("{{{ not json"));
    assert_eq!(s.store().get_library().row_count(), 0);
    assert!(s.has("Tidewave"), "the store still starts and shows apps");
    let set_aside = s.state_file.with_file_name("demo-state.json.corrupt");
    assert_eq!(std::fs::read_to_string(set_aside).unwrap(), "{{{ not json");
}

#[test]
fn the_home_page_adds_the_lists_a_source_keeps() {
    // On Linux with Flatpak, what is popular on Flathub follows the bundled apps.
    let s = start("linux-x64-debian", Shell::Desktop);
    let titles: Vec<String> = s.store().get_sections().iter().map(|section| section.title.to_string()).collect();
    assert!(titles.iter().any(|t| t == "Popular on Flathub"), "{titles:?}");
    assert!(titles.iter().position(|t| t == "Popular on Flathub") > titles.iter().position(|t| t == "Developer tools"));
    let popular = s.store().get_sections().iter().find(|section| section.title == "Popular on Flathub").unwrap();
    assert_eq!(popular.apps.row_data(0).unwrap().id, "flathub:org.reelstudio.Reel", "most installed first");

    // What Flathub files under a kind joins the row of that kind, among the rows of
    // kinds and ahead of the web apps, and is not then repeated in the lists after it.
    let tools = s.store().get_sections().iter().find(|section| section.title == "Handy tools").expect("{titles:?}");
    let tool = tools.apps.row_data(tools.apps.row_count() - 1).unwrap().id;
    assert!(tool.starts_with("flathub:"), "{tool}");
    assert!(titles.iter().position(|t| t == "Handy tools") < titles.iter().position(|t| t == "Web apps"));
    let listed =
        s.store().get_sections().iter().filter(|section| section.apps.iter().any(|app| app.id == tool)).count();
    assert_eq!(listed, 1, "in one row only");

    // Where Flathub is not a source, there are no such lists and nothing is asked for.
    let windows = start("windows-x64", Shell::Desktop);
    assert!(windows.store().get_sections().iter().all(|section| !section.title.contains("Flathub")));
}
