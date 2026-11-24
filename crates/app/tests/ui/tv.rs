//! The TV layout, driven by a remote.

use super::*;

#[test]
fn moving_to_another_row_after_scrolling_along_one_starts_at_its_beginning() {
    let s = start("linux-tv", Shell::Tv);
    let rows = s.store().get_tv_rows();
    let long = (0..rows.row_count() - 1)
        .find(|r| rows.row_data(*r).unwrap().apps.row_count() > 5 && rows.row_data(r + 1).unwrap().apps.row_count() > 1)
        .unwrap_or_else(|| {
            let sizes: Vec<usize> = rows.iter().map(|row| row.apps.row_count()).collect();
            panic!("a long row with another under it: {sizes:?}")
        });
    for _ in 0..long {
        s.press(Key::DownArrow);
    }
    let first_below = rows.row_data(long + 1).unwrap().apps.row_data(0).unwrap().id;
    let second_below = rows.row_data(long + 1).unwrap().apps.row_data(1).unwrap().id;
    // One step along, nothing has scrolled: down is the app straight below.
    s.press(Key::RightArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_app().id, second_below);
    s.press(Key::Escape);
    s.press(Key::UpArrow);
    // Far along the row, it has scrolled: the next row is entered at its first app.
    for _ in 0..4 {
        s.press(Key::RightArrow);
    }
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_app().id, first_below);
}

#[test]
fn tv_is_driven_entirely_by_the_remote() {
    let s = start("android-tv", Shell::Tv);
    assert_eq!(s.ui.get_effective(), Shell::Tv);
    let rows = s.store().get_tv_rows();
    assert_eq!(rows.row_data(0).unwrap().title, "Made for TV");
    let second = rows.row_data(0).unwrap().apps.row_data(1).unwrap();

    // Right, OK: open the second app in the first row.
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_app().id, second.id);

    // OK installs the focused (best) option; APK hand-off is not implemented, and the
    // interface says so instead of pretending.
    s.press(Key::Return);
    assert_eq!(s.store().get_install_state(), 3);
    assert!(s.store().get_install_message().contains("not implemented"));

    // Back returns to the grid; Down and OK opens the first app of the next row.
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
    s.press(Key::LeftArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    let first_of_next = rows.row_data(1).unwrap().apps.row_data(0).unwrap();
    assert_eq!(s.store().get_app().id, first_of_next.id);
}

#[test]
fn the_top_bar_is_reachable_from_an_app_page_on_a_tv() {
    let s = start("linux-tv", Shell::Tv);
    let first = s.store().get_tv_rows().row_data(0).unwrap().apps.row_data(0).unwrap();
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    // Up from the page's buttons is the top bar; Down is the page again, still open,
    // and OK there is the page's own button, not the bar's.
    s.press(Key::UpArrow);
    s.press(Key::DownArrow);
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_page(), 0);
    // Up, along to Library, OK: the app's page is put away and the library is shown.
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(!s.store().get_details_open());
    assert_eq!(s.store().get_page(), 2);
    // And Home from an app's page, the same way.
    s.store().invoke_open(first.id.clone());
    assert!(s.store().get_details_open());
    s.press(Key::UpArrow);
    s.press(Key::LeftArrow);
    s.press(Key::Return);
    assert!(!s.store().get_details_open());
    assert_eq!(s.store().get_page(), 0);
    // Back in the bar, on an app's page, is the page again; Back again closes it.
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    s.press(Key::UpArrow);
    s.press(Key::Escape);
    assert!(s.store().get_details_open());
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
}

#[test]
fn tv_focus_survives_rows_changing_under_it() {
    let s = start("android-tv", Shell::Tv);
    // Move to the last row and its last tile, then make the grid shorter by searching
    // for something with a single result row in front.
    for _ in 0..6 {
        s.press(Key::DownArrow);
    }
    for _ in 0..6 {
        s.press(Key::RightArrow);
    }
    s.app.search(&s.ui, "play");
    // Whatever is focused now must be a real app: OK opens it.
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    let landed = s.store().get_app().id;
    assert!(landed.starts_with("github:") || landed.starts_with("web:"), "focus landed on {landed:?}");
    assert!(s.store().get_details_error().is_empty());
}

