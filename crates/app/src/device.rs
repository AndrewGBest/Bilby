//! The page about this device, and the operating system's own updates.

use crate::gate::Guarded;
use crate::text::{l, plural};
use crate::{App, AppWindow, Fact, Store, SystemUpdate};
use bilby_core::state::now_unix;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::sync::Arc;

/// What an updater is doing, from a line it printed, in a few words for the screen:
/// "Updating through Flatpak…", or the name of the app or package it is on.
pub(crate) fn system_step(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with("Updating through ") {
        return Some(line.to_string());
    }
    // Flatpak: "Updating app/org.example.App/x86_64/stable".
    for start in ["Updating ", "Installing "] {
        if let Some(name) = line.strip_prefix(start).and_then(|rest| rest.split('/').nth(1)) {
            return Some(name.to_string());
        }
    }
    // pacman and dnf: "(3/12) upgrading foo", "[3/12] Upgrading foo-1.2".
    let rest = line.strip_prefix(['(', '['])?.split_once([')', ']'])?.1.trim();
    let (verb, name) = rest.split_once(' ')?;
    ["upgrading", "installing", "reinstalling"]
        .contains(&verb.to_ascii_lowercase().as_str())
        .then(|| name.split_whitespace().next().unwrap_or(name).to_string())
}

impl App {
    /// Put the system's details and the last look for updates on screen. `state` is
    /// what is going on right now: 0 for "as last found", 1 checking, 4 installing.
    pub(crate) fn show_system(&self, ui: &AppWindow, state: i32, log: &str) {
        let store = ui.global::<Store>();
        let report = &self.system;
        store.set_system_name(report.name.as_str().into());
        store.set_system_tagline(l(report.tagline.as_str()));
        store.set_system_facts(ModelRc::new(VecModel::from(
            report.facts.iter().map(|f| Fact { name: l(f.name), value: l(&f.value) }).collect::<Vec<_>>(),
        )));
        store.set_system_supported(!report.managers.is_empty());
        store.set_system_note(l(report.note.as_str()));
        store.set_system_handoff(l(report.handoff.as_ref().map(|h| h.label).unwrap_or_default()));
        if !log.is_empty() {
            store.set_system_log(log.into());
        }
        let check = self.system_check.lock().unwrap().clone();
        let pending = check.as_ref().map(|c| c.pending.clone()).unwrap_or_default();
        let rows: Vec<SystemUpdate> = pending
            .iter()
            .map(|p| SystemUpdate {
                name: p.name.as_str().into(),
                change: if p.from.is_empty() { p.to.clone() } else { format!("{} → {}", p.from, p.to) }.into(),
                via: p.manager.label().into(),
            })
            .collect();
        store.set_system_updates(ModelRc::new(VecModel::from(rows)));
        store.set_system_restart(check.as_ref().is_some_and(|c| c.restart_needed));
        let problems = check.as_ref().map(|c| c.problems.join(" ")).unwrap_or_default();
        let (state, headline) = match (state, &check) {
            (1, _) => (1, "Checking for updates…".to_string()),
            (4, _) => (4, "Installing updates…".to_string()),
            // Begun earlier and still going (Bilby was closed and opened again).
            (_, Some(c)) if c.installing => (4, "Installing updates…".to_string()),
            (_, None) => (0, "Not checked yet".to_string()),
            // A whole new release of the system is named, not counted.
            (_, Some(_)) if pending.iter().any(|p| p.release.is_some()) => {
                let release = pending.iter().find(|p| p.release.is_some()).map(|p| p.name.clone()).unwrap_or_default();
                match pending.len() - 1 {
                    0 => (3, format!("{release} is available")),
                    more => (3, format!("{release} and {} available", plural(more, "update"))),
                }
            }
            (_, Some(_)) if pending.len() == 1 => (3, "1 update available".to_string()),
            (_, Some(_)) if !pending.is_empty() => (3, format!("{} updates available", pending.len())),
            (_, Some(_)) if !problems.is_empty() => (5, "Could not check for updates".to_string()),
            (_, Some(c)) if c.restart_needed => (2, "Restart to finish updating".to_string()),
            (_, Some(_)) => (2, "Up to date".to_string()),
        };
        // Where a check relies on a list fetched earlier, say how old that list is.
        let age =
            check.as_ref().and_then(|c| c.lists_fetched).map(|then| match now_unix().saturating_sub(then) / 3600 {
                0 => "The list of updates was fetched within the last hour.".to_string(),
                hours if hours < 48 => format!("The list of updates was fetched {hours} hours ago."),
                hours => format!("The list of updates was fetched {} days ago, so there may be more.", hours / 24),
            });
        let detail =
            [problems, age.unwrap_or_default()].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        store.set_system_state(state);
        store.set_system_headline(l(headline));
        store.set_system_detail(l(detail));
    }

