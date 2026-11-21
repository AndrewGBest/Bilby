//! Ordering search results by how likely each one is what the person meant.
//!
//! Every source ranks its own hits, but they cannot be compared across sources, and
//! "what GitHub thinks matches" is not the same as "what someone typing a name wants".
//! So every hit gets one score from three things:
//!
//! * **how close the name is to what was typed**,
//! * **how many people use it** (downloads, ratings, stars), on a scale that suits its
//!   source, weighed almost as heavily: a well-used app that is plainly about what was
//!   typed belongs above an obscure one that merely has the word in its name,
//! * **small nudges** from anything else that is known: on the bundled starter list for the Discover
//!   page, already installed, where the source itself placed it, flagged out of date.
//!
//! A hit whose source publishes no numbers (a distribution's repository) gets a middle
//! value for the missing part rather than zero: no data is not bad data.

use crate::model::{AppSummary, SourceKind};

/// The share of the score each part can contribute. They sum to 1.
const CLOSENESS: f32 = 0.40;
const POPULARITY: f32 = 0.45;
const POSITION: f32 = 0.05;
const NUDGES: f32 = 0.10;

/// Added for the app whose name is exactly what was typed: that one is first.
const EXACT_NAME: f32 = 0.20;

/// What a hit with no popularity figure is assumed to have: a little under the middle,
/// so that an app known to be widely used is not outranked by one nothing is known of.
const UNKNOWN_POPULARITY: f32 = 0.4;

/// Things known about a hit beyond the summary itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// Where its own source placed it (0 is first) among how many.
    pub position: usize,
    pub of: usize,
    /// It is on the bundled Discover list.
    pub featured: bool,
    /// It is one of the apps kept on record for the service that was searched for
    /// (VacuumTube, for "youtube"): what the person is looking for, whatever its name.
    pub related: bool,
}

/// 0.0 (unlikely) to 1.0 (certainly what was meant).
pub fn score(query: &str, app: &AppSummary, context: Context) -> f32 {
    let position = if context.of > 1 { 1.0 - context.position as f32 / (context.of - 1) as f32 } else { 1.0 };
    let close = closeness(query, app);
    // An app for the service that was named counts as a near match on its name.
    let close = if context.related { close.max(0.85) } else { close };
    CLOSENESS * close
        + if close >= 1.0 { EXACT_NAME } else { 0.0 }
        + POPULARITY * popularity(app)
        + POSITION * position
        + NUDGES * nudges(app, context.featured)
}

/// Sort hits, best first. Ties keep the order they came in.
pub fn sort(query: &str, hits: Vec<(AppSummary, Context)>) -> Vec<AppSummary> {
    let mut scored: Vec<(f32, AppSummary)> = hits.into_iter().map(|(app, c)| (score(query, &app, c), app)).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, app)| app).collect()
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// How well the app's name (and, failing that, its description) matches the query.
fn closeness(query: &str, app: &AppSummary) -> f32 {
    let wanted = words(query);
    if wanted.is_empty() {
        return 0.0;
    }
    // Packaging suffixes are not part of what people call the app.
    let mut name = words(&app.name);
    while name.len() > 1 && matches!(name.last().map(String::as_str), Some("bin" | "git" | "appimage" | "desktop")) {
        name.pop();
    }
    let joined_name = name.join(" ");
    let joined_query = wanted.join(" ");
    let squashed = |text: &str| text.replace(' ', "");

    if joined_name == joined_query || squashed(&joined_name) == squashed(&joined_query) {
        return 1.0;
    }
    if joined_name.starts_with(&joined_query) || squashed(&joined_name).starts_with(&squashed(&joined_query)) {
        // "fire" for "firefox": closer the more of the name was typed.
        let typed =
            squashed(&joined_query).chars().count() as f32 / squashed(&joined_name).chars().count().max(1) as f32;
        return 0.80 + 0.15 * typed;
    }
    let in_name = |w: &String| name.iter().any(|n| n.starts_with(w.as_str()));
    let anywhere_in_name = |w: &String| joined_name.contains(w.as_str());
    let described = words(&format!("{} {}", app.summary, app.author));
    let in_description = |w: &String| described.iter().any(|d| d.starts_with(w.as_str()));

    let total = wanted.len() as f32;
    let name_starts = wanted.iter().filter(|w| in_name(w)).count() as f32;
    let name_contains = wanted.iter().filter(|w| anywhere_in_name(w)).count() as f32;
    let covered = wanted.iter().filter(|w| anywhere_in_name(w) || in_description(w)).count() as f32;

    if name_starts == total {
        0.75
    } else if name_contains == total {
        0.60
    } else if covered == total {
        // Every word found, some only in the description: it says it is about this.
        0.55 + 0.04 * (name_contains / total)
    } else {
        // The source matched on something we cannot see (a topic, the README).
        0.10 + 0.15 * (covered / total)
    }
}

