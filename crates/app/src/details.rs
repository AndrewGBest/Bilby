//! An app's own page: opening it, its description, versions and files.

use crate::cards::{at_the_top, find_card, option_row, tint_for, CATEGORY_CARD, SYSTEM_CARD};
use crate::gate::Guarded;
use crate::pictures::{decode_picture, decode_screenshot, picture_for, picture_sizes, PICTURES};
use crate::text::{initials, l, links_in, plain_about, plain_licence, plain_notes};
use crate::{App, AppCard, AppWindow, Current, HiddenRow, ReviewRow, Store};
use bilby_core::install::{self, Action};
use bilby_core::model::{age_label, AppDetails, AppId, SourceKind};
use bilby_core::state::{now_unix, Favourite};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// What an app's page fetches after it opens: the description, screenshots, the
/// project's logo, what people wrote, and where a review can be left.
pub(crate) type PageExtras = (
    String,
    Vec<slint::SharedPixelBuffer<slint::Rgba8Pixel>>,
    Option<PathBuf>,
    Vec<bilby_core::ratings::Review>,
    Vec<bilby_core::reviewing::Place>,
);

/// Put the links of what is being shown where a remote can pick them.
pub(crate) fn show_links(store: &Store) {
    let text = format!("{}\n{}", store.get_about(), store.get_notes());
    let links: Vec<SharedString> = links_in(&text).into_iter().map(SharedString::from).collect();
    store.set_about_links(ModelRc::new(VecModel::from(links)));
}

impl App {
    /// Install from a list. The app's page opens so what is happening can be seen and
    /// stopped, and the file that suits this device starts downloading as soon as it is
    /// known. Packages that are installed with a command just get their page.
    pub fn quick_install(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        // On the device already, from somewhere else, with an update waiting: that
        // update is what the button on its row means.
        if self.outside_update(id).is_some() {
            return self.update_app(ui, id);
        }
        *self.install_when_loaded.lock().unwrap() = Some(id.to_string());
        self.open(ui, id);
    }

    pub fn open(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        let store = ui.global::<Store>();
        if id == SYSTEM_CARD {
            store.set_details_open(false);
            store.set_page(4);
            return;
        }
        if let Some(title) = id.strip_prefix(CATEGORY_CARD) {
            return self.browse_category(ui, title, true);
        }
        // Something for adults: ask first, once, or for the PIN; never in Family mode.
        let listed_adult = find_card(&store, id).is_some_and(|card| card.adult)
            || id.strip_prefix("web:").and_then(bilby_core::source::web::listed).is_some_and(|site| site.adult)
            || self.known_adult.lock().unwrap().contains(id);
        if listed_adult {
            use bilby_core::parental::Gate;
            self.known_adult.lock().unwrap().insert(id.to_string());
            match self.gate_for_adult() {
                Gate::Open => {}
                Gate::Blocked => {
                    self.toast(ui, "This is not available while Family mode is on.", true);
                    return;
                }
                Gate::ConfirmAdult => {
                    let waiting = Guarded::OpenAdult(id.to_string());
                    self.ask(ui, 1, "Adult content", "This is meant for adults. Are you 18 or over?", waiting);
                    return;
                }
                Gate::Pin => {
                    let waiting = Guarded::OpenAdult(id.to_string());
                    self.ask(
                        ui,
                        2,
                        "Enter the PIN",
                        "This is meant for adults, and a PIN guards it on this device.",
                        waiting,
                    );
                    return;
                }
            }
        }
        let generation = self.details_generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Show what the list already knew while the releases load.
        let known = find_card(&store, id);
        store.set_is_favourite(self.state().is_favourite(id));
        store.set_can_open(self.launchable(id).is_some());
        store.set_in_library(self.state.lock().unwrap().installed(id).is_some());
        store.set_can_uninstall(false);
        store.set_outside_update(false);
        store.set_details_open(true);
        store.set_details_error(SharedString::new());
        store.set_unavailable(SharedString::new());
        store.set_warning(SharedString::new());
        store.set_notes(SharedString::new());
        show_links(&store);
        store.set_about(SharedString::new());
        show_links(&store);
        store.set_screenshots(ModelRc::default());
        store.set_picture(-1);
        store.set_reviews(ModelRc::default());
        self.show_review_places(ui, Vec::new());
        store.set_release_date(SharedString::new());
        store.set_release_age(SharedString::new());
        store.set_license(SharedString::new());
        store.set_homepage(SharedString::new());
        store.set_source_page(SharedString::new());
        store.set_options(ModelRc::default());
        store.set_hidden(ModelRc::default());
        // Still being downloaded from an earlier visit to this page: it shows.
        store.set_install_state(if self.installing_is(id) { 1 } else { 0 });
        store.set_queued(self.is_queued(id));
        store.set_installed_version(
            self.state.lock().unwrap().installed(id).map(|a| a.version.clone()).unwrap_or_default().into(),
        );
        store.set_versions(ModelRc::default());
        store.set_selected_version(0);
        *self.current.lock().unwrap() = Current { id: id.to_string(), details: None, selected: None };
        let mut card = known.unwrap_or_else(|| {
            let name = id.rsplit(['/', ':']).next().unwrap_or(id);
            AppCard {
                id: id.into(),
                name: name.into(),
                source: AppId::parse(id).map(|parsed| parsed.source.label()).unwrap_or_default().into(),
                initials: initials(name).into(),
                tint: tint_for(id),
                icon: picture_for(id),
                ..Default::default()
            }
        });
        // Library cards carry "old → new" as their version; the page shows the release's.
        card.version = SharedString::new();
        store.set_app(at_the_top(card));
        // What its last install did, folded away until asked for.
        store.set_log_open(false);
        self.show_install_log(ui);
        // Known before anything is fetched for a Flatpak or a Steam game; for a
        // system package, once its page has loaded.
        store.set_can_uninstall(self.can_uninstall(&self.state(), &store));
        self.show_outside_update(&store);
        self.show_app_sources(ui);
        let Some(app_id) = AppId::parse(id) else {
            store.set_details_loading(false);
            store.set_details_error(l(format!("“{id}” is not an app id.")));
            return;
        };
        store.set_details_loading(true);
        let about_id = app_id.clone();
        // Taken now, so it applies to this opening only and never to a later visit.
        let auto_install = self.install_when_loaded.lock().unwrap().take();
        self.job(
            move |app| app.catalog().details(&app_id).map_err(|e| e.to_string()),
            move |app, ui, result| {
                if app.details_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                match result {
                    Ok(details) => {
                        let id = details.summary.id.to_string();
                        app.show_details(ui, details);
                        app.load_about(about_id, generation);
                        // Asked for from a list: carry on to the install.
                        let direct = {
                            let current = app.current.lock().unwrap();
                            current
                                .release()
                                .and_then(|r| r.options.first())
                                .is_some_and(|o| o.plan.action != Action::ShowCommand)
                        };
                        if auto_install.as_deref() == Some(id.as_str()) && direct {
                            app.install(ui, 0);
                        }
                    }
                    Err(message) => {
                        let store = ui.global::<Store>();
                        store.set_details_loading(false);
                        store.set_details_error(l(message));
                    }
                }
            },
        );
    }

