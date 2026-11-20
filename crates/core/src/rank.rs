//! Ordering search results by how likely each one is what the person meant.
//!
//! Every source ranks its own hits, but they cannot be compared across sources, and
//! "what GitHub thinks matches" is not the same as "what someone typing a name wants".
//! So every hit gets one score from three things:
//!
//! * **how close the name is to what was typed** (most of the score),
//! * **how many people use it** (stars, votes), on a scale that suits its source,
//! * **small nudges** from anything else that is known: hand-picked for the Discover
//!   page, already installed, where the source itself placed it, flagged out of date.
//!
//! A hit whose source publishes no numbers (a distribution's repository) gets a middle
//! value for the missing part rather than zero: no data is not bad data.

use crate::model::{AppSummary, SourceKind};

/// The share of the score each part can contribute. They sum to 1.
const CLOSENESS: f32 = 0.60;
const POPULARITY: f32 = 0.25;
const POSITION: f32 = 0.05;
const NUDGES: f32 = 0.10;

/// What a hit with no popularity figure is assumed to have: the middle.
const UNKNOWN_POPULARITY: f32 = 0.5;

/// Things known about a hit beyond the summary itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// Where its own source placed it (0 is first) among how many.
    pub position: usize,
    pub of: usize,
    /// It is on the hand-picked Discover list.
    pub featured: bool,
}

/// 0.0 (unlikely) to 1.0 (certainly what was meant).
pub fn score(query: &str, app: &AppSummary, context: Context) -> f32 {
    let position = if context.of > 1 { 1.0 - context.position as f32 / (context.of - 1) as f32 } else { 1.0 };
    CLOSENESS * closeness(query, app)
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
        // Every word found, some only in the description.
        0.30 + 0.25 * (name_contains / total)
    } else {
        // The source matched on something we cannot see (a topic, the README).
        0.10 + 0.15 * (covered / total)
    }
}

/// Stars and votes on a 0..1 scale. Each source has its own sense of "a lot": a
/// thousand stars is modest on GitHub and remarkable on Codeberg.
fn popularity(app: &AppSummary) -> f32 {
    let a_lot = match app.id.source {
        SourceKind::GitHub => 50_000.0_f32,
        SourceKind::Codeberg => 3_000.0,
        SourceKind::Aur => 1_500.0,
        // Distribution repositories publish no such number.
        SourceKind::Pacman | SourceKind::Apt => return UNKNOWN_POPULARITY,
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
    if matches!(app.id.source, SourceKind::Pacman | SourceKind::Apt) {
        value += 0.2;
    }
    // The source itself flags a problem (an AUR package marked out of date).
    if app.warning.is_some() {
        value -= 0.4;
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
        }
    }

    fn order(query: &str, apps: Vec<AppSummary>) -> Vec<String> {
        let of = apps.len();
        let hits = apps.into_iter().enumerate().map(|(position, a)| (a, Context { position, of, featured: false }));
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
        assert_eq!(got, ["github:plain/notes", "github:other/supernotes", "github:big/markdown-suite"]);
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
        let first = score("tool", &plain, Context { position: 0, of: 5, featured: false });
        let last = score("tool", &plain, Context { position: 4, of: 5, featured: false });
        assert!(first > last && first - last < 0.06);
    }
}
