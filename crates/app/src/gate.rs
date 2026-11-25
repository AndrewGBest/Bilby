//! Questions that stand in front of an action: is the person an adult, what is the PIN,
//! restart now or later.

use crate::settings::Change;
use crate::text::l;
use crate::{App, AppWindow, Store};
use slint::{ComponentHandle, SharedString};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Something that may need an answer first: that the person is an adult, or the PIN.
#[derive(Debug, Clone)]
pub(crate) enum Guarded {
    OpenAdult(String),
    Setting(Change),
    Install(i32),
    Remove(String),
    /// Not a guard: FUSE was added and the question is whether to restart now.
    Restart,
    /// Not a guard: a newer Bilby is in place and the question is whether to restart
    /// Bilby into it now.
    RestartBilby,
    /// Not a guard: updates that need the person are waiting, and the question is
    /// whether to finish them now.
    FinishUpdates,
    /// Not a guard: one app's update comes only with all of the system's, and the
    /// question is whether to install them all now.
    UpdateSystem,
}

impl App {
    /// What stands between the person and something for adults, right now.
    pub(crate) fn gate_for_adult(&self) -> bilby_core::parental::Gate {
        let settings = self.state().settings;
        // Asked to show adult things as they are: that was the question, answered.
        if settings.adult_uncovered && !settings.family_mode {
            return bilby_core::parental::Gate::Open;
        }
        let policy =
            bilby_core::parental::Policy { family_mode: settings.family_mode, adult_on_home: settings.adult_on_home };
        bilby_core::parental::to_open_adult(
            policy,
            !settings.pin_hash.is_empty(),
            settings.adult_confirmed,
            self.adult_unlocked.load(Ordering::SeqCst),
        )
    }

    /// Installing and removing ask for the PIN where the person chose that. Entering
    /// it clears exactly one action.
    pub(crate) fn needs_pin_to_install(&self) -> bool {
        let settings = self.state().settings;
        settings.pin_for_installs
            && !settings.pin_hash.is_empty()
            && !self.install_cleared.swap(false, Ordering::SeqCst)
    }

    /// Put the question on screen: 1 "are you an adult?", 2 the PIN, 3 a new PIN,
    /// 4 the new PIN again. What was being done waits for the answer.
    pub(crate) fn ask(&self, ui: &AppWindow, kind: i32, title: &str, text: &str, waiting: Guarded) {
        *self.pending.lock().unwrap() = Some(waiting);
        let store = ui.global::<Store>();
        store.set_gate_title(l(title));
        store.set_gate_text(l(text));
        store.set_gate_error(SharedString::new());
        store.set_gate_no(l("No"));
        store.set_gate_yes(l("Yes, I am 18 or over"));
        store.set_gate(kind);
    }

    /// FUSE has been added and works from the next restart: now, or later?
    pub(crate) fn ask_restart(&self, ui: &AppWindow) {
        self.ask(
            ui,
            1,
            "Restart to finish",
            "Bilby has added FUSE to this system, which this app needs to run. It starts working after a restart.",
            Guarded::Restart,
        );
        let store = ui.global::<Store>();
        store.set_gate_no(l("Restart later"));
        store.set_gate_yes(l("Restart now"));
    }

    pub(crate) fn close_gate(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        store.set_gate(0);
        store.set_gate_error(SharedString::new());
    }

    /// The person stepped back from the question: nothing happens.
    pub fn gate_cancel(&self, ui: &AppWindow) {
        *self.pending.lock().unwrap() = None;
        *self.new_pin.lock().unwrap() = None;
        self.close_gate(ui);
        self.refresh_settings(ui);
    }

