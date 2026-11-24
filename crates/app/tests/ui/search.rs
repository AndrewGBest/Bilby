//! Searching, suggestions and what several sources make of one app.

use super::*;

#[test]
fn search_lists_only_installable_apps_and_says_what_it_left_out() {
    let mac = start("macos-arm64", Shell::Desktop);
    mac.app.search(&mac.ui, "monitor");
    assert_eq!(mac.store().get_page(), 1);
    assert!(mac.store().get_searched() && !mac.store().get_searching());
    let names: Vec<String> = mac.store().get_results().iter().map(|a| a.id.to_string()).collect();
    assert_eq!(names, ["github:barley/barley"]);
    assert!(mac.store().get_search_note().contains("1 more matched but do not work on this device"));
    assert_eq!(mac.store().get_results().row_data(0).unwrap().badge, "Disk image (.dmg)");

    let phone = start("android-arm64", Shell::Mobile);
    phone.app.search(&phone.ui, "monitor");
    assert_eq!(phone.store().get_results().row_count(), 0);
    assert!(phone.store().get_search_note().contains("2 more matched"));
    assert!(phone.has("No apps for this device") || phone.store().get_results().row_count() == 0);
}

#[test]
fn pasting_a_link_opens_the_app_and_searches_are_remembered() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.search(&s.ui, "https://github.com/lumen-labs/lumen/releases");
    assert!(s.store().get_details_open());
    assert_eq!(s.store().get_app().id, "github:lumen-labs/lumen");
    assert_eq!(s.option_labels()[0], "Debian package · x86_64");
    assert!(s.saved().recent_searches.is_empty(), "a link is not a search");

    s.app.search(&s.ui, "editor");
    assert!(!s.store().get_details_open(), "searching leaves the app page");
    s.app.search(&s.ui, "backup");
    assert_eq!(s.saved().recent_searches, ["backup", "editor"]);
    assert_eq!(s.store().get_recent().row_count(), 2);

    // An AUR link on a system without the AUR is treated as plain text, not opened.
    s.app.search(&s.ui, "aur:lumen-bin");
    assert!(!s.store().get_details_open());

    s.app.search(&s.ui, "x");
    assert_eq!(s.store().get_search_note(), "Type at least two characters.");
}

#[test]
fn typing_suggests_featured_apps_without_searching() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.typing(&s.ui, "lum");
    assert_eq!(s.store().get_page(), 1, "typing brings up the search page");
    let names: Vec<String> = s.store().get_suggestions().iter().map(|a| a.name.to_string()).collect();
    assert_eq!(names, ["Lumen"]);
    assert!(!s.store().get_searched() && !s.store().get_searching(), "nothing was sent anywhere");
    assert!(s.saved().recent_searches.is_empty(), "keystrokes are not history");
    assert!(s.has("Lumen"));

    // A suggestion opens like any other tile.
    s.click("Lumen");
    assert_eq!(s.store().get_app().id, "github:lumen-labs/lumen");
    s.click("Back");

    // Enter runs the real search and replaces the suggestions.
    s.app.search(&s.ui, "lum");
    assert_eq!(s.store().get_suggestions().row_count(), 0);
    assert_eq!(s.store().get_results().row_data(0).unwrap().id, "github:lumen-labs/lumen");

    // Typing again clears the old results so the two never mix.
    s.app.typing(&s.ui, "tide");
    assert_eq!(s.store().get_results().row_count(), 0);
    assert_eq!(s.store().get_suggestions().row_data(0).unwrap().name, "Tidewave");
    s.app.typing(&s.ui, "");
    assert_eq!(s.store().get_suggestions().row_count(), 0);
}

