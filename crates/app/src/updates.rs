//! Keeping the library and Bilby itself up to date, asked for or on their own.

use crate::cards::at_the_top;
use crate::gate::Guarded;
use crate::options::Options;
use crate::text::{l, plural};
use crate::{i18n, instance, App, AppWindow, Current, Store};
use bilby_core::catalog::UpdateStatus;
use bilby_core::install::{self, Dirs};
use bilby_core::model::{AppId, AppSummary, SourceKind};
use bilby_core::state::{now_unix, InstalledApp, State};
use slint::{ComponentHandle, SharedString};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Set for a copy that was started by an older one handing over, so that two copies
/// can never pass the person back and forth.
pub(crate) const HANDED_OVER: &str = "BILBY_HANDED_OVER";

/// A newer copy of the store than the one starting, which the store itself placed and
/// which is still there.
pub(crate) fn newer_own_copy(options: &Options) -> Option<PathBuf> {
    if std::env::var_os(HANDED_OVER).is_some() {
        return None;
    }
    let dirs = options.dirs.clone().unwrap_or_else(Dirs::for_current_user);
    let state = State::load(&options.state_file.clone().unwrap_or_else(|| dirs.config.join("state.json")));
    let own = options
        .store_app
        .clone()
        .unwrap_or_else(|| AppId::new(SourceKind::GitHub, bilby_core::catalog::STORE_REPOSITORY).to_string());
    newer_copy_in(&state, &own, &dirs, env!("CARGO_PKG_VERSION"), options.self_program.as_deref())
}

/// The rule itself, apart from the machine: the library's record of the store, when it
/// names a file in the store's own Applications folder, of a newer version, that is
/// not the one already running.
pub fn newer_copy_in(
    state: &State,
    own: &str,
    dirs: &Dirs,
    running: &str,
    program: Option<&std::path::Path>,
) -> Option<PathBuf> {
    let entry = state.library.iter().find(|app| app.id.eq_ignore_ascii_case(own))?;
    let path = PathBuf::from(entry.path.as_deref()?);
    let placed = entry.store_managed && path.starts_with(&dirs.applications) && path.is_file();
    let newer = bilby_core::version::is_newer(&entry.version, running);
    (placed && newer && program != Some(path.as_path())).then_some(path)
}

impl App {
    /// Update every library app that has an update waiting, one after another. A failure
    /// is reported and the rest carry on.
    pub fn update_all(self: &Arc<Self>, ui: &AppWindow) {
        self.start_updates(ui, false);
    }

    /// With "update apps automatically" on: install what the last check found, with
    /// nobody asked. Updates that need the person stay waiting in the Library.
    pub(crate) fn update_unattended(self: &Arc<Self>, ui: &AppWindow) {
        if self.state().settings.auto_update && self.update_queue.lock().unwrap().is_empty() {
            self.start_updates(ui, true);
        }
    }

    /// How many updates the store had to leave for the person (a password, an
    /// installer), and whether Bilby or the device waits for a restart.
    pub(crate) fn left_for_the_person(&self, ui: &AppWindow) -> (usize, bool, bool) {
        let apps = {
            let updates = self.updates.lock().unwrap();
            let library = self.state().library;
            library.iter().filter(|app| matches!(updates.get(&app.id), Some(UpdateStatus::Available { .. }))).count()
        };
        let check = self.system_check.lock().unwrap().clone();
        let system = check.as_ref().map(|c| c.pending.len()).unwrap_or(0);
        let device = check.as_ref().is_some_and(|c| c.restart_needed);
        let _ = ui;
        (apps + system, self.self_new.lock().unwrap().is_some(), device)
    }

