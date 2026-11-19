//! What a person sees when the network misbehaves: offline, rate limited, then back.
//! The demo catalog is wrapped in a transport whose mood the test controls.

use slint::{ComponentHandle, Model};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use untitledstore::{build, App, AppWindow, CatalogFactory, Options, Shell, Store};
use ustore_core::demo::{self, DemoHttp};
use ustore_core::http::{Http, Quota};
use ustore_core::install::Dirs;
use ustore_core::{Catalog, Error, Platform, Result};

const ONLINE: u8 = 0;
const OFFLINE: u8 = 1;
const RATE_LIMITED: u8 = 2;

struct Moody {
    mood: Arc<AtomicU8>,
}

impl Moody {
    fn check(&self) -> Result<()> {
        match self.mood.load(Ordering::SeqCst) {
            OFFLINE => {
                Err(Error::Network("couldn't connect to api.github.com. Check your internet connection.".into()))
            }
            RATE_LIMITED => Err(Error::RateLimited {
                source: "GitHub",
                hint: "Add a GitHub token in settings to raise the limit, or try again later.".into(),
            }),
            _ => Ok(()),
        }
    }
}

impl Http for Moody {
    fn get(&self, url: &str) -> Result<String> {
        self.check()?;
        DemoHttp.get(url)
    }
    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
        self.check()?;
        DemoHttp.download(url, dest, progress)
    }
    fn quota(&self) -> Option<Quota> {
        let remaining = if self.mood.load(Ordering::SeqCst) == RATE_LIMITED { 0 } else { 37 };
        Some(Quota { remaining, limit: 60, resets_at: u64::MAX })
    }
}

struct Session {
    ui: AppWindow,
    app: Arc<App>,
    mood: Arc<AtomicU8>,
    dirs: Dirs,
}

fn start(seed: &str) -> Session {
    i_slint_backend_testing::init_no_event_loop();
    demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("ustore-offline-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&root);
    let dirs = Dirs {
        downloads: root.join("downloads"),
        applications: root.join("Applications"),
        cache: root.join("cache"),
        config: root.join("config"),
    };
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), seed).unwrap();
    let mood = Arc::new(AtomicU8::new(ONLINE));
    let factory_mood = mood.clone();
    let factory: CatalogFactory = Arc::new(move |platform, _settings, _dirs| {
        let http: Arc<dyn Http> = Arc::new(Moody { mood: factory_mood.clone() });
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
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.show().unwrap();
    Session { ui, app, mood, dirs }
}

impl Session {
    fn store(&self) -> Store<'_> {
        self.ui.global::<Store>()
    }
    fn go(&self, mood: u8) {
        self.mood.store(mood, Ordering::SeqCst);
    }
}

const WELCOMED: &str = r#"{"settings":{"welcomed":true}}"#;

#[test]
fn offline_browsing_still_works_and_lookups_explain_themselves() {
    let s = start(WELCOMED);
    s.go(OFFLINE);

    // Discover and suggestions come from the bundled catalog: no network needed.
    s.app.refresh(&s.ui);
    assert!(s.store().get_sections().row_count() >= 3);
    s.app.typing(&s.ui, "lum");
    assert_eq!(s.store().get_suggestions().row_count(), 1);

    // An app page says what is wrong, in words, and offers a retry.
    s.app.open(&s.ui, "github:lumen-labs/lumen");
    assert!(s.store().get_details_open());
    assert!(!s.store().get_details_loading());
    assert_eq!(s.store().get_details_error(), "couldn't connect to api.github.com. Check your internet connection.");
    assert_eq!(s.store().get_app().name, "Lumen", "what the catalog already knew is still shown");
    assert_eq!(s.store().get_options().row_count(), 0);

    // Back online: the same retry now loads the page.
    s.go(ONLINE);
    s.store().invoke_retry_details();
    assert_eq!(s.store().get_details_error(), "");
    assert_eq!(s.store().get_options().row_data(0).unwrap().label, "Debian package · x86_64");
}

#[test]
fn a_failed_search_keeps_the_page_usable() {
    let s = start(WELCOMED);
    s.go(OFFLINE);
    s.app.search(&s.ui, "editor");
    assert!(!s.store().get_searching(), "the spinner stops");
    assert!(s.store().get_searched());
    assert_eq!(s.store().get_results().row_count(), 0);
    assert_eq!(
        s.store().get_search_error(),
        "GitHub search failed: couldn't connect to api.github.com. Check your internet connection."
    );
    assert_eq!(s.store().get_search_note(), "", "no claim about results when the search did not happen");

    s.go(ONLINE);
    s.app.search(&s.ui, "editor");
    assert_eq!(s.store().get_search_error(), "");
    assert_eq!(s.store().get_results().row_count(), 1);
}