#[test]
fn the_tv_has_a_library_page_and_a_home_that_is_the_store() {
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:marquee-tv/marquee","name":"Marquee","version":"v20.0"},
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.4.0"}]}"#;
    let s = start_with("android-tv", Shell::Tv, Some(seed));
    // Home is the store: what is installed is not what it leads with.
    let home: Vec<String> = s.store().get_tv_rows().iter().map(|row| row.title.to_string()).collect();
    assert_eq!(home[0], "Made for TV", "{home:?}");
    assert!(!home.iter().any(|t| t == "Developer tools"), "nobody writes code from the sofa: {home:?}");

    s.app.check_updates(&s.ui, false);
    assert_eq!(s.store().get_toast(), "", "a start-up check stays quiet");
    let library: Vec<String> = s.store().get_tv_library_rows().iter().map(|row| row.title.to_string()).collect();
    assert_eq!(
        library,
        ["This TV", "Updates available", "Installed with Bilby"],
        "the device and the store itself come first"
    );
    assert!(s.has("Library  ·  1 update"), "the top bar says an update is waiting");

    // With the remote: Up to search, Up to the tabs, Right to Library, OK.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 2);
    assert!(s.has("Check for updates"));
    // The library's buttons have the focus; Down goes to the device's own row, Down
    // again to the apps, OK opens the first.
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_app().id, "github:marquee-tv/marquee", "the app with an update comes first");

    // Back closes the app, Back again goes up to the bar, and Back from there goes Home.
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());
    assert_eq!(s.store().get_page(), 2);
    s.press(Key::Escape);
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 0);
}

#[test]
fn the_tv_app_page_has_one_main_button_and_the_rest_a_step_away() {
    let s = start("android-tv", Shell::Tv);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    let id = s.store().get_app().id.to_string();
    assert_eq!(id, "github:marquee-tv/marquee");
    assert_eq!(s.store().get_screenshots().row_count(), 3, "the page has pictures to show");

    // Right moves to Favourite; OK stars the app without leaving the page.
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.store().get_is_favourite());
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_install_state(), 0, "nothing was installed by starring");

    // Right again is Versions; OK opens the list, Down picks the second file, OK takes it.
    s.press(Key::RightArrow);
    s.press(Key::Return);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_install_index(), 1, "the second file, not the recommended one");

    // Back closes the list first, and only then the page.
    s.press(Key::Escape);
    assert!(s.store().get_details_open());
    s.press(Key::Escape);
    assert!(!s.store().get_details_open());

    // Opening another app starts on its main button again: OK goes straight to installing.
    s.press(Key::LeftArrow);
    s.press(Key::Return);
    assert_ne!(s.store().get_app().id, id);
    s.press(Key::Return);
    assert_eq!(s.store().get_install_index(), 0);
}

