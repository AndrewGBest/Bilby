//! Bilby in other languages: every language says everything, with the same things
//! filled in, and the interface follows the one that is chosen.
//!
//! One test, in a file of its own: the language is one setting for the whole program.

use bilby::{build, i18n, Options, Shell, Store};
use bilby_core::install::Dirs;
use bilby_core::Platform;
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, Model};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("i18n")
}

fn read(name: &str) -> serde_json::Value {
    let path = folder().join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// How many things are filled in, and whether a number is.
fn gaps(text: &str) -> (usize, bool) {
    let names: Vec<&str> =
        text.split('{').skip(1).filter_map(|rest| rest.split_once('}')).map(|(name, _)| name).collect();
    (names.iter().filter(|name| **name != "n").count(), names.contains(&"n"))
}

fn forms(code: &str) -> usize {
    match code {
        "zh" | "ja" | "ko" => 1,
        "ru" => 3,
        "ar" => 6,
        _ => 2,
    }
}

#[test]
fn every_language_says_everything_and_the_interface_follows_the_one_chosen() {
    // ---- the translations themselves ----
    let english = read("en");
    let list = |name: &str| -> Vec<String> {
        english[name].as_array().unwrap().iter().map(|text| text.as_str().unwrap().to_string()).collect()
    };
    let plain: Vec<String> = list("interface").into_iter().chain(list("messages")).collect();
    let counted: Vec<String> =
        english["counted"].as_array().unwrap().iter().map(|pair| pair[0].as_str().unwrap().to_string()).collect();
    assert!(plain.len() > 900 && counted.len() >= 9, "the English was gathered");

    let codes: Vec<&str> = i18n::languages().into_iter().map(|(code, _)| code).filter(|code| *code != "en").collect();
    assert_eq!(codes, ["es", "fr", "de", "pt", "it", "ru", "zh", "ja", "ko", "hi", "ar"]);
    let mut library = BTreeMap::new();
    for code in &codes {
        let translated = read(code);
        let mut same = 0;
        for text in &plain {
            let said = translated[text].as_str().unwrap_or_else(|| panic!("{code}: no translation of {text:?}"));
            assert!(!said.trim().is_empty() || text.trim().is_empty(), "{code}: {text:?} is translated as nothing");
            assert_eq!(gaps(said), gaps(text), "{code}: {text:?} became {said:?}");
            same += usize::from(said == text);
        }
        // Names of things stay as they are; sentences do not.
        assert!(same * 3 < plain.len(), "{code}: {same} of {} are still in English", plain.len());
        for text in &counted {
            let said = translated[text].as_array().unwrap_or_else(|| panic!("{code}: no forms of {text:?}"));
            assert_eq!(said.len(), forms(code), "{code}: {text:?}");
            for form in said {
                assert!(form.as_str().unwrap().contains("{n}"), "{code}: {text:?} lost its number");
            }
        }
        library.insert(*code, translated["Library"].as_str().unwrap().to_string());
        // What Slint reads was written from the same file.
        let po = std::fs::read_to_string(folder().join("po").join(code).join("LC_MESSAGES").join("bilby.po")).unwrap();
        assert!(po.matches("\nmsgid ").count() > 350, "{code}: scripts/i18n.py has not been run");
    }

    // ---- what the code puts on screen ----
    i18n::set_language("fr");
    let in_library = read("fr")["{} is in your library"].as_str().unwrap().replace("{}", "Plex HTPC");
    assert_eq!(i18n::tr("Plex HTPC is in your library"), in_library, "a filled-in message, by its pattern");
    assert_ne!(i18n::tr("Up to date"), "Up to date");
    assert_eq!(i18n::tr("Plex HTPC"), "Plex HTPC", "a name stays a name");
    assert_eq!(
        i18n::trn("{n} update", "{n} updates", 1),
        read("fr")["{n} update"][0].as_str().unwrap().replace("{n}", "1")
    );
    assert_eq!(
        i18n::trn("{n} update", "{n} updates", 5),
        read("fr")["{n} update"][1].as_str().unwrap().replace("{n}", "5")
    );
    i18n::set_language("en");
    assert_eq!(i18n::tr("Plex HTPC is in your library"), "Plex HTPC is in your library");
    assert_eq!(i18n::trn("{n} update", "{n} updates", 5), "5 updates");

    // ---- the interface ----
    i_slint_backend_testing::init_no_event_loop();
    bilby_core::demo::DOWNLOAD_STEP_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("bilby-languages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dirs = Dirs {
        downloads: root.join("downloads"),
        applications: root.join("Applications"),
        menu: root.join("menu"),
        flatpak: root.join("flatpak-program"),
        elevate: root.join("elevate-program"),
        terminal: None,
        flatpak_roots: vec![root.join("flatpak")],
        programs: Vec::new(),
        cache: root.join("cache"),
        config: root.join("config"),
        desktop: None,
    };
    std::fs::create_dir_all(&dirs.config).unwrap();
    let state = dirs.config.join("demo-state.json");
    std::fs::write(&state, r#"{"settings":{"welcomed":true}}"#).unwrap();
    let start = |language: Option<&str>| {
        let options = Options {
            demo: true,
            inline: true,
            platform: Platform::preset("linux-x64-debian"),
            shell: Some(Shell::Desktop),
            dirs: Some(dirs.clone()),
            opener: Some(Arc::new(|_| Ok(()))),
            language: language.map(str::to_string),
            ..Default::default()
        };
        let (ui, app) = build(&options).unwrap();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 2400.0));
        ui.show().unwrap();
        (ui, app)
    };
    let shows =
        |ui: &bilby::AppWindow, label: &str| ElementHandle::find_by_accessible_label(ui, label).next().is_some();

    // A preview of another device with nothing chosen is in English.
    let (ui, _app) = start(None);
    assert!(shows(&ui, "Library") && i18n::language() == "en");
    let store = ui.global::<Store>();
    let choices: Vec<String> = store.get_languages().iter().map(|choice| choice.label.to_string()).collect();
    assert_eq!(choices.len(), 13, "the system's own, and twelve by name: {choices:?}");
    assert!(
        choices[0].starts_with("System")
            && choices.contains(&"Français".to_string())
            && choices.contains(&"日本語".to_string())
    );

    // Chosen in Settings: the interface changes at once, and so does what the code says.
    store.invoke_set_language("de".into());
    assert_eq!(i18n::language(), "de");
    assert!(shows(&ui, &library["de"]) && !shows(&ui, "Library"), "the sidebar is in German");
    assert_ne!(store.get_system_headline(), "Not checked yet");
    assert!(store.get_languages().iter().any(|choice| choice.id == "de" && choice.on));
    assert!(std::fs::read_to_string(&state).unwrap().contains(r#""language": "de""#), "and it is kept");
    drop(ui);

    // The next start is in German without being told; one run can be told otherwise.
    let (ui, _app) = start(None);
    assert!(shows(&ui, &library["de"]));
    drop(ui);
    let (ui, _app) = start(Some("ja"));
    assert!(shows(&ui, &library["ja"]));
    // Back to the system's own, which for a preview is English.
    ui.global::<Store>().invoke_set_language("".into());
    assert!(shows(&ui, "Library") && i18n::language() == "en");
    // A remote steps through them in order.
    ui.global::<Store>().invoke_next_language();
    assert_eq!(i18n::language(), "en", "after the system's own comes English by name");
    ui.global::<Store>().invoke_next_language();
    assert_eq!(i18n::language(), "es");
    assert!(shows(&ui, &library["es"]));
    let _ = std::fs::remove_dir_all(&root);
}
