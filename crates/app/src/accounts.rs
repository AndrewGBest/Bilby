//! Signing in to other services (GitHub, Steam, GOG) to raise a limit or to mark what
//! the person already owns.

use crate::pictures::code_image;
use crate::{App, AppWindow, Shell, Store};
use bilby_core::model::{AppId, SourceKind};
use bilby_core::{install, Error};
use slint::{ComponentHandle, SharedString};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// The store's sign-in window while it is open, so it can be closed from here.
pub(crate) static SIGNIN_WINDOW: Mutex<Option<std::process::Child>> = Mutex::new(None);

/// The program that shows the sign-in window. On Windows and macOS it is this program
/// run another way; on Linux it is a separate file beside this one, so that the store
/// itself starts on systems without the web engine the window needs.
pub(crate) fn signin_command() -> Option<std::process::Command> {
    let me = std::env::current_exe().ok()?;
    if cfg!(any(windows, target_os = "macos")) {
        let mut command = std::process::Command::new(me);
        command.arg("--signin");
        return Some(command);
    }
    let beside = me.parent().map(|dir| dir.join("bilby-signin")).filter(|p| p.is_file());
    let on_path =
        || std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join("bilby-signin")).find(|p| p.is_file());
    beside.or_else(on_path).map(std::process::Command::new)
}

/// Close the sign-in window, if one is open.
pub(crate) fn close_signin_window() {
    if let Some(mut window) = SIGNIN_WINDOW.lock().unwrap().take() {
        let _ = window.kill();
        let _ = window.wait();
    }
}