#[test]
fn on_a_linux_tv_a_website_is_added_and_opened_like_an_app() {
    let s = start("linux-tv", Shell::Tv);
    let rows = s.store().get_tv_rows();
    let websites = rows.iter().find(|r| r.title == "Web apps").expect("a Web apps row on a Linux TV");
    assert!(websites.apps.iter().any(|a| a.id == "web:lichess.org"));

    // The site's picture, as the store keeps it once it has been fetched.
    std::fs::create_dir_all(s.dirs.cache.join("icons")).unwrap();
    std::fs::write(s.dirs.cache.join("icons").join("app-web-lichess-org.png"), b"\x89PNG stand-in").unwrap();
    // Without Longboard the page says the store will fetch it, and adding the site does:
    // nobody is sent to install it themselves.
    s.app.open(&s.ui, "web:lichess.org");
    assert_eq!(s.store().get_app().name, "Lichess");
    assert!(s.store().get_warning().contains("Bilby installs it along with this"), "{}", s.store().get_warning());
    s.press(Key::Return);
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert!(s.store().get_install_message().contains("Longboard was installed for it"));
    let longboard = s.dirs.applications.join("Longboard-x86_64.AppImage");
    assert!(longboard.is_file(), "Longboard was fetched and placed");
    let library: Vec<String> = s.saved().library.iter().map(|a| a.id.clone()).collect();
    assert_eq!(library, ["web:lichess.org", "github:AndrewGBest/Longboard"], "both are in the library");
    assert!(s.dirs.menu.join("bilby-github-andrewgbest-longboard.desktop").exists());

    // With Longboard there the page has nothing to warn about, and nothing more is fetched.
    s.app.remove_from_library(&s.ui, "web:lichess.org");
    s.app.open(&s.ui, "web:lichess.org");
    assert_eq!(s.store().get_warning(), "");
    s.press(Key::Return);
    assert_eq!(s.store().get_install_state(), 2, "{}", s.store().get_install_message());
    assert!(!s.store().get_install_message().contains("Longboard was installed"));
    assert_eq!(s.saved().library[0].id, "web:lichess.org");
    let entry = std::fs::read_to_string(s.dirs.menu.join("bilby-web-lichess-org.desktop")).unwrap();
    assert!(entry.contains("Name=Lichess\n"), "{entry}");
    assert!(entry.contains("Longboard-x86_64.AppImage\" \"--url=https://lichess.org/\"\n"), "{entry}");
    // It carries the site's own picture, so the TV's list does not show a blank tile
    // or Longboard's picture for every website.
    assert!(entry.contains("\nIcon=") && entry.contains("app-web-lichess-org"), "{entry}");

    // The same button now opens it: Longboard is started on the site's address.
    assert!(s.store().get_can_open());
    s.press(Key::Return);
    assert_eq!(
        s.web_opened.lock().unwrap().as_slice(),
        [(longboard.clone(), "--url=https://lichess.org/".to_string())]
    );

    // Removing it takes the entry out of the TV's app list.
    s.app.remove_from_library(&s.ui, "web:lichess.org");
    assert!(!s.dirs.menu.join("bilby-web-lichess-org.desktop").exists());
    assert_eq!(s.store().get_toast(), "Lichess removed from this TV's app list");
}

#[test]
fn on_a_tv_a_review_is_written_with_the_remote_and_the_on_screen_keyboard() {
    let s = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    s.app.load_ratings(&s.ui);
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_review_places().row_count(), 1);
    // Down to the foot of the page and one back up is the row of reviews; Left of
    // the first review is the button to write one.
    for _ in 0..5 {
        s.press(Key::DownArrow);
    }
    s.press(Key::UpArrow);
    assert!(s.has("Write a review"), "offered beside the reviews");
    let type_out = |words: &str| {
        for letter in words.chars() {
            s.press(SharedString::from(letter.to_string()));
        }
    };

    s.press(Key::LeftArrow);
    s.press(Key::Return);
    assert!(s.has("Title of your review") && s.has("Your review") && s.has("Name to show next to your review"));
    // Sending straight away says what is missing.
    s.activate("Send your review");
    assert_eq!(s.store().get_review_problem(), "Choose how many stars first.");

    // The stars: Right adds one. Then down the form, OK on a row to type into it.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    for _ in 0..4 {
        s.press(Key::RightArrow);
    }
    assert!(s.has("4 stars"));
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.has("On-screen keyboard") && s.has("What you have typed"));
    assert!(s.has("done"), "the last key puts the keyboard away; nothing is searched for");
    type_out("Does the job");
    s.press(Key::Return);
    assert!(!s.has("On-screen keyboard"));
    assert_eq!(s.store().get_review_title(), "Does the job");
    assert_eq!(s.store().get_query(), "", "the search was not typed into");

    s.press(Key::DownArrow);
    s.press(Key::Return);
    type_out("Starts fast and stays out of the wayy");
    s.press(Key::Backspace);
    s.press(Key::Return);
    assert_eq!(s.store().get_review_text(), "Starts fast and stays out of the way");

    s.press(Key::DownArrow);
    s.press(Key::Return);
    type_out("Ana");
    s.press(Key::Return);

    // Down to "Send review", and OK.
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_review_state(), 2, "{}", s.store().get_review_problem());
    assert!(s.store().get_review_result().starts_with("Sent, thank you."));
    assert_eq!(s.saved().reviewer_name, "Ana");
    // OK closes it, back on the app's page.
    s.press(Key::Return);
    assert!(!s.has("Title of your review") && s.store().get_details_open());

    // Back leaves a review half written without sending anything.
    s.app.open(&s.ui, "flathub:org.sundial.Sundial");
    s.press(Key::DownArrow);
    if s.has("Write a review") {
        s.activate("Write a review");
        assert!(s.has("Title of your review"));
        assert_eq!(s.store().get_review_title(), "", "the last app's words do not linger");
        s.press(Key::Escape);
        assert!(!s.has("Title of your review"), "Back closes the form");
        assert!(s.store().get_details_open(), "and the app's page stays");
    }
}