/// Stars and votes on a 0..1 scale. Each source has its own sense of "a lot": a
/// thousand stars is modest on GitHub and remarkable on Codeberg.
fn popularity(app: &AppSummary) -> f32 {
    let counted = usage(app);
    match app.rating {
        // What people think of it. A handful of ratings is pulled towards the middle,
        // so three five-star votes do not outrank a well-loved app.
        Some(rating) => {
            let trust = rating.count as f32 / (rating.count as f32 + 30.0);
            let rated = UNKNOWN_POPULARITY * (1.0 - trust) + (rating.stars / 5.0).clamp(0.0, 1.0) * trust;
            // How many people bothered to rate it says how widely it is used, much as
            // stars do on GitHub: the same rating from 2,000 people outranks it from 40.
            let reach = ((rating.count as f32 + 1.0).ln() / 3_001.0_f32.ln()).clamp(0.0, 1.0);
            let rated = rated * (0.7 + 0.3 * reach);
            // With a count of users from the source as well, each tells half the story.
            if has_usage(app) {
                (rated + counted) / 2.0
            } else {
                rated
            }
        }
        None => counted,
    }
}

fn has_usage(app: &AppSummary) -> bool {
    app.popularity.is_some()
        && !matches!(
            app.id.source,
            SourceKind::Pacman | SourceKind::Apt | SourceKind::Dnf | SourceKind::Zypper | SourceKind::Web
        )
}

/// How many people use it, on the source's own scale.
fn usage(app: &AppSummary) -> f32 {
    let liked = liked(app);
    // Downloads say more about use than stars do (starring costs nothing), so where the
    // host counts them they carry half of this.
    match app.downloads {
        Some(n) => (liked + ((n as f32 + 1.0).ln() / 500_001.0_f32.ln()).clamp(0.0, 1.0)) / 2.0,
        None => liked,
    }
}

/// Stars, votes or installs, each on a scale that suits where it was counted.
fn liked(app: &AppSummary) -> f32 {
    let a_lot = match app.id.source {
        SourceKind::GitHub => 50_000.0_f32,
        SourceKind::Codeberg | SourceKind::GitLab => 3_000.0,
        // Installs in the last thirty days.
        SourceKind::Homebrew => 30_000.0,
        // Recommendations on Steam; ratings on the App Store.
        SourceKind::Steam => 100_000.0,
        SourceKind::MacAppStore => 5_000.0,
        // Installs in the last month.
        SourceKind::Flathub => 100_000.0,
        SourceKind::Aur => 1_500.0,
        // Distribution repositories publish no such number.
        SourceKind::Pacman
        | SourceKind::Apt
        | SourceKind::Dnf
        | SourceKind::Zypper
        | SourceKind::Web
        | SourceKind::Winget
        | SourceKind::Snap
        | SourceKind::FDroid => return UNKNOWN_POPULARITY,
    };
    match app.popularity {
        // Logarithmic: the step from 10 to 100 matters as much as from 1,000 to 10,000.
        Some(n) => ((n as f32 + 1.0).ln() / (a_lot + 1.0).ln()).clamp(0.0, 1.0),
        None => UNKNOWN_POPULARITY,
    }
}