#[test]
fn system_repositories_are_searched_first_and_installed_packages_marked() {
    // An Arch-style machine with CachyOS repositories configured.
    let s = start("linux-x64-arch", Shell::Desktop);
    let facts: Vec<(String, String)> =
        s.store().get_facts().iter().map(|f| (f.name.to_string(), f.value.to_string())).collect();
    assert!(facts.contains(&("System repositories".into(), "cachyos-v3, extra, core".into())), "{facts:?}");

    s.app.search(&s.ui, "kernel");
    let hit = s.store().get_results().row_data(0).unwrap();
    assert_eq!(hit.id, "pacman:swiftkernel");
    assert_eq!(hit.source, "cachyos-v3", "the card names the repository");
    assert_eq!(hit.badge, "Repository package");
    assert_eq!(hit.version, "6.17.4-2");
    assert!(!hit.installed);

    s.app.open(&s.ui, "pacman:swiftkernel");
    assert!(s.store().get_is_package());
    let option = s.store().get_options().row_data(0).unwrap();
    assert_eq!(option.label, "Repository package · x86_64");
    assert_eq!(option.verb, "Install");
    assert_eq!(option.command, "sudo pacman -S --noconfirm --needed swiftkernel");
    assert!(s.store().get_notes().contains("“cachyos-v3” repository"));
    // One press: pacman installs it behind the system's password prompt, and keeps
    // track of it itself, so nothing goes in the store's library.
    s.app.install(&s.ui, 0);
    #[cfg(unix)]
    assert_eq!(s.store().get_install_message(), "Installed.");
    assert!(s.saved().library.is_empty());

    // A package pacman already has, with a newer version in the repository.
    s.app.search(&s.ui, "containers");
    let harbor = s.store().get_results().row_data(0).unwrap();
    assert_eq!((harbor.id.as_str(), harbor.installed), ("pacman:harbor", true));
    s.app.open(&s.ui, "pacman:harbor");
    assert_eq!(s.store().get_installed_version(), "5.2.0-1");
    assert_eq!(s.store().get_app().version, "5.3.1-1");

    // The same machine type without pacman has no such source; apt takes its place.
    let debian = start("linux-x64-debian", Shell::Desktop);
    debian.app.search(&debian.ui, "containers");
    let hit = debian.store().get_results().row_data(0).unwrap();
    assert_eq!(hit.id, "apt:harbor");
    assert_eq!(hit.source, "noble-updates/universe");
    debian.app.open(&debian.ui, "apt:harbor");
    assert_eq!(debian.store().get_options().row_data(0).unwrap().command, "sudo apt-get install -y harbor");
    debian.app.open(&debian.ui, "pacman:harbor");
    assert!(!debian.store().get_details_error().is_empty(), "pacman packages are not offered on a Debian system");

    // And nowhere else.
    let windows = start("windows-x64", Shell::Desktop);
    windows.app.search(&windows.ui, "containers");
    assert_eq!(windows.store().get_results().row_count(), 0);
}

#[test]
fn aur_results_appear_only_on_arch() {
    let arch = start("linux-x64-arch", Shell::Desktop);
    arch.app.search(&arch.ui, "lumen");
    let sources: Vec<String> = arch.store().get_results().iter().map(|a| a.source.to_string()).collect();
    assert_eq!(sources, ["GitHub", "AUR", "AUR"]);

    let debian = start("linux-x64-debian", Shell::Desktop);
    debian.app.search(&debian.ui, "lumen");
    let sources: Vec<String> = debian.store().get_results().iter().map(|a| a.source.to_string()).collect();
    assert_eq!(sources, ["GitHub"]);
}

#[test]
fn back_and_home_both_leave_a_search() {
    // On a TV: Back from what was found is Home, with the box emptied.
    let s = start("linux-tv", Shell::Tv);
    s.app.search(&s.ui, "kestrel");
    s.store().set_query("kestrel".into());
    assert!(s.store().get_page() == 1 && s.store().get_results().row_count() > 0);
    let found = s.store().get_tv_rows().row_data(0).unwrap().title;
    assert!(found.starts_with("Top results"), "{found}");
    s.press(Key::Escape);
    assert_eq!(s.store().get_page(), 0);
    assert_eq!((s.store().get_query().as_str(), s.store().get_results().row_count()), ("", 0));
    let home = s.store().get_tv_rows().row_data(0).unwrap().title;
    assert!(!home.starts_with("Top results"), "Home shows the store again, not the search: {home}");

    // And so is the Home tab: up to the bar, across to Home, OK.
    s.app.search(&s.ui, "kestrel");
    s.store().set_query("kestrel".into());
    s.press(Key::UpArrow);
    s.press(Key::UpArrow);
    s.press(Key::Return);
    assert_eq!(s.store().get_page(), 0);
    assert_eq!((s.store().get_query().as_str(), s.store().get_results().row_count()), ("", 0));

    // On a desktop: the Back button beside the search box, Escape, and Discover.
    let d = start("linux-x64-debian", Shell::Desktop);
    for leave in ["Back", "Escape", "Discover"] {
        d.app.search(&d.ui, "kestrel");
        d.store().set_query("kestrel".into());
        assert_eq!(d.store().get_page(), 1);
        match leave {
            "Escape" => d.press(Key::Escape),
            label => d.click(label),
        }
        assert_eq!(d.store().get_page(), 0, "{leave}");
        assert_eq!((d.store().get_query().as_str(), d.store().get_results().row_count()), ("", 0), "{leave}");
    }
    // Back from an app opened from a search is still the search.
    d.app.search(&d.ui, "kestrel");
    d.app.open(&d.ui, "github:kestrel-sh/kestrel");
    d.press(Key::Escape);
    assert!(d.store().get_page() == 1 && !d.store().get_details_open());
    assert!(d.store().get_results().row_count() > 0);
}