    /// Fetch the project's own description once the page is usable. It is extra: if it
    /// cannot be had (no README, offline, out of requests) the page simply has no
    /// About section.
    pub(crate) fn load_about(self: &Arc<Self>, id: AppId, generation: u64) {
        let key = bilby_core::icons::app_key(&id);
        let logo_known = PICTURES.with(|p| p.borrow().contains_key(&key));
        self.job(
            move |app| {
                let catalog = app.catalog();
                let http = catalog.http();
                let readme = catalog.about(&id).ok().flatten().unwrap_or_default();
                // Pictures are fetched and decoded here, off the interface's thread. The
                // source's own screenshots first (Flathub has them), else the README's.
                let mut urls = catalog.pictures(&id);
                if urls.is_empty() {
                    urls = bilby_core::screenshots::from_readme(&readme, &id);
                }
                let dir = app.dirs.cache.join("screenshots");
                let pictures = urls
                    .iter()
                    .filter_map(|url| bilby_core::screenshots::fetch(http.as_ref(), &dir, url).ok().flatten())
                    .filter_map(|path| decode_screenshot(&path))
                    .collect();
                // The project's own logo: what its README shows, else the usual places.
                let mut logo = None;
                if !logo_known && id.source == SourceKind::GitHub {
                    let icons = app.dirs.cache.join("icons");
                    let key = bilby_core::icons::app_key(&id);
                    let places = bilby_core::screenshots::logo_from_readme(&readme, &id)
                        .into_iter()
                        .chain(bilby_core::screenshots::usual_logo_places(&id));
                    for (n, url) in places.enumerate() {
                        // Each place is remembered separately, so a miss is not asked again.
                        let attempt = format!("{key}-try{n}");
                        if let Ok(Some(path)) =
                            bilby_core::icons::fetch_picture(http.as_ref(), &icons, &attempt, &url, 2 * 1024 * 1024)
                        {
                            // Kept under the app's own name, where lists look for it.
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_string();
                            let kept = icons.join(format!("{key}.{ext}"));
                            if std::fs::copy(&path, &kept).is_ok() {
                                logo = Some(kept);
                            }
                            break;
                        }
                    }
                }
                // What people wrote, for apps the ratings service knows; none otherwise.
                // Which apps it knows comes from the ratings file, so that is read first
                // (a page opened right at start-up would otherwise find none).
                if !app.state().settings.hide_ratings {
                    catalog.load_ratings(&app.dirs.cache);
                }
                let reviews = catalog.reviews(&id);
                let places = app.review_places_for(&catalog, &id);
                (plain_about(&readme), pictures, logo, reviews, places)
            },
            move |app, ui, (about, pictures, logo, reviews, places): PageExtras| {
                if let Some(image) = logo.as_deref().and_then(|path| decode_picture(path, picture_sizes(ui))) {
                    PICTURES.with(|p| p.borrow_mut().insert(key.clone(), image));
                    app.repaint_pictures(ui);
                }
                if app.details_generation.load(Ordering::SeqCst) == generation {
                    let store = ui.global::<Store>();
                    store.set_about(about.into());
                    show_links(&store);
                    let images: Vec<slint::Image> = pictures.into_iter().map(slint::Image::from_rgba8).collect();
                    store.set_screenshots(ModelRc::new(VecModel::from(images)));
                    let rows: Vec<ReviewRow> = reviews
                        .iter()
                        .map(|r| {
                            let mut meta: Vec<String> = Vec::new();
                            if !r.author.is_empty() {
                                meta.push(r.author.clone());
                            }
                            if !r.date.is_empty() {
                                meta.push(r.date.clone());
                            }
                            if !r.version.is_empty() {
                                meta.push(format!("version {}", r.version));
                            }
                            ReviewRow {
                                stars: format!("{}{}", "★".repeat(r.stars as usize), "☆".repeat(5 - r.stars as usize))
                                    .into(),
                                title: l(r.title.as_str()),
                                text: r.text.as_str().into(),
                                meta: meta.join("  ·  ").into(),
                            }
                        })
                        .collect();
                    store.set_reviews(ModelRc::new(VecModel::from(rows)));
                    app.show_review_places(ui, places);
                    let mut card = store.get_app();
                    card.icon = picture_for(&card.id);
                    store.set_app(at_the_top(card));
                }
            },
        );
    }

