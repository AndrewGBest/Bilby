//! App pictures: fetched off the interface's thread, decoded once, and painted onto
//! the tiles that are waiting for them.

use crate::{App, AppCard, AppWindow, Shell, Store, ICON_PNG};
use bilby_core::model::{AppId, SourceKind};
use slint::{ComponentHandle, Model, ModelRc};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

thread_local! {
    /// Decoded app pictures by owner. Images belong to the UI thread, so they live here
    /// rather than in `App`, which worker threads share.
    pub(crate) static PICTURES: RefCell<HashMap<String, Picture>> = RefCell::new(HashMap::new());
}

/// A picture at the two sizes it is drawn at: in a tile, and at the top of a page.
#[derive(Clone)]
pub(crate) struct Picture {
    small: slint::Image,
    large: slint::Image,
}

/// How many pixels across a tile's picture and a page's picture are, on this screen
/// and in this layout.
pub(crate) fn picture_sizes(ui: &AppWindow) -> (u32, u32) {
    let window = ui.window();
    let scale = window.scale_factor().max(1.0);
    let (small, large) = match ui.global::<Store>().get_shell() {
        // The TV layout is drawn in 540ths of the screen's height.
        Shell::Tv => {
            let unit = window.size().height as f32 / 540.0;
            (32.0 * unit, 60.0 * unit)
        }
        Shell::Mobile => (48.0 * scale, 64.0 * scale),
        _ => (44.0 * scale, 88.0 * scale),
    };
    ((small.round() as u32).clamp(24, 256), (large.round() as u32).clamp(48, 512))
}

/// Decode a cached PNG or JPEG into something Slint can draw, at exactly the sizes it
/// is shown at. The drawing code scales a picture by picking pixels, which leaves a
/// 128-pixel icon shown at 44 jagged; scaled here, with care, it is drawn as it is.
/// Implausible sizes are refused: a huge image would only cost memory.
pub(crate) fn decode_picture(path: &std::path::Path, (small, large): (u32, u32)) -> Option<Picture> {
    let decoded = image::open(path).ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    let at = |edge: u32| {
        // The shorter side fills the square it is shown in; the rest is cropped there.
        let shorter = width.min(height) as f32;
        let (w, h) = (
            ((width as f32 * edge as f32 / shorter).round() as u32).max(1),
            ((height as f32 * edge as f32 / shorter).round() as u32).max(1),
        );
        let sized = if (w, h) == (width, height) {
            decoded.to_rgba8()
        } else {
            let filter =
                if w < width { image::imageops::FilterType::Lanczos3 } else { image::imageops::FilterType::CatmullRom };
            image::imageops::resize(&decoded.to_rgba8(), w, h, filter)
        };
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(sized.as_raw(), w, h);
        slint::Image::from_rgba8(buffer)
    };
    Some(Picture { small: at(small), large: at(large) })
}

/// Decode a cached screenshot, scaled down to what a page can use. Tiny images are
/// not screenshots, and absurd ones are refused before they cost memory.
pub(crate) fn decode_screenshot(path: &std::path::Path) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let (width, height) = image::image_dimensions(path).ok()?;
    if width < 200 || height < 120 || width > 8000 || height > 8000 {
        return None;
    }
    let decoded = image::open(path).ok()?;
    // Shrink what is larger than a screen can show; never enlarge. Kept at up to a
    // full 1080p screen, so a picture opened large is sharp and not stretched.
    let decoded = if width > 1920 || height > 1080 {
        decoded.resize(1920, 1080, image::imageops::FilterType::Triangle)
    } else {
        decoded
    }
    .into_rgba8();
    let (width, height) = decoded.dimensions();
    Some(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(decoded.as_raw(), width, height))
}

/// The app's own logo when one is known, else its owner's picture, at the size of a tile.
pub(crate) fn picture_for(id: &str) -> slint::Image {
    picture_at(id, false)
}