    /// The answer to "are you 18 or over?". A yes is remembered; a no changes nothing.
    pub fn gate_answer(self: &Arc<Self>, ui: &AppWindow, yes: bool) {
        if matches!(*self.pending.lock().unwrap(), Some(Guarded::Restart)) {
            *self.pending.lock().unwrap() = None;
            self.close_gate(ui);
            match yes.then(|| (self.restart)()) {
                Some(Ok(())) => self.toast(ui, "Restarting…", false),
                Some(Err(e)) => {
                    self.toast(ui, &format!("Couldn't restart from here ({e}). Restart from your system's menu."), true)
                }
                None => self.toast(ui, "It will run after the next restart.", false),
            }
            return;
        }
        if matches!(*self.pending.lock().unwrap(), Some(Guarded::FinishUpdates)) {
            *self.pending.lock().unwrap() = None;
            self.close_gate(ui);
            if yes {
                // Asked for by the person now: passwords and installers may appear.
                self.update_all(ui);
                let system = self.system_check.lock().unwrap().as_ref().is_some_and(|c| !c.pending.is_empty());
                if system {
                    self.update_system(ui);
                }
            } else {
                self.toast(ui, "They wait in your Library until you are ready.", false);
            }
            return;
        }
        if matches!(*self.pending.lock().unwrap(), Some(Guarded::UpdateSystem)) {
            *self.pending.lock().unwrap() = None;
            self.close_gate(ui);
            if yes {
                self.update_system(ui);
            }
            return;
        }
        if matches!(*self.pending.lock().unwrap(), Some(Guarded::RestartBilby)) {
            *self.pending.lock().unwrap() = None;
            self.close_gate(ui);
            match yes {
                true => self.restart_into_new(ui),
                false => self.toast(ui, "The new Bilby starts the next time you open it.", false),
            }
            return;
        }
        if ui.global::<Store>().get_gate() != 1 || !yes {
            self.gate_cancel(ui);
            return;
        }
        self.update_state(ui, |state| state.settings.adult_confirmed = true);
        self.close_gate(ui);
        self.carry_on(ui);
    }

    /// Four digits were entered at the gate.
    pub fn gate_pin(self: &Arc<Self>, ui: &AppWindow, entry: &str) {
        use bilby_core::parental;
        let store = ui.global::<Store>();
        match store.get_gate() {
            2 => {
                let settings = self.state().settings;
                if parental::valid_pin(entry) && parental::hash_pin(&settings.pin_salt, entry) == settings.pin_hash {
                    // Given to open adult content, it holds for the rest of this
                    // sitting; given for anything else, it covers that one thing.
                    if matches!(*self.pending.lock().unwrap(), Some(Guarded::OpenAdult(_))) {
                        self.adult_unlocked.store(true, Ordering::SeqCst);
                    }
                    self.close_gate(ui);
                    self.carry_on(ui);
                } else {
                    store.set_gate_error(l("That is not the PIN. Try again."));
                }
            }
            3 => {
                if parental::valid_pin(entry) {
                    *self.new_pin.lock().unwrap() = Some(entry.to_string());
                    store.set_gate_title(l("Enter the new PIN again"));
                    store.set_gate_text(l("To be sure it is the one you meant."));
                    store.set_gate_error(SharedString::new());
                    store.set_gate(4);
                }
            }
            4 => {
                let first = self.new_pin.lock().unwrap().take();
                if first.as_deref() == Some(entry) && parental::valid_pin(entry) {
                    let salt = parental::new_salt();
                    let hash = parental::hash_pin(&salt, entry);
                    self.update_state(ui, |state| {
                        state.settings.pin_salt = salt;
                        state.settings.pin_hash = hash;
                    });
                    *self.pending.lock().unwrap() = None;
                    self.close_gate(ui);
                    self.refresh_settings(ui);
                    self.toast(ui, "The PIN is set. Remember it: it cannot be shown again.", false);
                } else {
                    store.set_gate_title(l("Choose a PIN"));
                    store.set_gate_text(l("Those two were not the same. Choose four digits."));
                    store.set_gate(3);
                }
            }
            _ => {}
        }
    }

    /// Do what was waiting on the answer.
    pub(crate) fn carry_on(self: &Arc<Self>, ui: &AppWindow) {
        let Some(waiting) = self.pending.lock().unwrap().take() else { return };
        match waiting {
            Guarded::OpenAdult(id) => self.open(ui, &id),
            Guarded::Install(index) => {
                self.install_cleared.store(true, Ordering::SeqCst);
                self.install(ui, index);
            }
            Guarded::Remove(id) => {
                self.install_cleared.store(true, Ordering::SeqCst);
                self.remove_from_library(ui, &id);
            }
            Guarded::Setting(change) => self.apply_setting(ui, change),
            Guarded::Restart | Guarded::RestartBilby | Guarded::FinishUpdates | Guarded::UpdateSystem => {}
        }
    }
}