    pub(crate) fn show_details(&self, ui: &AppWindow, details: AppDetails) {
        let store = ui.global::<Store>();
        store.set_details_loading(false);
        // The list's display name ("Tidewave") reads better than the repository's ("tidewave").
        // Something for adults that was opened by its link, not chosen from a covered
        // tile: it is closed again, and the question is asked when it is next opened.
        if details.summary.adult && self.gate_for_adult() != bilby_core::parental::Gate::Open {
            self.known_adult.lock().unwrap().insert(details.summary.id.to_string());
            store.set_details_open(false);
            store.invoke_open(details.summary.id.to_string().into());
            return;
        }
        let mut card = self.card_open(&details.summary);
        let shown = store.get_app();
        if shown.id == card.id && !shown.name.is_empty() && !shown.author.is_empty() && !shown.adult {
            card.name = shown.name;
            card.initials = shown.initials;
            // What the list already knew is kept when the app's own page does not repeat it.
            if card.popularity.is_empty() {
                card.popularity = shown.popularity;
            }
            if card.headline.is_empty() {
                card.headline = shown.headline;
            }
            card.verified |= shown.verified;
        }
        store.set_app(at_the_top(card));
        store.set_license(plain_licence(details.summary.license.as_deref().unwrap_or_default()).into());
        store.set_homepage(details.summary.homepage.as_str().into());
        // The app's own page where it is downloaded from, always a press away.
        let distro = self.platform.read().unwrap().linux.as_ref().map(|linux| linux.distro.clone()).unwrap_or_default();
        let source_page = bilby_core::model::source_page(&details.summary.id, &distro).or_else(|| {
            // A store whose pages go by a name: the page its own entry gives.
            let page = details.summary.homepage.clone();
            (details.summary.id.source == SourceKind::Gog && install::store_link_ok(&page)).then_some(page)
        });
        store.set_source_page(source_page.unwrap_or_default().into());
        store.set_unavailable(l(details.unavailable_reason.clone().unwrap_or_default()));
        store.set_versions(ModelRc::new(VecModel::from(
            details.versions.iter().map(|r| SharedString::from(&r.version)).collect::<Vec<_>>(),
        )));
        // Installed with a command the person runs: AUR and repository packages. Flathub
        // apps are installed for them, so they read like any other app.
        store.set_is_package(matches!(
            details.summary.id.source,
            SourceKind::Aur
                | SourceKind::Pacman
                | SourceKind::Apt
                | SourceKind::Dnf
                | SourceKind::Zypper
                | SourceKind::Web
                | SourceKind::Snap
                | SourceKind::Winget
        ));
        store.set_is_website(details.summary.id.source == SourceKind::Web);
        // What the system's package manager has, when the store's own library has nothing.
        if store.get_installed_version().is_empty() {
            store.set_installed_version(details.summary.installed_version.clone().unwrap_or_default().into());
        }
        store.set_can_uninstall(self.can_uninstall(&self.state(), &store));
        self.show_outside_update(&store);
        // Start on the release the catalog picked (the newest stable one that fits).
        let picked = details.release.as_ref().map(|r| r.version.clone());
        let selected = details.versions.iter().position(|r| Some(&r.version) == picked.as_ref());
        {
            let mut current = self.current.lock().unwrap();
            current.details = Some(details);
            current.selected = selected;
        }
        self.show_release(ui);
    }