/// The same, at the size it has at the top of a page.
pub(crate) fn large_picture_for(id: &str) -> slint::Image {
    picture_at(id, true)
}

pub(crate) fn picture_at(id: &str, large: bool) -> slint::Image {
    let Some(id) = AppId::parse(id) else { return slint::Image::default() };
    let own = bilby_core::icons::app_key(&id);
    // The store's own picture is the one it carries, wherever it is shown: not its
    // owner's picture from GitHub, nor a logo that happens to arrive later.
    if let Some(picture) =
        OWN_PICTURE.with(|kept| kept.borrow().as_ref().filter(|(key, _)| *key == own).map(|(_, p)| p.clone()))
    {
        return picture;
    }
    PICTURES.with(|p| {
        let pictures = p.borrow();
        pictures
            .get(&own)
            .or_else(|| bilby_core::icons::icon_key(&id).and_then(|owner| pictures.get(&owner)))
            .map(|picture| if large { picture.large.clone() } else { picture.small.clone() })
            .unwrap_or_default()
    })
}

/// A website's picture. Its own, as the site offers it; but many sites offer only a
/// tiny one (Spotify's is 32 pixels across), so where the service has a program of
/// its own on Flathub, with a full-size icon published there, that one is used.
pub(crate) fn site_picture(
    http: &dyn bilby_core::http::Http,
    dir: &std::path::Path,
    key: &str,
    id: &AppId,
) -> bilby_core::Result<Option<PathBuf>> {
    let own = bilby_core::icons::fetch_site(http, dir, key, &id.key);
    let small = |path: &PathBuf| image::image_dimensions(path).is_ok_and(|(width, _)| width < 96);
    if !matches!(&own, Ok(Some(path)) if !small(path)) {
        let families = bilby_core::related::families();
        let programs_icon = bilby_core::related::family_of(families, &id.to_string())
            .and_then(|family| {
                family
                    .members
                    .iter()
                    .find(|m| m.official && m.id.starts_with("flathub:"))
                    .and_then(|m| AppId::parse(&m.id))
            })
            .and_then(|program| bilby_core::icons::own_logo_url(&program));
        if let Some(url) = programs_icon {
            let borrowed = format!("{key}-large");
            if let Ok(Some(path)) = bilby_core::icons::fetch_picture(http, dir, &borrowed, &url, 2 * 1024 * 1024) {
                return Ok(Some(path));
            }
        }
    }
    own
}

/// A code for a phone's camera, as a picture: one dot per square with a clear margin
/// around it, to be drawn large without smoothing.
pub(crate) fn code_image(text: &str) -> slint::Image {
    let Some(code) = bilby_core::steam_qr::code(text) else { return slint::Image::default() };
    const MARGIN: usize = 2;
    let side = code.size + 2 * MARGIN;
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(side as u32, side as u32);
    for (at, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (at % side, at / side);
        let inside = (MARGIN..MARGIN + code.size).contains(&x) && (MARGIN..MARGIN + code.size).contains(&y);
        let dark = inside && code.dark[(y - MARGIN) * code.size + (x - MARGIN)];
        let shade = if dark { 0 } else { 255 };
        *pixel = slint::Rgba8Pixel { r: shade, g: shade, b: shade, a: 255 };
    }
    slint::Image::from_rgba8(pixels)
}

thread_local! {
    /// The store's own picture, decoded once, under the name its own app goes by.
    pub(crate) static OWN_PICTURE: RefCell<Option<(String, slint::Image)>> = const { RefCell::new(None) };
}

/// The store's own picture, for wherever the store itself is shown.
pub(crate) fn own_icon() -> slint::Image {
    if let Some(picture) = OWN_PICTURE.with(|kept| kept.borrow().as_ref().map(|(_, picture)| picture.clone())) {
        return picture;
    }
    let Ok(picture) = image::load_from_memory(ICON_PNG) else { return slint::Image::default() };
    let picture = picture.thumbnail(128, 128).to_rgba8();
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        picture.as_raw(),
        picture.width(),
        picture.height(),
    );
    slint::Image::from_rgba8(buffer)
}

