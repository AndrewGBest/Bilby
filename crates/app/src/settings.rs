//! The Settings page, and applying what is chosen there: language, country, layout,
//! sources, where downloads go.

use crate::gate::Guarded;
use crate::options::Picked;
use crate::text::l;
use crate::{i18n, sound, App, AppWindow, Shell, SourceChoice, Store, Theme, WindowsApp, ICON_PNG};
use bilby_core::install::{self, Dirs};
use bilby_core::model::{AppId, SourceKind};
use bilby_core::platform::{FormFactor, Os};
use bilby_core::state::Settings;
use bilby_core::{Catalog, Platform};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// A change to the family and adult-content settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    FamilyMode(bool),
    AdultOnHome(bool),
    AdultUncovered(bool),
    PinForInstalls(bool),
    AutoUpdate(bool),
    SetPin,
    RemovePin,
}

/// The country the system is set to, for prices: the United States where it does not say.
pub(crate) fn system_country() -> String {
    sys_locale::get_locale()
        .and_then(|locale| bilby_core::region::country_of(&locale))
        .or_else(bilby_core::region::from_env)
        .unwrap_or_else(|| "US".to_string())
}

/// The language of ours that the system is set to; English when not asked to look.
pub(crate) fn system_language(look: bool) -> &'static str {
    match sys_locale::get_locale() {
        Some(locale) if look => i18n::for_locale(&locale),
        _ => "en",
    }
}

/// Show everything in this language from now on: the interface's own text and what
/// the code puts on screen.
pub(crate) fn apply_language(code: &str) {
    i18n::set_language(code);
    let _ = slint::select_bundled_translation(match i18n::language() {
        "en" => "",
        known => known,
    });
}

pub(crate) fn theme_mode(name: &str) -> i32 {
    match name {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    }
}

/// 0 decide for me, 1 desktop, 2 TV: the saved layout choice as the interface shows it.
pub(crate) fn interface_mode(saved: &str) -> i32 {
    match saved {
        "desktop" => 1,
        "tv" => 2,
        _ => 0,
    }
}

/// The shape to behave as, given the saved layout choice and what was detected.
pub(crate) fn form_for(saved: &str, detected: FormFactor) -> FormFactor {
    match saved {
        "tv" => FormFactor::Tv,
        // "Desktop" on a phone still means the phone's own layout.
        "desktop" if detected == FormFactor::Tv => FormFactor::Desktop,
        _ => detected,
    }
}

pub(crate) fn shell_for(form_factor: FormFactor) -> Shell {
    match form_factor {
        FormFactor::Desktop => Shell::Desktop,
        FormFactor::Mobile => Shell::Mobile,
        FormFactor::Tv => Shell::Tv,
    }
}

/// The source with this short name (`flathub`, `steam`).
pub(crate) fn source_named(name: &str) -> Option<SourceKind> {
    AppId::parse(&format!("{name}:x")).map(|id| id.source)
}

pub(crate) fn make_catalog(platform: &Platform, demo: bool, settings: &Settings, dirs: &Dirs) -> Arc<Catalog> {
    let mut catalog = if demo {
        Catalog::demo(platform.clone())
    } else {
        // A token typed into Settings wins over the environment.
        let token = Some(settings.github_token.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("GITHUB_TOKEN").ok());
        // Prices are those of the country the person chose, else the system's own.
        let country = bilby_core::region::country(&settings.country).unwrap_or_else(system_country);
        Catalog::live_in(platform.clone(), token, dirs.cache.clone(), &country)
    };
    catalog.include_prereleases = settings.include_prereleases;
    catalog.show_everything = settings.show_everything;
    catalog.disabled = settings.disabled_sources.iter().filter_map(|name| source_named(name)).collect();
    catalog.use_own_sources(
        settings
            .own_sources
            .iter()
            .map(|own| bilby_core::source::own::Added { name: own.name.clone(), url: own.url.clone() })
            .collect(),
    );
    // (The made-up demo catalog has it off already: the list names real apps.)
    catalog.essentials = catalog.essentials && !settings.hide_essentials;
    catalog.prefer = bilby_core::compat::Prefer::named(&settings.prefer_kind);
    catalog.parental =
        bilby_core::parental::Policy { family_mode: settings.family_mode, adult_on_home: settings.adult_on_home };
    // In the language Bilby is being shown in, which is set before any catalog is made.
    catalog.use_flatpak(dirs.flatpak_roots.clone(), i18n::language());
    Arc::new(catalog)
}