#[test]
fn a_tv_remote_reaches_the_system_page_and_updates_from_it() {
    let s = start("linux-tv", Shell::Tv);
    s.app.check_system(&s.ui, false);
    assert_eq!(s.store().get_system_state(), 3);
    // Up to search, Up to the tabs, Right along them (past Library, Categories and Settings) to the device's, OK.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 4);
    assert!(s.has("Update now") && s.has("Check again"));
    // The first button is the update.
    s.press(Key::Return);
    assert_eq!(s.store().get_system_headline(), "Up to date");
    assert!(!s.has("Update now"));
    // Back returns to the apps.
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 0);
}

#[test]
fn the_tv_layout_has_a_way_back_that_a_remote_can_reach() {
    let (ui, _app, state_file) =
        start_unforced("linux-x64-debian", r#"{"settings":{"welcomed":true,"interface":"tv"}}"#, false);
    let press = |key: slint::platform::Key| {
        let text: SharedString = key.into();
        ui.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        ui.window().dispatch_event(WindowEvent::KeyReleased { text });
    };
    // Up to search, Up to the tabs, Right along them to "This TV", OK; then right to the last button.
    press(Key::UpArrow);
    press(Key::UpArrow);
    press(Key::RightArrow);
    press(Key::RightArrow);
    press(Key::RightArrow);
    press(Key::RightArrow);
    press(Key::Return);
    assert_eq!(ui.global::<Store>().get_page(), 4);
    press(Key::RightArrow);
    press(Key::RightArrow);
    press(Key::Return);
    assert_eq!(ui.get_effective(), Shell::Desktop);
    assert_eq!(State::load(&state_file).settings.interface, "desktop");
}

#[test]
fn a_tv_remote_reaches_settings_and_can_remove_an_app() {
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.4.0"}]}"#;
    let s = start_with("linux-tv", Shell::Tv, Some(seed));

    // Settings have a tab of their own: Up past search to the tabs, Right to it, OK, and
    // the first setting is highlighted.
    assert!(s.has("Settings"));
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 3);
    assert!(s.store().get_show_ratings());
    s.press(Key::Return);
    assert!(!s.store().get_show_ratings(), "OK switches the focused setting");
    assert!(s.saved().settings.hide_ratings);
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.saved().settings.include_prereleases);
    // Up climbs back through the settings to the buttons, then to the top bar.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::LeftArrow);
    s.press(Key::LeftArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 2, "Library, from the top bar");

    // Past the device's own row to the installed app: its page offers Remove, which asks twice.
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open() && s.store().get_in_library());
    assert!(s.has("Install again"), "an installed app is not offered as if it were new");
    for _ in 0..3 {
        s.press(Key::RightArrow);
    }
    s.press(Key::Return);
    assert!(s.has("Confirm") && s.saved().library.len() == 1, "the first OK only asks");
    s.press(Key::Return);
    assert!(s.saved().library.is_empty());
    assert!(!s.store().get_in_library() && !s.has("Remove"));
}

