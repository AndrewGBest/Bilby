//! The desktop and phone layouts: keys, fitting the screen, names for screen readers.

use super::*;

#[test]
fn escape_leaves_the_app_page_on_desktop() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.click("Tidewave");
    assert!(s.store().get_details_open());
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
}

#[test]
fn desktop_works_without_a_mouse() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // Tab walks the sidebar: Discover, Search, Library, Settings.
    s.press(Key::Tab);
    s.press(Key::Tab);
    s.press(Key::Tab);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 2, "third Tab stop is Library");
    s.press(Key::Tab);
    s.press(" ");
    assert_eq!(s.store().get_page(), 3, "Space activates like Enter");

    // Keep tabbing from the sidebar into the page until an app opens with Enter.
    let s = start("linux-x64-debian", Shell::Desktop);
    let mut opened = false;
    for _ in 0..12 {
        s.press(Key::Tab);
        if s.labelled("View Tidewave").is_empty() {
            break;
        }
    }
    for _ in 0..12 {
        s.press(Key::Return);
        if s.store().get_details_open() {
            opened = true;
            break;
        }
        s.press(Key::Tab);
    }
    assert!(opened, "an app can be opened with Tab and Enter alone");
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
}

#[test]
fn mobile_has_bottom_navigation_and_back() {
    let s = start("android-arm64", Shell::Mobile);
    assert_eq!(s.ui.get_effective(), Shell::Mobile);
    assert!(s.has("Fern Launcher"), "Android-only apps are listed on a phone");
    assert!(!s.has("Lumen"), "desktop-only apps are not");
    s.click("Tidewave");
    assert!(s.store().get_details_open());
    assert_eq!(s.option_labels()[0], "APK · arm64");
    assert!(!s.option_labels().iter().any(|l| l.contains("TV")), "TV builds are hidden on a phone");
    s.click("Back");
    assert!(!s.store().get_details_open());
    // Android's Back button does the same.
    s.click("Tidewave");
    s.press(Key::Back);
    assert!(!s.store().get_details_open());
    s.click("Library");
    assert_eq!(s.store().get_page(), 2);
    s.click("Settings");
    assert_eq!(s.store().get_page(), 3);
    s.click("Discover");
    assert_eq!(s.store().get_page(), 0);
}

#[test]
fn everything_fits_on_the_narrowest_phone() {
    let seed = r#"{"settings":{"welcomed":true},"recent_searches":["editor","backup"],"library":[
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.3.0","label":"APK · arm64","installed_at":1794000000},
        {"id":"github:relay-app/relay","name":"Relay","version":"v1.15.0","label":"APK · arm64","installed_at":1790000000}]}"#;
    let s = start_with("android-arm64", Shell::Mobile, Some(seed));
    s.ui.window().set_size(slint::LogicalSize::new(320.0, 640.0));
    s.app.check_updates(&s.ui, true);
    assert_eq!(s.store().get_updates_count(), 2, "so that Update all is on the page being measured");
    assert_fits(&s, "discover");
    for (page, name) in [(1, "search"), (2, "library"), (3, "settings")] {
        s.store().set_page(page);
        assert_fits(&s, name);
    }
    s.app.search(&s.ui, "play");
    assert_fits(&s, "search results");
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert_fits(&s, "app page");
    s.app.open(&s.ui, "github:nobody/nothing");
    assert_fits(&s, "app page with an error");

    // And the first-run welcome.
    let fresh = start_with("android-arm64", Shell::Mobile, None);
    fresh.ui.window().set_size(slint::LogicalSize::new(320.0, 640.0));
    assert_fits(&fresh, "welcome");
}

#[test]
fn every_button_has_a_name_a_screen_reader_can_say() {
    use slint::private_unstable_api::re_exports::AccessibleRole;
    let seed = r#"{"recent_searches":["editor"],"library":[{"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    for (preset, shell) in [("linux-x64-arch", Shell::Desktop), ("android-arm64", Shell::Mobile)] {
        let s = start_with(preset, shell, Some(seed));
        s.app.check_updates(&s.ui, false);
        let check = |what: &str| {
            for role in [AccessibleRole::Button, AccessibleRole::Tab, AccessibleRole::Switch] {
                for element in s.ui.root_element().query_descendants().match_accessible_role(role).find_all() {
                    let label = element.accessible_label().unwrap_or_default();
                    assert!(!label.trim().is_empty(), "{preset} {what}: a control has no accessible label");
                }
            }
        };
        check("welcome and discover");
        for page in 1..=3 {
            s.store().set_page(page);
            check(&format!("page {page}"));
        }
        s.app.search(&s.ui, "lumen");
        check("search results");
        s.app.open(&s.ui, "github:tidewave/tidewave");
        check("app page");
        s.app.open(&s.ui, "github:nobody/nothing");
        check("app page with an error");
    }
}

#[test]
fn window_width_picks_desktop_or_mobile() {
    init_backend();
    let root = temp_root();
    let options = Options {
        demo: true,
        inline: true,
        platform: Some(Platform::preset("linux-x64-debian").unwrap()),
        dirs: Some(dirs_in(&root)),
        ..Default::default()
    };
    let (ui, _app) = build(&options).unwrap();
    ui.show().unwrap();
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    assert_eq!(ui.get_effective(), Shell::Desktop);
    ui.window().set_size(slint::LogicalSize::new(420.0, 800.0));
    assert_eq!(ui.get_effective(), Shell::Mobile);
    ui.window().set_size(slint::LogicalSize::new(900.0, 800.0));
    assert_eq!(ui.get_effective(), Shell::Desktop);
}
