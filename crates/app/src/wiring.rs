//! Building the window and connecting each thing the interface can ask for to the
//! code that does it.

use crate::accounts::signin_window;
#[cfg(target_os = "linux")]
use crate::activation::wayland_activation;
use crate::options::{restart_device, system_picker, CatalogFactory, Options};
use crate::pictures::{own_icon, OWN_PICTURE};
use crate::settings::{
    apply_language, form_for, make_catalog, shell_for, source_named, system_language, theme_mode, Change,
};
use crate::text::l;
use crate::tray::show_tray;
use crate::{askpass, i18n, osk, sound, App, AppWindow, Current, Elsewhere, Fact, Shell, Store, Theme, UpdateTable};
use bilby_core::install::{self, Dirs};
use bilby_core::model::{AppId, SourceKind};
use bilby_core::platform::FormFactor;
use bilby_core::state::{Settings, State};
use bilby_core::{Error, Platform};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Create the window and wire it to the catalog, without starting the event loop.
pub fn build(options: &Options) -> Result<(AppWindow, Arc<App>), Box<dyn std::error::Error>> {
    let mut platform = options.platform.clone().unwrap_or_else(Platform::detect);
    let detected = platform.form_factor;
    let mut dirs = options.dirs.clone().unwrap_or_else(Dirs::for_current_user);
    // On a real Linux device, what needs the system's password goes through a
    // stand-in that asks for it on screen when the TV layout is showing (see
    // `askpass`), since nobody can type in the system's own prompt with a remote.
    let asking = (cfg!(target_os = "linux") && !options.demo && options.platform.is_none() && options.dirs.is_none())
        .then(|| askpass::set_up(&dirs.cache.join("askpass")))
        .flatten();
    if let Some(asking) = &asking {
        dirs.elevate = asking.elevate.clone();
    }
    // The demo catalog gets its own library so fictional apps never mix with real ones.
    let state_path = options
        .state_file
        .clone()
        .unwrap_or_else(|| dirs.config.join(if options.demo { "demo-state.json" } else { "state.json" }));
    let state = State::load(&state_path);
    // Which layout: the command line first, then the choice saved in Settings, then
    // what the device looks like. The TV layout also means "prefer TV builds".
    match options.shell {
        Some(Shell::Tv) => platform.form_factor = FormFactor::Tv,
        Some(_) if platform.form_factor == FormFactor::Tv => platform.form_factor = FormFactor::Desktop,
        Some(_) => {}
        None => platform.form_factor = form_for(&state.settings.interface, detected),
    }
    // Windows programs, set up to run here through Proton: only on the real device,
    // where the settings ask for it and what it needs is there.
    if options.platform.is_none() && !options.demo {
        platform.offer_windows(!state.settings.hide_windows_apps);
    }
    // A controller with nothing else pointing at a TV: worth one question.
    let ask_tv = options.shell.is_none()
        && platform.form_factor == FormFactor::Desktop
        && state.settings.interface.is_empty()
        && !state.settings.asked_about_tv
        && options
            .controller
            .unwrap_or_else(|| options.platform.is_none() && bilby_core::platform::controller_attached());
    let demo = options.demo;
    let catalog_factory: CatalogFactory = options.catalog_factory.clone().unwrap_or_else(|| {
        Arc::new(move |platform: &Platform, settings: &Settings, dirs: &Dirs| {
            make_catalog(platform, demo, settings, dirs)
        })
    });
    // The language: what was asked for this once, else what was chosen in Settings,
    // else the system's own on a real device (a preview of another device, and every
    // test, is in English). Known before the catalog is made: the stores are asked
    // for names and descriptions in it.
    let language = options
        .language
        .clone()
        .or_else(|| Some(state.settings.language.clone()))
        .filter(|code| !code.is_empty())
        .unwrap_or_else(|| system_language(options.platform.is_none()).to_string());
    i18n::set_language(&language);
    let catalog = catalog_factory(&platform, &state.settings, &dirs);

    let ui = AppWindow::new()?;
    // Slint's own text follows, now that there is a window to follow it.
    apply_language(&language);
    let store = ui.global::<Store>();
    let shell = options.shell.unwrap_or_else(|| shell_for(platform.form_factor));
    store.set_shell(shell);
    // A forced interface stays put; otherwise desktop and mobile follow the window width.
    store.set_adaptive(options.shell.is_none() && shell != Shell::Tv);
    store.set_ask_tv(ask_tv);
    store.set_demo(options.demo);
    store.set_app_version(env!("CARGO_PKG_VERSION").into());
    store.set_platform_title(l(platform.title()));
    store.set_device_name(l(platform.friendly()));
    let mut facts = platform.facts();
    let sources: Vec<_> = catalog.active_sources().iter().map(|s| s.label()).collect();
    facts.push(("Sources".to_string(), sources.join(", ")));
    store.set_facts(ModelRc::new(VecModel::from(
        facts.into_iter().map(|(name, value)| Fact { name: l(name), value: value.into() }).collect::<Vec<_>>(),
    )));

    // A demo or a preview of another device must never look at, let alone change, the
    // real system: it gets a made-up one.
    let pretend = options.demo || options.platform.is_some();
    bilby_core::icons::forget_moved(&dirs.cache.join("icons"));
    let host: Arc<dyn bilby_core::os::Host> = match &options.host {
        Some(host) => host.clone(),
        None if pretend => Arc::new(bilby_core::os::demo_host(&platform)),
        None => match &asking {
            Some(asking) => {
                Arc::new(askpass::AskingHost { inner: bilby_core::os::RealHost, elevate: asking.elevate.clone() })
            }
            None => Arc::new(bilby_core::os::RealHost),
        },
    };
    let mut system = bilby_core::os::describe(host.as_ref(), &platform);
    if pretend && !options.demo && options.host.is_none() {
        system.note = format!("This is a preview of another device, so these details are made up. {}", system.note);
    }

    let app = Arc::new(App {
        ui: ui.as_weak(),
        catalog: RwLock::new(catalog),
        platform: RwLock::new(platform),
        detected,
        theme_override: options.theme.clone(),
        inline: options.inline,
        dirs,
        state_path,
        state: Mutex::new(state),
        results: Mutex::new((String::new(), Vec::new())),
        updates: Mutex::new(UpdateTable::default()),
        last_checked: AtomicU64::new(0),
        current: Mutex::new(Current::default()),
        search_generation: AtomicU64::new(0),
        details_generation: AtomicU64::new(0),
        install_when_loaded: Mutex::new(None),
        toast_generation: Arc::new(AtomicU64::new(0)),
        cancel_install: Arc::new(AtomicBool::new(false)),
        installing: Mutex::new(None),
        real_device: options.platform.is_none(),
        osk: Mutex::new(osk::Model::default()),
        asking,
        password: Mutex::new(String::new()),
        password_reply: Mutex::new(None),
        store_app: match &options.store_app {
            Some(id) => AppId::parse(id),
            None if options.demo => None,
            None => Some(AppId::new(SourceKind::GitHub, bilby_core::catalog::STORE_REPOSITORY)),
        },
        update_queue: Mutex::new(std::collections::VecDeque::new()),
        browsing: Mutex::new(None),
        download_queue: Mutex::new(std::collections::VecDeque::new()),
        unattended: std::sync::atomic::AtomicBool::new(false),
        quiet_install: std::sync::atomic::AtomicBool::new(false),
        pictures_requested: Mutex::new(HashSet::new()),
        picture_failures: Mutex::new(HashMap::new()),
        opener: options.opener.clone().unwrap_or_else(|| Arc::new(install::open_with_system)),
        launcher: options.launcher.clone().unwrap_or_else(|| Arc::new(install::launch)),
        runner: options.runner.clone().unwrap_or_else(|| Arc::new(install::launch_with)),
        icon_urls: Mutex::new(HashMap::new()),
        catalog_factory,
        host,
        system,
        system_check: Mutex::new(None),
        collections: Mutex::new(Vec::new()),
        self_program: options.self_program.clone(),
        signin_generation: AtomicU64::new(0),
        steam_libraries: options.steam_libraries.clone(),
        result_filter: Mutex::new(None),
        alternates: Mutex::new(HashMap::new()),
        signin: options.signin.clone().unwrap_or_else(|| Arc::new(signin_window)),
        // The demo is a made-up device with everything in place, unless told otherwise.
        fuse: std::sync::RwLock::new(options.fuse.unwrap_or_else(|| match options.demo {
            true => bilby_core::needs::Fuse { library: true, mounter: true },
            false => bilby_core::needs::fuse_here(),
        })),
        fuse_given: options.fuse.is_some() || options.demo,
        layering: options.layering.clone().or_else(|| match options.demo {
            true => None,
            false => bilby_core::needs::layering_here(),
        }),
        fuse_staged: std::sync::atomic::AtomicBool::new(false),
        fuse_just_staged: std::sync::atomic::AtomicBool::new(false),
        restart: options.restart.clone().unwrap_or_else(|| Arc::new(restart_device)),
        notifier: options.notifier.clone().unwrap_or_else(|| {
            let real = !options.demo && options.platform.is_none() && options.dirs.is_none();
            Arc::new(move |title: &str, body: &str| {
                if real {
                    install::notify(title, body);
                }
            })
        }),
        in_tray: std::sync::atomic::AtomicBool::new(false),
        offered: std::sync::atomic::AtomicBool::new(false),
        notified: Mutex::new(String::new()),
        needed: Mutex::new(None),
        lacks_fuse: std::sync::atomic::AtomicBool::new(false),
        steam: std::sync::RwLock::new(None),
        steam_generation: AtomicU64::new(0),
        gog: std::sync::RwLock::new(None),
        gog_generation: AtomicU64::new(0),
        review_places: Mutex::new(Vec::new()),
        elsewhere: std::sync::RwLock::new(Elsewhere::default()),
        heard: options.heard.clone(),
        #[cfg(not(target_os = "android"))]
        player: std::sync::OnceLock::new(),
        silent: options.inline || options.screenshot.is_some(),
        browser: options.browser.clone().unwrap_or_else(|| Arc::new(install::open_url)),
        picker: options.picker.clone().unwrap_or_else(|| Arc::new(system_picker)),
        asked_folder: Mutex::new(None),
        self_new: Mutex::new(None),
        launch_waiting: AtomicBool::new(false),
        fetched: Mutex::new(None),
        pending: Mutex::new(None),
        known_adult: Mutex::new(HashSet::new()),
        new_pin: Mutex::new(None),
        adult_unlocked: std::sync::atomic::AtomicBool::new(false),
        install_cleared: std::sync::atomic::AtomicBool::new(false),
    });
    // The store's own picture, kept under its own app's name before anything is drawn.
    if let Some(own) = &app.store_app {
        let picture = own_icon();
        OWN_PICTURE.with(|kept| *kept.borrow_mut() = Some((bilby_core::icons::app_key(own), picture)));
    }
    app.note_self(&ui);
    app.scan_installed(&ui);
    *app.steam.write().unwrap() = app.state().steam;
    *app.gog.write().unwrap() = app.state().gog;
    app.apply_menu_entry(&ui);
    app.show_system(&ui, 0, "");

    // The TV interface is designed dark; elsewhere the saved preference applies.
    let theme = options.theme.clone().unwrap_or_else(|| app.state().settings.theme);
    ui.global::<Theme>().set_mode(if shell == Shell::Tv { 2 } else { theme_mode(&theme) });
    app.refresh(&ui);
    app.refresh_settings(&ui);
    app.load_pictures(&ui);
    app.load_collections(&ui);
    // Read the system's package databases in the background: it can take a moment on a
    // large distribution, and the first search should not be the one to pay for it.
    app.job(
        |app| app.catalog().system_repositories(),
        |_app, ui, repositories| {
            if repositories.is_empty() {
                return;
            }
            let store = ui.global::<Store>();
            let mut facts: Vec<Fact> = store.get_facts().iter().collect();
            facts.push(Fact { name: l("System repositories"), value: repositories.join(", ").into() });
            store.set_facts(ModelRc::new(VecModel::from(facts)));
        },
    );
    if let Some(theme) = &options.theme {
        store.set_theme_mode(theme_mode(theme));
    }

    // Each handler looks the window up again: callbacks must not keep it alive.
    macro_rules! on {
        ($setter:ident, |$app:ident, $ui:ident $(, $arg:ident)*| $body:block) => {
            store.$setter({
                let $app = app.clone();
                move |$($arg),*| {
                    if let Some($ui) = $app.ui.upgrade() {
                        let $ui = &$ui;
                        $body
                    }
                }
            });
        };
    }
    on!(on_search, |app, ui, query| { app.search(ui, &query) });
    on!(on_leave_search, |app, ui| { app.leave_search(ui) });
    on!(on_set_language, |app, ui, code| { app.set_language(ui, &code) });
    on!(on_next_language, |app, ui| { app.next_language(ui) });
    on!(on_set_country, |app, ui, code| { app.set_country(ui, &code) });
    on!(on_step_country, |app, ui, forward| { app.step_country(ui, forward) });
    on!(on_osk_open, |app, ui| {
        *app.osk.lock().unwrap() = osk::Model::default();
        app.show_osk(ui);
    });
    on!(on_osk_move, |app, ui, dir| {
        if let Some(dir) = osk::Dir::from(dir) {
            app.osk.lock().unwrap().step(dir);
            ui.global::<Store>().set_osk_at(app.osk.lock().unwrap().flat() as i32);
        }
    });
    on!(on_osk_erase, |app, ui| { app.osk_erase(ui) });
    // A letter from a real keyboard while the on-screen one is up: typed as it is,
    // and the highlight goes to "search", so Enter then searches.
    on!(on_osk_type, |app, ui, text| {
        app.osk_add(ui, &text);
        app.osk.lock().unwrap().point_to(osk::Kind::Submit);
        ui.global::<Store>().set_osk_at(app.osk.lock().unwrap().flat() as i32);
    });
    on!(on_password_submit, |app, ui| { app.answer_password(ui, true) });
    on!(on_password_cancel, |app, ui| { app.answer_password(ui, false) });
    // The system wants its password: asked for here, on the screen, and waited for.
    if let Some(asking) = &app.asking {
        asking.on_tv(shell == Shell::Tv);
        let asker = app.clone();
        asking.serve(move |again| {
            let (reply, answer) = std::sync::mpsc::channel();
            let app = asker.clone();
            slint::invoke_from_event_loop(move || match app.ui.upgrade() {
                Some(ui) => app.show_password(&ui, again, reply),
                None => drop(reply),
            })
            .ok()?;
            answer.recv().ok().flatten()
        });
    }
    for tapped in [false, true] {
        let app = app.clone();
        let press = move |at: i32| {
            let Some(ui) = app.ui.upgrade() else { return false };
            if tapped {
                app.osk.lock().unwrap().point_at(at.max(0) as usize);
                ui.global::<Store>().set_osk_at(app.osk.lock().unwrap().flat() as i32);
            }
            app.osk_press(&ui)
        };
        match tapped {
            false => store.on_osk_press(move || press(0)),
            true => store.on_osk_tap(press),
        }
    }
    on!(on_open, |app, ui, id| { app.open(ui, &id) });
    on!(on_close_details, |app, ui| { app.close_details(ui) });
    on!(on_retry_details, |app, ui| {
        let id = app.current.lock().unwrap().id.clone();
        if !id.is_empty() {
            app.open(ui, &id);
        }
    });
    on!(on_select_version, |app, ui, index| { app.select_version(ui, index) });
    on!(on_install, |app, ui, index| { app.install(ui, index) });
    on!(on_cancel_install, |app, ui| {
        let _ = ui;
        app.cancel_install.store(true, Ordering::SeqCst);
    });
    on!(on_check_updates, |app, ui| { app.check_updates(ui, true) });
    on!(on_update_app, |app, ui, id| { app.update_app(ui, &id) });
    on!(on_update_all, |app, ui| { app.update_all(ui) });
    on!(on_remove_from_library, |app, ui, id| { app.remove_from_library(ui, &id) });
    on!(on_skip_update, |app, ui, id| { app.skip_update(ui, &id) });
    on!(on_toggle_favourite, |app, ui| { app.toggle_favourite(ui) });
    on!(on_launch, |app, ui| { app.launch(ui) });
    on!(on_quick_install, |app, ui, id| { app.quick_install(ui, &id) });
    on!(on_download_hidden, |app, ui, index| { app.download_hidden(ui, index) });
    on!(on_set_theme, |app, ui, mode| {
        let name = ["system", "light", "dark"].get(mode as usize).copied().unwrap_or("system");
        app.update_state(ui, |state| state.settings.theme = name.to_string());
        if ui.global::<Store>().get_shell() != Shell::Tv {
            ui.global::<Theme>().set_mode(theme_mode(name));
        }
        app.refresh_settings(ui);
    });
    on!(on_set_prereleases, |app, ui, enabled| {
        app.update_state(ui, |state| state.settings.include_prereleases = enabled);
        app.rebuild_catalog();
        app.refresh_settings(ui);
    });
    on!(on_set_ratings, |app, ui, enabled| {
        app.update_state(ui, |state| state.settings.hide_ratings = !enabled);
        if enabled {
            app.load_ratings(ui);
        } else {
            app.catalog().drop_ratings();
            app.refresh(ui);
        }
        app.refresh_settings(ui);
    });
    on!(on_set_check_on_start, |app, ui, enabled| {
        app.update_state(ui, |state| state.settings.skip_update_check_on_start = !enabled);
        app.refresh_settings(ui);
    });
    on!(on_open_downloads, |app, ui| {
        let folder = app.downloads_folder();
        let result = std::fs::create_dir_all(&folder).map_err(Error::from).and_then(|_| (app.opener)(&folder));
        if let Err(e) = result {
            app.toast(ui, &format!("Couldn't open the downloads folder: {e}"), true);
        }
    });
    on!(on_github_signin, |app, ui| { app.github_signin(ui) });
    on!(on_github_cancel, |app, ui| { app.github_cancel(ui) });
    on!(on_steam_signin, |app, ui| { app.steam_signin(ui) });
    on!(on_check_store_update, |app, ui| { app.check_store_update_now(ui) });
    on!(on_steam_signin_qr, |app, ui| { app.steam_signin_qr(ui) });
    on!(on_steam_cancel, |app, ui| { app.steam_cancel(ui) });
    on!(on_steam_signout, |app, ui| { app.steam_signout(ui) });
    on!(on_gog_signin, |app, ui| { app.gog_signin(ui) });
    on!(on_gog_cancel, |app, ui| { app.gog_cancel(ui) });
    on!(on_gog_signout, |app, ui| { app.gog_signout(ui) });
    on!(on_toggle_review_place, |app, ui, id| { app.toggle_review_place(ui, &id) });
    on!(on_send_review, |app, ui, stars, title, text, name| {
        let draft = bilby_core::reviewing::Draft {
            stars: stars.clamp(0, 5) as u8,
            title: title.to_string(),
            text: text.to_string(),
            name: name.to_string(),
        };
        app.send_review(ui, draft)
    });
    on!(on_save_token, |app, ui, token| {
        let token = token.trim().to_string();
        let cleared = token.is_empty();
        app.update_state(ui, |state| state.settings.github_token = token);
        app.rebuild_catalog();
        app.refresh_settings(ui);
        app.toast(ui, if cleared { "Signed out of GitHub" } else { "Token saved on this device" }, false);
    });
    on!(on_clear_cache, |app, ui| {
        // Only the store's own cache folders, never a path from outside.
        let result = ["http", "icons"].iter().try_for_each(|name| {
            let folder = app.dirs.cache.join(name);
            if folder.exists() {
                std::fs::remove_dir_all(&folder)
            } else {
                Ok(())
            }
        });
        match result {
            Ok(()) => app.toast(ui, "Cached catalog data cleared", false),
            Err(e) => app.toast(ui, &format!("Couldn't clear the cache: {e}"), true),
        }
    });
    on!(on_clear_recent, |app, ui| {
        app.update_state(ui, |state| state.recent_searches.clear());
        app.refresh(ui);
    });
    on!(on_open_url, |app, ui, url| {
        if let Err(e) = (app.browser)(&url) {
            app.toast(ui, &e.to_string(), true);
        }
    });
    on!(on_set_in_menu, |app, ui, shown| {
        app.update_state(ui, |state| state.settings.hide_from_menu = !shown);
        app.apply_menu_entry(ui);
        app.refresh_settings(ui);
    });
    on!(on_set_reduce_motion, |app, ui, on| {
        app.update_state(ui, |state| state.settings.reduce_motion = on);
        app.refresh_settings(ui);
    });
    on!(on_play, |app, _ui, kind| {
        if let Some(sound) = sound::Sound::from_number(kind) {
            app.play(sound);
        }
    });
    on!(on_set_sounds, |app, ui, on| {
        app.update_state(ui, |state| state.settings.mute_sounds = !on);
        app.refresh_settings(ui);
        // Switching them on is heard at once; switching them off, of course, is not.
        app.play(sound::Sound::Toggle);
    });
    on!(on_add_own_source, |app, ui, url| { app.add_own_source(ui, &url) });
    on!(on_remove_own_source, |app, ui, url| { app.remove_own_source(ui, &url) });
    on!(on_choose_downloads, |app, ui| { app.choose_downloads(ui) });
    on!(on_set_downloads, |app, ui, path| { app.set_downloads(ui, &path) });
    on!(on_set_ask_where, |app, ui, on| {
        app.update_state(ui, |state| state.settings.ask_where = on);
        app.refresh_settings(ui);
    });
    on!(on_set_prefer_kind, |app, ui, kind| {
        let kind = bilby_core::compat::Prefer::from_number(kind);
        app.update_state(ui, |state| state.settings.prefer_kind = kind.name().to_string());
        // Which file is offered first depends on it: build the catalog again.
        app.rebuild_catalog();
        app.refresh_settings(ui);
        app.refresh(ui);
    });
    on!(on_set_close_to_tray, |app, ui, on| {
        app.update_state(ui, |state| state.settings.quit_on_close = !on);
        app.refresh_settings(ui);
        show_tray(on);
    });
    on!(on_browse_category, |app, ui, name| { app.browse_category(ui, &name, false) });
    on!(on_set_auto_update, |app, ui, on| { app.request_setting(ui, Change::AutoUpdate(on)) });
    on!(on_set_windows_apps, |app, ui, on| {
        if !app.real_device {
            return;
        }
        app.update_state(ui, |state| state.settings.hide_windows_apps = !on);
        app.platform.write().unwrap().offer_windows(on);
        // What can be installed here has changed: a new catalog, and the search made again.
        app.rebuild_catalog();
        app.refresh_settings(ui);
        app.refresh(ui);
        let query = ui.global::<Store>().get_query().to_string();
        if ui.global::<Store>().get_page() == 1 && !query.trim().is_empty() {
            app.search(ui, &query);
        }
    });
    on!(on_set_windows_proton, |app, ui, which| {
        app.change_windows(ui, None, |choices| {
            choices.proton =
                if which == 1 { bilby_core::winrun::Proton::Ge } else { bilby_core::winrun::Proton::Unset };
        });
    });
    on!(on_set_windows_overlay, |app, ui, on| {
        app.change_windows(ui, None, |choices| choices.overlay = on.then_some(true));
    });
    on!(on_set_windows_gamemode, |app, ui, on| {
        app.change_windows(ui, None, |choices| choices.gamemode = on.then_some(true));
    });
    on!(on_set_windows_app, |app, ui, id, proton, overlay| {
        use bilby_core::winrun::Proton;
        app.change_windows(ui, Some(&id), |choices| {
            choices.proton = match proton {
                1 => Proton::Standard,
                2 => Proton::Ge,
                _ => Proton::Unset,
            };
            choices.overlay = match overlay {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            };
        });
    });
    on!(on_open_windows_tool, |app, ui, id| {
        let opened = app
            .windows_folder(&id)
            .ok_or_else(|| Error::Unsupported("That Windows app is not installed.".to_string()))
            .and_then(|folder| bilby_core::winrun::open_tool(&app.dirs, &folder, "winecfg"));
        match opened {
            Ok(()) => app.toast(ui, "Opening its Windows settings…", false),
            Err(e) => app.toast(ui, &e.to_string(), true),
        }
    });
    on!(on_open_windows_folder, |app, ui, id| {
        let opened = app
            .windows_folder(&id)
            .ok_or_else(|| Error::Unsupported("That Windows app is not installed.".to_string()))
            .and_then(|folder| (app.opener)(&folder.join("prefix").join("drive_c")));
        if let Err(e) = opened {
            app.toast(ui, &e.to_string(), true);
        }
    });
    on!(on_set_show_everything, |app, ui, on| {
        app.update_state(ui, |state| state.settings.show_everything = on);
        // What a search finds depends on it: a new catalog, and the search made again.
        app.rebuild_catalog();
        app.refresh_settings(ui);
        let query = ui.global::<Store>().get_query().to_string();
        if ui.global::<Store>().get_page() == 1 && !query.trim().is_empty() {
            app.search(ui, &query);
        }
    });
    on!(on_set_essentials, |app, ui, on| {
        app.update_state(ui, |state| state.settings.hide_essentials = !on);
        // What is recommended depends on it: build the catalog again and redraw.
        app.rebuild_catalog();
        app.collections.lock().unwrap().clear();
        app.load_collections(ui);
        app.refresh_settings(ui);
        app.refresh(ui);
    });
    on!(on_set_family_mode, |app, ui, on| { app.request_setting(ui, Change::FamilyMode(on)) });
    on!(on_set_adult_on_home, |app, ui, on| { app.request_setting(ui, Change::AdultOnHome(on)) });
    on!(on_set_adult_uncovered, |app, ui, on| { app.request_setting(ui, Change::AdultUncovered(on)) });
    on!(on_set_pin_for_installs, |app, ui, on| { app.request_setting(ui, Change::PinForInstalls(on)) });
    on!(on_set_pin, |app, ui| { app.request_setting(ui, Change::SetPin) });
    on!(on_remove_pin, |app, ui| { app.request_setting(ui, Change::RemovePin) });
    on!(on_gate_answer, |app, ui, yes| { app.gate_answer(ui, yes) });
    on!(on_gate_pin, |app, ui, entry| { app.gate_pin(ui, &entry) });
    on!(on_gate_cancel, |app, ui| { app.gate_cancel(ui) });
    on!(on_set_source, |app, ui, name, on| {
        let name = name.to_string();
        app.update_state(ui, |state| {
            state.settings.disabled_sources.retain(|n| *n != name);
            if !on {
                state.settings.disabled_sources.push(name.clone());
            }
        });
        // What is listed depends on it: build the catalog again and redraw.
        app.rebuild_catalog();
        app.collections.lock().unwrap().clear();
        app.load_collections(ui);
        app.refresh(ui);
        app.refresh_settings(ui);
    });
    on!(on_filter_results, |app, ui, name| {
        *app.result_filter.lock().unwrap() = source_named(name.as_str());
        app.refresh(ui);
    });
    on!(on_set_interface, |app, ui, mode| { app.set_interface(ui, mode) });
    on!(on_answer_tv, |app, ui, yes| {
        if yes {
            app.set_interface(ui, 2);
        } else {
            app.update_state(ui, |state| state.settings.asked_about_tv = true);
            ui.global::<Store>().set_ask_tv(false);
        }
    });
    on!(on_system_check, |app, ui| { app.check_system(ui, true) });
    on!(on_system_update, |app, ui| { app.update_system(ui) });
    on!(on_system_restart_now, |app, ui| { app.ask_restart_for_updates(ui) });
    on!(on_system_handoff_open, |app, ui| {
        if let Some(handoff) = &app.system.handoff {
            if let Err(e) = (app.opener)(std::path::Path::new(handoff.target)) {
                app.toast(ui, &e.to_string(), true);
            }
        }
    });
    on!(on_dismiss_welcome, |app, ui| {
        app.update_state(ui, |state| state.settings.welcomed = true);
        app.refresh_settings(ui);
    });
    on!(on_view_store_update, |app, ui| {
        // The store's own page is an app page like any other: same files, same checks.
        if let Some(id) = app.store_app.clone() {
            app.open(ui, &id.to_string());
        }
    });
    on!(on_dismiss_toast, |app, ui| {
        let _ = &app;
        ui.global::<Store>().set_toast(SharedString::new());
    });

    if let Some(page) = options.page {
        store.set_page(page);
    }
    Ok((ui, app))
}

