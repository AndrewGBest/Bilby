//! Downloading and installing: the button, the queue, what an app needs first, and
//! reporting how it went.

use crate::cards::HIDDEN_BASE;
use crate::gate::Guarded;
use crate::options::Picked;
use crate::text::l;
use crate::{App, AppWindow, Store, Waiting, ICON_PNG};
use bilby_core::catalog::UpdateStatus;
use bilby_core::install::{self, Action, Dirs};
use bilby_core::model::{AppId, AppSummary, SourceKind};
use bilby_core::platform::{FormFactor, Os};
use bilby_core::state::{now_unix, InstalledApp};
use bilby_core::Error;
use slint::{ComponentHandle, SharedString};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl App {
    /// Fetch a file the store would not install on this device. It is checked like any
    /// other download and shown in the downloads folder; nothing is run or recorded.
    pub fn download_hidden(self: &Arc<Self>, ui: &AppWindow, index: i32) {
        let store = ui.global::<Store>();
        if store.get_install_state() == 1 {
            return;
        }
        let asset = {
            let current = self.current.lock().unwrap();
            let Some(release) = current.release() else { return };
            let Some(hidden) = release.hidden.get(index.max(0) as usize) else { return };
            hidden.asset.clone()
        };
        // Past any real option's index, so no install button takes it for its own.
        store.set_install_index(HIDDEN_BASE + index);
        store.set_install_state(1);
        store.set_install_progress(0.0);
        self.cancel_install.store(false, Ordering::SeqCst);
        let cancel = self.cancel_install.clone();
        self.job(
            move |app| {
                let http = app.catalog().http();
                let mut progress = |_: u64, _: Option<u64>| !cancel.load(Ordering::SeqCst);
                install::execute(
                    &install::save_only(),
                    &asset,
                    http.as_ref(),
                    &app.dirs_for_download(),
                    app.opener.as_ref(),
                    &mut progress,
                )
            },
            |app, ui, result| {
                ui.global::<Store>().set_install_state(0);
                match result {
                    Ok(outcome) => app.toast(ui, &outcome.message, false),
                    Err(Error::Cancelled) => app.toast(ui, "Download cancelled", false),
                    Err(e) => app.toast(ui, &format!("Download failed: {e}"), true),
                }
            },
        );
    }

    /// Is this the app being downloaded right now?
    pub(crate) fn installing_is(&self, id: &str) -> bool {
        self.installing.lock().unwrap().as_ref().is_some_and(|(busy, _)| busy.eq_ignore_ascii_case(id))
    }

    pub fn install(self: &Arc<Self>, ui: &AppWindow, index: i32) {
        let store = ui.global::<Store>();
        if store.get_install_state() == 1 {
            return;
        }
        // Already waiting its turn: asking again takes it out of the queue.
        let here = self.current.lock().unwrap().id.clone();
        let gone = {
            let mut queue = self.download_queue.lock().unwrap();
            let at = queue.iter().position(|w| w.details.summary.id.to_string().eq_ignore_ascii_case(&here));
            at.and_then(|at| queue.remove(at))
        };
        if let Some(gone) = gone {
            store.set_queued(false);
            self.toast(ui, &format!("{} is no longer queued", gone.name), false);
            return;
        }
        // An update the store installs by itself asks nothing: the person agreed to
        // that once, in Settings (with the PIN, where there is one).
        let quiet = self.quiet_install.swap(false, Ordering::SeqCst);
        if !quiet && self.needs_pin_to_install() {
            let why = "A PIN is needed to install or remove apps on this device.";
            self.ask(ui, 2, "Enter the PIN", why, Guarded::Install(index));
            return;
        }
        // "Ask where to save each download": now, before anything is fetched.
        let action = {
            let current = self.current.lock().unwrap();
            current.release().and_then(|r| r.options.get(index.max(0) as usize).map(|o| o.plan.action.clone()))
        };
        if !quiet && action.is_some_and(|action| !self.ask_where_to_save(&action)) {
            return;
        }
        let (details, option, version) = {
            let current = self.current.lock().unwrap();
            let Some(details) = current.details.clone() else { return };
            let Some(release) = current.release() else { return };
            let Some(option) = release.options.get(index.max(0) as usize).cloned() else { return };
            (details, option, release.version.clone())
        };
        let placing = option.plan.action == Action::PlaceAppImage;
        let requirement = if placing { self.needed.lock().unwrap().clone() } else { None };
        let lacks_fuse = placing && self.lacks_fuse.load(Ordering::SeqCst);
        let name = self.name_on_page(&store, &details.summary);
        let mut waiting = Waiting { details, option, version, index, name, folder: None, requirement, lacks_fuse };
        // One at a time: behind another app's download, this one waits its turn.
        let busy = self.installing.lock().unwrap().as_ref().map(|(_, name)| name.clone());
        if let Some(busy) = busy {
            waiting.folder = self.asked_folder.lock().unwrap().take();
            let name = waiting.name.clone();
            self.download_queue.lock().unwrap().push_back(waiting);
            store.set_queued(true);
            self.toast(ui, &format!("{name} is queued. It starts when {busy} has finished."), false);
            return;
        }
        self.start_install(ui, waiting, false);
    }

    /// For tests, where a download is over before the next thing can be asked for:
    /// say that one is going (`Some((id, name))`), or that it has just finished.
    #[doc(hidden)]
    pub fn hold_download(self: &Arc<Self>, ui: &AppWindow, going: Option<(String, String)>) {
        let finished = going.is_none();
        *self.installing.lock().unwrap() = going;
        if finished {
            self.start_queued(ui);
            self.update_next(ui);
        }
    }

    /// An app's name as its page gives it, when that page is the one open.
    pub(crate) fn name_on_page(&self, store: &Store, app: &AppSummary) -> String {
        let shown = store.get_app();
        if shown.id.as_str().eq_ignore_ascii_case(&app.id.to_string()) && !shown.name.is_empty() {
            shown.name.to_string()
        } else {
            app.name.clone()
        }
    }

    /// Is this app waiting its turn to be downloaded?
    pub(crate) fn is_queued(&self, id: &str) -> bool {
        self.download_queue.lock().unwrap().iter().any(|w| w.details.summary.id.to_string().eq_ignore_ascii_case(id))
    }

    /// Start the next download that was waiting, if nothing is going.
    pub(crate) fn start_queued(self: &Arc<Self>, ui: &AppWindow) {
        if self.installing.lock().unwrap().is_some() {
            return;
        }
        let next = self.download_queue.lock().unwrap().pop_front();
        if let Some(next) = next {
            self.start_install(ui, next, true);
        }
    }

    pub(crate) fn start_install(self: &Arc<Self>, ui: &AppWindow, waiting: Waiting, queued: bool) {
        let store = ui.global::<Store>();
        let Waiting { details, option, version, index, name: asked_name, folder, requirement, lacks_fuse } = waiting;
        // From the queue, the page that is open may be some other app's: its buttons
        // are left as they are.
        let on_page = store.get_app().id.as_str().eq_ignore_ascii_case(&details.summary.id.to_string());
        if on_page || !queued {
            store.set_install_index(index);
            store.set_install_state(1);
            store.set_install_message(SharedString::new());
            store.set_queued(false);
        }
        store.set_install_progress(0.0);
        if folder.is_some() {
            *self.asked_folder.lock().unwrap() = folder;
        }
        *self.installing.lock().unwrap() = Some((details.summary.id.to_string(), asked_name.clone()));
        self.cancel_install.store(false, Ordering::SeqCst);

        let cancel = self.cancel_install.clone();
        let inline = self.inline;
        let weak = self.ui.clone();
        let work_option = option.clone();
        // What it needs first (FUSE, for an AppImage) is installed first, as part of this.
        let layering = if lacks_fuse { self.layering.clone() } else { None };
        self.job(
            move |app| {
                // FUSE from the system's repositories where it can be had. Where it
                // cannot (no such package, or the install did not go through), the app
                // is unpacked instead, which needs none. Either way the person is not
                // sent to do anything.
                let mut unpack = lacks_fuse && requirement.is_none() && layering.is_none();
                if let Some(package) = &requirement {
                    unpack = app.install_requirement(package).is_err();
                }
                let mut last_percent = 0u64;
                let mut progress = |done: u64, total: Option<u64>| {
                    if let Some(total) = total.filter(|t| *t > 0) {
                        let percent = done * 100 / total;
                        // One UI update per percent is plenty.
                        if percent != last_percent {
                            last_percent = percent;
                            let set =
                                move |ui: AppWindow| ui.global::<Store>().set_install_progress(percent as f32 / 100.0);
                            if inline {
                                if let Some(ui) = weak.upgrade() {
                                    set(ui);
                                }
                            } else {
                                let weak = weak.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(ui) = weak.upgrade() {
                                        set(ui);
                                    }
                                });
                            }
                        }
                    }
                    !cancel.load(Ordering::SeqCst)
                };
                // A website on a TV opens in Longboard: fetched first when it is not there.
                let on_a_tv = work_option.asset.info.as_ref().is_some_and(|info| info.tv);
                if work_option.plan.action == Action::WebApp && on_a_tv {
                    app.ensure_longboard(&mut progress)?;
                }
                let http = app.catalog().http();
                // A website's entry among the device's apps carries the site's own
                // picture: fetched now if the page has not got it yet.
                if work_option.plan.action == Action::WebApp {
                    let host = work_option.asset.url.trim_start_matches("https://").trim_end_matches('/');
                    if let Some(id) = AppId::parse(&format!("web:{host}")) {
                        let key = bilby_core::icons::app_key(&id);
                        let _ = bilby_core::icons::fetch_site(http.as_ref(), &app.dirs.cache.join("icons"), &key, &id.key);
                    }
                }
                let mut outcome = install::execute(
                    &work_option.plan,
                    &work_option.asset,
                    http.as_ref(),
                    &app.dirs_for_download(),
                    app.opener.as_ref(),
                    &mut progress,
                )?;
                // On a read-only system, once the file is here and is seen to need it:
                // FUSE is added on top of the image, to work from the next restart.
                if let (Some(tool), Some(file)) = (&layering, outcome.file.as_ref()) {
                    if bilby_core::needs::wants_fuse_library(file) != Some(false) {
                        match app.stage_fuse(tool) {
                            true => {
                                outcome.message = format!(
                                    "{} FUSE, which it needs, has been added to this system and starts working after a restart.",
                                    outcome.message
                                )
                            }
                            false => unpack = true,
                        }
                    }
                }
                if unpack {
                    if let Some(file) = outcome.file.clone() {
                        match install::unpack_appimage(&file) {
                            Ok(program) => {
                                outcome.file = Some(program);
                                outcome.message = format!("{} Unpacked, so it runs here without FUSE.", outcome.message);
                            }
                            // Left as the file it is; opening it will say what is wrong.
                            Err(e) => outcome.message = format!("{} {e}", outcome.message),
                        }
                    }
                }
                Ok(outcome)
            },
            move |app, ui, result| {
                if app.fuse_just_staged.swap(false, Ordering::SeqCst) {
                    app.ask_restart(ui);
                }
                let store = ui.global::<Store>();
                let fetched = app.record_fetched(ui);
                // Started from the Library page rather than an app page?
                let updating_in_place = !store.get_updating_id().is_empty();
                store.set_updating_id(SharedString::new());
                *app.installing.lock().unwrap() = None;
                // Is its page the one that is open? If the person has moved on to
                // another app's, that page is not the place to say how this one went.
                let on_page = store.get_app().id.as_str().eq_ignore_ascii_case(&details.summary.id.to_string());
                match result {
                    Ok(mut outcome) => {
                        if fetched {
                            outcome.message = format!("{} Longboard was installed for it.", outcome.message);
                        }
                        if on_page {
                            store.set_install_state(2);
                            store.set_install_message(l(outcome.message.as_str()));
                        }
                        // A command to run, or another store taking over: nothing was
                        // installed by this store, so nothing goes in its library.
                        if matches!(option.plan.action, Action::ShowCommand | Action::OpenStore | Action::RunInTerminal)
                        {
                            app.start_queued(ui);
                            return app.update_next(ui);
                        }
                        // The system's package manager installed it and keeps track of
                        // it; the page says so, and the store's own library stays as it is.
                        if option.plan.action == Action::RunCommand {
                            if on_page {
                                store.set_installed_version(if version.is_empty() {
                                    "installed".into()
                                } else {
                                    version.as_str().into()
                                });
                            }
                            app.toast(ui, &format!("{} is installed", details.summary.name), false);
                            app.start_queued(ui);
                            return app.update_next(ui);
                        }
                        // Its name as its page gives it. Updated from the Library, the
                        // page that is open (if any) is some other app's: then its own.
                        let shown = store.get_app();
                        let name = if shown.id.as_str().eq_ignore_ascii_case(&details.summary.id.to_string()) {
                            shown.name.to_string()
                        } else {
                            asked_name.clone()
                        };
                        let mut entry = InstalledApp::new(
                            &details.summary,
                            &version,
                            &option.label(),
                            &option.asset.name,
                            now_unix(),
                        );
                        entry.name = name.clone();
                        entry.path = outcome.file.as_ref().map(|p| p.display().to_string());
                        entry.store_managed = outcome.store_managed;
                        entry.flatpak = outcome.flatpak_app.clone().unwrap_or_default();
                        let id = entry.id.clone();
                        let is_self = app.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(&id));
                        let say = |message: String| {
                            if on_page {
                                store.set_install_message(l(message));
                            }
                        };
                        if let (true, true, Some(file)) = (is_self, outcome.store_managed, outcome.file.as_ref()) {
                            // A newer copy of the store itself. Its own menu entry is pointed
                            // at the new file, so the next start is the new version, and it
                            // offers to restart into it now. No second entry is made.
                            let _ = install::register_self(&app.dirs, file, ICON_PNG);
                            *app.self_new.lock().unwrap() = Some(file.clone());
                            // Asked wherever the update was started from, the Library
                            // included, where there is no Open button to find.
                            // (Installed by the store itself with nobody asking, it is
                            // offered with whatever else is left, when the round is over.)
                            if !app.unattended.load(Ordering::SeqCst) {
                                app.ask_restart_bilby(ui, &version);
                            }
                            say(format!("Bilby {version} is on this device. Press Open to restart into it; it also starts next time you open Bilby from your app menu."));
                            let previous =
                                app.state().installed(&id).filter(|a| a.store_managed).and_then(|a| a.path.clone());
                            if let Some(previous) = previous.map(PathBuf::from) {
                                if previous != *file {
                                    install::remove_placed(&app.dirs, &previous);
                                }
                            }
                        } else if let (true, Some(file)) = (outcome.store_managed, outcome.file.as_ref()) {
                            // Start it from the app menu like anything else that is installed.
                            if install::add_menu_entry(&app.dirs, &id, &name, file).is_ok() {
                                say(format!("{} It is in your app menu.", outcome.message));
                            }
                            // The version this replaces was a different file: do not leave it behind.
                            let previous =
                                app.state().installed(&id).filter(|a| a.store_managed).and_then(|a| a.path.clone());
                            if let Some(previous) = previous.map(PathBuf::from) {
                                if previous != *file {
                                    install::remove_placed(&app.dirs, &previous);
                                }
                            }
                        }
                        app.update_state(ui, |state| state.record_install(entry));
                        // Installing an older version on purpose leaves an update pending.
                        let newest = details.versions.first().map(|r| r.version.clone()).unwrap_or_default();
                        let status = if bilby_core::version::is_newer(&newest, &version) {
                            let label = details.versions[0].options.first().map(|o| o.label()).unwrap_or_default();
                            UpdateStatus::Available { version: newest, label }
                        } else {
                            UpdateStatus::UpToDate
                        };
                        app.updates.lock().unwrap().insert(id, status);
                        if on_page {
                            store.set_installed_version(version.as_str().into());
                        }
                        app.refresh(ui);
                        if updating_in_place {
                            store.set_install_state(0);
                            app.toast(ui, &format!("{name} updated to {version}"), false);
                        } else {
                            app.toast(ui, &format!("{name} is in your library"), false);
                        }
                    }
                    Err(Error::Cancelled) => {
                        store.set_install_state(0);
                        // Cancelling one stops the rest of an "Update all" too.
                        app.update_queue.lock().unwrap().clear();
                        app.toast(ui, "Download cancelled", false);
                    }
                    Err(e) if !on_page => {
                        app.toast(ui, &format!("{} could not be installed: {e}", details.summary.name), true);
                    }
                    Err(e) => {
                        if updating_in_place {
                            // There is no app page open to show the message on.
                            store.set_install_state(0);
                            app.toast(ui, &format!("Update failed: {e}"), true);
                        } else {
                            store.set_install_state(3);
                            store.set_install_message(l(e.to_string()));
                        }
                    }
                }
                // Whatever was waiting its turn behind this one.
                app.start_queued(ui);
                app.update_next(ui);
            },
        );
    }

    /// The folders for the download about to start: as usual, but saving where the
    /// person just said to, if they were asked.
    pub(crate) fn dirs_for_download(&self) -> Dirs {
        let mut dirs = self.dirs.clone();
        dirs.downloads = self.asked_folder.lock().unwrap().take().unwrap_or_else(|| self.downloads_folder());
        dirs
    }

    /// With "ask where to save each download" on: ask now. False when the person
    /// backed out, which stops the download; true otherwise (also where this system
    /// has no dialog to ask with, so that the setting cannot block every install).
    pub(crate) fn ask_where_to_save(&self, action: &Action) -> bool {
        if !self.state.lock().unwrap().settings.ask_where || !install::saves_a_file(action) {
            return true;
        }
        match (self.picker)(&self.downloads_folder()) {
            Picked::Folder(folder) => {
                *self.asked_folder.lock().unwrap() = Some(folder);
                true
            }
            Picked::Cancelled => false,
            Picked::NoDialog => true,
        }
    }

    /// Install a package an app needs before it will run, from the system's own
    /// repository, the way any package is installed (the system asks for the password).
    pub(crate) fn install_requirement(&self, package: &AppId) -> bilby_core::Result<()> {
        let details = self.catalog().details(package)?;
        let option = details.release.as_ref().and_then(|release| release.options.first()).ok_or_else(|| {
            Error::Unsupported(format!("{} could not be installed: nothing to install was found.", package.key))
        })?;
        let http = self.catalog().http();
        install::execute(&option.plan, &option.asset, http.as_ref(), &self.dirs, self.opener.as_ref(), &mut |_, _| {
            true
        })
        .map_err(|e| {
            Error::Unsupported(format!("What this app needs ({}) could not be installed: {e}", package.key))
        })?;
        // Look again at what the system has, so the notice goes once it is there.
        let now = match self.fuse_given {
            true => bilby_core::needs::Fuse { library: true, mounter: true },
            false => bilby_core::needs::fuse_here(),
        };
        *self.fuse.write().unwrap() = now;
        Ok(())
    }

    /// Longboard, ready to start: the copy on this device, or, when there is none, one
    /// the store fetches and installs here and now. The person is never sent to get it.
    /// What was installed waits in `fetched` for [`App::record_fetched`].
    pub(crate) fn ensure_longboard(
        &self,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> bilby_core::Result<install::Longboard> {
        if let Some(found) = install::find_longboard(&self.dirs) {
            return Ok(found);
        }
        let cannot = |why: String| {
            Error::Unsupported(format!(
                "This opens in Longboard, a browser for TVs, which Bilby tried to install for you and could not: {why}"
            ))
        };
        let id = AppId::new(SourceKind::GitHub, bilby_core::source::web::LONGBOARD);
        let details = self.catalog().details(&id).map_err(|e| cannot(e.to_string()))?;
        let release = details.release.as_ref().ok_or_else(|| cannot("it has no release yet.".into()))?;
        // The kinds the store can finish on its own, in the order it prefers them here.
        let option = release
            .options
            .iter()
            .find(|o| matches!(o.plan.action, Action::PlaceAppImage | Action::FlatpakBundle))
            .ok_or_else(|| cannot("none of its files suits this device.".into()))?;
        let http = self.catalog().http();
        let mut outcome =
            install::execute(&option.plan, &option.asset, http.as_ref(), &self.dirs, self.opener.as_ref(), progress)
                .map_err(|e| match e {
                    Error::Cancelled => Error::Cancelled,
                    e => cannot(e.to_string()),
                })?;
        // An AppImage on a system that cannot start one as it is: unpacked, which needs
        // nothing more and no password.
        if let Some(file) = outcome.file.clone().filter(|_| outcome.store_managed) {
            let fuse = *self.fuse.read().unwrap();
            let starts = match bilby_core::needs::wants_fuse_library(&file) {
                Some(false) => fuse.mounter,
                _ => fuse.library,
            };
            if !starts {
                if let Ok(program) = install::unpack_appimage(&file) {
                    outcome.file = Some(program);
                }
            }
            let _ = install::add_menu_entry(
                &self.dirs,
                &id.to_string(),
                &details.summary.name,
                outcome.file.as_ref().unwrap_or(&file),
            );
        }
        let mut entry =
            InstalledApp::new(&details.summary, &release.version, &option.label(), &option.asset.name, now_unix());
        entry.path = outcome.file.as_ref().map(|p| p.display().to_string());
        entry.store_managed = outcome.store_managed;
        entry.flatpak = outcome.flatpak_app.clone().unwrap_or_default();
        *self.fetched.lock().unwrap() = Some(entry);
        install::find_longboard(&self.dirs).ok_or_else(|| cannot("it was installed but cannot be found.".into()))
    }

    /// Write the Longboard that [`App::ensure_longboard`] installed into the library.
    /// Says whether there was one.
    pub(crate) fn record_fetched(&self, ui: &AppWindow) -> bool {
        let Some(entry) = self.fetched.lock().unwrap().take() else { return false };
        self.updates.lock().unwrap().insert(entry.id.clone(), UpdateStatus::UpToDate);
        self.update_state(ui, |state| state.record_install(entry));
        self.refresh(ui);
        true
    }

    /// On a Linux TV a website opens in Longboard; anywhere else in the usual browser.
    pub(crate) fn opens_sites_in_longboard(&self) -> bool {
        let platform = self.platform.read().unwrap();
        platform.os == Os::Linux && platform.form_factor == FormFactor::Tv
    }

    /// Add FUSE on top of a read-only system's image, once. True when it is in place
    /// and waiting for a restart.
    pub(crate) fn stage_fuse(&self, tool: &std::path::Path) -> bool {
        if self.fuse_staged.load(Ordering::SeqCst) {
            return true;
        }
        let added = bilby_core::needs::layer_fuse(tool).is_ok();
        if added {
            self.fuse_staged.store(true, Ordering::SeqCst);
            self.fuse_just_staged.store(true, Ordering::SeqCst);
        }
        added
    }
}
