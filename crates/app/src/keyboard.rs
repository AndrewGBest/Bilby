//! The on-screen keyboard of the TV layout, and the system's password asked for on it.

use crate::{i18n, osk, App, AppWindow, OskKey, Store};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

impl App {
    /// Draw the on-screen keyboard as it now stands.
    pub(crate) fn show_osk(&self, ui: &AppWindow) {
        let model = *self.osk.lock().unwrap();
        let asking = self.password_reply.lock().unwrap().is_some();
        let mut keys = Vec::new();
        for (row, line) in model.rows().iter().enumerate() {
            let mut col = 0;
            for key in line {
                // "search" where it searches; where a password is typed, "enter".
                let label = match key.kind {
                    osk::Kind::Char | osk::Kind::Symbols | osk::Kind::Letters => key.label.clone(),
                    osk::Kind::Submit if asking => i18n::tr("enter"),
                    // Typing a review: the last key only puts the keyboard away.
                    osk::Kind::Submit if ui.global::<Store>().get_osk_target() != 0 => i18n::tr("done"),
                    osk::Kind::Shift if key.label == "SHIFT" => i18n::tr("shift").to_uppercase(),
                    _ => i18n::tr(&key.label),
                };
                keys.push(OskKey {
                    label: label.as_str().into(),
                    row: row as i32,
                    col: col as i32,
                    span: key.span as i32,
                    wide: key.kind != osk::Kind::Char,
                    go: key.kind == osk::Kind::Submit,
                });
                col += key.span;
            }
        }
        let store = ui.global::<Store>();
        store.set_osk_keys(ModelRc::new(VecModel::from(keys)));
        store.set_osk_at(model.flat() as i32);
    }

    /// OK on the highlighted key. True when it was the one that says "search".
    pub(crate) fn osk_press(&self, ui: &AppWindow) -> bool {
        let key = self.osk.lock().unwrap().key();
        match key.kind {
            osk::Kind::Submit => return true,
            osk::Kind::Char => self.osk_add(ui, &key.label),
            osk::Kind::Space => self.osk_add(ui, " "),
            osk::Kind::Delete => self.osk_erase(ui),
            osk::Kind::Shift | osk::Kind::Symbols | osk::Kind::Letters => {
                self.osk.lock().unwrap().switch_face();
                self.show_osk(ui);
            }
        }
        false
    }

    /// Take the last letter off what has been typed.
    pub(crate) fn osk_erase(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        if self.password_reply.lock().unwrap().is_some() {
            let mut password = self.password.lock().unwrap();
            password.pop();
            store.set_password_length(password.chars().count() as i32);
            return;
        }
        let erase = |text: SharedString| -> SharedString {
            let mut text = text.to_string();
            text.pop();
            text.into()
        };
        match store.get_osk_target() {
            1 => store.set_review_title(erase(store.get_review_title())),
            2 => store.set_review_text(erase(store.get_review_text())),
            3 => store.set_review_name(erase(store.get_review_name())),
            _ => store.set_query(erase(store.get_query())),
        }
    }

    /// Add to what is being typed: the search, or the password when one is asked for.
    pub(crate) fn osk_add(&self, ui: &AppWindow, text: &str) {
        let store = ui.global::<Store>();
        if self.password_reply.lock().unwrap().is_some() {
            let mut password = self.password.lock().unwrap();
            password.push_str(text);
            store.set_password_length(password.chars().count() as i32);
            return;
        }
        // A review's words have the ratings service's limits; more is not taken.
        let add = |to: SharedString, limit: usize| -> SharedString {
            match to.chars().count() + text.chars().count() > limit {
                true => to,
                false => format!("{to}{text}").into(),
            }
        };
        use bilby_core::reviewing::{TEXT_LIMIT, TITLE_LIMIT};
        match store.get_osk_target() {
            1 => store.set_review_title(add(store.get_review_title(), TITLE_LIMIT)),
            2 => store.set_review_text(add(store.get_review_text(), TEXT_LIMIT)),
            3 => store.set_review_name(add(store.get_review_name(), 40)),
            _ => store.set_query(format!("{}{}", store.get_query(), text).into()),
        }
    }

    /// Ask for the system's password on screen, with the keyboard. `again`: the one
    /// just given was wrong. The answer goes to `reply`.
    pub(crate) fn show_password(&self, ui: &AppWindow, again: bool, reply: std::sync::mpsc::Sender<Option<String>>) {
        // One question at a time: an earlier one that was never answered is given up.
        if let Some(earlier) = self.password_reply.lock().unwrap().replace(reply) {
            let _ = earlier.send(None);
        }
        self.password.lock().unwrap().clear();
        *self.osk.lock().unwrap() = osk::Model::default();
        let store = ui.global::<Store>();
        store.set_password_length(0);
        store.set_password_wrong(again);
        store.set_password_ask(true);
        self.show_osk(ui);
    }

    /// The password prompt is answered (`give`) or put away.
    pub(crate) fn answer_password(&self, ui: &AppWindow, give: bool) {
        let typed = std::mem::take(&mut *self.password.lock().unwrap());
        if let Some(reply) = self.password_reply.lock().unwrap().take() {
            let _ = reply.send((give && !typed.is_empty()).then_some(typed));
        }
        let store = ui.global::<Store>();
        store.set_password_ask(false);
        store.set_password_length(0);
    }
}