    /// With "update apps automatically" on, when the store has done what it can by
    /// itself: say what is left for the person. With the window open they are asked,
    /// once each time it is opened, whether to finish now or later; with the store in
    /// the system tray, a notification says so instead.
    pub fn offer_to_finish(self: &Arc<Self>, ui: &AppWindow) {
        let store = ui.global::<Store>();
        if !self.state().settings.auto_update || store.get_gate() != 0 {
            return;
        }
        // Still at it: asked when it has finished.
        if self.unattended.load(Ordering::SeqCst)
            || !store.get_updating_id().is_empty()
            || matches!(store.get_system_state(), 1 | 4)
            || store.get_checking_updates()
        {
            return;
        }
        // Asked once about everything, not about each half as it comes in: wait until
        // both the apps and the system have been looked at.
        let apps_unknown = self.last_checked.load(Ordering::SeqCst) == 0 && !self.state().library.is_empty();
        let system_unknown = self.system_check.lock().unwrap().is_none() && !self.system.managers.is_empty();
        if apps_unknown || system_unknown {
            return;
        }
        let (updates, bilby, device) = self.left_for_the_person(ui);
        if updates == 0 && !bilby && !device {
            return;
        }
        let said = if updates > 0 {
            format!(
                "{} waiting for you to finish: a password, an installer or a restart is needed.",
                plural(updates, "update")
            )
        } else if bilby {
            "A newer Bilby is installed. Restart Bilby to start using it.".to_string()
        } else {
            "Updates installed. Restart this device to finish.".to_string()
        };
        if self.in_tray.load(Ordering::SeqCst) {
            let mut notified = self.notified.lock().unwrap();
            if *notified != said {
                (self.notifier)("Bilby", &i18n::tr(&said));
                *notified = said;
            }
            return;
        }
        if self.offered.swap(true, Ordering::SeqCst) {
            return;
        }
        if updates > 0 {
            self.ask(ui, 1, "Updates are waiting for you", &said, Guarded::FinishUpdates);
            store.set_gate_no(l("Later"));
            store.set_gate_yes(l("Finish now"));
        } else if bilby {
            self.ask(ui, 1, "Bilby is updated", &said, Guarded::RestartBilby);
            store.set_gate_no(l("Later"));
            store.set_gate_yes(l("Restart Bilby now"));
        } else {
            self.ask_restart_for_updates(ui);
        }
    }

    /// The window has been opened again from the system tray (or the store started a
    /// second time): what is waiting for the person is offered afresh.
    pub fn window_opened(self: &Arc<Self>, ui: &AppWindow) {
        self.in_tray.store(false, Ordering::SeqCst);
        self.offered.store(false, Ordering::SeqCst);
        self.offer_to_finish(ui);
    }

    /// The window has been closed and the store stays in the system tray.
    pub fn window_closed_to_tray(&self) {
        self.in_tray.store(true, Ordering::SeqCst);
    }

    /// Look for everything there is to update, as "update apps automatically" does when
    /// the store opens and every few hours after: apps, Bilby itself and the system.
    pub fn check_everything(self: &Arc<Self>, ui: &AppWindow) {
        self.check_updates(ui, false);
        self.check_store_update(ui);
        self.check_system(ui, false);
    }

    pub(crate) fn start_updates(self: &Arc<Self>, ui: &AppWindow, unattended: bool) {
        if !ui.global::<Store>().get_updating_id().is_empty() {
            return;
        }
        let waiting: Vec<String> = {
            let updates = self.updates.lock().unwrap();
            self.state()
                .library
                .iter()
                .filter(|app| matches!(updates.get(&app.id), Some(UpdateStatus::Available { .. })))
                .map(|app| app.id.clone())
                .collect()
        };
        self.unattended.store(unattended, Ordering::SeqCst);
        *self.update_queue.lock().unwrap() = waiting.into();
        self.update_next(ui);
    }

    /// Start the next queued update, if any and if nothing is running.
    pub(crate) fn update_next(self: &Arc<Self>, ui: &AppWindow) {
        let store = ui.global::<Store>();
        while store.get_updating_id().is_empty()
            && store.get_install_state() != 1
            && self.installing.lock().unwrap().is_none()
        {
            let Some(id) = self.update_queue.lock().unwrap().pop_front() else {
                // The store's own round is over: what it had to leave is the person's.
                if self.unattended.swap(false, Ordering::SeqCst) {
                    self.offer_to_finish(ui);
                }
                return;
            };
            self.update_app(ui, &id);
        }
    }

    /// Update a library app in place: look up its newest release for this device and
    /// install the best option, without leaving the Library page.
    pub fn update_app(self: &Arc<Self>, ui: &AppWindow, id: &str) {
        let store = ui.global::<Store>();
        if store.get_install_state() == 1
            || !store.get_updating_id().is_empty()
            || self.installing.lock().unwrap().is_some()
        {
            // Something is downloading: this update takes its turn after it.
            let mut queue = self.update_queue.lock().unwrap();
            if self.state.lock().unwrap().installed(id).is_some() && !queue.iter().any(|waiting| waiting == id) {
                queue.push_back(id.to_string());
            }
            return;
        }
        let Some(app_id) = AppId::parse(id) else { return };
        let Some(entry) = self.state.lock().unwrap().installed(id).cloned() else { return };
        store.set_updating_id(id.into());
        store.set_install_progress(0.0);
        let id = id.to_string();
        self.job(
            move |app| app.catalog().details(&app_id).map_err(|e| e.to_string()),
            move |app, ui, result| {
                let store = ui.global::<Store>();
                let fail = |message: String| {
                    store.set_updating_id(SharedString::new());
                    app.toast(ui, &message, true);
                    app.update_next(ui);
                };
                let details = match result {
                    Ok(details) => details,
                    Err(e) => return fail(format!("Couldn't update {}: {e}", entry.name)),
                };
                let Some(picked) =
                    details.release.as_ref().filter(|r| !r.options.is_empty()).map(|r| r.version.clone())
                else {
                    return fail(format!("{} no longer has a file for this device", entry.name));
                };
                let selected = details.versions.iter().position(|r| r.version == picked);
                if app.unattended.load(Ordering::SeqCst) {
                    let alone = details.release.as_ref().is_some_and(|r| install::unattended(&r.options[0].plan));
                    if !alone {
                        // It needs the person: it stays waiting in the Library.
                        store.set_updating_id(SharedString::new());
                        return app.update_next(ui);
                    }
                    app.quiet_install.store(true, Ordering::SeqCst);
                }
                // The install step reads the app in focus; make it this one.
                store.set_app(at_the_top(app.library_card(&entry)));
                *app.current.lock().unwrap() = Current { id, details: Some(details), selected };
                app.install(ui, 0);
                // Nothing started (should not happen once an option exists): do not get stuck.
                if store.get_install_state() == 0 && !store.get_updating_id().is_empty() {
                    store.set_updating_id(SharedString::new());
                }
            },
        );
    }

