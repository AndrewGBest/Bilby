//! Writing a review: where it can be left, and sending it.

use crate::text::l;
use crate::{i18n, App, AppWindow, ReviewPlace, Store};
use bilby_core::model::{AppId, SourceKind};
use bilby_core::{Catalog, Error};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::sync::Arc;

impl App {
    /// Where a review of this app can be left: the open ratings service, and the
    /// stores the same app comes from. A store the signed-in account does not own
    /// the game at is left out, since it would refuse the review.
    pub(crate) fn review_places_for(&self, catalog: &Catalog, id: &AppId) -> Vec<bilby_core::reviewing::Place> {
        if self.state().settings.hide_ratings {
            return Vec::new();
        }
        let mut ids = vec![id.clone()];
        if let Some(members) = self.alternates.lock().unwrap().get(&id.to_string()) {
            ids.extend(members.iter().filter_map(|(other, _)| AppId::parse(other)).filter(|other| other != id));
        }
        let signed_in = |source: SourceKind| match source {
            SourceKind::Steam => self.steam.read().unwrap().is_some(),
            SourceKind::Gog => self.gog.read().unwrap().is_some(),
            _ => false,
        };
        catalog.review_places(&ids, &|other| signed_in(other.source) && !self.owns(other))
    }

    /// Show where a review can go, each place chosen to begin with.
    pub(crate) fn show_review_places(&self, ui: &AppWindow, places: Vec<bilby_core::reviewing::Place>) {
        let store = ui.global::<Store>();
        let rows: Vec<ReviewPlace> = places
            .iter()
            .map(|place| ReviewPlace {
                id: place.id.as_str().into(),
                name: l(&place.name),
                note: l(&place.note),
                from_here: place.from_here,
                chosen: true,
            })
            .collect();
        store.set_review_copies(places.iter().any(|place| !place.from_here));
        store.set_review_from_here(places.iter().any(|place| place.from_here));
        store.set_review_places(ModelRc::new(VecModel::from(rows)));
        store.set_review_name(self.state().reviewer_name.as_str().into());
        store.set_review_title(SharedString::new());
        store.set_review_text(SharedString::new());
        store.set_review_problem(SharedString::new());
        store.set_review_result(SharedString::new());
        store.set_review_state(0);
        *self.review_places.lock().unwrap() = places;
    }

    /// Choose, or stop choosing, one of the places a review goes to.
    pub fn toggle_review_place(&self, ui: &AppWindow, id: &str) {
        let store = ui.global::<Store>();
        let places = store.get_review_places();
        for n in 0..places.row_count() {
            if let Some(mut row) = places.row_data(n) {
                if row.id == id {
                    row.chosen = !row.chosen;
                    places.set_row_data(n, row);
                }
            }
        }
        store.set_review_copies(places.iter().any(|row| row.chosen && !row.from_here));
        store.set_review_from_here(places.iter().any(|row| row.chosen && row.from_here));
        store.set_review_problem(SharedString::new());
    }

    /// Leave the review at every chosen place: sent from here to the open service,
    /// and for a store, its own page opened (the words were copied by the page that
    /// called this, ready to paste).
    pub fn send_review(self: &Arc<Self>, ui: &AppWindow, draft: bilby_core::reviewing::Draft) {
        use bilby_core::reviewing;
        let store = ui.global::<Store>();
        if store.get_review_state() == 1 {
            return;
        }
        let chosen: Vec<String> =
            store.get_review_places().iter().filter(|row| row.chosen).map(|row| row.id.to_string()).collect();
        let places: Vec<reviewing::Place> =
            self.review_places.lock().unwrap().iter().filter(|place| chosen.contains(&place.id)).cloned().collect();
        let from_here = places.iter().find(|place| place.from_here).cloned();
        let problem = if places.is_empty() {
            Some("Choose where to leave your review.")
        } else {
            draft.problem().or_else(|| from_here.as_ref().and_then(|_| draft.problem_from_here()))
        };
        if let Some(problem) = problem {
            store.set_review_problem(l(problem));
            return;
        }
        store.set_review_problem(SharedString::new());
        // The stores' own pages, each with its box to write in.
        let mut said: Vec<String> = Vec::new();
        for place in places.iter().filter(|place| !place.from_here) {
            let opened = if !reviewing::link_ok(&place.target) {
                Err(Error::Unsupported("That page cannot be opened from here.".into()))
            } else if place.target.starts_with("https://") {
                (self.browser)(&place.target)
            } else {
                (self.opener)(std::path::Path::new(&place.target))
            };
            let name = i18n::tr(&place.name);
            said.push(i18n::tr(&match opened {
                Ok(()) => format!("Opened {name}: paste your review into its box there."),
                Err(e) => format!("Could not open the page for {name}: {e}"),
            }));
        }
        let Some(service) = from_here else {
            store.set_review_result(
                said.join(
                    "
",
                )
                .into(),
            );
            store.set_review_state(2);
            return;
        };
        // The made-up id that marks this person's reviews, from the first one on.
        let mut writer_id = self.state().reviewer;
        if !reviewing::writer_id_ok(&writer_id) {
            writer_id = reviewing::new_writer_id();
        }
        let name = draft.name.trim().to_string();
        let kept = writer_id.clone();
        self.update_state(ui, move |state| {
            state.reviewer = kept;
            state.reviewer_name = name;
        });
        let version = store.get_installed_version().to_string();
        let page = store.get_app().id.to_string();
        store.set_review_state(1);
        self.job(
            move |app| {
                let catalog = app.catalog();
                let writer = reviewing::Writer {
                    id: &writer_id,
                    locale: i18n::language(),
                    system: &catalog.platform.os_name,
                    version: &version,
                };
                reviewing::send(catalog.http().as_ref(), &service.target, &draft, &writer)
            },
            move |_app, ui, sent| {
                let store = ui.global::<Store>();
                // Another app's page by now: its form is not this review's.
                if store.get_app().id != page.as_str() {
                    return;
                }
                let mut said = said;
                match sent {
                    Ok(()) => {
                        said.insert(
                            0,
                            i18n::tr("Sent, thank you. It can take a while to show, here and in other app stores."),
                        );
                        store.set_review_result(
                            said.join(
                                "
",
                            )
                            .into(),
                        );
                        store.set_review_state(2);
                    }
                    Err(e) => {
                        store.set_review_problem(l(e.to_string()));
                        store.set_review_result(
                            said.join(
                                "
",
                            )
                            .into(),
                        );
                        store.set_review_state(0);
                    }
                }
            },
        );
    }
}