/// Show a store's sign-in page (`steam`, `gog`) in the store's own small window and
/// wait for it to hand back its answer, or to be closed.
pub(crate) fn signin_window(site: &str, data_dir: &std::path::Path) -> bilby_core::Result<Option<String>> {
    use std::io::Read;
    use std::process::Stdio;
    let cannot = || {
        let needs = if cfg!(target_os = "linux") {
            " It needs WebKitGTK (the package libwebkit2gtk-4.1) on this system."
        } else {
            ""
        };
        Error::Unsupported(format!("The sign-in window could not be opened.{needs}"))
    };
    let mut command = signin_command().ok_or_else(cannot)?;
    std::fs::create_dir_all(data_dir)?;
    // One sign-in window at a time: starting another closes the first.
    close_signin_window();
    command.arg(site).arg("--data-dir").arg(data_dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut window = command.spawn().map_err(|_| cannot())?;
    let mut said = window.stdout.take();
    *SIGNIN_WINDOW.lock().unwrap() = Some(window);
    let mut line = String::new();
    if let Some(out) = said.as_mut() {
        let _ = out.read_to_string(&mut line);
    }
    // Gone already means it was closed from the store: the same as the person closing it.
    let Some(mut window) = SIGNIN_WINDOW.lock().unwrap().take() else { return Ok(None) };
    match window.wait()?.code() {
        Some(0) if !line.trim().is_empty() => Ok(Some(line)),
        Some(3) => Ok(None),
        _ => Err(cannot()),
    }
}

impl App {
    /// Sign in to GitHub with a code: show it, then wait for the person to enter it at
    /// GitHub. The token that comes back is kept exactly like a pasted one.
    pub fn github_signin(self: &Arc<Self>, ui: &AppWindow) {
        use bilby_core::github_login::{self, Poll};
        let generation = self.signin_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let store = ui.global::<Store>();
        store.set_signin_code(SharedString::new());
        store.set_signin_state(1);
        self.job(
            |app| github_login::start(app.catalog().http().as_ref()),
            move |app, ui, started| {
                let store = ui.global::<Store>();
                if app.signin_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let code = match started {
                    Ok(code) => code,
                    Err(e) => {
                        store.set_signin_state(0);
                        app.toast(ui, &format!("Could not start signing in: {e}"), true);
                        return;
                    }
                };
                store.set_signin_code(code.user_code.as_str().into());
                store.set_signin_state(2);
                app.job(
                    move |app| {
                        let http = app.catalog().http();
                        let started = std::time::Instant::now();
                        let mut interval = code.interval;
                        loop {
                            // Wait in short steps, so cancelling is noticed promptly.
                            let wait_until = std::time::Instant::now() + std::time::Duration::from_secs(interval);
                            while std::time::Instant::now() < wait_until {
                                if app.signin_generation.load(Ordering::SeqCst) != generation {
                                    return Ok(None);
                                }
                                std::thread::sleep(std::time::Duration::from_millis(200));
                            }
                            if app.signin_generation.load(Ordering::SeqCst) != generation {
                                return Ok(None);
                            }
                            match github_login::poll(http.as_ref(), &code)? {
                                Poll::Done(token) => return Ok(Some(token)),
                                Poll::Pending => {}
                                Poll::SlowDown(seconds) => interval = seconds,
                                Poll::Denied => {
                                    return Err(Error::Unsupported("Signing in was declined at GitHub.".into()))
                                }
                                Poll::Expired => {
                                    return Err(Error::Unsupported(
                                        "The code ran out before it was entered. Try again.".into(),
                                    ))
                                }
                            }
                            if started.elapsed().as_secs() > code.expires_in.max(60) {
                                return Err(Error::Unsupported(
                                    "The code ran out before it was entered. Try again.".into(),
                                ));
                            }
                        }
                    },
                    move |app, ui, outcome: bilby_core::Result<Option<String>>| {
                        if app.signin_generation.load(Ordering::SeqCst) != generation {
                            return;
                        }
                        let store = ui.global::<Store>();
                        store.set_signin_state(0);
                        store.set_signin_code(SharedString::new());
                        match outcome {
                            Ok(Some(token)) => {
                                app.update_state(ui, |state| state.settings.github_token = token);
                                app.rebuild_catalog();
                                app.refresh_settings(ui);
                                app.toast(ui, "Signed in to GitHub", false);
                            }
                            Ok(None) => {}
                            Err(e) => app.toast(ui, &e.to_string(), true),
                        }
                    },
                );
            },
        );
    }

    /// Does the signed-in Steam account own this app?
    pub fn owns(&self, id: &AppId) -> bool {
        match id.source {
            SourceKind::Steam => self.steam.read().unwrap().as_ref().is_some_and(|account| account.owns_key(&id.key)),
            SourceKind::Gog => self.gog.read().unwrap().as_ref().is_some_and(|account| account.owns_key(&id.key)),
            _ => false,
        }
    }

    /// Sign in to GOG on GOG's own page, in a small window of the store's, to learn
    /// what the account owns. The store receives a one-time code, asks GOG once for
    /// the list, and keeps only the list.
    pub fn gog_signin(self: &Arc<Self>, ui: &AppWindow) {
        let generation = self.gog_generation.fetch_add(1, Ordering::SeqCst) + 1;
        ui.global::<Store>().set_gog_state(1);
        self.job(
            move |app| -> bilby_core::Result<Option<bilby_core::gog_account::Account>> {
                use bilby_core::gog_account;
                let Some(line) = (app.signin)("gog", &app.dirs.config.join("gog-session"))? else { return Ok(None) };
                let code = gog_account::code_of(&line)
                    .ok_or_else(|| Error::Parse("GOG's answer could not be read.".into()))?;
                gog_account::library(app.catalog().http().as_ref(), &code).map(Some)
            },
            move |app, ui, result| {
                if app.gog_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                ui.global::<Store>().set_gog_state(0);
                match result {
                    Ok(Some(account)) => {
                        let count = account.owned.len();
                        *app.gog.write().unwrap() = Some(account.clone());
                        app.update_state(ui, |state| state.gog = Some(account));
                        app.refresh_settings(ui);
                        app.refresh(ui);
                        app.toast(ui, &format!("Signed in to GOG. {count} games are in your library."), false);
                    }
                    // Closed or cancelled: nothing changes.
                    Ok(None) => {}
                    Err(e) => app.toast(ui, &format!("Could not read your GOG library: {e}"), true),
                }
            },
        );
    }

    /// Stop waiting for a GOG sign-in, closing the sign-in window if it is open.
    pub fn gog_cancel(&self, ui: &AppWindow) {
        self.gog_generation.fetch_add(1, Ordering::SeqCst);
        close_signin_window();
        ui.global::<Store>().set_gog_state(0);
    }

    /// Forget the GOG account: what it owns, and the sign-in window's own data.
    pub fn gog_signout(&self, ui: &AppWindow) {
        self.gog_generation.fetch_add(1, Ordering::SeqCst);
        *self.gog.write().unwrap() = None;
        self.update_state(ui, |state| state.gog = None);
        let _ = std::fs::remove_dir_all(self.dirs.config.join("gog-session"));
        self.refresh_settings(ui);
        self.refresh(ui);
        self.toast(ui, "Signed out of GOG on this device", false);
    }

    /// Sign in to Steam on Steam's own page, to learn what the account owns. On a TV
    /// the page is shown by Longboard, which a remote can drive; elsewhere in a small
    /// window of the store's own. Either way the store receives only the account's
    /// number and the ids of what it owns.
    pub fn steam_signin(self: &Arc<Self>, ui: &AppWindow) {
        let generation = self.steam_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let store = ui.global::<Store>();
        let via_longboard = store.get_shell() == Shell::Tv;
        store.set_steam_state(if via_longboard { 2 } else { 1 });
        self.job(
            move |app| app.steam_ask(via_longboard, generation),
            move |app, ui, result| {
                app.record_fetched(ui);
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                ui.global::<Store>().set_steam_state(0);
                match result {
                    Ok(Some(account)) => app.steam_signed_in(ui, account),
                    // Closed, declined or cancelled: nothing changes.
                    Ok(None) => {}
                    Err(e) => app.toast(ui, &e.to_string(), true),
                }
            },
        );
    }

    /// What a finished Steam sign-in leaves behind, whichever way it was done.
    pub(crate) fn steam_signed_in(&self, ui: &AppWindow, account: bilby_core::steam_account::Account) {
        let count = account.owned.len();
        *self.steam.write().unwrap() = Some(account.clone());
        self.update_state(ui, |state| state.steam = Some(account));
        self.refresh_settings(ui);
        self.refresh(ui);
        self.toast(ui, &format!("Signed in to Steam. {count} games and tools are in your library."), false);
    }

    /// Sign in to Steam with a code: draw it, and wait for the person to scan it with
    /// the Steam app on their phone and approve there.
    pub fn steam_signin_qr(self: &Arc<Self>, ui: &AppWindow) {
        use bilby_core::steam_qr;
        let generation = self.steam_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let store = ui.global::<Store>();
        store.set_steam_qr(slint::Image::default());
        store.set_steam_qr_scanned(false);
        store.set_steam_state(3);
        self.job(
            |app| steam_qr::start(app.catalog().http().as_ref(), "Bilby"),
            move |app, ui, started| {
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                match started {
                    Ok(session) => {
                        ui.global::<Store>().set_steam_qr(code_image(&session.challenge_url));
                        app.steam_qr_wait(session, generation, std::time::Instant::now());
                    }
                    Err(e) => {
                        ui.global::<Store>().set_steam_state(0);
                        app.toast(ui, &format!("Could not start signing in to Steam: {e}"), true);
                    }
                }
            },
        );
    }

    /// Ask Steam once, after the wait it asks for, whether the code was approved; then
    /// show what changed and ask again.
    pub(crate) fn steam_qr_wait(
        self: &Arc<Self>,
        mut session: bilby_core::steam_qr::Session,
        generation: u64,
        since: std::time::Instant,
    ) {
        use bilby_core::steam_qr::{self, Poll};
        let shown = session.challenge_url.clone();
        self.job(
            move |app| {
                // Wait in short steps, so cancelling is noticed promptly.
                let until = std::time::Instant::now() + std::time::Duration::from_secs(session.interval);
                while std::time::Instant::now() < until && app.steam_generation.load(Ordering::SeqCst) == generation {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return (session, Ok(Poll::Expired));
                }
                let answer = steam_qr::poll(app.catalog().http().as_ref(), &mut session);
                (session, answer)
            },
            move |app, ui, (session, answer)| {
                if app.steam_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let store = ui.global::<Store>();
                let stop = |message: &str| {
                    store.set_steam_state(0);
                    store.set_steam_qr(slint::Image::default());
                    app.toast(ui, message, true);
                };
                match answer {
                    Ok(Poll::Done(account)) => {
                        store.set_steam_state(0);
                        store.set_steam_qr(slint::Image::default());
                        app.steam_signed_in(ui, account);
                    }
                    Ok(Poll::Waiting { .. }) if since.elapsed().as_secs() > 300 => {
                        stop("Nobody scanned the code in five minutes. Try again when you are ready.")
                    }
                    Ok(Poll::Waiting { scanned }) => {
                        store.set_steam_qr_scanned(scanned);
                        if session.challenge_url != shown {
                            store.set_steam_qr(code_image(&session.challenge_url));
                        }
                        app.steam_qr_wait(session, generation, since);
                    }
                    Ok(Poll::Expired) => {
                        stop("The code ran out, or the sign-in was refused in the Steam app. Try again.")
                    }
                    Err(e) => stop(&e.to_string()),
                }
            },
        );
    }

    pub(crate) fn steam_ask(
        &self,
        via_longboard: bool,
        generation: u64,
    ) -> bilby_core::Result<Option<bilby_core::steam_account::Account>> {
        use bilby_core::steam_account;
        if !via_longboard {
            let line = (self.signin)("steam", &self.dirs.config.join("steam-session"))?;
            return line
                .map(|line| {
                    steam_account::parse(&line).ok_or_else(|| Error::Parse("Steam's answer could not be read.".into()))
                })
                .transpose();
        }
        let going = || self.steam_generation.load(Ordering::SeqCst) == generation;
        // Installing is behind the PIN where one is set, and nobody was asked for it here.
        if install::find_longboard(&self.dirs).is_none() && self.needs_pin_to_install() {
            return Err(Error::Unsupported(
                "On a TV you sign in to Steam in Longboard, a browser for TVs. Installing it needs this device's PIN: open Longboard's page, install it there, then sign in.".into(),
            ));
        }
        // Not there yet: fetched first, without the person having to go and get it.
        let longboard = self.ensure_longboard(&mut |_, _| going())?;
        // The answer comes back as a file only this user can read, removed once read.
        let dir = self.dirs.cache.join("hand-back");
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let file = dir.join("steam.json");
        let _ = std::fs::remove_file(&file);
        (self.runner)(&longboard.program, &longboard.args_writing(&steam_account::longboard_args(&file), &dir))?;
        let answer = steam_account::await_hand_back(&file, std::time::Duration::from_secs(600), &going);
        let _ = std::fs::remove_file(&file);
        answer
    }

    /// Stop waiting for a Steam sign-in, closing the sign-in window if it is open.
    pub fn steam_cancel(&self, ui: &AppWindow) {
        self.steam_generation.fetch_add(1, Ordering::SeqCst);
        close_signin_window();
        ui.global::<Store>().set_steam_state(0);
    }

    /// Forget the Steam account: what it owns, and the sign-in window's own data.
    pub fn steam_signout(&self, ui: &AppWindow) {
        self.steam_generation.fetch_add(1, Ordering::SeqCst);
        *self.steam.write().unwrap() = None;
        self.update_state(ui, |state| state.steam = None);
        let _ = std::fs::remove_dir_all(self.dirs.config.join("steam-session"));
        self.refresh_settings(ui);
        self.refresh(ui);
        self.toast(ui, "Signed out of Steam on this device", false);
    }

    /// Stop waiting for a sign-in.
    pub fn github_cancel(&self, ui: &AppWindow) {
        self.signin_generation.fetch_add(1, Ordering::SeqCst);
        let store = ui.global::<Store>();
        store.set_signin_state(0);
        store.set_signin_code(SharedString::new());
    }
}
