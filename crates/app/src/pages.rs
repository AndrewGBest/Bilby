//! Filling the pages from the catalog and the saved state: home, the library and the
//! lists that load after the window is up.

use crate::cards::{at_the_top, spread_shares, tint_for, CATEGORY_CARD, SYSTEM_CARD};
use crate::pictures::picture_for;
use crate::text::{initials, kind_of, l, ratings_text};
use crate::{i18n, App, AppCard, AppWindow, LibraryRow, Section, SourceChoice, Store, TvChoice, VersionChoice};
use bilby_core::catalog::UpdateStatus;
use bilby_core::index::Section as CoreSection;
use bilby_core::install;
use bilby_core::model::{age_label, date_from_unix, AppId, AppSummary, SourceKind};
use bilby_core::platform::FormFactor;
use bilby_core::state::now_unix;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl App {
    /// Discover, the TV rows and the library all depend on the same data; redraw them
    /// together so "Installed" and "Update" markers never disagree between pages.
    pub fn refresh(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        // The bundled starter list first, then the lists the sources keep themselves.
        let mut featured = self.catalog().featured();
        // What a row above already shows is not shown again further down: the same
        // few well-known apps would otherwise fill the start of every list.
        let mut seen: HashSet<String> = featured.iter().flat_map(|s| &s.apps).map(|a| a.id.to_string()).collect();
        let on_tv = self.platform.read().unwrap().form_factor == FormFactor::Tv;
        for mut section in self.collections.lock().unwrap().iter().cloned() {
            section.apps.retain(|app| seen.insert(app.id.to_string()));
            if section.apps.is_empty() {
                continue;
            }
            // A source's list of one kind of app (Flathub's games) joins the row of
            // that kind, after the names everyone knows.
            match featured.iter_mut().find(|row| row.title == section.title) {
                Some(row) => row.apps.extend(section.apps),
                // Nobody writes code from the sofa: a TV gets no rows it did not have.
                None if on_tv && bilby_core::essentials::is_kind_row(&section.title) => {}
                None => {
                    // Rows of a kind stay together, ahead of "Web apps" and the
                    // sources' own lists of what is popular.
                    let at = match bilby_core::essentials::is_kind_row(&section.title) {
                        true => featured.iter().position(|row| row.title == "Web apps").unwrap_or(featured.len()),
                        false => featured.len(),
                    };
                    featured.insert(at, section);
                }
            }
        }
        // The category chosen to browse shows everything in it, not a home page's few.
        let browsing = self.browsing.lock().unwrap().clone();
        if let Some((title, apps)) = browsing.as_ref().filter(|(_, apps)| !apps.is_empty()) {
            if let Some(row) = featured.iter_mut().find(|row| row.title == *title) {
                row.apps = apps.clone();
            }
        }
        let to_sections = |list: &[CoreSection]| -> Vec<Section> {
            list.iter().map(|s| Section { title: l(&s.title), apps: self.cards(&s.apps) }).collect()
        };
        // Built once and shared with the TV's rows below: the same tiles, not a second set.
        let home = to_sections(&featured);
        store.set_sections(ModelRc::new(VecModel::from(home.clone())));
        // Set only when they differ: the chips are made again when this is, and the
        // one just pressed would lose the keyboard's focus.
        let categories: Vec<SharedString> = featured.iter().map(|s| l(&s.title)).collect();
        if store.get_categories().iter().ne(categories.iter().cloned()) {
            store.set_categories(ModelRc::new(VecModel::from(categories)));
        }
        if !featured.iter().any(|s| i18n::tr(&s.title) == store.get_category().as_str()) {
            store.set_category(SharedString::new());
        }
        store.set_spotlight(at_the_top(
            featured.first().and_then(|s| s.apps.first()).map(|a| self.card(a)).unwrap_or_default(),
        ));

        let state = self.state();
        let updates = self.updates.lock().unwrap().clone();
        let now = now_unix();
        let mut rows: Vec<LibraryRow> = state
            .library
            .iter()
            .map(|app| {
                // Entries that came from a backup or an older file have no install time.
                let date = if app.installed_at == 0 {
                    String::new()
                } else {
                    age_label(&date_from_unix(app.installed_at as i64), now)
                };
                // The kind of file is on the app's page; here the version and age are enough.
                let mut detail = app.version.clone();
                if !date.is_empty() {
                    detail.push_str(&format!("  ·  installed {date}"));
                }
                let (status, status_text) = match updates.get(&app.id) {
                    None => (0, String::new()),
                    Some(UpdateStatus::UpToDate) if !app.skipped_version.is_empty() => {
                        (2, format!("Up to date  ·  skipping {}", app.skipped_version))
                    }
                    Some(UpdateStatus::UpToDate) => (2, "Up to date".to_string()),
                    Some(UpdateStatus::Available { version, .. }) => (3, format!("Update available: {version}")),
                    Some(UpdateStatus::Unknown(why)) => (4, format!("Couldn't check: {why}")),
                };
                LibraryRow {
                    card: self.library_card(app),
                    detail: l(detail),
                    status,
                    status_text: l(status_text),
                    store_managed: app.store_managed,
                }
            })
            .collect();
        // Apps with an update waiting come first; otherwise most recently installed first.
        rows.sort_by_key(|r| r.status != 3);
        let update_count = rows.iter().filter(|r| r.status == 3).count();
        store.set_updates_count(update_count as i32);
        store.set_library(ModelRc::new(VecModel::from(rows)));
        let favourites: Vec<AppCard> = state.favourites.iter().map(|f| self.favourite_card(f)).collect();
        store.set_favourites(ModelRc::new(VecModel::from(favourites)));
        store.set_is_favourite(state.is_favourite(store.get_app().id.as_str()));
        store.set_can_open(self.launchable(store.get_app().id.as_str()).is_some());
        store.set_in_library(state.installed(store.get_app().id.as_str()).is_some());
        store.set_can_uninstall(self.can_uninstall(&state, &store));
        self.show_outside_update(&store);
        store.set_queued(self.is_queued(store.get_app().id.as_str()));
        let checked = self.last_checked.load(Ordering::SeqCst);
        store.set_last_checked(l(match now.saturating_sub(checked) {
            _ if checked == 0 => String::new(),
            s if s < 90 => "just now".to_string(),
            s if s < 3600 => format!("{} minutes ago", s / 60),
            s => format!("{} hours ago", s / 3600),
        }));
        store.set_recent(ModelRc::new(VecModel::from(
            state.recent_searches.iter().map(SharedString::from).collect::<Vec<_>>(),
        )));

        // Search results keep their markers in sync too.
        let (query, results) = self.results.lock().unwrap().clone();
        // The sources these results came from, for the filter under the search box.
        let mut found: Vec<(SourceKind, i32)> = Vec::new();
        for app in &results {
            match found.iter_mut().find(|(kind, _)| *kind == app.id.source) {
                Some((_, n)) => *n += 1,
                None => found.push((app.id.source, 1)),
            }
        }
        let filter = self.result_filter.lock().unwrap().filter(|kind| found.iter().any(|(k, _)| k == kind));
        store.set_source_filter(filter.map(|k| k.prefix()).unwrap_or_default().into());
        store.set_result_sources(ModelRc::new(VecModel::from(
            found
                .iter()
                .map(|(kind, n)| SourceChoice { id: kind.prefix().into(), label: l(kind.label()), on: true, count: *n })
                .collect::<Vec<_>>(),
        )));
        // One app from several sources is one result; its page offers the sources.
        let groups = bilby_core::merge::group(results.clone());
        let mut alternates = HashMap::new();
        for group in groups.iter().filter(|g| !g.others.is_empty()) {
            let members: Vec<(String, String)> =
                group.members().map(|m| (m.id.to_string(), bilby_core::merge::source_label(m))).collect();
            for member in group.members() {
                alternates.insert(member.id.to_string(), members.clone());
            }
        }
        *self.alternates.lock().unwrap() = alternates;
        let cards: Vec<AppCard> = match filter {
            // Narrowed to one source, each entry is shown as that source has it.
            Some(kind) => results.iter().filter(|app| app.id.source == kind).map(|app| self.card(app)).collect(),
            None => groups
                .iter()
                .map(|group| {
                    let mut card = self.card(&group.lead);
                    card.more_sources = group.others.len() as i32;
                    card
                })
                .collect(),
        };
        let mut cards = cards;
        if filter.is_none() {
            // A service was named ("youtube"): where the search found none of the ways
            // to use it, the first that works here is put at the head of the list. Its
            // page leads to the others.
            let available = self.catalog().available_sources();
            let families = bilby_core::related::families();
            if let Some(family) = bilby_core::related::family_named(families, &query) {
                let platform = self.platform.read().unwrap().clone();
                let here = family.here(&available, &platform);
                let found = |id: &str| results.iter().any(|app| app.id.to_string().eq_ignore_ascii_case(id));
                if let Some(first) = here.first().filter(|_| !here.iter().any(|m| found(&m.id))) {
                    if let Some(id) = AppId::parse(&first.id) {
                        let mut app = AppSummary::new(id, first.label.clone());
                        app.summary = first.note.clone();
                        cards.insert(0, self.card(&app));
                    }
                }
            }
        }
        // Everything for adults goes to the end of the list, covered.
        let (mut cards, adult): (Vec<AppCard>, Vec<AppCard>) = cards.into_iter().partition(|card| !card.adult);
        cards.extend(adult);
        let found = cards.clone();
        store.set_results(ModelRc::new(VecModel::from(cards)));
        self.show_app_sources(ui);

        // TV: the Library page has updates first, then the rest, then favourites.
        let section =
            |title: &str, cards: Vec<AppCard>| Section { title: l(title), apps: ModelRc::new(VecModel::from(cards)) };
        let (waiting, current): (Vec<AppCard>, Vec<AppCard>) =
            state.library.iter().map(|a| self.library_card(a)).partition(|card| card.has_update);
        let mut tv_library = Vec::new();
        // The device itself and the store come first: what they run and whether
        // either has an update, whoever installed what.
        let mut device = Vec::new();
        if store.get_system_supported() || !store.get_system_name().is_empty() {
            device.push(AppCard {
                id: SYSTEM_CARD.into(),
                name: if store.get_system_name().is_empty() {
                    store.get_device_name()
                } else {
                    store.get_system_name()
                },
                author: "This device's own software".into(),
                summary: store.get_system_headline(),
                has_update: store.get_system_state() == 3,
                installed: true,
                tint: tint_for(SYSTEM_CARD),
                // No picture of its own: its initials, like any app without one.
                initials: initials(&if store.get_system_name().is_empty() {
                    store.get_device_name()
                } else {
                    store.get_system_name()
                })
                .into(),
                ..Default::default()
            });
        }
        if !device.is_empty() {
            tv_library.push(section(&store.get_device_name(), device));
        }
        // Everything else on the device, however it got there. What has an update
        // waiting with the tool that installed it comes first, and says so.
        let mut others: Vec<AppCard> = self
            .elsewhere
            .read()
            .unwrap()
            .apps
            .iter()
            .filter(|(id, _)| state.installed(id).is_none())
            .map(|(id, name)| AppCard {
                id: id.as_str().into(),
                name: name.as_str().into(),
                source: AppId::parse(id).map(|id| id.source.label()).unwrap_or("").into(),
                author: AppId::parse(id).map(|id| id.source.label()).unwrap_or("").into(),
                installed: true,
                tint: tint_for(id),
                initials: initials(name).into(),
                icon: picture_for(id),
                ..Default::default()
            })
            .collect();
        for card in &mut others {
            if let Some(waiting) = self.outside_update(card.id.as_str()) {
                card.has_update = true;
                card.summary = crate::outside::change_of(&waiting).into();
            }
        }
        others.sort_by_key(|card| !card.has_update);
        // The desktop's and the phone's library list them too, under the store's own.
        store.set_also_installed(ModelRc::new(VecModel::from(others.clone())));
        // On a TV an update is an update, whoever installed the app.
        let (others_waiting, others): (Vec<AppCard>, Vec<AppCard>) =
            others.into_iter().partition(|card| card.has_update);
        let mut waiting = waiting;
        waiting.extend(others_waiting);
        if !waiting.is_empty() {
            tv_library.push(section("Updates available", waiting));
        }
        if !current.is_empty() {
            tv_library.push(section("Installed with Bilby", current));
        }
        if !others.is_empty() {
            tv_library.push(section("Also installed here", others));
        }
        if !state.favourites.is_empty() {
            tv_library.push(section("Favourites", state.favourites.iter().map(|f| self.favourite_card(f)).collect()));
        }
        store.set_tv_library_rows(ModelRc::new(VecModel::from(tv_library)));
        // The kinds of app there are, each one a way in to all of that kind. They have a
        // page of their own, behind a button at the top: they are not a row among the rows.
        let kinds: Vec<AppCard> = self
            .catalog()
            .categories()
            .iter()
            .map(|title| {
                let id = format!("{CATEGORY_CARD}{title}");
                AppCard { name: l(title), tint: tint_for(&id), id: id.into(), ..Default::default() }
            })
            .collect();
        if store.get_tv_categories().iter().ne(kinds.iter().cloned()) {
            store.set_tv_categories(ModelRc::new(VecModel::from(kinds)));
        }
        // TV home: search results, then the store.
        // TV: a search is the whole page. The likeliest few first, then the rest by
        // kind, so a game is not buried under packages or the other way round.
        let mut tv = Vec::new();
        let browsed = browsing.filter(|_| store.get_page() == 1 && found.is_empty());
        if store.get_searching() {
            // A search is under way: the page waits for it, empty, however often the
            // rest of the store is redrawn meanwhile (a picture arriving does that).
        } else if let Some((title, apps)) = browsed {
            // A category in full: row after row of it, under its name.
            let cards = self.cards(&apps);
            let all: Vec<AppCard> = cards.iter().collect();
            for (n, row) in all.chunks(8).enumerate() {
                tv.push(Section {
                    title: if n == 0 { l(&title) } else { SharedString::new() },
                    apps: ModelRc::new(VecModel::from(row.to_vec())),
                });
            }
        } else if found.is_empty() {
            tv.extend(home);
        } else {
            const TOP: usize = 6;
            // Things for adults are never among the top results or the other rows:
            // they have a row of their own, last, each one covered.
            let (adult, found): (Vec<AppCard>, Vec<AppCard>) = found.into_iter().partition(|card| card.adult);
            let top: Vec<AppCard> = found.iter().take(TOP).cloned().collect();
            if !top.is_empty() {
                tv.push(Section {
                    title: l(format!("Top results for “{query}”")),
                    apps: ModelRc::new(VecModel::from(top)),
                });
            }
            for kind in
                ["Apps", "Games", "Web apps", "From your own sources", "From developers", "System packages", "Adult"]
            {
                let of_kind: Vec<AppCard> = if kind == "Adult" {
                    adult.clone()
                } else {
                    found.iter().skip(TOP).filter(|card| kind_of(card.id.as_str()) == kind).take(24).cloned().collect()
                };
                if !of_kind.is_empty() {
                    tv.push(Section { title: l(kind), apps: ModelRc::new(VecModel::from(of_kind)) });
                }
            }
        }
        store.set_tv_rows(ModelRc::new(VecModel::from(tv)));
    }

    /// Show how much of GitHub's hourly allowance is left, once GitHub has told us.
    pub(crate) fn refresh_quota(&self, ui: &AppWindow) {
        let text = self.catalog().quota().map(|q| q.describe(now_unix())).unwrap_or_default();
        ui.global::<Store>().set_quota_text(l(text));
    }

    /// Put the open app's sources beside its button, when it has more than one.
    pub(crate) fn show_app_sources(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let id = store.get_app().id.to_string();
        let choices: Vec<SourceChoice> = self
            .alternates
            .lock()
            .unwrap()
            .get(&id)
            .map(|members| {
                members
                    .iter()
                    .map(|(app, label)| SourceChoice {
                        id: app.as_str().into(),
                        label: l(label.as_str()),
                        on: *app == id,
                        count: 0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        store.set_app_sources(ModelRc::new(VecModel::from(choices)));

        // The app as one version of a service (VacuumTube, of YouTube): the versions
        // this device can use, to choose between.
        let available = self.catalog().available_sources();
        let families = bilby_core::related::families();
        // The page may be showing the app from a source the list does not name (its AUR
        // package, say): then it is known by one of its other sources.
        let known_as = std::iter::once(id.clone())
            .chain(store.get_app_sources().iter().map(|source| source.id.to_string()))
            .find(|id| bilby_core::related::family_of(families, id).is_some())
            .unwrap_or_else(|| id.clone());
        let family = bilby_core::related::family_of(families, &known_as);
        // One entry per app, however many sources have it: the one being shown, or
        // the first this device can use.
        let device = self.platform.read().unwrap().clone();
        let here = family.map(|family| family.here(&available, &device)).unwrap_or_default();
        let showing = family
            .and_then(|family| family.members.iter().find(|m| m.id.eq_ignore_ascii_case(&known_as)))
            .map(|m| m.label.as_str());
        // The other ways to use the service: on the service's own app, everything
        // else; on an app someone else made, the service's own. Never the app being
        // shown, from whichever source: that is a choice of source, made elsewhere.
        let versions: Vec<VersionChoice> = family
            .map(|family| family.others(&known_as, &available, &device))
            .unwrap_or_default()
            .into_iter()
            .filter(|member| !store.get_app_sources().iter().any(|source| source.id.eq_ignore_ascii_case(&member.id)))
            .map(|member| VersionChoice {
                id: member.id.as_str().into(),
                name: member.label.as_str().into(),
                note: l(member.note.as_str()),
                official: member.official,
                on: false,
            })
            .collect();
        // The other sources of the app being shown, when a search has not already said.
        if store.get_app_sources().row_count() == 0 {
            let same: Vec<SourceChoice> = here
                .iter()
                .filter(|m| showing == Some(m.label.as_str()))
                .filter_map(|m| {
                    let source = AppId::parse(&m.id)?.source.label();
                    Some(SourceChoice {
                        id: m.id.as_str().into(),
                        label: l(source),
                        on: m.id.eq_ignore_ascii_case(&id),
                        count: 0,
                    })
                })
                .collect();
            if same.len() > 1 {
                store.set_app_sources(ModelRc::new(VecModel::from(same)));
            }
        }
        store.set_app_family_name(family.map(|f| f.name.as_str()).unwrap_or_default().into());
        store.set_app_family(ModelRc::new(VecModel::from(versions)));

        // A Steam game without Steam: offer Steam itself, from wherever this device gets it.
        let mut helper = (String::new(), String::new());
        if id.starts_with("steam:") {
            use bilby_core::source::steam;
            let platform = self.platform.read().unwrap().clone();
            let libraries = self.steam_libraries.clone().unwrap_or_else(|| steam::default_libraries(platform.os));
            if libraries.is_empty() {
                if let Some(steam_app) = steam::steam_itself(&platform, &available) {
                    helper = (steam_app.to_string(), "Install Steam".to_string());
                }
            }
        }
        // A web app on a computer whose browsers cannot give a website a window of its
        // own: said on its page, with a browser that can one press away. (A TV opens
        // them in Longboard, which is made for it.)
        let mut web_note = String::new();
        if id.starts_with("web:") && !self.opens_sites_in_longboard() && install::find_app_browser(&self.dirs).is_none()
        {
            let platform = self.platform.read().unwrap().clone();
            if let Some(browser) = install::app_browser_to_install(&platform, &available) {
                helper = (browser.to_string(), "Install Chromium".to_string());
                web_note = "Web apps open in a window of their own, like any other app, in a browser that can do that: Chrome, Chromium, Brave, Edge or Vivaldi. None of them is on this computer. Install Chromium for that, or add this anyway and it opens as a page in your usual browser.".to_string();
            }
        }
        // An AppImage on a system that lacks what AppImages need to start: said before
        // it is downloaded, with the package that provides it one press away.
        let mut need_note = String::new();
        let mut needed = None;
        // The store's own page: it is running, so its kind of file starts here. It is
        // never unpacked or given a requirement on the way to updating itself.
        let own_page = self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(&id));
        // Installed here as something other than an AppImage (its Flatpak, or already
        // unpacked): what runs needs no FUSE, so there is nothing to say about it.
        let mut another_kind = false;
        if helper.0.is_empty() && !own_page {
            let running = self.launchable(&id);
            let placed = running.as_ref().and_then(|path| bilby_core::needs::as_appimage(path));
            another_kind = running.is_some() && placed.is_none();
            let offered = store.get_options().row_data(0).is_some_and(|option| option.label.contains("AppImage"));
            if placed.is_some() || offered {
                let platform = self.platform.read().unwrap().clone();
                let catalog = self.catalog();
                let in_repositories = |package: &AppId| catalog.source_summary(package).is_ok();
                if let Some(need) = bilby_core::needs::for_appimage(
                    &platform,
                    *self.fuse.read().unwrap(),
                    placed.as_deref(),
                    &in_repositories,
                ) {
                    need_note = need.why;
                    // Installed along with the app, or just before it is opened: the
                    // person is told, not sent to fetch it.
                    needed = need.package;
                    // A read-only system takes it on top of its image, from a restart on.
                    if self.layering.is_some() {
                        needed = None;
                        need_note = match self.fuse_staged.load(Ordering::SeqCst) {
                            true => "FUSE 2, which this app needs to run, has been added to this system and starts working after a restart.".to_string(),
                            false => "This app needs FUSE 2 to run, which this system does not have. Bilby will add it to your system for you; it starts working after a restart, which Bilby will offer.".to_string(),
                        };
                    }
                }
            }
        }
        // Kept for installing: choosing the AppImage from the list of files still gets
        // what it needs. The notice itself is only for when the AppImage is the choice.
        self.lacks_fuse.store(!need_note.is_empty(), Ordering::SeqCst);
        if another_kind {
            need_note.clear();
        }
        store.set_need_note(l(if need_note.is_empty() { web_note } else { need_note }));
        *self.needed.lock().unwrap() = needed;
        // The same choices as rows for the TV, in two groups that each say what they
        // are: other apps for the service, then other places to get this one.
        let mut rows: Vec<TvChoice> = Vec::new();
        let service = store.get_app_family_name();
        for (n, other) in store.get_app_family().iter().enumerate() {
            let whose = if other.official { "official" } else { "unofficial" };
            rows.push(TvChoice {
                id: other.id,
                name: other.name,
                note: if other.note.is_empty() { whose.into() } else { format!("{}  ·  {whose}", other.note).into() },
                heading: if n == 0 { format!("Other ways to use {service}").into() } else { SharedString::new() },
            });
        }
        let name = store.get_app().name;
        for (n, source) in store.get_app_sources().iter().filter(|c| !c.on).enumerate() {
            rows.push(TvChoice {
                id: source.id,
                name: format!("From {}", source.label).into(),
                note: l(format!("The same {name}, from another source")),
                heading: if n == 0 { format!("Other places to get {name}").into() } else { SharedString::new() },
            });
        }
        store.set_tv_choices(ModelRc::new(VecModel::from(rows)));
        store.set_helper_id(helper.0.into());
        store.set_helper_label(l(helper.1));
    }

    /// Fetch the lists the sources keep (what is popular on Flathub and the like) and
    /// add them to the home page. The page is usable before they arrive.
    pub fn load_collections(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        self.job(
            |app| app.catalog().collections(24),
            |app, ui, sections| {
                *app.collections.lock().unwrap() = sections;
                app.refresh(ui);
            },
        );
    }

    /// Fetch star ratings in the background (at most once a day), unless turned off.
    pub fn load_ratings(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        if self.state().settings.hide_ratings {
            return;
        }
        self.job(
            |app| app.catalog().load_ratings(&app.dirs.cache),
            |app, ui, loaded| {
                if loaded {
                    app.refresh(ui);
                    let store = ui.global::<Store>();
                    // Only where the ratings service knows the app: a rating its own
                    // store gave it (Steam's, the Microsoft Store's) is left as it is.
                    let open = AppId::parse(store.get_app().id.as_str());
                    if let Some(rating) = open.and_then(|id| app.catalog().rating_for(&id)) {
                        let mut card = store.get_app();
                        card.rating = bilby_core::ratings::stars_text(rating).into();
                        card.rating_count = ratings_text(rating.count).into();
                        card.rating_spread = spread_shares(rating);
                        store.set_app(at_the_top(card));
                    }
                }
            },
        );
    }
}
