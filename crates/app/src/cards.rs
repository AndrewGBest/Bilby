//! Tiles: one app as the lists on every page show it.

use crate::pictures::{large_picture_for, own_icon, picture_for};
use crate::text::{initials, l, ratings_text};
use crate::{i18n, App, AppCard, OptionRow, Store};
use bilby_core::catalog::UpdateStatus;
use bilby_core::install::{self, Action};
use bilby_core::model::{human_count, human_size, AppId, AppSummary, InstallOption, SourceKind};
use bilby_core::state::{Favourite, InstalledApp};
use slint::{Model, ModelRc, SharedString, VecModel};

/// What a listing shows in place of something for adults: nothing of it but that it
/// is there, until it is chosen.
pub(crate) fn covered(id: &str) -> AppCard {
    AppCard {
        id: id.into(),
        name: l("Adult"),
        summary: l("Select to view"),
        initials: "A".into(),
        tint: slint::Color::from_rgb_u8(0x55, 0x5b, 0x68),
        adult: true,
        ..Default::default()
    }
}

pub(crate) const TINTS: [u32; 10] =
    [0x5b8def, 0x8e6cf0, 0xe0679a, 0xe8804a, 0xd9a521, 0x4fb477, 0x2fa7a0, 0x3d9bd8, 0x7a86a8, 0xc65fd0];

pub(crate) fn tint_for(id: &str) -> slint::Color {
    let key = id.split_once(':').map(|(_, key)| key).unwrap_or(id);
    let hash = key.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    slint::Color::from_argb_encoded(0xff00_0000 | TINTS[hash as usize % TINTS.len()])
}

/// Release notes are Markdown, often with HTML mixed in; show them as readable plain
/// text. Links keep their words, images and tags go, list markers become bullets.
/// `install-index` values from here up mean "the hidden file numbered index - this".
pub(crate) const HIDDEN_BASE: i32 = 1000;

/// The share of an app's ratings at five stars, then four, down to one. Empty when the
/// breakdown is not known.
pub(crate) fn spread_shares(rating: bilby_core::model::Rating) -> ModelRc<f32> {
    let total: u32 = rating.spread.iter().sum();
    if total == 0 {
        return ModelRc::default();
    }
    let shares: Vec<f32> = rating.spread.iter().rev().map(|n| *n as f32 / total as f32).collect();
    ModelRc::new(VecModel::from(shares))
}

pub(crate) fn option_row(o: &InstallOption) -> OptionRow {
    OptionRow {
        file: o.asset.name.clone().into(),
        label: l(o.label()),
        size: human_size(o.asset.size).into(),
        note: l(o.verdict.note.clone().unwrap_or_default()),
        native: o.verdict.level == bilby_core::compat::Level::Native,
        verified: install::checkable_digest(&o.asset).is_some() && o.plan.action != Action::ShowCommand,
        verb: l(&o.plan.verb),
        steps: ModelRc::new(VecModel::from(o.plan.steps.iter().map(l).collect::<Vec<_>>())),
        command: o.plan.command.clone().unwrap_or_default().into(),
    }
}

/// A card for the top of a page: the same app, with its picture at that size.
pub(crate) fn at_the_top(card: AppCard) -> AppCard {
    let large = large_picture_for(&card.id);
    if large.size().width > 0 {
        AppCard { icon: large, ..card }
    } else {
        card
    }
}

pub(crate) fn find_card(store: &Store, id: &str) -> Option<AppCard> {
    let in_sections = store.get_sections().iter().flat_map(|s| s.apps.iter().collect::<Vec<_>>()).find(|c| c.id == id);
    in_sections
        .or_else(|| store.get_results().iter().find(|c| c.id == id))
        .or_else(|| store.get_suggestions().iter().find(|c| c.id == id))
        .or_else(|| store.get_library().iter().map(|r| r.card).find(|c| c.id == id))
        .or_else(|| store.get_favourites().iter().find(|c| c.id == id))
}

