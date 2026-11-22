//! App pictures: fetched in the background, shown when they arrive, cached on disk,
//! and never in the way when they do not exist.

use bilby::{build, App, AppWindow, CatalogFactory, Options, Shell, Store};
use bilby_core::demo::{self, DemoHttp};
use bilby_core::http::Http;
use bilby_core::install::Dirs;
use bilby_core::{Catalog, Error, Platform, Result};
use slint::{ComponentHandle, Model};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A valid 8x8 PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 8, 0, 0, 0, 8, 8, 6, 0, 0, 0, 196, 15, 190,
    139, 0, 0, 0, 18, 73, 68, 65, 84, 120, 218, 99, 208, 168, 184, 243, 31, 31, 102, 24, 25, 10, 0, 131, 167, 158, 193,
    170, 214, 76, 33, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// The demo catalog, plus pictures for some owners.
struct WithPictures {
    downloads: Arc<AtomicUsize>,
}

impl Http for WithPictures {
    fn get(&self, url: &str) -> Result<String> {
        DemoHttp.get(url)
    }
    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
        // Two projects have no logo of their own either.
        let bare = ["lumen-labs", "kestrel-sh"];
        if bare.iter().any(|owner| url.starts_with(&format!("https://raw.githubusercontent.com/{owner}/"))) {
            return Err(Error::NotFound(url.to_string()));
        }
        let Some(owner) = url.strip_prefix("https://github.com/").and_then(|rest| rest.strip_suffix(".png?size=128"))
        else {
            return DemoHttp.download(url, dest, progress);
        };
        self.downloads.fetch_add(1, Ordering::SeqCst);
        match owner {
            // One owner has no picture, one serves something that is not an image.
            "lumen-labs" => Err(Error::NotFound(url.to_string())),
            "kestrel-sh" => {
                std::fs::write(dest, b"<html>rate limited</html>")?;
                Ok(String::new())
            }
            _ => {
                std::fs::write(dest, PNG)?;
                Ok(String::new())
            }
        }
    }
}

struct Session {
    ui: AppWindow,
    app: Arc<App>,
    downloads: Arc<AtomicUsize>,
    dirs: Dirs,
}

fn start(root: &Path) -> Session {
    demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
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
    };
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
    let downloads = Arc::new(AtomicUsize::new(0));
    let counter = downloads.clone();
    let factory: CatalogFactory = Arc::new(move |platform, _settings, _dirs| {
        let http: Arc<dyn Http> = Arc::new(WithPictures { downloads: counter.clone() });
        Arc::new(Catalog::new(platform.clone(), http, demo::index()))
    });
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-x64-debian"),
        shell: Some(Shell::Desktop),
        dirs: Some(dirs.clone()),
        opener: Some(Arc::new(|_| Ok(()))),
        catalog_factory: Some(factory),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    ui.show().unwrap();
    Session { ui, app, downloads, dirs }
}

fn icon_width(s: &Session, name: &str) -> u32 {
    let store = s.ui.global::<Store>();
    let card = store
        .get_sections()
        .iter()
        .flat_map(|section| section.apps.iter().collect::<Vec<_>>())
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} is not on Discover"));
    card.icon.size().width
}

#[test]
fn pictures_appear_are_cached_and_their_absence_is_harmless() {
    i_slint_backend_testing::init_no_event_loop();
    let root = std::env::temp_dir().join(format!("bilby-pictures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let s = start(&root);
    // Featured apps got their own logos as soon as the store opened, without anyone
    // opening their pages: what each project's README shows. Whatever size a picture
    // arrives at, it is kept at exactly the sizes it is drawn at: 44 in a tile, 88 at
    // the top of a page. (In the demo the logos are 128 pixels and an owner's picture
    // is 8; which one was used is checked on disk further down.)
    assert_eq!(icon_width(&s, "Tidewave"), 44);
    assert_eq!(icon_width(&s, "Relay"), 44);
    // No logo and no picture, or not an image: the initials stay, nothing breaks.
    assert_eq!(icon_width(&s, "Lumen"), 0);
    assert_eq!(icon_width(&s, "Kestrel"), 0);
    let first_round = s.downloads.load(Ordering::SeqCst);
    assert_eq!(first_round, 8, "one request per owner on screen");

    // An app's page shows the same logo.
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert_eq!(s.ui.global::<Store>().get_app().icon.size().width, 88);
    assert_eq!(icon_width(&s, "Tidewave"), 44);
    s.app.search(&s.ui, "editor");
    assert_eq!(s.downloads.load(Ordering::SeqCst), first_round, "nothing is fetched twice in a session");

    // An app that was not on screen before gets its picture when it is opened.
    s.app.open(&s.ui, "github:oldtimer/relic");
    assert_eq!(s.ui.global::<Store>().get_app().icon.size().width, 88);
    assert_eq!(s.downloads.load(Ordering::SeqCst), first_round + 1);

    // On disk: pictures under the owner's name, and a note for owners without one.
    let icons = s.dirs.cache.join("icons");
    assert!(icons.join("tidewave.png").is_file());
    assert!(icons.join("app-github-tidewave-tidewave.png").is_file(), "the project's logo, under the app's name");
    assert!(icons.join("lumen-labs.none").is_file());
    assert!(icons.join("kestrel-sh.none").is_file());

    // A second launch reads the cache: no requests at all, pictures still there.
    let again = start(&root);
    assert_eq!(icon_width(&again, "Tidewave"), 44, "the logo found last time is still used");
    assert_eq!(icon_width(&again, "Relay"), 44);
    assert_eq!(icon_width(&again, "Lumen"), 0);
    assert_eq!(again.downloads.load(Ordering::SeqCst), 0);

    // "Clear cached catalog data" removes them.
    again.ui.global::<Store>().invoke_clear_cache();
    assert!(!icons.exists());
}