impl App {
    pub(crate) fn refresh_settings(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let settings = self.state().settings;
        store.set_theme_mode(theme_mode(&settings.theme));
        store.set_include_prereleases(settings.include_prereleases);
        store.set_show_ratings(!settings.hide_ratings);
        store.set_check_updates_on_start(!settings.skip_update_check_on_start);
        store.set_has_token(!settings.github_token.trim().is_empty());
        store.set_sounds(!settings.mute_sounds);
        store.set_show_essentials(!settings.hide_essentials);
        store.set_show_everything(settings.show_everything);
        // Offered only on the real device, where what it needs was looked for.
        let possible = self.real_device && self.platform.read().unwrap().could_run_windows();
        store.set_windows_possible(possible);
        store.set_windows_apps(possible && !settings.hide_windows_apps);
        // How they run: in general, and what each installed one does differently.
        use bilby_core::winrun::{self, Choices, Proton};
        let general = Choices::read(&winrun::general_choices(&self.dirs));
        store.set_windows_proton(i32::from(general.proton == Proton::Ge));
        store.set_windows_overlay(general.overlay == Some(true));
        store.set_windows_gamemode(general.gamemode == Some(true));
        let on_path = |program: &str| self.dirs.programs.iter().any(|dir| dir.join(program).is_file());
        store.set_has_mangohud(on_path("mangohud"));
        store.set_has_gamemode(on_path("gamemoderun"));
        let installed: Vec<WindowsApp> = self
            .state()
            .library
            .iter()
            .filter_map(|app| {
                let folder = winrun::folder_of(&self.dirs, std::path::Path::new(app.path.as_deref()?))?;
                let own = Choices::read(&winrun::app_choices(&folder));
                Some(WindowsApp {
                    id: app.id.as_str().into(),
                    name: app.name.as_str().into(),
                    proton: match own.proton {
                        Proton::Unset => 0,
                        Proton::Standard => 1,
                        Proton::Ge => 2,
                    },
                    overlay: match own.overlay {
                        None => 0,
                        Some(true) => 1,
                        Some(false) => 2,
                    },
                })
            })
            .collect();
        store.set_windows_installed(ModelRc::new(VecModel::from(installed)));
        store.set_auto_update(settings.auto_update);
        store.set_close_to_tray(!settings.quit_on_close);
        store.set_prefer_kind(bilby_core::compat::Prefer::named(&settings.prefer_kind).number());
        store.set_prefer_applies(self.platform.read().unwrap().os == bilby_core::platform::Os::Linux);
        store.set_own_sources(ModelRc::new(VecModel::from(
            settings
                .own_sources
                .iter()
                .map(|own| SourceChoice {
                    id: own.url.as_str().into(),
                    label: l(own.name.as_str()),
                    on: true,
                    count: 0,
                })
                .collect::<Vec<_>>(),
        )));
        store.set_family_mode(settings.family_mode);
        store.set_adult_on_home(settings.adult_on_home);
        store.set_adult_uncovered(settings.adult_uncovered);
        store.set_has_pin(!settings.pin_hash.is_empty());
        store.set_pin_for_installs(settings.pin_for_installs);
        let steam = self.steam.read().unwrap().clone();
        // Steam is a desktop program: its library is offered wherever it runs.
        let os = self.platform.read().unwrap().os;
        store.set_steam_available(matches!(os, Os::Windows | Os::MacOs | Os::Linux));
        let gog = self.gog.read().unwrap().clone();
        store.set_gog_available(matches!(os, Os::Windows | Os::MacOs | Os::Linux));
        store.set_gog_signed_in(gog.is_some());
        store.set_gog_summary(l(gog.map(|a| format!("{} games", a.owned.len())).unwrap_or_default()));
        store.set_steam_signed_in(steam.is_some());
        store.set_steam_summary(l(steam.map(|a| format!("{} games and tools", a.owned.len())).unwrap_or_default()));
        store.set_downloads_dir(self.downloads_folder().display().to_string().into());
        store.set_downloads_custom(!settings.downloads_dir.trim().is_empty());
        store.set_ask_where(settings.ask_where);
        store.set_show_welcome(!settings.welcomed);
        store.set_interface_mode(interface_mode(&settings.interface));
        // "System" first, then every language by its own name.
        let system = system_language(self.real_device);
        let own_name = |code: &str| {
            i18n::languages().into_iter().find(|(known, _)| *known == code).map(|(_, name)| name).unwrap_or("English")
        };
        let mut languages = vec![SourceChoice {
            id: SharedString::new(),
            label: format!("{} ({})", i18n::tr("System"), own_name(system)).into(),
            on: settings.language.is_empty(),
            count: 0,
        }];
        languages.extend(i18n::languages().into_iter().map(|(code, name)| SourceChoice {
            id: code.into(),
            label: name.into(),
            on: settings.language == code,
            count: 0,
        }));
        store.set_language_name(
            languages.iter().find(|choice| choice.on).map(|choice| choice.label.clone()).unwrap_or_default(),
        );
        store.set_languages(ModelRc::new(VecModel::from(languages)));
        // The countries, the same way: the system's own first, named.
        let chosen = bilby_core::region::country(&settings.country);
        let mut countries = vec![SourceChoice {
            id: SharedString::new(),
            label: format!("{} ({})", i18n::tr("System"), bilby_core::region::name_of(&system_country())).into(),
            on: chosen.is_none(),
            count: 0,
        }];
        countries.extend(bilby_core::region::countries().iter().map(|(code, name)| SourceChoice {
            id: code.into(),
            label: name.into(),
            on: chosen.as_deref() == Some(code.as_str()),
            count: 0,
        }));
        store.set_country_name(
            countries.iter().find(|choice| choice.on).map(|choice| choice.label.clone()).unwrap_or_default(),
        );
        store.set_countries(ModelRc::new(VecModel::from(countries)));
        store.set_sources(ModelRc::new(VecModel::from(
            self.catalog()
                .available_sources()
                .into_iter()
                .map(|kind| SourceChoice {
                    id: kind.prefix().into(),
                    label: l(kind.label()),
                    on: !settings.disabled_sources.iter().any(|name| name == kind.prefix()),
                    count: 0,
                })
                .collect::<Vec<_>>(),
        )));
        ui.global::<Theme>().set_reduce_motion(settings.reduce_motion);
        store.set_can_register(self.self_program.is_some());
        store.set_in_menu(self.self_program.is_some() && install::self_entry_path(&self.dirs).is_file());
        self.refresh_quota(ui);
    }