#[test]
fn rate_limiting_says_how_to_get_out_of_it() {
    let s = start(WELCOMED);
    s.go(RATE_LIMITED);
    s.app.search(&s.ui, "editor");
    assert_eq!(
        s.store().get_search_error(),
        "GitHub's request limit for this device is used up for now. Add a GitHub token in settings to raise the limit, or try again later."
    );
    // Settings shows the allowance GitHub last reported.
    assert!(s.store().get_quota_text().starts_with("0 of 60 GitHub requests left"), "{}", s.store().get_quota_text());

    s.go(ONLINE);
    s.app.search(&s.ui, "editor");
    assert!(s.store().get_quota_text().starts_with("37 of 60 GitHub requests left"));
}

#[test]
fn an_update_check_without_network_marks_apps_as_unchecked_not_current() {
    let seed = r#"{"settings":{"welcomed":true},"library":[{"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    let s = start(seed);
    s.go(OFFLINE);
    s.app.check_updates(&s.ui, true);
    let row = s.store().get_library().row_data(0).unwrap();
    assert_eq!(row.status, 4, "not 'up to date': the store does not know");
    assert!(row.status_text.starts_with("Couldn't check: couldn't connect"));
    assert_eq!(s.store().get_updates_count(), 0);
    assert!(s.store().get_toast_is_error());
    assert!(!s.store().get_checking_updates());

    s.go(ONLINE);
    s.app.check_updates(&s.ui, true);
    assert_eq!(s.store().get_library().row_data(0).unwrap().status, 3);
    assert_eq!(s.store().get_toast(), "1 update available");
}

#[test]
fn an_update_that_fails_is_reported_and_can_be_repeated() {
    let seed = r#"{"settings":{"welcomed":true},"library":[{"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    let s = start(seed);
    s.app.check_updates(&s.ui, false);
    assert_eq!(s.store().get_updates_count(), 1);

    s.go(OFFLINE);
    s.app.update_app(&s.ui, "github:kestrel-sh/kestrel");
    assert!(
        s.store().get_toast().starts_with("Couldn't update Kestrel: couldn't connect"),
        "{}",
        s.store().get_toast()
    );
    assert!(s.store().get_toast_is_error());
    assert_eq!(s.store().get_updating_id(), "", "the row is not left spinning");
    assert_eq!(s.store().get_updates_count(), 1, "the update is still on offer");

    s.go(ONLINE);
    s.app.update_app(&s.ui, "github:kestrel-sh/kestrel");
    assert_eq!(s.store().get_toast(), "Kestrel updated to v1.21.0");
    assert_eq!(s.store().get_updates_count(), 0);
}

#[test]
fn update_all_offline_reports_and_leaves_everything_on_offer() {
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"},
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.0.0"}]}"#;
    let s = start(seed);
    s.app.check_updates(&s.ui, false);
    assert_eq!(s.store().get_updates_count(), 2);
    s.go(OFFLINE);
    s.app.update_all(&s.ui);
    assert!(s.store().get_toast_is_error());
    assert_eq!(s.store().get_updating_id(), "", "the queue ran dry instead of hanging");
    assert_eq!(s.store().get_updates_count(), 2);
    s.go(ONLINE);
    s.app.update_all(&s.ui);
    assert_eq!(s.store().get_updates_count(), 0);
}

#[test]
fn a_download_that_fails_leaves_the_library_alone() {
    let s = start(WELCOMED);
    s.app.open(&s.ui, "github:tidewave/tidewave");
    assert_eq!(s.store().get_options().row_count(), 3);
    // The connection drops between loading the page and pressing Install.
    s.go(OFFLINE);
    s.app.install(&s.ui, 1);
    assert_eq!(s.store().get_install_state(), 3);
    assert!(s.store().get_install_message().contains("Check your internet connection"));
    assert_eq!(s.store().get_library().row_count(), 0);
    assert_eq!(s.store().get_installed_version(), "");
    assert!(!s.dirs.applications.exists() || std::fs::read_dir(&s.dirs.applications).unwrap().next().is_none());

    // Trying again once the connection is back works from the same page.
    s.go(ONLINE);
    s.app.install(&s.ui, 1);
    assert_eq!(s.store().get_install_state(), 2);
    assert_eq!(s.store().get_library().row_count(), 1);
}