#[test]
fn a_tv_remote_reads_reviews_and_picks_an_older_version() {
    // Without motion: the page is where it is going at once, as the test's clock stands still.
    let s = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    s.app.load_ratings(&s.ui);

    // An app with reviews: they are a row on its page, and OK on one reads it in full.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_reviews().row_count(), 3);
    // Down to the foot of the page and one back up is the row of reviews; OK reads the first.
    for _ in 0..5 {
        s.press(Key::DownArrow);
    }
    s.press(Key::UpArrow);
    assert!(s.has("Review: Exactly what I wanted"), "the reviews are on the page");
    s.press(Key::Return);
    assert!(s.has("Review 1: Exactly what I wanted"));
    s.press(Key::DownArrow);
    assert!(s.has("Review 2: Good, with one wish") && !s.has("Review 1: Exactly what I wanted"));
    s.press(Key::DownArrow);
    s.press(Key::DownArrow);
    assert!(s.has("Review 3: Lost my window layout"), "it stops at the last one");
    // Back closes the reviews, not the page.
    s.press(Key::Escape);
    assert!(s.store().get_details_open() && !s.has("Review 3: Lost my window layout"));

    // Versions: a list to pick from. Left leaves the list instead of changing version.
    s.app.open(&s.ui, "github:kestrel-sh/kestrel");
    s.press(Key::RightArrow);
    s.press(Key::LeftArrow);
    assert!(s.has("☆  Favourite"), "the favourites button says what it is");
    let versions = s.store().get_versions().row_count();
    assert!(versions > 1, "the demo app has older releases");
    // Main button, Favourite, Versions: OK opens the list, on its "Change version" row.
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.has("Change version"));
    assert_eq!(s.store().get_selected_version(), 0);
    let newest = s.store().get_app().version;
    // OK opens the releases; Down and OK takes the next one, and the files follow it.
    s.press(Key::Return);
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_selected_version(), 1, "an older version was picked from the list");
    assert_ne!(s.store().get_app().version, newest);
    assert!(s.has("Change version"), "and the list of releases closed again");
    // Left and Right do not change the version; Left leaves the list for the buttons.
    s.press(Key::RightArrow);
    assert_eq!(s.store().get_selected_version(), 1);
    s.press(Key::LeftArrow);
    assert_eq!(s.store().get_selected_version(), 1);
    assert!(!s.has("Change version") && s.store().get_details_open());
}

#[test]
fn a_tv_remote_can_switch_version_from_the_versions_list() {
    let s = start("linux-tv", Shell::Tv);
    s.app.open(&s.ui, "web:youtube.com");
    let choices: Vec<bilby::TvChoice> = s.store().get_tv_choices().iter().collect();
    // Each row is an app's own name with a few words on what it is; the group says
    // what the rows are, once.
    assert!(choices.len() >= 2, "{}", choices.len());
    assert_eq!(choices[0].heading, "Other ways to use YouTube");
    assert!(choices[1..].iter().all(|c| c.heading.is_empty()));
    assert!(choices.iter().all(|c| !c.name.contains("instead") && c.note.ends_with("unofficial")));
    let first = choices[0].clone();
    // Main button, star, then Versions: OK opens the list with the choices on top.
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert!(s.has(first.name.as_str()) && s.has("Other ways to use YouTube"));
    s.press(Key::Return);
    assert_eq!(s.store().get_app().id, first.id, "OK on a version opens it");
    assert!(s.store().get_tv_choices().iter().any(|c| c.id == "web:youtube.com"), "and the way back is offered");
}

#[test]
fn on_a_tv_steam_sign_in_happens_in_longboard_which_hands_the_list_back() {
    let s = start("linux-tv", Shell::Tv);
    // Without Longboard the store fetches it first, then signs in through it.
    let longboard = s.dirs.applications.join("Longboard-x86_64.AppImage");
    assert!(!longboard.exists());
    s.store().invoke_steam_signin();
    assert!(longboard.is_file(), "{}", s.store().get_toast());
    assert_eq!(s.saved().library[0].id, "github:AndrewGBest/Longboard");
    let (program, args) = s.web_opened.lock().unwrap().last().cloned().expect("Longboard was started");
    assert_eq!(program, longboard);
    assert!(args.starts_with("--hand-back=steam-library --hand-back-to="), "{args}");
    assert!(args.ends_with("--url=https://store.steampowered.com/login/?redir=&redir_ssl=1"), "{args}");
    assert!(s.store().get_steam_signed_in());
    assert_eq!(s.saved().steam.unwrap().owned, [440, 570]);
    assert!(!s.dirs.cache.join("hand-back").join("steam.json").exists(), "the answer is removed once read");
}