    pub(crate) fn rebuild_catalog(&self) {
        let settings = self.state().settings;
        let fresh = (self.catalog_factory)(&self.platform.read().unwrap(), &settings, &self.dirs);
        // Ratings fetched earlier stay; the factory hands back a shared catalog, so the
        // old one's are passed on only where that is possible.
        let old = self.catalog();
        let fresh = match Arc::try_unwrap(fresh) {
            Ok(mut catalog) => {
                catalog.share_ratings_with(&old);
                Arc::new(catalog)
            }
            Err(shared) => shared,
        };
        *self.catalog.write().unwrap() = fresh;
    }

    /// Use the desktop or the TV layout, now and from the next start on. `mode` is 0 to
    /// let the store decide, 1 for desktop, 2 for TV.
    pub fn set_interface(self: &Arc<Self>, ui: &AppWindow, mode: i32) {
        let name = ["", "desktop", "tv"].get(mode as usize).copied().unwrap_or("");
        self.update_state(ui, |state| {
            state.settings.interface = name.to_string();
            // Having chosen is an answer to the question, too.
            state.settings.asked_about_tv = true;
        });
        let form = form_for(name, self.detected);
        let changed = {
            let mut platform = self.platform.write().unwrap();
            let changed = platform.form_factor != form;
            platform.form_factor = form;
            changed
        };
        if let Some(asking) = &self.asking {
            asking.on_tv(form == FormFactor::Tv);
        }
        let store = ui.global::<Store>();
        store.set_ask_tv(false);
        store.set_shell(shell_for(form));
        store.set_adaptive(form != FormFactor::Tv);
        // A TV has no use for window borders; a desktop wants them back.
        if !self.inline {
            ui.window().set_fullscreen(form == FormFactor::Tv);
        }
        // The TV layout is designed dark; elsewhere the chosen theme applies.
        let theme = self.theme_override.clone().unwrap_or_else(|| self.state().settings.theme);
        ui.global::<Theme>().set_mode(if form == FormFactor::Tv { 2 } else { theme_mode(&theme) });
        store.set_page(0);
        store.set_details_open(false);
        if changed {
            // What suits a TV differs (TV builds first, websites on Linux), so the lists
            // are fetched again for the new shape.
            self.rebuild_catalog();
            self.collections.lock().unwrap().clear();
            self.load_collections(ui);
            let platform = self.platform.read().unwrap().clone();
            store.set_platform_title(l(platform.title()));
            store.set_device_name(l(platform.friendly()));
            self.refresh(ui);
        }
        self.refresh_settings(ui);
    }