    /// Fill the page from the release that is currently selected.
    pub(crate) fn show_release(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let current = self.current.lock().unwrap();
        let Some(details) = current.details.as_ref() else { return };
        let release = current.release();
        let (options, hidden, notes, date) = match release {
            Some(r) => (r.options.clone(), r.hidden.clone(), plain_notes(&r.notes), r.published.clone()),
            None => (vec![], vec![], String::new(), String::new()),
        };
        store.set_selected_version(current.selected.unwrap_or(0) as i32);
        let mut card = store.get_app();
        card.version = release.map(|r| r.version.as_str()).unwrap_or_default().into();
        card.badge = options.first().map(|o| o.label()).unwrap_or_default().into();
        store.set_app(at_the_top(card));
        let shown = store.get_app().id.to_string();
        store.set_install_state(if self.installing_is(&shown) { 1 } else { 0 });
        store.set_options(ModelRc::new(VecModel::from(options.iter().map(option_row).collect::<Vec<_>>())));
        // Now that the kind of file is known: does this system have what it needs?
        self.show_app_sources(ui);
        store.set_hidden(ModelRc::new(VecModel::from(
            hidden
                .iter()
                .map(|h| HiddenRow { file: h.name.clone().into(), reason: h.reason.clone().into() })
                .collect::<Vec<_>>(),
        )));
        store.set_notes(notes.into());
        show_links(&store);
        // Things worth knowing before installing, most important first.
        let age = age_label(&date, now_unix());
        let mut warnings: Vec<String> = details.summary.warning.clone().into_iter().collect();
        if details.summary.id.source == SourceKind::Web
            && self.opens_sites_in_longboard()
            && install::find_longboard(&self.dirs).is_none()
        {
            warnings.push("Websites open in Longboard, a browser for TVs. It is not on this TV yet, so Bilby installs it along with this.".to_string());
        }
        let newest = details.versions.first().map(|r| r.version.as_str());
        let is_newest = release.map(|r| r.version.as_str()) == newest;
        if let (false, Some(newest)) = (is_newest, newest) {
            warnings.push(format!("This is an older version. The newest for this device is {newest}."));
        }
        if release.is_some_and(|r| r.prerelease && !r.options.is_empty()) {
            warnings.push("This is a pre-release. It may be unfinished or unstable.".to_string());
        }
        if let (true, Some(years)) = (is_newest, age.strip_suffix(" years ago").and_then(|n| n.parse::<u32>().ok())) {
            warnings.push(format!("The latest release is {years} years old. The project may no longer be maintained."));
        }
        store.set_warning(l(warnings.join("\n")));
        store.set_release_age(l(age));
        store.set_release_date(date.into());
    }

    pub fn select_version(&self, ui: &AppWindow, index: i32) {
        {
            let mut current = self.current.lock().unwrap();
            let count = current.details.as_ref().map(|d| d.versions.len()).unwrap_or(0);
            if index < 0 || index as usize >= count || ui.global::<Store>().get_install_state() == 1 {
                return;
            }
            current.selected = Some(index as usize);
        }
        self.show_release(ui);
    }

    pub fn close_details(&self, ui: &AppWindow) {
        // Invalidate any details request still in flight.
        self.details_generation.fetch_add(1, Ordering::SeqCst);
        ui.global::<Store>().set_details_open(false);
    }

    /// Add the open app to the favourites, or take it out.
    pub fn toggle_favourite(&self, ui: &AppWindow) {
        let card = ui.global::<Store>().get_app();
        if AppId::parse(card.id.as_str()).is_none() {
            return;
        }
        let favourite = Favourite {
            id: card.id.to_string(),
            name: card.name.to_string(),
            author: card.author.to_string(),
            summary: card.summary.to_string(),
        };
        let mut added = false;
        self.update_state(ui, |state| added = state.toggle_favourite(favourite));
        self.refresh(ui);
        let message = if added { "Added to favourites. Find them in your library." } else { "Removed from favourites" };
        self.toast(ui, message, false);
    }
}
