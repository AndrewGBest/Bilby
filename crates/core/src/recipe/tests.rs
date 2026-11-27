use super::*;
use crate::error::{Error, Result};
use std::collections::HashMap;

/// A made-up web: each address answers with what the test gave it, and an address
/// under "redirects" leads where the test says.
#[derive(Default)]
pub(crate) struct Web {
    pub pages: HashMap<&'static str, &'static str>,
    pub redirects: HashMap<&'static str, &'static str>,
}

impl Http for Web {
    fn get(&self, url: &str) -> Result<String> {
        self.pages.get(url).map(|body| body.to_string()).ok_or_else(|| Error::NotFound(format!("nothing at {url}")))
    }
    fn lands_at(&self, url: &str) -> Result<String> {
        self.redirects.get(url).map(|to| to.to_string()).ok_or_else(|| Error::NotFound(format!("nothing at {url}")))
    }
    fn download(&self, _: &str, _: &std::path::Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
        unreachable!()
    }
}

pub(crate) const TIDE: &str = r#"{
  "id": "tide", "name": "Tide", "summary": "Charts of the sea", "author": "Tide Works",
  "homepage": "https://tide.example/", "hosts": ["tide.example"],
  "version": { "page": "https://tide.example/download", "find": "Version ([0-9.]+) is out" },
  "files": [
    { "url": "https://cdn.tide.example/{version}/tide-{version}-x86_64.AppImage",
      "sha256": { "page": "https://tide.example/sums/{version}.txt", "find": "([0-9a-fA-F]{64})  tide-{version}-x86_64.AppImage" } },
    { "url": { "page": "https://tide.example/download", "find": "href=\"([^\"]+\\.dmg[^\"]*)\"" } },
    { "name": "tide_{version}_amd64.deb", "url": { "redirect": "https://tide.example/latest/deb" } },
    { "url": { "json": "https://tide.example/api/latest", "at": "files.1.url" } }
  ] }"#;

pub(crate) fn tide_web() -> Web {
    Web {
        pages: HashMap::from([
            (
                "https://tide.example/download",
                r#"<p>Version 3.2.1 is out</p> <a href="/files/Tide-3.2.1.dmg?a=1&amp;b=2">Mac</a>"#,
            ),
            (
                "https://tide.example/sums/3.2.1.txt",
                "ABCDEFabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123  tide-3.2.1-x86_64.AppImage\n",
            ),
            (
                "https://tide.example/api/latest",
                r#"{ "files": [ { "url": "no" }, { "url": "https://tide.example/win/tide-3.2.1-setup.exe" } ] }"#,
            ),
        ]),
        redirects: HashMap::from([("https://tide.example/latest/deb", "https://cdn.tide.example/get?id=77")]),
    }
}

fn tide() -> Recipe {
    serde_json::from_str(TIDE).unwrap()
}

#[test]
fn a_recipe_finds_the_version_and_each_file_in_the_three_ways_it_can_look() {
    let recipe = tide();
    assert_eq!(recipe.problem(), None);
    let found = recipe.run(&tide_web()).unwrap();
    assert_eq!(found.version, "3.2.1");
    assert_eq!(found.missing, Vec::<String>::new());
    let files: Vec<(&str, &str)> = found.files.iter().map(|file| (file.name.as_str(), file.url.as_str())).collect();
    assert_eq!(
        files,
        [
            // Written out, with the version put in.
            ("tide-3.2.1-x86_64.AppImage", "https://cdn.tide.example/3.2.1/tide-3.2.1-x86_64.AppImage"),
            // Found in a page: made whole against the page, and "&amp;" read as HTML.
            ("Tide-3.2.1.dmg", "https://tide.example/files/Tide-3.2.1.dmg?a=1&b=2"),
            // Where a "latest" link leads, under the name the recipe gives.
            ("tide_3.2.1_amd64.deb", "https://cdn.tide.example/get?id=77"),
            // Picked out of a JSON answer.
            ("tide-3.2.1-setup.exe", "https://tide.example/win/tide-3.2.1-setup.exe"),
        ]
    );
    assert_eq!(
        found.files[0].sha256.as_deref(),
        Some("abcdefabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123"),
        "the maker's checksum, in lower case"
    );
    assert_eq!(found.files[1].sha256, None);
}