#[test]
fn slash_and_ctrl_k_start_a_search_from_the_keyboard() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // "/" moves the focus to the search field; what is typed next lands in it.
    s.press("/");
    s.press("l");
    s.press("u");
    assert_eq!(s.store().get_query(), "lu");
    assert_eq!(s.store().get_page(), 1, "typing brings up suggestions");
    assert_eq!(s.store().get_suggestions().row_data(0).unwrap().name, "Lumen");
    // Enter searches.
    s.press(Key::Return);
    assert!(s.store().get_searched());
    assert_eq!(
        s.store().get_results().row_data(0).unwrap().id,
        "github:lumen-labs/lumen",
        "the app first, web apps after"
    );
    assert_eq!(s.saved().recent_searches, ["lu"]);
    // Escape leaves the field; Ctrl+K comes back to it and selects the text, so typing replaces it.
    s.press(Key::Escape);
    s.ui.window().dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    s.press("k");
    s.ui.window().dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    s.press("t");
    assert_eq!(s.store().get_query(), "t");
}

#[test]
fn sources_can_be_switched_off_and_results_filtered_by_source() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // A search that several sources answer.
    s.app.search(&s.ui, "notes");
    let sources: Vec<(String, i32)> =
        s.store().get_result_sources().iter().map(|c| (c.id.to_string(), c.count)).collect();
    assert!(sources.len() >= 2 && sources.iter().any(|(id, _)| id == "flathub"), "{sources:?}");
    let all = s.store().get_results().row_count();

    // Narrow to one source, and back.
    s.store().invoke_filter_results("flathub".into());
    assert_eq!(s.store().get_source_filter(), "flathub");
    assert!(s.store().get_results().iter().all(|c| c.id.starts_with("flathub:")));
    assert!(s.store().get_results().row_count() < all);
    s.store().invoke_filter_results("".into());
    assert_eq!(s.store().get_results().row_count(), all);

    // Switched off in Settings: gone from searches and the home page, and remembered.
    s.store().set_page(3);
    assert!(s.store().get_sources().iter().any(|c| c.id == "flathub" && c.on));
    s.store().invoke_set_source("flathub".into(), false);
    assert_eq!(s.saved().settings.disabled_sources, ["flathub"]);
    assert!(s.store().get_sources().iter().any(|c| c.id == "flathub" && !c.on), "still listed, to switch back on");
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().all(|c| !c.id.starts_with("flathub:")));
    assert!(s.store().get_sections().iter().all(|section| !section.title.contains("Flathub")));
    // A link to an app from it still opens: the person asked for that app.
    s.app.search(&s.ui, "flathub:io.quillpad.Quillpad");
    assert!(s.store().get_details_open());

    s.store().invoke_set_source("flathub".into(), true);
    assert!(s.saved().settings.disabled_sources.is_empty());
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().any(|c| c.id.starts_with("flathub:")));
}

#[test]
fn an_app_found_at_several_sources_is_one_result_with_its_sources_by_the_button() {
    // On an Arch-based system the demo has Relay both in pacman's repositories and on GitHub.
    let s = start("linux-x64-arch", Shell::Desktop);
    s.app.search(&s.ui, "relay");
    let results: Vec<AppCard> = s.store().get_results().iter().collect();
    let harbors: Vec<&AppCard> = results.iter().filter(|c| c.name.to_lowercase() == "relay").collect();
    assert_eq!(
        harbors.len(),
        1,
        "one tile, not one per source: {:?}",
        results.iter().map(|c| c.id.to_string()).collect::<Vec<_>>()
    );
    assert!(harbors[0].more_sources >= 1);
    let lead = harbors[0].id.to_string();

    // Its page says which source the button uses and offers the others beside it.
    s.app.open(&s.ui, &lead);
    let sources: Vec<SourceChoice> = s.store().get_app_sources().iter().collect();
    assert!(sources.len() >= 2);
    assert_eq!(sources.iter().filter(|c| c.on).count(), 1);
    assert_eq!(sources.iter().find(|c| c.on).unwrap().id, lead.as_str());
    let other = sources.iter().find(|c| !c.on).unwrap().clone();
    assert!(s.has(&format!("Switch to {}", other.label)));

    // Choosing another source shows the app as that source has it, with the choice kept.
    s.click(&format!("Switch to {}", other.label));
    assert_eq!(s.store().get_app().id, other.id);
    let now: Vec<SourceChoice> = s.store().get_app_sources().iter().collect();
    assert_eq!(now.iter().find(|c| c.on).unwrap().id, other.id);
    assert_eq!(now.len(), sources.len());

    // Filtering by source shows each entry as that source has it.
    s.store().set_details_open(false);
    s.store().invoke_filter_results("github".into());
    assert!(s.store().get_results().iter().all(|c| c.id.starts_with("github:") && c.more_sources == 0));

    // An app only one source has offers no choice.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_app_sources().row_count(), 0);
}