    pub fn check_updates(self: &Arc<Self>, ui: &AppWindow, announce: bool) {
        let store = ui.global::<Store>();
        let apps: Vec<(AppId, String)> =
            self.state().library.iter().filter_map(|a| Some((AppId::parse(&a.id)?, a.version.clone()))).collect();
        if apps.is_empty() || store.get_checking_updates() {
            return;
        }
        store.set_checking_updates(true);
        let work_apps = apps.clone();
        self.job(
            move |app| app.catalog().check_updates(&work_apps),
            move |app, ui, statuses| {
                let available = statuses.iter().filter(|s| matches!(s, UpdateStatus::Available { .. })).count();
                let failed = statuses.iter().filter(|s| matches!(s, UpdateStatus::Unknown(_))).count();
                {
                    let mut updates = app.updates.lock().unwrap();
                    let state = app.state();
                    for ((id, _), status) in apps.iter().zip(statuses) {
                        let id = id.to_string();
                        // A version the person chose to skip is not announced again.
                        let skipped = state.installed(&id).map(|a| a.skipped_version.clone()).unwrap_or_default();
                        let status = match status {
                            UpdateStatus::Available { version, .. } if version == skipped => UpdateStatus::UpToDate,
                            other => other,
                        };
                        updates.insert(id, status);
                    }
                }
                app.last_checked.store(now_unix(), Ordering::SeqCst);
                ui.global::<Store>().set_checking_updates(false);
                app.refresh(ui);
                if announce {
                    let message = match (available, failed) {
                        (0, 0) => "Everything is up to date".to_string(),
                        (0, f) => format!("Couldn't check {}", plural(f, "app")),
                        (n, _) => format!("{} available", plural(n, "update")),
                    };
                    app.toast(ui, &message, available == 0 && failed > 0);
                }
                app.update_unattended(ui);
            },
        );
    }

    /// Is there a newer release of the store itself with a file for this device? One
    /// request, and quiet: the answer appears in Settings.
    pub fn check_store_update(self: &Arc<Self>, ui: &AppWindow) {
        let _ = ui;
        let Some(id) = self.store_app.clone() else { return };
        self.job(
            move |app| app.catalog().check_update(&id, env!("CARGO_PKG_VERSION")),
            |_app, ui, status| {
                let version = match status {
                    UpdateStatus::Available { version, .. } => version,
                    // Up to date, nothing for this device, or could not be checked: say nothing.
                    _ => String::new(),
                };
                ui.global::<Store>().set_store_update(version.into());
            },
        );
    }

