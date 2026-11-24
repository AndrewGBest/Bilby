//! The icon in the system tray, where Bilby waits when its window is closed.

#[cfg(not(target_os = "android"))]
use crate::pictures::own_icon;
use crate::{instance, App, AppWindow, BilbyTray, Store};
use slint::ComponentHandle;
use std::sync::Arc;

thread_local! {
    /// The store's icon in the system tray, where there is a tray to put it in.
    pub(crate) static TRAY: std::cell::RefCell<Option<BilbyTray>> = const { std::cell::RefCell::new(None) };
}

/// Show the tray icon, or take it away. With it showing, closing the window leaves
/// the store running there.
pub(crate) fn show_tray(on: bool) {
    TRAY.with(|tray| {
        if let Some(tray) = tray.borrow().as_ref() {
            tray.set_kept(on);
        }
    });
}

/// Is the store's icon in the system tray right now?
pub(crate) fn tray_showing() -> bool {
    TRAY.with(|tray| tray.borrow().as_ref().is_some_and(|tray| tray.get_kept()))
}

/// Open the window again, from the tray or because the store was started a second time.
pub(crate) fn bring_back(ui: &AppWindow, app: &Arc<App>) {
    let _ = ui.show();
    ui.window().set_minimized(false);
    app.window_opened(ui);
}

#[cfg(not(target_os = "android"))]
pub(crate) fn set_up_tray(ui: &AppWindow, app: &Arc<App>) {
    if !instance::has_a_tray() {
        return;
    }
    let Ok(tray) = BilbyTray::new() else { return };
    tray.set_picture(own_icon());
    let (opened, weak) = (app.clone(), ui.as_weak());
    tray.on_open(move || {
        if let Some(ui) = weak.upgrade() {
            bring_back(&ui, &opened);
        }
    });
    let (checking, weak) = (app.clone(), ui.as_weak());
    tray.on_check(move || {
        if let Some(ui) = weak.upgrade() {
            checking.check_everything(&ui);
        }
    });
    tray.on_quit(|| {
        show_tray(false);
        let _ = slint::quit_event_loop();
    });
    tray.set_kept(!app.state().settings.quit_on_close);
    TRAY.with(|kept| *kept.borrow_mut() = Some(tray));
    ui.global::<Store>().set_tray_available(true);
    // Closing the window hides it either way. With the icon in the tray the store
    // goes on running there; without it, nothing is left and the program ends.
    let closing = app.clone();
    ui.window().on_close_requested(move || {
        if tray_showing() {
            closing.window_closed_to_tray();
        }
        slint::CloseRequestResponse::HideWindow
    });
}