#[test]
fn a_tv_app_page_is_sections_to_move_down_through_and_open() {
    // Without motion, so the page is where it is going at once: the test's clock stands still.
    let s = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    s.press(Key::RightArrow);
    s.press(Key::LeftArrow);
    assert!(s.has("Description"), "the description is a thing to select, below the buttons");

    // OK on the description opens it to read; Back closes it and the page stays.
    s.activate("Description");
    assert!(!s.has("Description") || s.store().get_details_open());
    s.press(Key::DownArrow);
    s.press(Key::Escape);
    assert!(s.store().get_details_open(), "Back closes the description, not the page");
    s.press(Key::Escape);
    assert!(!s.store().get_details_open(), "and Back again closes the page");
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    s.press(Key::RightArrow);
    s.press(Key::LeftArrow);

    // Further down are the app's own details, not the device's.
    let from = format!("From: {}", s.store().get_app().source);
    let mut seen = s.has(&from);
    for _ in 0..6 {
        s.press(Key::DownArrow);
        seen |= s.has(&from);
    }
    assert!(seen, "{from}");
    assert!(!s.has("C library: glibc"));
    // And Up comes all the way back to the buttons, and past them to the top bar; Down
    // from there is the buttons again, which still work.
    for _ in 0..8 {
        s.press(Key::UpArrow);
    }
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(
        s.store().get_install_state() != 0 || s.store().get_in_library() || s.store().get_can_open(),
        "OK presses the main button again"
    );
}

#[test]
fn links_in_a_description_are_found_for_the_remote() {
    let text = "Docs at https://example.org/docs. Chat (https://chat.example.org/room), again https://example.org/docs and http://old.example.net; not a link: https://nodot or ftp://x.y";
    assert_eq!(
        bilby::links_in(text),
        ["https://example.org/docs", "https://chat.example.org/room", "http://old.example.net"]
    );
    assert!(bilby::links_in("nothing here").is_empty());
}