    /// Look for system updates. A quiet check (`asked` false) never asks for a
    /// password. One the person asked for first fetches a fresh list of updates where the
    /// one on disk is more than a day old (apt), which does.
    pub fn check_system(self: &Arc<Self>, ui: &AppWindow, asked: bool) {
        if self.system.managers.is_empty() || matches!(ui.global::<Store>().get_system_state(), 1 | 4) {
            return;
        }
        self.show_system(ui, 1, "");
        let fetched = self.system_check.lock().unwrap().as_ref().and_then(|c| c.lists_fetched);
        let stale = asked && fetched.is_some_and(|then| now_unix().saturating_sub(then) > 86_400);
        self.job(
            move |app| {
                let refreshed = match stale {
                    true => bilby_core::os::refresh_lists(app.host.as_ref(), &app.system, &mut |_| {}),
                    false => Ok(()),
                };
                (refreshed, bilby_core::os::check(app.host.as_ref(), &app.system))
            },
            move |app, ui, (refreshed, check)| {
                // The Library's card for the device says what was found.
                let after = app.clone();
                let weak = ui.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_millis(0), move || {
                    if let Some(ui) = weak.upgrade() {
                        after.refresh(&ui);
                    }
                });
                let installing = check.installing;
                *app.system_check.lock().unwrap() = Some(check);
                app.show_system(ui, 0, "");
                if installing {
                    let store = ui.global::<Store>();
                    store.set_system_progress(0.0);
                    store.set_system_step(l("Started earlier and still going"));
                    app.watch_system(ui);
                }
                if let Err(e) = refreshed {
                    app.toast(ui, &e.to_string(), true);
                }
                // "Update automatically": what needs no password is installed now; the
                // rest is the person's to finish.
                let waiting: Vec<bilby_core::os::Manager> = {
                    let check = app.system_check.lock().unwrap();
                    check.as_ref().map(|c| c.pending.iter().map(|p| p.manager).collect()).unwrap_or_default()
                };
                let alone = !waiting.is_empty() && waiting.iter().all(|manager| !manager.asks_for_password());
                if app.state().settings.auto_update && !asked && !installing && alone {
                    app.update_system(ui);
                } else {
                    app.offer_to_finish(ui);
                }
            },
        );
    }

    /// An update begun before Bilby was opened is still being installed: look again
    /// every few seconds, without the page flickering, until it is over.
    pub(crate) fn watch_system(self: &Arc<Self>, ui: &AppWindow) {
        let (app, weak) = (self.clone(), ui.as_weak());
        slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
            if weak.upgrade().is_none() {
                return;
            }
            app.job(
                |app| bilby_core::os::check(app.host.as_ref(), &app.system),
                |app, ui, check| {
                    // Something else took over meanwhile (an update started from here).
                    if !app.system_check.lock().unwrap().as_ref().is_some_and(|c| c.installing) {
                        return;
                    }
                    let (installing, restart) = (check.installing, check.restart_needed);
                    *app.system_check.lock().unwrap() = Some(check);
                    app.show_system(ui, 0, "");
                    if installing {
                        app.watch_system(ui);
                        return;
                    }
                    app.refresh(ui);
                    if restart {
                        app.toast(ui, "Updates installed. Restart this device to finish.", false);
                    }
                },
            );
        });
    }

    /// Install every waiting system update, then look again.
    pub fn update_system(self: &Arc<Self>, ui: &AppWindow) {
        if self.system.managers.is_empty() || matches!(ui.global::<Store>().get_system_state(), 1 | 4) {
            return;
        }
        self.show_system(ui, 4, "");
        let store = ui.global::<Store>();
        store.set_system_progress(0.0);
        store.set_system_step(SharedString::new());
        let pending = self.system_check.lock().unwrap().as_ref().map(|c| c.pending.clone()).unwrap_or_default();
        let waiting = pending.len();
        let (weak, inline) = (self.ui.clone(), self.inline);
        self.job(
            move |app| {
                let mut log = Vec::new();
                let mut progress = bilby_core::os::Progress::new(waiting);
                let mut shown = (-1, String::new());
                let result = bilby_core::os::update_waiting(app.host.as_ref(), &app.system, &pending, &mut |line| {
                    log.push(line.to_string());
                    // How far it has got and what it is on, sent to the screen when
                    // either changes.
                    let percent = (progress.line(line) * 100.0) as i32;
                    // What it is on: a name where one can be read, else what it
                    // just said, so that something is seen to be happening.
                    let said: String = line.trim().chars().take(70).collect();
                    let step = system_step(line)
                        .or_else(|| (said.chars().filter(|c| c.is_alphabetic()).count() > 3).then_some(said))
                        .unwrap_or_else(|| shown.1.clone());
                    if (percent, &step) == (shown.0, &shown.1) {
                        return;
                    }
                    shown = (percent, step.clone());
                    let set = move |ui: AppWindow| {
                        let store = ui.global::<Store>();
                        store.set_system_progress(percent as f32 / 100.0);
                        store.set_system_step(l(step.as_str()));
                    };
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
                });
                // Long output is cut from the top: the end is where a failure explains itself.
                let from = log.len().saturating_sub(300);
                (result, log[from..].join("\n"), bilby_core::os::check(app.host.as_ref(), &app.system))
            },
            |app, ui, (result, log, check)| {
                let restart = check.restart_needed;
                let left = check.pending.len();
                *app.system_check.lock().unwrap() = Some(check);
                app.show_system(ui, 0, if log.is_empty() { " " } else { &log });
                // The Library's card and the top bar say what is waiting now.
                app.refresh(ui);
                match result {
                    // Said to have worked, and something is still waiting: said so, not
                    // "up to date" over a list that has not changed.
                    Ok(()) if left > 0 => app.toast(
                        ui,
                        &format!(
                            "{} could not be installed. The details show what the updater said.",
                            plural(left, "update")
                        ),
                        true,
                    ),
                    Ok(()) if restart => app.toast(ui, "Updates installed. Restart this device to finish.", false),
                    Ok(()) => app.toast(ui, "The system is up to date.", false),
                    Err(e) => app.toast(ui, &e.to_string(), true),
                }
                app.offer_to_finish(ui);
            },
        );
    }

    /// Updates are in place and wait for a restart: offer one, and ask before doing it.
    pub fn ask_restart_for_updates(&self, ui: &AppWindow) {
        self.ask(
            ui,
            1,
            "Restart to finish updating",
            "Updates installed. Restart this device to finish.",
            Guarded::Restart,
        );
        let store = ui.global::<Store>();
        store.set_gate_no(l("Restart later"));
        store.set_gate_yes(l("Restart now"));
    }
}