pub(crate) const SITE_ICON: &str = "site:";

/// Stands for "ask the project itself for its logo" (a GitHub project's README).
pub(crate) const PROJECT_LOGO: &str = "project:";

impl App {
    /// Fetch pictures for every app on screen that does not have one yet. Runs after
    /// each job, so whatever a job put on screen gets its pictures; owners already asked
    /// for are skipped, which is also what ends the chain.
    pub fn load_pictures(self: &Arc<Self>, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let mut shown: Vec<String> = Vec::new();
        for section in store.get_sections().iter() {
            shown.extend(section.apps.iter().map(|c| c.id.to_string()));
        }
        shown.extend(store.get_library().iter().map(|r| r.card.id.to_string()));
        for section in store.get_tv_library_rows().iter() {
            shown.extend(section.apps.iter().map(|c| c.id.to_string()));
        }
        shown.push(store.get_app().id.to_string());
        // The home page, the library and the open app: few enough that a project's
        // own logo is looked for. A page of search results is not.
        let few: HashSet<String> = shown.iter().cloned().collect();
        shown.extend(store.get_results().iter().map(|c| c.id.to_string()));

        // For each app: its own logo (an address the source gave, or one found earlier
        // and kept on disk), and its owner's picture to fall back on.
        let wanted: Vec<(String, AppId, Option<String>)> = {
            let mut requested = self.pictures_requested.lock().unwrap();
            let urls = self.icon_urls.lock().unwrap();
            let mut wanted = Vec::new();
            let icons = self.dirs.cache.join("icons");
            for id in shown.iter().filter_map(|id| AppId::parse(id)) {
                let own = bilby_core::icons::app_key(&id);
                match urls.get(&id.to_string()) {
                    Some(url) => {
                        if requested.insert(own.clone()) {
                            wanted.push((own, id.clone(), Some(url.clone())));
                        }
                    }
                    // No address yet. A logo found on an earlier visit is used; otherwise
                    // the question stays open, because the address may still arrive.
                    // A website's picture comes from the site itself.
                    None if id.source == SourceKind::Web => {
                        if requested.insert(own.clone()) {
                            wanted.push((own, id.clone(), Some(SITE_ICON.to_string())));
                        }
                    }
                    None if requested.contains(&own) => {}
                    None => {
                        let kept = bilby_core::icons::cached(&icons, &own);
                        // Where the address follows from the app's name, or the project
                        // can be asked for its logo, a list shows it without the app's
                        // page having been opened first.
                        let how = if matches!(kept, Some(Some(_))) {
                            Some(String::new())
                        } else if let Some(url) = bilby_core::icons::own_logo_url(&id) {
                            Some(url)
                        } else if id.source == SourceKind::GitHub && kept.is_none() && few.contains(&id.to_string()) {
                            Some(PROJECT_LOGO.to_string())
                        } else {
                            None
                        };
                        if let Some(how) = how {
                            requested.insert(own.clone());
                            wanted.push((own, id.clone(), Some(how)));
                        }
                    }
                }
                if let Some(owner) = bilby_core::icons::icon_key(&id) {
                    if requested.insert(owner.clone()) {
                        wanted.push((owner, id, None));
                    }
                }
            }
            wanted
        };
        if wanted.is_empty() {
            return;
        }
        // A few at a time, each group on its own: a slow site holds up only its group,
        // and pictures appear as they arrive instead of all at the end.
        for group in wanted.chunks(4).map(<[_]>::to_vec) {
            let dir = self.dirs.cache.join("icons");
            self.job(
                move |app| {
                    let http = app.catalog().http();
                    group
                        .into_iter()
                        .filter_map(|(key, id, own)| {
                            let fetched = match own {
                                // No address: only what an earlier visit to its page left on disk.
                                Some(url) if url.is_empty() && id.source != SourceKind::Web => {
                                    Ok(bilby_core::icons::cached(&dir, &key).flatten())
                                }
                                Some(url) if url.is_empty() || url == SITE_ICON => {
                                    site_picture(http.as_ref(), &dir, &key, &id)
                                }
                                Some(url) if url == PROJECT_LOGO => {
                                    bilby_core::icons::fetch_project_logo(http.as_ref(), &dir, &key, &id)
                                }
                                Some(url) => {
                                    bilby_core::icons::fetch_picture(http.as_ref(), &dir, &key, &url, 2 * 1024 * 1024)
                                }
                                None => bilby_core::icons::fetch(http.as_ref(), &dir, &id),
                            };
                            match fetched {
                                Ok(path) => Some((key, path?)),
                                // The network let it down, which says nothing about the
                                // picture: it is asked for again later, a couple of times.
                                Err(_) => {
                                    let mut failures = app.picture_failures.lock().unwrap();
                                    let count = failures.entry(key.clone()).or_insert(0);
                                    *count += 1;
                                    if *count <= 2 {
                                        app.pictures_requested.lock().unwrap().remove(&key);
                                    }
                                    None
                                }
                            }
                        })
                        .collect::<Vec<_>>()
                },
                |app, ui, found| {
                    if found.is_empty() {
                        return;
                    }
                    let sizes = picture_sizes(ui);
                    PICTURES.with(|pictures| {
                        for (key, path) in found {
                            // A file that will not decode is simply not shown.
                            if let Some(image) = decode_picture(&path, sizes) {
                                pictures.borrow_mut().insert(key, image);
                            }
                        }
                    });
                    app.repaint_pictures(ui);
                },
            );
        }
    }