    /// Keep the store's own app-menu entry in step with the setting: there, and pointing
    /// at the file it is running from, or gone.
    pub(crate) fn apply_menu_entry(&self, ui: &AppWindow) {
        let Some(program) = &self.self_program else { return };
        if self.state().settings.hide_from_menu {
            install::unregister_self(&self.dirs);
        } else if let Err(e) = install::register_self(&self.dirs, program, ICON_PNG) {
            self.toast(ui, &format!("Could not add Bilby to the app menu: {e}"), true);
        }
    }

    /// Change the language Bilby is shown in. An empty code means the system's own.
    pub fn set_language(self: &Arc<Self>, ui: &AppWindow, code: &str) {
        let known = code.is_empty() || i18n::languages().iter().any(|(known, _)| *known == code);
        if !known {
            return;
        }
        self.update_state(ui, |state| state.settings.language = code.to_string());
        apply_language(if code.is_empty() { system_language(self.real_device) } else { code });
        // What the stores are asked for changes with it, and so does everything the
        // code has already put on screen.
        self.rebuild_catalog();
        let store = ui.global::<Store>();
        let platform = self.platform.read().unwrap().clone();
        store.set_platform_title(l(platform.title()));
        store.set_device_name(l(platform.friendly()));
        self.refresh_settings(ui);
        self.show_system(ui, 0, "");
        self.load_collections(ui);
        self.refresh(ui);
        if store.get_details_open() {
            let id = store.get_app().id.to_string();
            self.open(ui, &id);
        }
    }

    /// The folder of an installed Windows app, by the app's id.
    pub(crate) fn windows_folder(&self, id: &str) -> Option<PathBuf> {
        let path = self.state().installed(id)?.path.clone()?;
        bilby_core::winrun::folder_of(&self.dirs, std::path::Path::new(&path))
    }

    /// Change how Windows apps run: in general, or (given an id) for one app alone.
    /// The launchers read the choices each time they start, so nothing else changes.
    pub(crate) fn change_windows(
        &self,
        ui: &AppWindow,
        id: Option<&str>,
        change: impl FnOnce(&mut bilby_core::winrun::Choices),
    ) {
        use bilby_core::winrun::{self, Choices};
        let file = match id {
            None => winrun::general_choices(&self.dirs),
            Some(id) => match self.windows_folder(id) {
                Some(folder) => winrun::app_choices(&folder),
                None => return,
            },
        };
        let mut choices = Choices::read(&file);
        change(&mut choices);
        if let Err(e) = choices.write(&file) {
            self.toast(ui, &format!("That could not be saved: {e}"), true);
        }
        self.refresh_settings(ui);
    }

    /// Change the country whose prices the stores are asked for. An empty code means
    /// the system's own.
    pub fn set_country(self: &Arc<Self>, ui: &AppWindow, code: &str) {
        let known = code.is_empty() || bilby_core::region::countries().iter().any(|(known, _)| known == code);
        if !known || self.state().settings.country == code {
            return;
        }
        self.update_state(ui, |state| state.settings.country = code.to_string());
        // The stores are asked afresh: what is on screen carries the old prices.
        self.rebuild_catalog();
        self.refresh_settings(ui);
        self.load_collections(ui);
        self.refresh(ui);
        let store = ui.global::<Store>();
        if store.get_details_open() {
            let id = store.get_app().id.to_string();
            self.open(ui, &id);
        }
    }

    /// The country after this one in the list (or before it), and round again: for a remote.
    pub fn step_country(self: &Arc<Self>, ui: &AppWindow, forward: bool) {
        let mut codes = vec![""];
        codes.extend(bilby_core::region::countries().iter().map(|(code, _)| code.as_str()));
        let now = self.state().settings.country;
        let at = codes.iter().position(|code| *code == now).unwrap_or(0);
        let next = if forward { at + 1 } else { at + codes.len() - 1 } % codes.len();
        self.set_country(ui, codes[next]);
    }