/// Everything else that is known, as one 0..1 value starting from the middle.
fn nudges(app: &AppSummary, featured: bool) -> f32 {
    let mut value: f32 = 0.5;
    if featured {
        value += 0.3;
    }
    if app.installed_version.is_some() {
        value += 0.2;
    }
    // A distribution's own repository: maintained, signed and already trusted here.
    if matches!(app.id.source, SourceKind::Pacman | SourceKind::Apt | SourceKind::Dnf | SourceKind::Zypper) {
        value += 0.2;
    }
    // The source itself flags a problem (an AUR package marked out of date).
    if app.warning.is_some() {
        value -= 0.4;
    }
    // Its developer is confirmed by the source.
    if app.verified {
        value += 0.1;
    }
    // People are turning to it right now, where the source measures that.
    if let Some(momentum) = app.momentum {
        value += 0.2 * (momentum - 0.25);
    }
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppId;

    fn app(id: &str, name: &str, summary: &str, popularity: Option<u64>) -> AppSummary {
        AppSummary {
            id: AppId::parse(id).unwrap(),
            name: name.into(),
            author: "someone".into(),
            summary: summary.into(),
            popularity,
            homepage: String::new(),
            license: None,
            category: None,
            version: None,
            install_label: None,
            warning: None,
            origin: None,
            installed_version: None,
            rating: None,
            icon_url: None,
            downloads: None,
            verified: false,
            momentum: None,
            price: None,
            adult: false,
            restricted: false,
        }
    }

    fn order(query: &str, apps: Vec<AppSummary>) -> Vec<String> {
        let of = apps.len();
        let hits = apps
            .into_iter()
            .enumerate()
            .map(|(position, a)| (a, Context { position, of, featured: false, related: false }));
        sort(query, hits.collect()).into_iter().map(|a| a.id.to_string()).collect()
    }

    #[test]
    fn the_app_with_that_name_comes_before_apps_that_mention_it() {
        let got = order(
            "notes",
            vec![
                app("github:big/markdown-suite", "markdown-suite", "Editor with notes, tasks and more", Some(90_000)),
                app("github:plain/notes", "notes", "Plain notes", Some(800)),
                app("github:other/supernotes", "supernotes", "Notes, but super", Some(4_000)),
            ],
        );
        // The exact name is first. After it, the app tens of thousands use and that says
        // it does notes comes before a little-used one with the word inside its name.
        assert_eq!(got, ["github:plain/notes", "github:big/markdown-suite", "github:other/supernotes"]);
    }

    #[test]
    fn among_equally_close_names_the_one_more_people_use_comes_first() {
        let got = order(
            "player",
            vec![
                app("github:a/player", "player", "", Some(12)),
                app("github:b/player", "player", "", Some(40_000)),
                app("github:c/player", "player", "", Some(900)),
            ],
        );
        assert_eq!(got, ["github:b/player", "github:c/player", "github:a/player"]);
    }

    #[test]
    fn having_no_numbers_does_not_sink_a_result() {
        // A distribution package publishes no stars. It must not land below a barely
        // used project of the same name just because that one has a (small) number.
        let got = order(
            "harbor",
            vec![
                app("github:tiny/harbor", "harbor", "", Some(3)),
                app("apt:harbor", "harbor", "", None),
                app("github:huge/harbor", "harbor", "", Some(60_000)),
            ],
        );
        assert_eq!(got, ["github:huge/harbor", "apt:harbor", "github:tiny/harbor"]);
        // And a GitHub project whose count is missing sits mid-table, not last.
        let unknown = popularity(&app("github:x/y", "y", "", None));
        assert!(unknown > popularity(&app("github:x/y", "y", "", Some(50))));
        assert!(unknown < popularity(&app("github:x/y", "y", "", Some(20_000))));
    }

    #[test]
    fn a_good_rating_from_many_people_lifts_an_app_and_a_few_votes_barely_count() {
        use crate::model::Rating;
        let plain = app("apt:calc", "calc", "", None);
        let rated = |stars: f32, count: u32| {
            let mut a = plain.clone();
            a.rating = Some(Rating { stars, count, spread: [0; 5] });
            popularity(&a)
        };
        assert!(rated(4.7, 900) > popularity(&plain), "well loved beats unknown");
        assert!(rated(1.8, 900) < popularity(&plain), "widely disliked sinks below unknown");
        assert!(rated(4.7, 900) > rated(5.0, 5), "five perfect votes are not much evidence");
        assert!((rated(5.0, 5) - popularity(&plain)).abs() < 0.1);
        assert!(rated(4.5, 2_000) > rated(4.5, 40), "more people rating it is itself a sign of use");
        // With stars on GitHub as well, both count.
        let mut both = app("github:a/b", "b", "", Some(40_000));
        let stars_only = popularity(&both);
        both.rating = Some(Rating { stars: 2.0, count: 500, spread: [0; 5] });
        assert!(popularity(&both) < stars_only);
    }

    #[test]
    fn downloads_verification_and_recent_interest_all_count() {
        let base = app("github:a/tool", "tool", "", Some(2_000));
        let score_of = |a: &AppSummary| score("tool", a, Context::default());
        let mut downloaded = base.clone();
        downloaded.downloads = Some(300_000);
        assert!(score_of(&downloaded) > score_of(&base), "widely downloaded beats merely starred");
        let mut barely = base.clone();
        barely.downloads = Some(12);
        assert!(score_of(&barely) < score_of(&base), "starred but hardly ever downloaded is a weaker sign");
        let mut verified = base.clone();
        verified.verified = true;
        assert!(score_of(&verified) > score_of(&base));
        let (mut rising, mut fading) = (app("aur:tool", "tool", "", Some(300)), app("aur:tool", "tool", "", Some(300)));
        rising.momentum = Some(0.9);
        fading.momentum = Some(0.0);
        assert!(score_of(&rising) > score_of(&fading), "the same votes, but one is being voted for now");
    }

    #[test]
    fn each_source_is_measured_on_its_own_scale() {
        let codeberg = popularity(&app("codeberg:a/b", "b", "", Some(1_800)));
        let github = popularity(&app("github:a/b", "b", "", Some(1_800)));
        assert!(codeberg > github, "1,800 stars is a lot on Codeberg and modest on GitHub");
        assert!(popularity(&app("aur:b", "b", "", Some(400))) > 0.7);
        assert_eq!(popularity(&app("github:a/b", "b", "", Some(10_000_000))), 1.0);
    }

    #[test]
    fn closeness_understands_how_people_type_names() {
        let c = |query: &str, name: &str, summary: &str| closeness(query, &app("github:a/b", name, summary, None));
        assert_eq!(c("Lumen", "lumen", ""), 1.0, "case does not matter");
        assert_eq!(c("lumen", "lumen-bin", ""), 1.0, "packaging suffixes are not part of the name");
        assert_eq!(c("vs code", "vscode", ""), 1.0, "nor are spaces");
        assert!(c("fire", "firefox", "") > c("fire", "campfire", ""), "the start of a name beats its middle");
        assert!(c("firef", "firefox", "") > c("fi", "firefox", ""), "more of the name typed is closer");
        assert!(c("text editor", "lumen", "Fast text editor") > c("text editor", "lumen", "Fast"));
        assert!(c("media player", "media-player", "") > c("media player", "player", "for media"));
        assert!(c("zzz", "lumen", "Fast text editor") < 0.2);
        assert_eq!(c("  ", "lumen", ""), 0.0);
    }

    #[test]
    fn what_else_is_known_breaks_ties() {
        let plain = app("github:a/tool", "tool", "", Some(500));
        let base = score("tool", &plain, Context::default());
        assert!(score("tool", &plain, Context { featured: true, ..Default::default() }) > base);
        let mut installed = plain.clone();
        installed.installed_version = Some("1.0".into());
        assert!(score("tool", &installed, Context::default()) > base);
        let mut flagged = plain.clone();
        flagged.warning = Some("out of date".into());
        assert!(score("tool", &flagged, Context::default()) < base);
        // The source's own order decides only when nothing else does.
        let first = score("tool", &plain, Context { position: 0, of: 5, ..Default::default() });
        let last = score("tool", &plain, Context { position: 4, of: 5, ..Default::default() });
        assert!(first > last && first - last < 0.06);
    }
}