/// The id of the Library's card for the device itself, which opens its page.
pub(crate) const SYSTEM_CARD: &str = "bilby:this-device";

/// A TV tile that is a category, not an app: what follows is the category's title.
pub(crate) const CATEGORY_CARD: &str = "category:";

impl App {
    /// The card for a listing. Something for adults is covered: see `covered`.
    pub fn card(&self, app: &AppSummary) -> AppCard {
        // (Read and let go of at once: the card itself looks at the state again.)
        let uncovered = {
            let state = self.state.lock().unwrap();
            state.settings.adult_uncovered && !state.settings.family_mode
        };
        if app.adult && !uncovered {
            return covered(&app.id.to_string());
        }
        // Shown as it is: then it is a card like any other.
        let mut card = self.card_open(app);
        card.adult = false;
        card
    }

    /// The card as it is, for the app's own page.
    pub(crate) fn card_open(&self, app: &AppSummary) -> AppCard {
        let id = app.id.to_string();
        let popularity = match (app.popularity, app.id.source) {
            // A count of nothing is left out: it reads as a verdict, and is usually just
            // an app nobody has got round to starring.
            (Some(0), _) | (None, _) => String::new(),
            (Some(n), SourceKind::GitHub | SourceKind::Codeberg | SourceKind::GitLab) => {
                format!("{} stars", human_count(n))
            }
            (Some(n), SourceKind::Flathub | SourceKind::Homebrew) => format!("{} installs a month", human_count(n)),
            (Some(n), SourceKind::Aur) => format!("{} votes", human_count(n)),
            (Some(n), SourceKind::Steam) => format!("{} recommendations", human_count(n)),
            _ => String::new(),
        };
        let downloads = match app.downloads {
            Some(n) if n > 0 => format!("{} downloads", human_count(n)),
            _ => String::new(),
        };
        let (popularity, downloads) = (i18n::tr(&popularity), i18n::tr(&downloads));
        // Downloads say "people use this" in words everyone knows; stars need explaining.
        let headline = if downloads.is_empty() { popularity.clone() } else { downloads.clone() };
        let rating = app.rating.or_else(|| self.catalog().rating_for(&app.id));
        // The one figure a tile always shows: what people think of it, else how many
        // use it, and what it costs where it costs anything.
        let price =
            if self.owns(&app.id) { i18n::tr("Owned") } else { i18n::tr(&app.price.clone().unwrap_or_default()) };
        let mut stat = match rating {
            Some(r) => format!("★ {}  ·  {}", bilby_core::ratings::stars_text(r), i18n::tr(&ratings_text(r.count))),
            None => headline.clone(),
        };
        if !price.is_empty() {
            stat = if stat.is_empty() { price.clone() } else { format!("{price}  ·  {stat}") };
        }
        // Shown although it does not run here: that is the one thing its tile says.
        let not_here = app.install_label.as_deref() == Some(bilby_core::catalog::NOT_HERE);
        if not_here {
            stat = i18n::tr(bilby_core::catalog::NOT_HERE);
        }
        if let Some(url) = &app.icon_url {
            self.icon_urls.lock().unwrap().insert(id.clone(), url.clone());
        }
        // In the store's library, or already installed by the system's package manager.
        let installed = self.state.lock().unwrap().installed(&id).is_some()
            || app.installed_version.is_some()
            || self.installed_elsewhere(&app.id, &app.name);
        let has_update = matches!(self.updates.lock().unwrap().get(&id), Some(UpdateStatus::Available { .. }));
        let rating_stars: SharedString = rating.map(bilby_core::ratings::stars_text).unwrap_or_default().into();
        AppCard {
            adult: app.adult,
            stat: l(stat),
            tint: tint_for(&id),
            icon: picture_for(&id),
            id: id.into(),
            name: app.name.clone().into(),
            author: app.author.clone().into(),
            summary: i18n::exact(&app.summary).into(),
            // A repository package says which repository; the rest say which service.
            source: l(app.origin.clone().unwrap_or_else(|| app.id.source.label().to_string())),
            badge: l(app.install_label.clone().unwrap_or_default()),
            version: app.version.clone().unwrap_or_default().into(),
            popularity: l(popularity),
            initials: initials(&app.name).into(),
            installed,
            rating_count: l(rating.map(|r| ratings_text(r.count)).unwrap_or_default()),
            rating_spread: rating.map(spread_shares).unwrap_or_default(),
            // A forge counts the files of the releases it listed; Flathub counts every install.
            downloads_note: l(if app.id.source == SourceKind::Flathub { "in total" } else { "of recent versions" }),
            downloads: downloads.into(),
            price: l(price),
            headline: if not_here { l(bilby_core::catalog::NOT_HERE) } else { l(headline) },
            rating: if not_here { SharedString::new() } else { rating_stars },
            verified: app.verified,
            more_sources: 0,
            has_update: installed && has_update,
        }
    }