    /// The language after this one in the list, and round again: for a remote.
    pub fn next_language(self: &Arc<Self>, ui: &AppWindow) {
        let mut codes = vec![""];
        codes.extend(i18n::languages().into_iter().map(|(code, _)| code));
        let now = self.state().settings.language;
        let at = codes.iter().position(|code| *code == now).unwrap_or(0);
        self.set_language(ui, codes[(at + 1) % codes.len()]);
    }

    /// Where downloads go: the folder the person chose in Settings, else the usual one.
    pub(crate) fn downloads_folder(&self) -> PathBuf {
        let chosen = self.state.lock().unwrap().settings.downloads_dir.trim().to_string();
        if chosen.is_empty() {
            self.dirs.downloads.clone()
        } else {
            PathBuf::from(chosen)
        }
    }

    /// Add a list of apps on the person's own server or storage, by its address. It
    /// is fetched first: something that is not such a list is refused with the reason.
    pub fn add_own_source(self: &Arc<Self>, ui: &AppWindow, url: &str) {
        let url = url.trim().to_string();
        if url.is_empty() {
            return;
        }
        if self.state().settings.own_sources.iter().any(|own| own.url == url) {
            self.toast(ui, "That source is already among yours.", true);
            return;
        }
        self.toast(ui, "Looking at that address…", false);
        let address = url.clone();
        self.job(
            move |app| bilby_core::source::own::look(app.catalog().http().as_ref(), &address),
            move |app, ui, found| match found {
                Ok((name, apps)) => {
                    // Two sources may not go by the same short name: their apps' ids would clash.
                    let taken = |candidate: &str| {
                        let slug = bilby_core::source::own::slug(candidate);
                        app.state()
                            .settings
                            .own_sources
                            .iter()
                            .any(|own| bilby_core::source::own::slug(&own.name) == slug)
                    };
                    let mut called = name.clone();
                    let mut n = 2;
                    while taken(&called) {
                        called = format!("{name} {n}");
                        n += 1;
                    }
                    let own = bilby_core::state::OwnSource { name: called.clone(), url: url.clone() };
                    app.update_state(ui, |state| state.settings.own_sources.push(own));
                    app.rebuild_catalog();
                    app.collections.lock().unwrap().clear();
                    app.load_collections(ui);
                    app.refresh_settings(ui);
                    app.refresh(ui);
                    let apps = if apps == 1 { "1 app".to_string() } else { format!("{apps} apps") };
                    app.toast(ui, &format!("Added “{called}”: {apps}"), false);
                }
                Err(e) => app.toast(ui, &format!("Not added. {e}"), true),
            },
        );
    }

    /// Take one of the person's own sources away. What was installed from it stays.
    pub fn remove_own_source(self: &Arc<Self>, ui: &AppWindow, url: &str) {
        let url = url.to_string();
        self.update_state(ui, |state| state.settings.own_sources.retain(|own| own.url != url));
        self.rebuild_catalog();
        self.collections.lock().unwrap().clear();
        self.load_collections(ui);
        self.refresh_settings(ui);
        self.refresh(ui);
        self.toast(ui, "Source removed", false);
    }

    /// Choose the folder downloads go to, with the system's own dialog.
    pub fn choose_downloads(&self, ui: &AppWindow) {
        match (self.picker)(&self.downloads_folder()) {
            Picked::Folder(folder) => self.set_downloads(ui, &folder.display().to_string()),
            Picked::Cancelled => {}
            Picked::NoDialog => self.toast(
                ui,
                "This system has no folder chooser Bilby can call. Type the folder's path instead.",
                true,
            ),
        }
    }

    /// Set the folder downloads go to. Empty goes back to the usual one. A folder
    /// that cannot be made or written to is refused, with the reason.
    pub fn set_downloads(&self, ui: &AppWindow, path: &str) {
        let path = path.trim();
        if !path.is_empty() {
            let folder = PathBuf::from(path);
            let usable = folder.is_absolute()
                && std::fs::create_dir_all(&folder).is_ok()
                && std::fs::write(folder.join(".bilby-write-test"), b"").is_ok();
            let _ = std::fs::remove_file(folder.join(".bilby-write-test"));
            if !usable {
                self.toast(
                    ui,
                    "That folder cannot be used: it must be a full path that this account can write to.",
                    true,
                );
                return;
            }
        }
        let path = path.to_string();
        self.update_state(ui, |state| state.settings.downloads_dir = path);
        self.refresh_settings(ui);
        self.toast(ui, &format!("Downloads now go to {}", self.downloads_folder().display()), false);
    }