#[cfg(test)]
mod weight_tests {
    use super::*;
    use crate::model::{AppId, Rating};

    fn app(id: &str, name: &str, summary: &str) -> AppSummary {
        let mut app = AppSummary::new(AppId::parse(id).unwrap(), name);
        app.summary = summary.to_string();
        app
    }

    #[test]
    fn a_well_used_app_about_the_thing_outranks_obscure_ones_named_for_it() {
        // What Flathub says of VacuumTube (2026-11-21): installs a month, downloads, ratings.
        let mut vacuum = app("flathub:rocks.shy.VacuumTube", "VacuumTube", "YouTube for TV, on your PC");
        vacuum.popularity = Some(15_000);
        vacuum.downloads = Some(216_000);
        vacuum.rating = Some(Rating { stars: 4.2, count: 81, spread: [0; 5] });
        vacuum.verified = true;
        let mut obscure = app("github:someone/youtube-thing", "youtube-thing", "A script");
        obscure.popularity = Some(12);
        let package = app("apt:youtube-helper", "youtube-helper", "helper for video sites");
        let ranked = sort(
            "youtube",
            vec![(obscure, Context::default()), (package, Context::default()), (vacuum.clone(), Context::default())],
        );
        assert_eq!(ranked[0].name, "VacuumTube", "{:?}", ranked.iter().map(|a| a.name.as_str()).collect::<Vec<_>>());

        // The app whose name is exactly what was typed is still first, popular or not.
        let exact = app("github:someone/youtube", "youtube", "Unofficial client");
        let ranked = sort("youtube", vec![(vacuum.clone(), Context::default()), (exact, Context::default())]);
        assert_eq!(ranked[0].name, "youtube");

        // And an app on record for the service counts as a near match even with no
        // word of it in its name or description.
        let listed = app("flathub:io.freetubeapp.FreeTube", "FreeTube", "A private video player");
        let plain = score("youtube", &listed, Context::default());
        let related = score("youtube", &listed, Context { related: true, ..Default::default() });
        assert!(related > plain + 0.25, "{related} vs {plain}");
    }
}
