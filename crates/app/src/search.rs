//! Searching and browsing a category.

use crate::text::{l, plural};
use crate::{i18n, App, AppWindow, Store};
use bilby_core::catalog::SearchOutcome;
use bilby_core::model::AppId;
use slint::{ComponentHandle, ModelRc, SharedString};
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl App {
    /// Out of a search and back to Home: the box is emptied and the results put away,
    /// so Home shows the store again and not what was last looked for.
    /// Browse one category in full. `name` is its title as shown, or nothing to go
    /// back to every category; `as_page` gives it the page to itself, as on a TV.
    pub fn browse_category(self: &Arc<Self>, ui: &AppWindow, name: &str, as_page: bool) {
        let store = ui.global::<Store>();
        let mut titles: Vec<String> = self.catalog().categories();
        titles.extend(self.catalog().featured().into_iter().map(|row| row.title));
        titles.extend(self.collections.lock().unwrap().iter().map(|row| row.title.clone()));
        let Some(title) = titles.into_iter().find(|title| title == name || i18n::tr(title) == name) else {
            *self.browsing.lock().unwrap() = None;
            return self.refresh(ui);
        };
        if self.browsing.lock().unwrap().as_ref().is_some_and(|(now, _)| *now == title) && !as_page {
            return;
        }
        *self.browsing.lock().unwrap() = Some((title.clone(), Vec::new()));
        if as_page {
            self.close_details(ui);
            *self.results.lock().unwrap() = (String::new(), Vec::new());
            store.set_query(SharedString::new());
            store.set_results(ModelRc::default());
            store.set_page(1);
            store.set_searching(true);
        }
        self.refresh(ui);
        self.job(
            move |app| (app.catalog().category(&title, 200), title),
            move |app, ui, (apps, title)| {
                // Only if this is still the category that is wanted.
                let mut browsing = app.browsing.lock().unwrap();
                let wanted = browsing.as_ref().is_some_and(|(now, _)| *now == title);
                if wanted {
                    *browsing = Some((title, apps));
                }
                drop(browsing);
                if wanted {
                    if as_page {
                        ui.global::<Store>().set_searching(false);
                    }
                    app.refresh(ui);
                }
            },
        );
    }

    pub fn leave_search(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        *self.browsing.lock().unwrap() = None;
        // A search still under way has nowhere to show its answer.
        self.search_generation.fetch_add(1, Ordering::SeqCst);
        *self.result_filter.lock().unwrap() = None;
        *self.results.lock().unwrap() = (String::new(), Vec::new());
        store.set_query(SharedString::new());
        store.set_searching(false);
        store.set_searched(false);
        store.set_search_note(SharedString::new());
        store.set_search_error(SharedString::new());
        store.set_results(ModelRc::default());
        self.close_details(ui);
        store.set_page(0);
        self.refresh(ui);
    }

    pub fn search(self: &Arc<Self>, ui: &AppWindow, query: &str) {
        let query = query.trim().to_string();
        if query.is_empty() {
            return;
        }
        // A pasted link or owner/repo goes straight to that app.
        // A new search starts unfiltered.
        *self.result_filter.lock().unwrap() = None;
        if let Some(id) = AppId::from_user_input(&query) {
            // A link opens even when its source is switched off: the person asked for it.
            if self.catalog().available_sources().contains(&id.source) {
                self.open(ui, &id.to_string());
                return;
            }
        }
        let store = ui.global::<Store>();
        if query.chars().count() < 2 {
            store.set_page(1);
            store.set_search_note(l("Type at least two characters."));
            return;
        }
        self.close_details(ui);
        store.set_page(1);
        store.set_searching(true);
        // What was on screen before (the home page's rows) goes now: the search page
        // waits, with its own animation, until the results come.
        store.set_results(ModelRc::default());
        store.set_tv_rows(ModelRc::default());
        store.set_search_note(SharedString::new());
        store.set_search_error(SharedString::new());
        self.update_state(ui, |state| state.add_search(&query));
        let generation = self.search_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let work_query = query.clone();
        self.job(
            move |app| app.catalog().search(&work_query),
            move |app, ui, outcome| {
                if app.search_generation.load(Ordering::SeqCst) == generation {
                    app.show_search(ui, &query, &outcome);
                }
            },
        );
    }

    pub(crate) fn show_search(&self, ui: &AppWindow, query: &str, outcome: &SearchOutcome) {
        let store = ui.global::<Store>();
        store.set_searching(false);
        store.set_searched(true);
        let mut note = if outcome.apps.is_empty() {
            String::new()
        } else {
            format!("{} for “{query}”.", plural(outcome.apps.len(), "app"))
        };
        if outcome.hidden > 0 {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(&format!("{} more matched but do not work on this device.", outcome.hidden));
        }
        store.set_search_note(l(note));
        store.set_search_error(l(outcome.errors.join("\n")));
        *self.results.lock().unwrap() = (query.to_string(), outcome.apps.clone());
        self.refresh(ui);
    }
}
