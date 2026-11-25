//! Apps on the device that the store did not install: whether one has an update
//! waiting, bringing that update in, and starting what the system's packages put in
//! the app menu.
//!
//! Such an app is looked after by the tool that installed it (Flatpak, the system's
//! package manager). The store never keeps a second record of it: what is waiting is
//! what the last look for system updates found, and updating is asking that tool.

use crate::gate::Guarded;
use crate::text::l;
use crate::{App, AppWindow, Store};
use bilby_core::model::{AppId, SourceKind};
use bilby_core::os::{self, Manager, Pending};
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::Arc;

/// "1.2 → 1.3", as much of it as the tool said.
pub(crate) fn change_of(pending: &Pending) -> String {
    match (pending.from.is_empty(), pending.to.is_empty()) {
        (false, false) => format!("{} → {}", pending.from, pending.to),
        _ => pending.to.clone(),
    }
}

impl App {
    /// The name the device knows an app by, when the store did not install it.
    fn outside_name(&self, id: &str) -> Option<String> {
        let known = self.elsewhere.read().unwrap();
        known.apps.iter().find(|(app, _)| app.eq_ignore_ascii_case(id)).map(|(_, name)| name.clone())
    }

    /// The update waiting for an app the store did not install, as the last look for
    /// system updates found it. `None` for an app in the library: the store looks
    /// after those itself.
    pub(crate) fn outside_update(&self, id: &str) -> Option<Pending> {
        if self.state.lock().unwrap().installed(id).is_some() {
            return None;
        }
        self.outside_name(id)?;
        let app = AppId::parse(id)?;
        let manager = match app.source {
            SourceKind::Flathub => Manager::Flatpak,
            SourceKind::Pacman => Manager::Pacman,
            SourceKind::Apt => Manager::Apt,
            SourceKind::Dnf => Manager::Dnf,
            _ => return None,
        };
        let check = self.system_check.lock().unwrap();
        check
            .as_ref()?
            .pending
            .iter()
            .find(|p| p.manager == manager && p.release.is_none() && p.name == app.key)
            .cloned()
    }

    /// Say on the open app's page whether an update is waiting for it outside the store.
    pub(crate) fn show_outside_update(&self, store: &Store) {
        let waiting = self.outside_update(store.get_app().id.as_str());
        store.set_outside_change(waiting.as_ref().map(change_of).unwrap_or_default().into());
        store.set_outside_update(waiting.is_some());
    }

    /// Bring in the update waiting for an app the store did not install, by asking the
    /// tool that looks after it. Where that tool only updates everything together,
    /// the person is asked whether to do that.
    pub(crate) fn update_outside(self: &Arc<Self>, ui: &AppWindow, id: &str, pending: Pending) {
        let store = ui.global::<Store>();
        let name = self.outside_name(id).unwrap_or_else(|| pending.name.clone());
        // One change to the system at a time: two would wait on each other's lock.
        if matches!(store.get_system_state(), 1 | 4) {
            self.toast(ui, "The system is being updated. Try again when that has finished.", false);
            return;
        }
        if os::update_one_command(&pending).is_none() {
            let why = format!(
                "{name} is updated together with everything else {} looks after, so that all of it keeps working together.",
                pending.manager.label()
            );
            self.ask(ui, 1, "Update the whole system?", &why, Guarded::UpdateSystem);
            store.set_gate_no(l("Later"));
            store.set_gate_yes(l("Update everything"));
            return;
        }
        self.show_system(ui, 4, "");
        store.set_system_progress(0.0);
        store.set_system_step(name.as_str().into());
        self.toast(ui, &format!("Updating {name}…"), false);
        let id = id.to_string();
        self.job(
            move |app| {
                let mut log = Vec::new();
                let result = os::update_one(app.host.as_ref(), &pending, &mut |line| log.push(line.to_string()));
                // Long output is cut from the top: the end is where a failure explains itself.
                let from = log.len().saturating_sub(300);
                (result, log[from..].join("\n"), os::check(app.host.as_ref(), &app.system))
            },
            move |app, ui, (result, log, check)| {
                *app.system_check.lock().unwrap() = Some(check);
                app.show_system(ui, 0, if log.is_empty() { " " } else { &log });
                // Whether it is still waiting is asked of the tool, not assumed.
                let left = app.outside_update(&id).is_some();
                app.refresh(ui);
                match result {
                    Ok(()) if left => app.toast(
                        ui,
                        &format!("{name} could not be updated. The system's details show what the updater said."),
                        true,
                    ),
                    Ok(()) => app.toast(ui, &format!("{name} updated"), false),
                    Err(e) => app.toast(ui, &format!("{name} was not updated. {e}"), true),
                }
                app.offer_to_finish(ui);
            },
        );
    }

    /// The app-menu entry of an app the system's package manager installed.
    pub(crate) fn outside_entry(&self, id: &str) -> Option<PathBuf> {
        let entry = self.elsewhere.read().unwrap().entries.get(&id.to_ascii_lowercase()).cloned()?;
        entry.is_file().then_some(entry)
    }

    /// Start an app the system's package manager installed, the way its menu entry
    /// says to.
    pub(crate) fn launch_outside(&self, entry: &std::path::Path) -> bilby_core::Result<()> {
        let text = std::fs::read_to_string(entry)?;
        let (program, args) = bilby_core::source::system::menu_entry_command(&text)
            .ok_or_else(|| bilby_core::Error::Unsupported("Its menu entry does not say how to start it.".into()))?;
        (self.runner)(std::path::Path::new(&program), &args)
    }
}
