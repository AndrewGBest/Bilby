//! Signing in to GitHub, Steam and GOG.

use super::*;

#[test]
fn signing_in_to_gog_keeps_only_the_list_of_what_the_account_owns() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(!s.store().get_gog_signed_in());
    s.click("Sign in to GOG");
    assert!(s.store().get_gog_signed_in(), "{}", s.store().get_toast());
    assert_eq!(s.store().get_gog_summary(), "2 games");
    assert_eq!(s.saved().gog.unwrap().owned, [1207658924, 1495134320]);
    let kept = std::fs::read_to_string(&s.state_file).unwrap();
    assert!(!kept.contains("demo-gog-key") && !kept.contains("one-time-code"), "no key to the account is kept");
    assert!(s.app.owns(&AppId::parse("gog:1207658924").unwrap()));
    assert!(!s.app.owns(&AppId::parse("gog:7").unwrap()));

    s.click("Sign out of GOG");
    assert!(!s.store().get_gog_signed_in() && s.saved().gog.is_none());
    assert!(!s.dirs.config.join("gog-session").exists(), "the sign-in window's own data goes too");
    assert!(!s.app.owns(&AppId::parse("gog:1207658924").unwrap()));
}

#[test]
fn signing_in_to_github_shows_a_code_and_keeps_the_token_it_is_given() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(!s.store().get_has_token());
    // The demo's GitHub says yes the first time it is asked.
    // (The GitHub card is far down the Settings page, so it is pressed through its action.)
    s.store().invoke_github_signin();
    assert!(s.store().get_has_token());
    assert_eq!(s.saved().settings.github_token, "demo-token");
    assert_eq!((s.store().get_signin_state(), s.store().get_signin_code().as_str()), (0, ""));
    assert_eq!(s.store().get_toast(), "Signed in to GitHub");
    assert!(s.store().get_has_token(), "nothing to sign in to twice");

    s.store().invoke_save_token("".into());
    assert!(!s.store().get_has_token() && s.saved().settings.github_token.is_empty());
    assert!(!s.store().get_has_token());

    // On a TV it is under Accounts on the Settings tab, reached with the remote.
    let tv = start("linux-tv", Shell::Tv);
    tv.press(Key::UpArrow);
    tv.press(Key::UpArrow);
    tv.press(Key::RightArrow);
    tv.press(Key::RightArrow);
    tv.press(Key::RightArrow);
    tv.press(Key::Return);
    for _ in 0..14 {
        tv.press(Key::DownArrow);
    }
    tv.press(Key::Return);
    assert!(tv.store().get_has_token(), "OK on the GitHub row signs in");
    tv.press(Key::Return);
    assert!(!tv.store().get_has_token(), "and OK again signs out");
}

#[test]
fn signing_in_to_steam_in_the_stores_window_records_what_the_account_owns() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    assert!(s.has("Sign in to Steam with a QR code"), "both ways are offered");
    assert!(s.has("Sign in to Steam in a browser") && !s.store().get_steam_signed_in());
    s.click("Sign in to Steam in a browser");
    assert!(s.store().get_steam_signed_in());
    assert_eq!(s.store().get_steam_summary(), "3 games and tools");
    assert!(s.store().get_toast().contains("3 games and tools"), "{}", s.store().get_toast());
    let saved = s.saved().steam.expect("kept for next time");
    assert_eq!((saved.steamid.as_str(), saved.owned.as_slice()), ("76561198000000001", &[10, 440, 620][..]));
    assert!(s.app.owns(&AppId::parse("steam:440").unwrap()) && !s.app.owns(&AppId::parse("steam:570").unwrap()));
    assert!(!s.app.owns(&AppId::parse("flathub:440").unwrap()), "only Steam's own ids");
    // Only the list is kept by the store: nothing that could act as the account.
    let on_disk = std::fs::read_to_string(&s.state_file).unwrap();
    assert!(!on_disk.contains("token") || on_disk.contains("github_token"), "{on_disk}");

    // Signing out forgets the list and the window's own data.
    assert!(s.dirs.config.join("steam-session").join("cookies").exists());
    s.click("Sign out of Steam");
    assert!(!s.store().get_steam_signed_in() && s.saved().steam.is_none());
    assert!(!s.dirs.config.join("steam-session").exists());
    assert!(!s.app.owns(&AppId::parse("steam:440").unwrap()));
}

#[test]
fn signing_in_to_steam_with_a_code_shows_it_and_keeps_only_the_list() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.store().set_page(3);
    // The demo's Steam swaps the code once and approves the second time it is asked.
    s.click("Sign in to Steam with a QR code");
    assert!(s.store().get_steam_signed_in() && s.store().get_steam_state() == 0);
    assert_eq!(s.saved().steam.unwrap().owned, [440, 620]);
    let on_disk = std::fs::read_to_string(&s.state_file).unwrap();
    assert!(!on_disk.contains("demo.") && !on_disk.contains("refresh"), "no key from Steam is kept: {on_disk}");
    assert_eq!(s.store().get_steam_qr().size().width, 0, "the code is taken down afterwards");
    assert!(!s.dirs.config.join("steam-session").exists(), "and no browser was involved");

    // A TV offers the same two ways, the code first.
    let tv = start("linux-tv", Shell::Tv);
    tv.store().invoke_steam_signin_qr();
    assert!(tv.store().get_steam_signed_in());
}