    pub(crate) fn cards(&self, apps: &[AppSummary]) -> ModelRc<AppCard> {
        ModelRc::new(VecModel::from(apps.iter().map(|a| self.card(a)).collect::<Vec<_>>()))
    }

    pub(crate) fn library_card(&self, app: &InstalledApp) -> AppCard {
        let update = self.updates.lock().unwrap().get(&app.id).cloned();
        let (version, has_update) = match update {
            Some(UpdateStatus::Available { version, .. }) => (format!("{} → {version}", app.version), true),
            _ => (app.version.clone(), false),
        };
        AppCard {
            id: app.id.as_str().into(),
            name: app.name.as_str().into(),
            author: app.author.as_str().into(),
            summary: i18n::exact(&app.summary).into(),
            source: l(AppId::parse(&app.id).map(|id| id.source.label()).unwrap_or("")),
            badge: l(&app.label),
            version: version.into(),
            popularity: SharedString::new(),
            initials: initials(&app.name).into(),
            tint: tint_for(&app.id),
            icon: if self.store_app.as_ref().is_some_and(|own| own.to_string().eq_ignore_ascii_case(&app.id)) {
                own_icon()
            } else {
                picture_for(&app.id)
            },
            installed: true,
            rating: SharedString::new(),
            rating_count: SharedString::new(),
            rating_spread: ModelRc::default(),
            downloads: SharedString::new(),
            downloads_note: SharedString::new(),
            price: SharedString::new(),
            headline: SharedString::new(),
            verified: false,
            adult: false,
            stat: SharedString::new(),
            more_sources: 0,
            has_update,
        }
    }

    pub(crate) fn favourite_card(&self, favourite: &Favourite) -> AppCard {
        let state = self.state.lock().unwrap();
        let installed = state.installed(&favourite.id);
        AppCard {
            id: favourite.id.as_str().into(),
            name: favourite.name.as_str().into(),
            author: favourite.author.as_str().into(),
            summary: i18n::exact(&favourite.summary).into(),
            source: l(AppId::parse(&favourite.id).map(|id| id.source.label()).unwrap_or("")),
            badge: SharedString::new(),
            version: installed.map(|a| a.version.clone()).unwrap_or_default().into(),
            popularity: SharedString::new(),
            initials: initials(&favourite.name).into(),
            tint: tint_for(&favourite.id),
            icon: picture_for(&favourite.id),
            installed: installed.is_some(),
            rating: SharedString::new(),
            rating_count: SharedString::new(),
            rating_spread: ModelRc::default(),
            downloads: SharedString::new(),
            downloads_note: SharedString::new(),
            price: SharedString::new(),
            headline: SharedString::new(),
            verified: false,
            adult: false,
            stat: SharedString::new(),
            more_sources: 0,
            has_update: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_keeps_its_tint() {
        assert_eq!(tint_for("github:a/b"), tint_for("github:a/b"));
    }
}