    /// Put pictures that have just arrived into the tiles already on screen. Only the
    /// tiles whose picture changed are touched: the lists stay as they are, where a
    /// full redraw would work every list out again and build every tile anew, once
    /// for each handful of pictures.
    pub(crate) fn repaint_pictures(&self, ui: &AppWindow) {
        let store = ui.global::<Store>();
        let own = self.store_app.as_ref().map(|id| id.to_string());
        // The store's own tile keeps the picture it was built with.
        let fresh = |card: &AppCard| -> Option<slint::Image> {
            if own.as_ref().is_some_and(|own| own.eq_ignore_ascii_case(card.id.as_str())) {
                return None;
            }
            let icon = picture_for(&card.id);
            (icon != card.icon).then_some(icon)
        };
        let repaint = |cards: ModelRc<AppCard>| {
            for row in 0..cards.row_count() {
                let Some(mut card) = cards.row_data(row) else { continue };
                if let Some(icon) = fresh(&card) {
                    card.icon = icon;
                    cards.set_row_data(row, card);
                }
            }
        };
        for rows in [store.get_sections(), store.get_tv_rows(), store.get_tv_library_rows()] {
            for section in rows.iter() {
                repaint(section.apps);
            }
        }
        repaint(store.get_results());
        let library = store.get_library();
        for row in 0..library.row_count() {
            let Some(mut entry) = library.row_data(row) else { continue };
            if let Some(icon) = fresh(&entry.card) {
                entry.card.icon = icon;
                library.set_row_data(row, entry);
            }
        }
        for (card, set) in [
            (store.get_app(), Box::new(|c| store.set_app(c)) as Box<dyn Fn(AppCard)>),
            (store.get_spotlight(), Box::new(|c| store.set_spotlight(c))),
        ] {
            let own_card = own.as_ref().is_some_and(|own| own.eq_ignore_ascii_case(card.id.as_str()));
            let icon = large_picture_for(&card.id);
            if !own_card && icon.size().width > 0 && icon != card.icon {
                set(AppCard { icon, ..card });
            }
        }
    }
}
