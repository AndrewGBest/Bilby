//! Leave for another program's window to come to the front, on Wayland.
//!
//! Wayland does not let a program put its own window in front of the one in use. A
//! program that is already running and is "started" again (Longboard, asked to open
//! another site) can therefore only come forward if the window the person just used
//! vouches for it: that window asks the window system for a token and hands it to the
//! program, which presents it.
//!
//! The window system honours a token only when it was asked for with the serial of
//! the person's latest key press or click (KWin: `Workspace::mayActivate`, read
//! 2026-11-21). The windowing library under Slint asks without one, so its tokens are
//! refused wherever that is checked. This asks properly: it listens on the store's own
//! connection for the serials of key presses and clicks, and asks with the newest.

use crate::AppWindow;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::ffi::c_void;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};

/// What has been heard so far: the newest serial, and the token last given.
#[derive(Default)]
struct Heard {
    serial: u32,
    token: Option<String>,
    keyboard: bool,
    pointer: bool,
}

pub struct Activation {
    queue: EventQueue<Heard>,
    heard: Heard,
    activation: xdg_activation_v1::XdgActivationV1,
    seat: wl_seat::WlSeat,
    surface: wl_surface::WlSurface,
}

impl Activation {
    /// Listen on the connection the window already has. `None` when the window system
    /// has no such thing to ask for.
    ///
    /// # Safety
    /// `display` must be the live `wl_display` of this program's Wayland connection and
    /// `surface` a live `wl_surface` on it, both outliving what is returned.
    pub unsafe fn on(display: *mut c_void, surface: *mut c_void) -> Option<Activation> {
        // SAFETY: the caller's promise; the connection is shared, not taken over.
        let backend = unsafe { wayland_backend::client::Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<Heard>(&connection).ok()?;
        let handle = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&handle, 1..=5, ()).ok()?;
        let activation: xdg_activation_v1::XdgActivationV1 = globals.bind(&handle, 1..=1, ()).ok()?;
        // SAFETY: the caller's promise that `surface` is a live wl_surface.
        let id =
            unsafe { wayland_backend::client::ObjectId::from_ptr(wl_surface::WlSurface::interface(), surface.cast()) }
                .ok()?;
        let surface = wl_surface::WlSurface::from_id(&connection, id).ok()?;
        let mut heard = Heard::default();
        // The seat says what it has (a keyboard, a pointer); those are then listened to.
        queue.roundtrip(&mut heard).ok()?;
        queue.roundtrip(&mut heard).ok()?;
        Some(Activation { queue, heard, activation, seat, surface })
    }

    /// Take in what has arrived since last time, so the newest serial is known and
    /// nothing piles up. Cheap; called as keys and clicks come in.
    pub fn listen(&mut self) {
        let _ = self.queue.dispatch_pending(&mut self.heard);
    }

    /// A token vouching for whatever is started next, asked for with the newest serial.
    pub fn token(&mut self) -> Option<String> {
        self.listen();
        self.heard.token = None;
        let handle = self.queue.handle();
        let request = self.activation.get_activation_token(&handle, ());
        if self.heard.serial != 0 {
            request.set_serial(self.heard.serial, &self.seat);
        }
        request.set_surface(&self.surface);
        request.commit();
        // The answer comes straight back; a second look covers a slow one.
        for _ in 0..2 {
            if self.queue.roundtrip(&mut self.heard).is_err() || self.heard.token.is_some() {
                break;
            }
        }
        request.destroy();
        self.heard.token.take()
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Heard {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Heard {
    fn event(
        heard: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(has) } = event {
            if has.contains(wl_seat::Capability::Keyboard) && !heard.keyboard {
                heard.keyboard = true;
                seat.get_keyboard(handle, ());
            }
            if has.contains(wl_seat::Capability::Pointer) && !heard.pointer {
                heard.pointer = true;
                seat.get_pointer(handle, ());
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Heard {
    fn event(
        heard: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A key press, or the keyboard arriving at this window: both carry a serial.
        // (The key map also arrives here, as a file; dropping the event closes it.)
        match event {
            wl_keyboard::Event::Key { serial, .. } | wl_keyboard::Event::Enter { serial, .. } => {
                heard.serial = heard.serial.max(serial);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Heard {
    fn event(
        heard: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Button { serial, .. } | wl_pointer::Event::Enter { serial, .. } => {
                heard.serial = heard.serial.max(serial);
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_activation_v1::XdgActivationV1, ()> for Heard {
    fn event(
        _: &mut Self,
        _: &xdg_activation_v1::XdgActivationV1,
        _: xdg_activation_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for Heard {
    fn event(
        heard: &mut Self,
        _: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            heard.token = Some(token);
        }
    }
}

thread_local! {
    /// The store's ear on its own Wayland connection, once it has been wanted: `None`
    /// inside when this is not Wayland or the window system has nothing to ask.
    static ACTIVATION: RefCell<Option<Option<Activation>>> = const { RefCell::new(None) };
}

/// Do something with the store's ear on its Wayland connection, setting it up the
/// first time. `None` when there is none (not Wayland, or no window yet).
pub(crate) fn wayland_activation<T>(ui: &AppWindow, with: impl FnOnce(&mut Activation) -> T) -> Option<T> {
    use slint::winit_030::winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    };
    use slint::winit_030::WinitWindowAccessor;
    ACTIVATION.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.is_none() {
            let made = ui.window().with_winit_window(|window| {
                let (display, surface) =
                    match (window.display_handle().ok()?.as_raw(), window.window_handle().ok()?.as_raw()) {
                        (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) => {
                            (display.display.as_ptr(), surface.surface.as_ptr())
                        }
                        _ => return None,
                    };
                // SAFETY: both come from the live window, which is kept for as long as
                // the program runs, on this same thread.
                unsafe { Activation::on(display, surface) }
            });
            // No window yet: try again another time. Otherwise the answer is kept.
            *kept = Some(made?);
        }
        kept.as_mut().and_then(|ear| ear.as_mut()).map(with)
    })
}