#[test]
fn a_recipe_cannot_read_or_download_from_a_host_it_does_not_name() {
    // Written to fetch from elsewhere: refused before anything is asked.
    for (what, change) in [
        ("another host", r#""https://cdn.tide.example/{version}/"#),
        ("plain http", r#""https://tide.example/download", "find": "Version"#),
        ("a name before the host", r#""https://tide.example/api/latest""#),
    ] {
        let to = match what {
            "another host" => r#""https://evil.example/{version}/"#,
            "plain http" => r#""http://tide.example/download", "find": "Version"#,
            _ => r#""https://tide.example@evil.example/api/latest""#,
        };
        let recipe: Recipe = serde_json::from_str(&TIDE.replacen(change, to, 1)).unwrap();
        assert!(recipe.problem().is_some(), "{what}");
        assert!(recipe.run(&tide_web()).is_err(), "{what}");
    }
    // A host that only ends the same way is another host.
    assert!(!tide().allows("https://nottide.example/x") && tide().allows("https://a.b.tide.example/x"));

    // Written properly, but the maker's page now links somewhere else: that file is
    // left out and said to be missing, and the rest still count.
    let mut web = tide_web();
    web.pages
        .insert("https://tide.example/download", r#"Version 3.2.1 is out <a href="https://evil.example/Tide.dmg">"#);
    web.redirects.insert("https://tide.example/latest/deb", "https://evil.example/tide.deb");
    let found = tide().run(&web).unwrap();
    assert_eq!(found.files.len(), 2);
    assert!(found.files.iter().all(|file| !file.url.contains("evil")));
    assert_eq!(found.missing.len(), 2);
    assert!(found.missing[0].contains("not one of the recipe's hosts"), "{}", found.missing[0]);
}

#[test]
fn a_website_that_changed_is_said_plainly() {
    let mut web = tide_web();
    web.pages.insert("https://tide.example/download", "<p>Now on the app shop only</p>");
    let said = tide().run(&web).unwrap_err();
    assert!(said.starts_with("version: nothing at https://tide.example/download matches"), "{said}");

    // What is found must look like a version: it goes into addresses and file names.
    web.pages.insert("https://tide.example/download", "Version ../../etc is out");
    let loose = TIDE.replace("Version ([0-9.]+) is out", "Version (.+) is out");
    let recipe: Recipe = serde_json::from_str(&loose).unwrap();
    assert!(recipe.run(&web).unwrap_err().contains("is not a version"));

    // A checksum that is not one loses the file, not the whole app.
    let mut web = tide_web();
    web.pages.insert("https://tide.example/sums/3.2.1.txt", "see our forum");
    let found = tide().run(&web).unwrap();
    assert_eq!((found.files.len(), found.missing.len()), (3, 1));
}

#[test]
fn what_is_wrong_with_a_recipe_is_said_without_fetching_anything() {
    let wrong = |change: &str, to: &str| -> String {
        assert!(TIDE.contains(change), "{change}");
        serde_json::from_str::<Recipe>(&TIDE.replacen(change, to, 1)).unwrap().problem().unwrap_or_default()
    };
    assert!(wrong(r#""id": "tide""#, r#""id": "Tide App""#).contains("id"));
    assert!(wrong(r#"["tide.example"]"#, r#"["example"]"#).contains("not a host name"));
    assert!(wrong(r#"["tide.example"]"#, "[]").contains("hosts"));
    assert!(wrong("Version ([0-9.]+) is out", "Version [0-9.]+ is out").contains("group in brackets"));
    assert!(wrong("Version ([0-9.]+) is out", "Version ([0-9.+ is out").contains("regular expression"));
    assert!(wrong(r#""at": "files.1.url""#, r#""find": "(x)""#).contains("needs \"at\""));
    assert!(wrong(r#""name": "tide_{version}_amd64.deb""#, r#""name": "../tide.deb""#).contains("plain file name"));
    assert!(wrong(r#""id": "tide""#, r#""id": "tide", "format": 2"#).contains("form 2"));
    // A lookup with a word this Bilby does not know is not a recipe at all, so that a
    // step added later is never quietly skipped.
    assert!(
        serde_json::from_str::<Recipe>(&TIDE.replace(r#""at": "files.1.url""#, r#""at": "a", "run": "rm""#)).is_err()
    );
}

#[test]
fn a_published_list_keeps_what_this_bilby_can_read_and_drops_the_rest() {
    let newer = TIDE.replacen(r#""id": "tide""#, r#""id": "newer", "format": 99, "steps": []"#, 1);
    let list = format!(r#"{{ "format": 1, "recipes": [ {TIDE}, {newer}, {{ "id": "broken" }}, {TIDE} ] }}"#);
    let read = shelf(&list).unwrap();
    assert_eq!(
        read.iter().map(|recipe| recipe.id.as_str()).collect::<Vec<_>>(),
        ["tide"],
        "once, and only the good one"
    );
    assert!(shelf("<html>Not found</html>").is_none());
    // What is written out is read back the same.
    assert_eq!(shelf(&as_shelf(&read)).unwrap(), read);
    // The list that comes with Bilby is all usable, and each id is there once.
    let bundled = bundled();
    let raw: Shelf = serde_json::from_str(include_str!("../../data/recipes.json")).unwrap();
    assert_eq!(bundled.len(), raw.recipes.len());
    assert!(!bundled.is_empty());
}

#[test]
fn a_folder_of_recipes_is_read_and_a_bad_file_is_named() {
    let folder = std::env::temp_dir().join(format!("bilby-recipes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("tide.json"), TIDE).unwrap();
    std::fs::write(folder.join("other-name.json"), TIDE).unwrap();
    std::fs::write(folder.join("notes.txt"), "not a recipe").unwrap();
    let (good, bad) = read_folder(&folder);
    assert_eq!(good.len(), 1);
    assert_eq!(bad.len(), 1);
    assert!(bad[0].1.contains("tide.json"), "{}", bad[0].1);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_page_that_lists_every_release_gives_the_newest() {
    let listing = r#"<a href="kelp-9.2.deb">9.2</a> <a href="kelp-10.0.deb">10.0</a> <a href="kelp-9.10.deb">9.10</a>"#;
    let mut web = tide_web();
    web.pages.insert("https://tide.example/files/", listing);
    let with = |pick: &str| -> String {
        let text = TIDE.replacen(
            r#"{ "page": "https://tide.example/download", "find": "Version ([0-9.]+) is out" }"#,
            &format!(r#"{{ "page": "https://tide.example/files/", "find": "kelp-([0-9.]+)\\.deb"{pick} }}"#),
            1,
        );
        let recipe: Recipe = serde_json::from_str(&text).unwrap();
        assert_eq!(recipe.problem(), None);
        recipe.look(&web, &recipe.version, None, false).unwrap()
    };
    assert_eq!(with(""), "9.2");
    assert_eq!(with(r#", "pick": "last""#), "9.10");
    // By what the versions mean, not by where they stand or how they spell.
    assert_eq!(with(r#", "pick": "newest""#), "10.0");
    let odd = TIDE.replacen(r#""find": "Version ([0-9.]+) is out""#, r#""find": "(x)", "pick": "best""#, 1);
    assert!(serde_json::from_str::<Recipe>(&odd).unwrap().problem().unwrap().contains("pick"));
}