#[test]
fn the_tv_has_a_settings_tab_with_every_setting_reachable_by_remote() {
    // Without motion, so the list is where it is going at once: the test's clock stands still.
    let s = start_with("linux-tv", Shell::Tv, Some(r#"{"settings":{"welcomed":true,"reduce_motion":true}}"#));
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 3);
    // Every setting comes into view on the way down the list.
    let mut seen: Vec<&str> = Vec::new();
    let wanted = [
        "Ratings and reviews",
        "Use the desktop layout, for a mouse and keyboard",
        "Family mode: nothing for adults is shown anywhere",
        "Set a PIN to guard these settings and adult content",
        "Sign in to Steam with a QR code, to see what you own",
        "Or sign in to Steam in a browser (Longboard)",
    ];
    for _ in 0..14 {
        seen.extend(wanted.iter().filter(|label| s.has(label)));
        s.press(Key::DownArrow);
    }
    for label in wanted {
        assert!(seen.contains(&label), "{label}");
    }
    for _ in 0..20 {
        s.press(Key::UpArrow);
    }
    s.press(Key::DownArrow);
    // Sources can be switched off from the sofa too: Down to the first of them, OK.
    let sources: Vec<SourceChoice> = s.store().get_sources().iter().collect();
    assert!(sources.len() >= 2 && sources.iter().all(|c| c.on));
    for _ in 0..17 {
        s.press(Key::DownArrow);
    }
    s.press(Key::Return);
    assert!(!s.store().get_sources().row_data(0).unwrap().on, "OK on a source switches it off");
    assert_eq!(s.saved().settings.disabled_sources, [sources[0].id.to_string()]);
    // Down stops at the last row; Up from the first goes back to the tabs; This TV is the next tab along.
    for _ in 0..40 {
        s.press(Key::DownArrow);
    }
    // The last row is whether apps are updated without being asked.
    assert!(!s.saved().settings.auto_update);
    s.press(Key::Return);
    assert!(s.saved().settings.auto_update && s.store().get_auto_update());
    // (Switching that on looks for updates at once, and may ask about finishing them.)
    if s.store().get_gate() != 0 {
        s.store().invoke_gate_answer(false);
    }
    // Above it is whether to show what does not run here.
    s.press(Key::UpArrow);
    assert!(!s.saved().settings.show_everything);
    s.press(Key::Return);
    assert!(s.saved().settings.show_everything && s.store().get_show_everything());
    // Above it is the country for prices: Right is the next in the list (from the
    // system's own to the first by name), Left the one before.
    s.press(Key::UpArrow);
    assert!(s.saved().settings.country.is_empty());
    s.press(Key::RightArrow);
    assert_eq!(s.saved().settings.country, "US");
    assert_eq!(s.store().get_country_name(), "United States");
    s.press(Key::LeftArrow);
    assert!(s.saved().settings.country.is_empty());
    // Above it is the language: OK steps to the next one in the list (from the
    // system's own to English by name, here).
    s.press(Key::UpArrow);
    assert!(s.saved().settings.language.is_empty());
    s.press(Key::Return);
    assert_eq!(s.saved().settings.language, "en");
    assert_eq!(s.store().get_language_name(), "English");
    // The row above it is the last of the sources.
    s.press(Key::UpArrow);
    s.press(Key::Return);
    assert!(!s.store().get_sources().iter().last().unwrap().on);
    for _ in 0..60 {
        s.press(Key::UpArrow);
    }
    s.press(Key::RightArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 4);
}

#[test]
fn down_from_a_tab_goes_into_the_page_showing_and_only_ok_changes_page() {
    let s = start("linux-tv", Shell::Tv);
    // Home: up to search, up to the tabs, right to Library, then Down.
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::RightArrow);
    s.press(Key::DownArrow);
    assert_eq!(s.store().get_page(), 0, "Down from the Library tab does not open the Library");
    // It went to search, the first thing on Home; Down again is the apps.
    s.press(Key::DownArrow);
    s.press(Key::Return);
    assert!(s.store().get_details_open(), "and the apps are below that");
}

#[test]
fn a_tv_remote_enters_the_pin_digit_by_digit() {
    let s = start("linux-tv", Shell::Tv);
    s.store().invoke_set_pin();
    assert_eq!(s.store().get_gate(), 3);
    // The digits are a row: Right twice is 2, OK enters it; number keys work as well.
    let two = || {
        s.press(Key::Return);
    };
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    for _ in 0..4 {
        two();
    }
    assert_eq!(s.store().get_gate(), 4, "2222 entered once");
    // A new question starts with the highlight back at the first digit.
    s.press(Key::RightArrow);
    s.press(Key::RightArrow);
    for _ in 0..4 {
        two();
    }
    assert!(s.store().get_has_pin() && s.store().get_gate() == 0);
    // Back steps away from a question without answering it.
    s.store().invoke_remove_pin();
    assert_eq!(s.store().get_gate(), 2);
    s.press(Key::Escape);
    assert!(s.store().get_gate() == 0 && s.store().get_has_pin());
}

#[test]
fn on_tv_typing_with_the_search_bar_highlighted_goes_straight_into_it() {
    let s = start("linux-tv", Shell::Tv);
    // Up to search: highlighted, not yet typing.
    s.press(Key::UpArrow);
    assert!(!s.store().get_query().contains('x'));
    // A keyboard, or a remote's keys, types without OK.
    s.press("t");
    s.press("i");
    s.press("d");
    s.press("e");
    assert_eq!(s.store().get_query(), "tide", "the letters went into the box");
    // Keys that act are not taken into the box: an arrow still moves the highlight.
    s.press(Key::RightArrow);
    assert!(s.store().get_query() == "tide", "arrow keys are not typed");
    // OK still searches, as before.
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 1);
}