#[test]
fn a_service_with_several_apps_leads_its_results_and_its_page_offers_the_others() {
    // A Linux TV can use YouTube's own site (in Longboard) and the desktop apps on Flathub and GitHub.
    let s = start("linux-tv", Shell::Tv);
    s.app.search(&s.ui, "youtube");
    let first = s.store().get_results().row_data(0).unwrap();
    // The results are the apps themselves: the service's own site first, as itself,
    // and no tile that stands for several things at once.
    assert_eq!((first.name.as_str(), first.id.as_str()), ("YouTube", "web:youtube.com"));
    assert_eq!(first.headline, "", "it is the website, not a bundle of versions");
    assert_eq!(first.more_sources, 0, "and a website is never one of a program's sources");
    let tiles = s.store().get_results().iter().filter(|c| c.id == "web:youtube.com").count();
    assert_eq!(tiles, 1);

    // The service's own app lists the other ways to use it, each once, and is not
    // among them itself.
    s.app.open(&s.ui, "web:youtube.com");
    assert_eq!(s.store().get_app_family_name(), "YouTube");
    let others: Vec<VersionChoice> = s.store().get_app_family().iter().collect();
    let names: Vec<&str> = others.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["VacuumTube", "FreeTube"], "only what this device can use, each once");
    let vacuum = &others[0];
    assert_eq!(
        (vacuum.id.as_str(), vacuum.official),
        ("flathub:rocks.shy.VacuumTube", false),
        "whose it is, is always said"
    );
    assert_eq!(vacuum.note, "YouTube's TV interface, as an app");
    // On the TV they are behind a button of their own, though a website has no files.
    assert!(s.has("Other ways to use YouTube"));

    // An app someone else made names the service's own, once, and not the other players.
    s.app.open(&s.ui, "flathub:rocks.shy.VacuumTube");
    let back: Vec<VersionChoice> = s.store().get_app_family().iter().collect();
    assert_eq!(back.len(), 1);
    assert_eq!((back[0].name.as_str(), back[0].id.as_str()), ("YouTube website", "web:youtube.com"));
    // Its sources beside the button, as before.
    let sources: Vec<(String, bool)> =
        s.store().get_app_sources().iter().map(|c| (c.label.to_string(), c.on)).collect();
    assert_eq!(sources, [("Flathub".to_string(), true), ("GitHub".to_string(), false)]);
    s.app.open(&s.ui, "github:shy1132/VacuumTube");
    assert_eq!(s.store().get_app_family().row_count(), 1, "the same from another source");

    // A service's own program lists its website and the apps others made; it is not
    // taken for one of those itself.
    s.app.open(&s.ui, "flathub:com.discordapp.Discord");
    let discord: Vec<String> = s.store().get_app_family().iter().map(|v| v.name.to_string()).collect();
    assert_eq!(discord, ["Discord website", "Vesktop"]);

    // On a phone the same family leads with the website and offers the Android apps.
    let phone = start("android-arm64", Shell::Mobile);
    phone.app.search(&phone.ui, "YouTube");
    let first = phone.store().get_results().row_data(0).unwrap();
    assert_eq!((first.name.as_str(), first.id.as_str()), ("YouTube", "web:youtube.com"));
    phone.app.open(&phone.ui, "web:youtube.com");
    let on_phone: Vec<String> = phone.store().get_app_family().iter().map(|v| v.name.to_string()).collect();
    assert!(on_phone.contains(&"NewPipe".to_string()) && !on_phone.contains(&"VacuumTube".to_string()), "{on_phone:?}");

    // An app that belongs to no family shows no such choice.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_app_family().row_count(), 0);
}