    /// The store is an app like any other and is, plainly, installed: it is kept in
    /// the library at the version that is running, so that it is listed, marked
    /// installed wherever it is shown, and offered updates like the rest.
    pub(crate) fn note_self(&self, ui: &AppWindow) {
        let Some(id) = self.store_app.clone() else { return };
        let running = format!("v{}", env!("CARGO_PKG_VERSION"));
        let program = self.self_program.as_ref().map(|path| path.display().to_string());
        // Nothing to note when the record already says this version, or names a newer
        // copy the store placed that is still there: then an older copy is running
        // (the update was put off until later) and the record of the newer one stands.
        let unchanged = self.state.lock().unwrap().installed(&id.to_string()).is_some_and(|own| {
            let placed =
                own.store_managed && own.path.as_deref().is_some_and(|path| std::path::Path::new(path).is_file());
            own.version == running || (placed && bilby_core::version::is_newer(&own.version, &running))
        });
        // An earlier version could write the store's name over another app's when that
        // app was updated from the Library. Such an entry gets its own name back.
        let own_id = id.to_string();
        let misnamed = |app: &InstalledApp| {
            app.name == "Bilby"
                && !app.id.eq_ignore_ascii_case(&own_id)
                && AppId::parse(&app.id).is_some_and(|id| id.source.is_forge() && !id.key.ends_with("/Bilby"))
        };
        let renamed: Vec<InstalledApp> =
            self.state.lock().unwrap().library.iter().filter(|a| misnamed(a)).cloned().collect();
        if !renamed.is_empty() {
            self.update_state(ui, |state| {
                for app in state.library.iter_mut().filter(|a| misnamed(a)) {
                    app.name = app.id.rsplit('/').next().unwrap_or(&app.id).to_string();
                }
            });
            for app in renamed {
                let name = app.id.rsplit('/').next().unwrap_or(&app.id).to_string();
                if let (true, Some(path)) = (app.store_managed, app.path.as_deref().map(PathBuf::from)) {
                    if path.exists() {
                        let _ = install::add_menu_entry(&self.dirs, &app.id, &name, &path);
                    }
                }
            }
        }
        if unchanged {
            return;
        }
        self.update_state(ui, |state| {
            let text = id.to_string();
            match state.library.iter_mut().find(|app| app.id.eq_ignore_ascii_case(&text)) {
                // A newer copy is running than the one last noted.
                Some(own) => {
                    own.version = running.clone();
                    own.skipped_version.clear();
                }
                None => {
                    let mut summary = AppSummary::new(id.clone(), "Bilby");
                    summary.author = "This store".to_string();
                    summary.summary = "The app store you are using.".to_string();
                    let mut own = InstalledApp::new(&summary, &running, "", "", now_unix());
                    own.path = program.clone();
                    state.library.push(own);
                }
            }
        });
    }

    /// Look for a newer Bilby because the person asked, and say what was found.
    pub fn check_store_update_now(self: &Arc<Self>, ui: &AppWindow) {
        let Some(id) = self.store_app.clone() else { return };
        self.toast(ui, "Looking for a newer Bilby…", false);
        self.job(
            move |app| app.catalog().check_update(&id, env!("CARGO_PKG_VERSION")),
            |app, ui, status| {
                let have = env!("CARGO_PKG_VERSION");
                let (version, message, bad) = match status {
                    UpdateStatus::Available { version, .. } => {
                        let message = format!("Bilby {version} is available. You have {have}.");
                        (version, message, false)
                    }
                    UpdateStatus::Unknown(why) => {
                        (String::new(), format!("Could not look for a newer Bilby: {why}"), true)
                    }
                    _ => (String::new(), format!("Bilby {have} is the newest version."), false),
                };
                ui.global::<Store>().set_store_update(version.into());
                app.refresh(ui);
                app.toast(ui, &message, bad);
            },
        );
    }

    /// Stop offering the update that is waiting for this app; the one after it is
    /// announced as usual.
    pub fn skip_update(&self, ui: &AppWindow, id: &str) {
        let waiting = self.updates.lock().unwrap().get(id).cloned();
        let Some(UpdateStatus::Available { version, .. }) = waiting else { return };
        let mut name = String::new();
        self.update_state(ui, |state| {
            if state.skip_version(id, &version) {
                name = state.installed(id).map(|a| a.name.clone()).unwrap_or_default();
            }
        });
        if name.is_empty() {
            return;
        }
        self.updates.lock().unwrap().insert(id.to_string(), UpdateStatus::UpToDate);
        self.refresh(ui);
        self.toast(ui, &format!("Skipping {name} {version}. You'll hear about the next one."), false);
    }

    /// Start the newer copy of the store that was just placed, and leave.
    pub(crate) fn restart_into_new(&self, ui: &AppWindow) {
        let Some(path) = self.self_new.lock().unwrap().clone() else { return };
        // The new copy must start as the store, not be taken for a second one of this.
        let _ = std::fs::remove_file(instance::note(&self.dirs.cache));
        match (self.launcher)(&path) {
            Ok(()) => {
                self.toast(ui, "Restarting into the new Bilby…", false);
                if !self.inline {
                    let _ = slint::quit_event_loop();
                }
            }
            Err(e) => self.toast(ui, &format!("The new Bilby is installed but could not be started: {e}"), true),
        }
    }

    /// A newer Bilby is in place: restart into it now, or later?
    pub(crate) fn ask_restart_bilby(&self, ui: &AppWindow, version: &str) {
        self.ask(
            ui,
            1,
            "Bilby is updated",
            &format!("Bilby {version} is installed. Restart Bilby to start using it."),
            Guarded::RestartBilby,
        );
        let store = ui.global::<Store>();
        store.set_gate_no(l("Later"));
        store.set_gate_yes(l("Restart Bilby now"));
    }
}