/// Translate what a TV remote sends into keys the interface understands.
///
/// Slint passes on the keys of a keyboard, but drops the "Back" key that remotes and
/// media keyboards send, so pressing it did nothing. Here it becomes Escape, which every
/// page already treats as "back". "Select" becomes Return. In the TV layout the right
/// and "back" mouse buttons do the same, because air-mouse remotes send those for Back.
#[cfg(not(target_os = "android"))]
pub(crate) fn translate_remote_keys(ui: &AppWindow, app: &Arc<App>) {
    use slint::platform::{Key, WindowEvent};
    use slint::winit_030::winit::event::{ElementState, MouseButton, WindowEvent as Raw};
    use slint::winit_030::winit::keyboard::{Key as RawKey, NamedKey};
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    let weak = ui.as_weak();
    let app = Arc::downgrade(app);
    ui.window().on_winit_window_event(move |window, event| {
        let on_tv = || weak.upgrade().is_some_and(|ui| ui.get_effective() == Shell::Tv);
        // Keep up with the serials of presses, which leave to bring another window
        // forward is asked with (activation.rs).
        #[cfg(target_os = "linux")]
        if matches!(event, Raw::KeyboardInput { .. } | Raw::MouseInput { .. } | Raw::Focused(true)) {
            if let Some(ui) = weak.upgrade() {
                wayland_activation(&ui, |activation| activation.listen());
            }
        }
        // Leave to bring another window to the front has arrived: start what waited for it.
        if let Raw::ActivationTokenDone { token, .. } = event {
            if let (Some(app), Some(ui)) = (app.upgrade(), weak.upgrade()) {
                app.launch_granted(&ui, Some(token.clone().into_raw()));
            }
            return EventResult::Propagate;
        }
        let (key, state) = match event {
            Raw::KeyboardInput { event, .. } => match &event.logical_key {
                RawKey::Named(NamedKey::BrowserBack | NamedKey::GoBack) => (Key::Escape, event.state),
                RawKey::Named(NamedKey::Select | NamedKey::Accept) => (Key::Return, event.state),
                _ => return EventResult::Propagate,
            },
            Raw::MouseInput { button: MouseButton::Right | MouseButton::Back, state, .. } if on_tv() => {
                (Key::Escape, *state)
            }
            _ => return EventResult::Propagate,
        };
        let text: SharedString = key.into();
        window.dispatch_event(match state {
            ElementState::Pressed => WindowEvent::KeyPressed { text },
            ElementState::Released => WindowEvent::KeyReleased { text },
        });
        EventResult::PreventDefault
    });
}
