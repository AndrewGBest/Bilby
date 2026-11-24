//! Ratings, reviews and the other signs of how much an app is used.

use super::*;

#[test]
fn star_ratings_are_shown_where_known_and_can_be_turned_off() {
    let s = start("linux-x64-debian", Shell::Desktop);
    // What the store does when it opens.
    s.app.load_ratings(&s.ui);
    assert!(s.dirs.cache.join("ratings.json").is_file(), "one file with every app's rating");

    // In search results and on the app's page, for an app the ratings service knows.
    s.app.search(&s.ui, "notes");
    let rated = s.store().get_results().iter().find(|c| c.id == "flathub:io.quillpad.Quillpad").unwrap();
    assert_eq!((rated.rating.as_str(), rated.rating_count.as_str()), ("4.6", "821 ratings"));
    let unrated = s.store().get_results().iter().find(|c| c.id == "github:driftnotes/drift").unwrap();
    assert_eq!(unrated.rating, "", "no rating is invented for an app the service does not know");
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_app().rating, "4.6");
    assert!(s.has("★ 4.6 821 ratings"));

    // Settings: off means gone from the screen straight away, and remembered.
    s.store().invoke_set_ratings(false);
    assert!(s.saved().settings.hide_ratings);
    assert!(!s.store().get_show_ratings());
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().all(|c| c.rating.is_empty()));

    // And back on.
    s.store().invoke_set_ratings(true);
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().any(|c| c.rating == "4.6"));
}

#[test]
fn each_app_shows_the_signs_of_use_its_source_has_and_no_others() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.load_ratings(&s.ui);

    // Flathub: a score with its breakdown, installs in total and a month, a confirmed developer.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    let app = s.store().get_app();
    let spread: Vec<f32> = app.rating_spread.iter().collect();
    assert_eq!(spread.len(), 5, "five stars first, one star last");
    assert!((spread.iter().sum::<f32>() - 1.0).abs() < 0.001 && spread[0] > spread[4], "{spread:?}");
    assert!(app.verified);
    assert_eq!((app.downloads.as_str(), app.downloads_note.as_str()), ("1.3M downloads", "in total"));
    assert_eq!(app.popularity, "41k installs a month");
    assert!(s.has("4.6 out of 5, from 821 ratings") && s.has("Quillpad contributors  ✓ verified"));

    // GitHub: stars and how often its recent files were downloaded; no rating, nobody confirmed.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    let app = s.store().get_app();
    assert!(
        app.popularity.ends_with("stars") && app.downloads.ends_with("downloads"),
        "{} {}",
        app.popularity,
        app.downloads
    );
    assert_eq!(app.downloads_note, "of recent versions");
    assert_eq!((app.rating.as_str(), app.rating_spread.row_count(), app.verified), ("", 0, false));
    assert!(!s.has("4.6 out of 5, from 821 ratings") && s.has("lumen-labs"));

    // A tile has room for one figure: the rating where there is one, else downloads or stars.
    s.app.search(&s.ui, "notes");
    let results = s.store().get_results();
    assert!(
        results.iter().all(|c| !c.rating.is_empty() || !c.headline.is_empty()),
        "every demo app has some sign of use"
    );
}

#[test]
fn with_ratings_turned_off_nothing_is_fetched() {
    let s =
        start_with("linux-x64-debian", Shell::Desktop, Some(r#"{"settings":{"welcomed":true,"hide_ratings":true}}"#));
    s.app.load_ratings(&s.ui);
    assert!(!s.dirs.cache.join("ratings.json").exists());
    s.app.search(&s.ui, "notes");
    assert!(s.store().get_results().iter().all(|c| c.rating.is_empty()));
}

#[test]
fn written_reviews_appear_only_for_apps_that_have_them() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.load_ratings(&s.ui);

    // An app the ratings service knows: its reviews are on its page.
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    let reviews = s.store().get_reviews();
    assert_eq!(reviews.row_count(), 3);
    let first = reviews.row_data(0).unwrap();
    assert_eq!((first.stars.as_str(), first.title.as_str()), ("★★★★★", "Exactly what I wanted"));
    assert_eq!(first.meta, "Marta  ·  2026-11-01  ·  version 4.2.0");
    assert_eq!(reviews.row_data(2).unwrap().stars, "★★☆☆☆");
    assert!(s.has("Ratings and reviews"));

    // An app it does not know: no section, and the last app's reviews do not linger.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_reviews().row_count(), 0);
    assert!(!s.has("Ratings and reviews"));

    // With ratings and reviews turned off, nothing is asked for and nothing is shown.
    s.store().invoke_set_ratings(false);
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_reviews().row_count(), 0);
}

#[test]
fn a_review_can_be_written_on_an_apps_page_and_sent_from_there() {
    let s = start("linux-x64-debian", Shell::Desktop);
    s.app.load_ratings(&s.ui);
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    let places = s.store().get_review_places();
    assert_eq!(places.row_count(), 1, "the ratings service the Linux app stores share");
    assert!(places.row_data(0).unwrap().from_here && places.row_data(0).unwrap().chosen);
    assert!(!s.has("Title of your review"));
    s.click("Write a review");
    assert!(s.has("Title of your review") && s.has("Your review") && s.has("Name to show next to your review"));

    // Nothing written yet: the form says what is missing and nothing is sent.
    s.activate("Send your review");
    assert_eq!(s.store().get_review_problem(), "Choose how many stars first.");
    s.activate("4 stars");
    s.activate("Send your review");
    assert!(s.store().get_review_problem().starts_with("Write a little more"), "{}", s.store().get_review_problem());
    let (title, text) = ("Does the job", "Starts fast and stays out of the way.");
    s.store().invoke_send_review(4, title.into(), text.into(), "".into());
    assert_eq!(s.store().get_review_problem(), "Say what name to show next to your review.");
    assert!(s.saved().reviewer.is_empty(), "nothing is made up for a review that was not sent");

    // Written in full: it is sent, and the page says so in place of the form.
    s.store().invoke_send_review(4, title.into(), text.into(), "Ana".into());
    assert_eq!(s.store().get_review_state(), 2, "{}", s.store().get_review_problem());
    assert!(s.store().get_review_result().starts_with("Sent, thank you."));
    assert!(!s.has("Title of your review") && !s.has("Write a review"));
    // The made-up id that marks this person's reviews, and the name for next time.
    let saved = s.saved();
    assert!(saved.reviewer.len() == 40 && saved.reviewer_name == "Ana");

    // The service takes one review of an app from each person; its refusal is shown.
    s.app.open(&s.ui, "flathub:org.sundial.Sundial");
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_review_state(), 0);
    assert_eq!(s.store().get_review_name(), "Ana");
    s.store().invoke_send_review(5, "Again".into(), text.into(), "Ana".into());
    assert_eq!(s.store().get_review_problem(), "You have already reviewed this app there.");
    assert_eq!(s.saved().reviewer, saved.reviewer, "the same person as before");

    // An app nowhere takes reviews of: nothing to write.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert_eq!(s.store().get_review_places().row_count(), 0);
    assert!(!s.has("Write a review"));

    // With ratings and reviews turned off, there is nowhere to send one.
    s.store().invoke_set_ratings(false);
    s.app.open(&s.ui, "flathub:io.quillpad.Quillpad");
    assert_eq!(s.store().get_review_places().row_count(), 0);
}
