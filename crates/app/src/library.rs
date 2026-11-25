//! What is on the device: finding it, starting it and taking it off.

#[cfg(target_os = "linux")]
use crate::activation::wayland_activation;
use crate::gate::Guarded;
use crate::text::plain;
use crate::{App, AppWindow, Elsewhere, Store};
use bilby_core::model::{AppId, SourceKind};
use bilby_core::state::State;
use bilby_core::{install, Error};
use slint::{ComponentHandle, SharedString};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl App {
    /// Find what is installed on the device that the store did not install: Flatpaks,
    /// Steam games and the system's packages. Read from disk, in the background.
    pub fn scan_installed(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        self.job(
            |app| {
                let os = app.platform.read().unwrap().os;
                let mut apps: Vec<(String, String)> =
                    bilby_core::source::flathub::installed_apps(&app.dirs.flatpak_roots)
                        .into_iter()
                        .map(|(id, name)| (format!("flathub:{id}"), name))
                        .collect();
                let libraries =
                    app.steam_libraries.clone().unwrap_or_else(|| bilby_core::source::steam::default_libraries(os));
                apps.extend(
                    bilby_core::source::steam::installed_games(&libraries)
                        .into_iter()
                        .map(|(id, name)| (format!("steam:{id}"), name)),
                );
                // What the system's package manager installed and shows in the app menu.
                let catalog = app.catalog();
                let mut entries = std::collections::HashMap::new();
                for (id, name, entry) in catalog.installed_programs() {
                    let id = id.to_string();
                    if !apps.iter().any(|(known, _)| known.eq_ignore_ascii_case(&id)) {
                        if let Some(entry) = entry {
                            entries.insert(id.to_ascii_lowercase(), entry);
                        }
                        apps.push((id, name));
                    }
                }
                let packages = catalog.installed_packages().iter().map(|name| plain(name)).collect();
                app.give_web_apps_their_own_windows();
                Elsewhere { apps, packages, entries }
            },
            |app, ui, found| {
                *app.elsewhere.write().unwrap() = found;
                app.refresh(ui);
            },
        );
    }

    /// Web apps added while no browser here could give them a window of their own
    /// open as pages in the usual browser. Once such a browser is installed, their
    /// menu entries are written again to use it.
    pub(crate) fn give_web_apps_their_own_windows(&self) {
        if self.opens_sites_in_longboard() || install::find_app_browser(&self.dirs).is_none() {
            return;
        }
        for app in self.state().library.iter().filter(|app| app.id.starts_with("web:")) {
            let entry = install::menu_entry_path(&self.dirs, &app.id);
            let plain = std::fs::read_to_string(&entry).is_ok_and(|text| !text.contains("--app="));
            if let (true, Some(host)) = (plain, app.id.strip_prefix("web:")) {
                let url = bilby_core::source::web::address(host);
                let _ = install::write_web_shortcut(&self.dirs, &app.id, &app.name, &url);
            }
        }
    }

    /// Is this app on the device although the store did not install it? Its own entry
    /// may be (a Flatpak, a Steam game), or the system's package of the same name: the
    /// VLC on Flathub's page is "installed" when the distribution's vlc is.
    pub(crate) fn installed_elsewhere(&self, id: &AppId, name: &str) -> bool {
        let known = self.elsewhere.read().unwrap();
        let text = id.to_string();
        if known.apps.iter().any(|(app, _)| app.eq_ignore_ascii_case(&text)) {
            return true;
        }
        // By name, only when the app's name and the last part of its id agree on it,
        // so that somebody's unrelated project called "notes" is not marked.
        let tail = plain(id.key.rsplit(['/', '.']).next().unwrap_or(&id.key));
        tail.chars().count() >= 3 && tail == plain(name) && known.packages.contains(&tail)
    }

    /// The file to run for an app the store placed itself, if it is still there.
    pub(crate) fn launchable(&self, id: &str) -> Option<PathBuf> {
        // The store itself is already open. Only a newer copy just placed can be
        // "opened": that restarts into it.
        if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(id)) {
            return self.self_new.lock().unwrap().clone();
        }
        let state = self.state.lock().unwrap();
        // A web app is opened by starting Longboard on its address on a TV, and in
        // the person's own browser anywhere else.
        if id.starts_with("web:") {
            state.installed(id)?;
            if !self.opens_sites_in_longboard() {
                return Some(PathBuf::from("browser"));
            }
            return install::find_longboard(&self.dirs).map(|longboard| longboard.program);
        }
        // A Flatpak is run by Flatpak, whoever installed it.
        if let Some(app_id) = id.strip_prefix("flathub:") {
            return bilby_core::source::flathub::is_installed(&self.dirs.flatpak_roots, app_id)
                .then(|| self.dirs.flatpak.clone());
        }
        // So is one that arrived as a bundle from the developer's own page.
        if let Some(app_id) = state.installed(id).map(|a| a.flatpak.as_str()).filter(|f| !f.is_empty()) {
            return install::flatpak_app_installed(&self.dirs, app_id).then(|| self.dirs.flatpak.clone());
        }
        // Something Steam has installed is started by asking Steam.
        if let Some(number) = id.strip_prefix("steam:") {
            use bilby_core::source::steam;
            let os = self.platform.read().unwrap().os;
            let libraries = self.steam_libraries.clone().unwrap_or_else(|| steam::default_libraries(os));
            return steam::is_installed(&libraries, number).then(|| steam::run_link(number)).flatten();
        }
        // What the system's package manager installed is started as its menu entry says.
        if state.installed(id).is_none() {
            if let Some(entry) = self.outside_entry(id) {
                return Some(entry);
            }
        }
        let app = state.installed(id).filter(|a| a.store_managed)?;
        let path = PathBuf::from(app.path.as_deref()?);
        (path.starts_with(&self.dirs.applications) && path.is_file()).then_some(path)
    }

    /// The Flatpak app behind this entry, when Flatpak is what runs it: one from Flathub,
    /// or one the store installed from a downloaded bundle.
    pub(crate) fn flatpak_app(&self, id: &str) -> Option<String> {
        if let Some(app_id) = id.strip_prefix("flathub:") {
            return Some(app_id.to_string());
        }
        let state = self.state.lock().unwrap();
        state.installed(id).map(|a| a.flatpak.clone()).filter(|f| install::flatpak_app_id_ok(f))
    }

    /// Start the open app, when the store installed it as a file it can run.
    ///
    /// The window system is first asked for leave to bring another window to the
    /// front (see `install::grant_activation`), which takes a moment to arrive; the app
    /// is started when it has, or shortly after if it never does.
    pub fn launch(self: &Arc<Self>, ui: &AppWindow) {
        #[cfg(target_os = "linux")]
        if !self.inline && !self.launch_waiting.load(Ordering::SeqCst) {
            use slint::winit_030::winit::platform::startup_notify::WindowExtStartupNotify;
            use slint::winit_030::WinitWindowAccessor;
            // On Wayland the store asks itself, with the serial of the press that led
            // here, and has the answer at once.
            if let Some(token) = wayland_activation(ui, |activation| activation.token()).flatten() {
                install::grant_activation(token);
                self.launch_now(ui);
                return;
            }
            // Elsewhere (the X Window System) the windowing library asks, and the
            // answer arrives as an event a moment later.
            let asked = ui.window().with_winit_window(|window| window.request_activation_token().is_ok());
            if asked == Some(true) {
                self.launch_waiting.store(true, Ordering::SeqCst);
                let (app, weak) = (self.clone(), ui.as_weak());
                slint::Timer::single_shot(std::time::Duration::from_millis(600), move || {
                    if let Some(ui) = weak.upgrade() {
                        app.launch_granted(&ui, None);
                    }
                });
                return;
            }
        }
        self.launch_now(ui);
    }

    /// The window system answered (or took too long to): start what was waiting, once.
    pub(crate) fn launch_granted(self: &Arc<Self>, ui: &AppWindow, token: Option<String>) {
        if !self.launch_waiting.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Some(token) = token {
            install::grant_activation(token);
        }
        self.launch_now(ui);
    }

    pub(crate) fn launch_now(self: &Arc<Self>, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let id = store.get_app().id.to_string();
        let Some(path) = self.launchable(&id) else { return };
        // The store's own newer copy: start it and leave, so the person is in the new
        // version at once.
        if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(&id)) {
            self.restart_into_new(ui);
            return;
        }
        let started = if let Some(host) = id.strip_prefix("web:").filter(|_| !self.opens_sites_in_longboard()) {
            // A window of its own where a browser here can give it one; else a page
            // in the usual browser.
            let url = bilby_core::source::web::address(host);
            match install::find_app_browser(&self.dirs) {
                Some(browser) => (self.runner)(&browser.program, &browser.args(&url)),
                None => (self.browser)(&url),
            }
        } else if let Some(host) = id.strip_prefix("web:") {
            match install::find_longboard(&self.dirs) {
                Some(longboard) => (self.runner)(
                    &longboard.program,
                    &longboard.args(&[format!("--url={}", bilby_core::source::web::address(host))]),
                ),
                None => Err(Error::Unsupported("Longboard is no longer on this device.".into())),
            }
        } else if let Some(app_id) = self.flatpak_app(&id) {
            (self.runner)(&path, &["run".to_string(), app_id])
        } else if id.starts_with("steam:") {
            // A link, handed to the system, which passes it to Steam.
            (self.opener)(&path)
        } else if self.outside_entry(&id).is_some_and(|entry| entry == path) {
            self.launch_outside(&path)
        } else {
            // A program is watched for a moment as it starts, away from the interface:
            // one that closes straight away says why.
            let requirement = self.needed.lock().unwrap().clone();
            // Only an AppImage needs FUSE; one already unpacked starts as it is.
            let lacks_fuse = self.lacks_fuse.load(Ordering::SeqCst) && bilby_core::needs::as_appimage(&path).is_some();
            let layering = if lacks_fuse { self.layering.clone() } else { None };
            // Already added and waiting on a restart: starting it now would only fail.
            if layering.is_some() && self.fuse_staged.load(Ordering::SeqCst) {
                self.ask_restart(ui);
                return;
            }
            let said = match (lacks_fuse, &requirement) {
                _ if layering.is_some() => format!("Adding what {} needs to this system…", store.get_app().name),
                (true, Some(_)) => format!("Installing what {} needs, then opening it…", store.get_app().name),
                (true, None) => {
                    format!("Getting {} ready to run on this system, then opening it…", store.get_app().name)
                }
                _ => format!("Opening {}…", store.get_app().name),
            };
            self.toast(ui, &said, false);
            let launcher = self.launcher.clone();
            let app_id = id.clone();
            let app_name = store.get_app().name.to_string();
            self.job(
                move |app| {
                    // As when installing: FUSE where it can be had, else unpack the app.
                    let mut program = path.clone();
                    let mut unpack = lacks_fuse && requirement.is_none();
                    if let Some(tool) = &layering {
                        // Added: it runs after a restart, which is asked about next.
                        if app.stage_fuse(tool) {
                            return Ok(None);
                        }
                        unpack = true;
                    } else if let (true, Some(package)) = (lacks_fuse, &requirement) {
                        unpack = app.install_requirement(package).is_err();
                    }
                    if unpack {
                        program = install::unpack_appimage(&path)?;
                    }
                    launcher(&program)?;
                    Ok((program != path).then_some(program))
                },
                move |app, ui, started: bilby_core::Result<Option<PathBuf>>| match started {
                    Ok(None) if app.fuse_just_staged.swap(false, Ordering::SeqCst) => {
                        app.show_app_sources(ui);
                        app.ask_restart(ui);
                    }
                    // It runs from its folder now: the library and the menu follow.
                    Ok(Some(program)) => {
                        let now = program.display().to_string();
                        app.update_state(ui, |state| {
                            if let Some(entry) = state.library.iter_mut().find(|a| a.id == app_id) {
                                entry.path = Some(now.clone());
                            }
                        });
                        let _ = install::add_menu_entry(&app.dirs, &app_id, &app_name, &program);
                        app.show_app_sources(ui);
                    }
                    Ok(None) => {}
                    Err(e) => app.toast(ui, &format!("Couldn't open it. {e}"), true),
                },
            );
            return;
        };
        match started {
            Ok(()) => self.toast(ui, &format!("Opening {}…", store.get_app().name), false),
            Err(e) => self.toast(ui, &format!("Couldn't open it: {e}"), true),
        }
    }

    /// Is the app on the page one the store did not install, but can take off the
    /// device: a Flatpak, a Steam game, or something the system's own tools look after?
    pub(crate) fn can_uninstall(&self, state: &State, store: &Store) -> bool {
        let id = store.get_app().id.to_string();
        if id.is_empty() || state.installed(&id).is_some() {
            return false;
        }
        let Some(app) = AppId::parse(&id) else { return false };
        match app.source {
            SourceKind::Flathub => bilby_core::source::flathub::is_installed(&self.dirs.flatpak_roots, &app.key),
            SourceKind::Steam => {
                self.elsewhere.read().unwrap().apps.iter().any(|(known, _)| known.eq_ignore_ascii_case(&id))
            }
            _ => {
                !store.get_installed_version().is_empty()
                    && install::removal(&app, &self.platform.read().unwrap()).is_some()
            }
        }
    }

    /// Take an app off the device that the store did not put there. Flatpak and Steam
    /// are asked to; what the system's own tools look after is removed with them, by
    /// itself or not at all.
    pub(crate) fn uninstall_outside(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        let Some(app_id) = AppId::parse(id) else { return };
        let store = ui.global::<Store>();
        let shown = store.get_app();
        let name = if shown.id == id && !shown.name.is_empty() { shown.name.to_string() } else { app_id.key.clone() };
        let forget =
            |app: &App| app.elsewhere.write().unwrap().apps.retain(|(known, _)| !known.eq_ignore_ascii_case(id));
        match app_id.source {
            SourceKind::Flathub => {
                let args = ["uninstall", "--noninteractive", "--assumeyes", app_id.key.as_str()].map(String::from);
                match (self.runner)(&self.dirs.flatpak, &args) {
                    Ok(()) => {
                        forget(self);
                        if shown.id == id {
                            store.set_installed_version(SharedString::new());
                        }
                        self.refresh(ui);
                        self.toast(ui, &format!("Uninstalling {name}…"), false);
                    }
                    Err(e) => self.toast(ui, &format!("{name} could not be uninstalled: {e}"), true),
                }
            }
            SourceKind::Steam => {
                // Steam removes its own games, and asks first.
                match (self.opener)(std::path::Path::new(&format!("steam://uninstall/{}", app_id.key))) {
                    Ok(()) => self.toast(ui, &format!("Steam asks whether to uninstall {name}"), false),
                    Err(e) => self.toast(ui, &format!("{name} could not be uninstalled: {e}"), true),
                }
            }
            _ => self.uninstall_with_system(ui, &app_id, &name),
        }
    }

    /// Have the system's own tool remove a package, a snap or a winget package.
    pub(crate) fn uninstall_with_system(self: &Arc<Self>, ui: &AppWindow, app_id: &AppId, name: &str) {
        let platform = self.platform.read().unwrap().clone();
        let Some(removal) = install::removal(app_id, &platform) else { return };
        let later = removal.command.contains("rpm-ostree");
        self.toast(ui, &format!("Uninstalling {name}…"), false);
        let (id, name) = (app_id.to_string(), name.to_string());
        self.job(
            move |app| install::remove_with(&removal, &app.dirs),
            move |app, ui, result| {
                let store = ui.global::<Store>();
                match result {
                    Ok(()) if later => app.toast(ui, &format!("{name} will be gone after a restart"), false),
                    Ok(()) => {
                        if store.get_app().id == id.as_str() {
                            store.set_installed_version(SharedString::new());
                        }
                        app.toast(ui, &format!("{name} uninstalled"), false);
                    }
                    Err(Error::Cancelled) => app.toast(ui, &format!("{name} was not uninstalled"), false),
                    Err(e) => app.toast(ui, &format!("{name} could not be uninstalled: {e}"), true),
                }
                app.scan_installed(ui);
                app.refresh(ui);
            },
        );
    }

    pub fn remove_from_library(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(id)) {
            self.toast(ui, "That is Bilby itself, which you are using. To remove it, delete its file.", true);
            return;
        }
        if self.needs_pin_to_install() {
            let why = "A PIN is needed to install or remove apps on this device.";
            self.ask(ui, 2, "Enter the PIN", why, Guarded::Remove(id.to_string()));
            return;
        }
        let mut removed = None;
        self.update_state(ui, |state| removed = state.remove(id));
        // Not the store's own: something else put it on the device.
        let Some(entry) = removed else { return self.uninstall_outside(ui, id) };
        self.updates.lock().unwrap().remove(id);
        let mut message = format!("{} removed from your library", entry.name);
        // Only files the store itself placed, and only inside its own Applications folder.
        if let (true, Some(path)) = (entry.store_managed, entry.path.as_deref()) {
            let path = PathBuf::from(path);
            if install::remove_placed(&self.dirs, &path) {
                message = format!("{} uninstalled", entry.name);
            }
        }
        // Whatever the store put in the app menu for it goes too (an AppImage, a web app).
        install::remove_menu_entry(&self.dirs, id);
        if id.starts_with("web:") {
            message = format!("{} removed from this TV's app list", entry.name);
        }
        // A Flatpak is removed by Flatpak; it carries on after this returns.
        let bundled = Some(entry.flatpak.as_str()).filter(|f| install::flatpak_app_id_ok(f));
        if let Some(app_id) = id.strip_prefix("flathub:").or(bundled) {
            let args = ["uninstall", "--noninteractive", "--assumeyes", app_id].map(String::from);
            message = match (self.runner)(&self.dirs.flatpak, &args) {
                Ok(()) => format!("Uninstalling {}…", entry.name),
                Err(_) => {
                    format!("{} removed from your library. Flatpak could not be started to uninstall it.", entry.name)
                }
            };
        }
        let store = ui.global::<Store>();
        if store.get_app().id == id {
            store.set_installed_version(SharedString::new());
        }
        self.refresh(ui);
        self.toast(ui, &message, false);
        // What the system's package manager installed for the store, it removes too.
        if let Some(app_id) = AppId::parse(id) {
            let there = self.catalog().installed_packages().contains(&app_id.key);
            if there || matches!(app_id.source, SourceKind::Snap | SourceKind::Winget) {
                self.uninstall_with_system(ui, &app_id, &entry.name);
            }
        }
    }
}