    /// Make one of the quiet sounds of moving about, unless they are switched off.
    pub fn play(&self, sound: sound::Sound) {
        if self.state.lock().unwrap().settings.mute_sounds {
            return;
        }
        if let Some(heard) = &self.heard {
            heard(sound);
            return;
        }
        #[cfg(not(target_os = "android"))]
        if !self.silent {
            self.player.get_or_init(sound::Player::start).play(sound);
        }
    }

    /// Change a family or adult-content setting, asking first where that is called for.
    pub fn request_setting(self: &Arc<Self>, ui: &AppWindow, change: Change) {
        use bilby_core::parental::{to_change_settings, Gate};
        let settings = self.state().settings;
        let has_pin = !settings.pin_hash.is_empty();
        let stricter = matches!(
            change,
            Change::FamilyMode(true)
                | Change::AdultOnHome(false)
                | Change::AdultUncovered(false)
                | Change::PinForInstalls(true)
                | Change::AutoUpdate(false)
        );
        if to_change_settings(has_pin, stricter) == Gate::Pin {
            self.ask(ui, 2, "Enter the PIN", "The PIN guards these settings.", Guarded::Setting(change));
            return;
        }
        // Without a PIN, asking for adult things on the home page is itself the
        // question of being an adult.
        if change == Change::AdultUncovered(true) && !settings.adult_confirmed {
            let text = "This shows things meant for adults as they are, with nothing covered. Are you 18 or over?";
            self.ask(ui, 1, "Adult content", text, Guarded::Setting(change));
            return;
        }
        if change == Change::AdultOnHome(true) && !settings.adult_confirmed {
            let text = "This shows things meant for adults on the home page, covered until chosen. Are you 18 or over?";
            self.ask(ui, 1, "Adult content", text, Guarded::Setting(change));
            return;
        }
        self.apply_setting(ui, change);
    }

    pub(crate) fn apply_setting(self: &Arc<Self>, ui: &AppWindow, change: Change) {
        match change {
            Change::SetPin => {
                *self.pending.lock().unwrap() = None;
                let store = ui.global::<Store>();
                store.set_gate_title(l("Choose a PIN"));
                store.set_gate_text(
                    "Four digits. It will be asked for to change these settings and to open adult content.".into(),
                );
                store.set_gate_error(SharedString::new());
                store.set_gate(3);
                return;
            }
            Change::RemovePin => {
                self.update_state(ui, |state| {
                    state.settings.pin_hash.clear();
                    state.settings.pin_salt.clear();
                    state.settings.pin_for_installs = false;
                });
                self.adult_unlocked.store(false, Ordering::SeqCst);
                self.toast(ui, "The PIN is removed", false);
            }
            Change::FamilyMode(on) => {
                self.update_state(ui, |state| state.settings.family_mode = on);
                let said = if on { "Family mode is on: nothing for adults is shown" } else { "Family mode is off" };
                self.toast(ui, said, false);
            }
            Change::AdultOnHome(on) => {
                self.update_state(ui, |state| {
                    state.settings.adult_on_home = on;
                    state.settings.adult_confirmed |= on;
                });
            }
            Change::AdultUncovered(on) => {
                self.update_state(ui, |state| {
                    state.settings.adult_uncovered = on;
                    state.settings.adult_confirmed |= on;
                });
            }
            Change::AutoUpdate(on) => {
                self.update_state(ui, |state| state.settings.auto_update = on);
                self.refresh_settings(ui);
                if on {
                    // Start now with whatever is waiting.
                    self.offered.store(false, Ordering::SeqCst);
                    self.check_everything(ui);
                }
                return;
            }
            Change::PinForInstalls(on) => {
                if on && self.state().settings.pin_hash.is_empty() {
                    self.toast(ui, "Set a PIN first", true);
                } else {
                    self.update_state(ui, |state| state.settings.pin_for_installs = on);
                }
            }
        }
        // What is listed depends on it: build the catalog again and redraw.
        self.rebuild_catalog();
        self.collections.lock().unwrap().clear();
        self.load_collections(ui);
        self.refresh_settings(ui);
        self.refresh(ui);
    }
}
