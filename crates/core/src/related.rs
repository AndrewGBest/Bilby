//! Apps that are different ways to use the same service.
//!
//! VacuumTube, FreeTube and NewPipe are not YouTube, but someone looking for YouTube is
//! looking for one of them, and which one depends on the device. No source records
//! that relationship, so it is kept by hand in `data/related.json`: a family per
//! service, its members in order of preference. The store shows a family as one app
//! and lets the person pick a version.
//!
//! Anything not made by the service's owner is marked unofficial wherever it is shown.

use crate::model::{AppId, SourceKind};
use crate::platform::{FormFactor, Os, Platform};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
pub struct Family {
    /// The service, as people call it: "YouTube".
    pub name: String,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Member {
    pub id: String,
    /// What this version is called: "VacuumTube".
    pub label: String,
    /// A few words on what kind of version it is.
    #[serde(default)]
    pub note: String,
    /// Made by the service's owner.
    #[serde(default)]
    pub official: bool,
    /// The systems it runs on (`linux`, `windows`, `macos`, `android`, `ios`). Empty
    /// for one that runs wherever its source is offered.
    #[serde(default)]
    pub os: Vec<String>,
    /// The kinds of device it is made for (`desktop`, `mobile`, `tv`). Empty for any.
    #[serde(default)]
    pub form: Vec<String>,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    families: Vec<Family>,
}

/// The bundled list. A broken file means no families, not a store that will not start.
pub fn families() -> &'static [Family] {
    static LIST: OnceLock<Vec<Family>> = OnceLock::new();
    LIST.get_or_init(|| parse(include_str!("../data/related.json")))
}

pub fn parse(json: &str) -> Vec<Family> {
    let mut families = serde_json::from_str::<File>(json).map(|f| f.families).unwrap_or_default();
    for family in &mut families {
        // Only ids the store could open are kept.
        family.members.retain(|m| AppId::parse(&m.id).is_some_and(|id| crate::model::valid_key(id.source, &id.key)));
    }
    families.retain(|f| !f.name.trim().is_empty() && !f.members.is_empty());
    families
}

/// The family an app belongs to, if any.
pub fn family_of<'a>(list: &'a [Family], id: &str) -> Option<&'a Family> {
    list.iter().find(|family| family.members.iter().any(|m| m.id.eq_ignore_ascii_case(id)))
}

/// A family whose name is what was searched for ("youtube", "You Tube").
pub fn family_named<'a>(list: &'a [Family], query: &str) -> Option<&'a Family> {
    let squash =
        |text: &str| -> String { text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect() };
    let query = squash(query);
    (query.len() >= 3).then(|| list.iter().find(|family| squash(&family.name) == query)).flatten()
}

impl Family {
    /// The members this device can use, in the family's order: those whose source
    /// applies here and that are made for this system and kind of device.
    pub fn here(&self, sources: &[SourceKind], platform: &Platform) -> Vec<&Member> {
        let os = match platform.os {
            Os::Linux => "linux",
            Os::Windows => "windows",
            Os::MacOs => "macos",
            Os::Android => "android",
            Os::Ios => "ios",
        };
        let form = match platform.form_factor {
            FormFactor::Desktop => "desktop",
            FormFactor::Mobile => "mobile",
            FormFactor::Tv => "tv",
        };
        let fits =
            |wanted: &[String], have: &str| wanted.is_empty() || wanted.iter().any(|w| w.eq_ignore_ascii_case(have));
        self.members
            .iter()
            .filter(|m| AppId::parse(&m.id).is_some_and(|id| sources.contains(&id.source)))
            .filter(|m| fits(&m.os, os) && fits(&m.form, form))
            .collect()
    }
}

impl Family {
    /// The other ways to use the service, for the page of the member `id`. Each app
    /// is listed once, however many sources have it, and never the one being shown.
    ///
    /// The service's own apps (its website, its own program) list everything else:
    /// their other forms first, then the apps others made for it. An app someone
    /// else made lists only the service's own, which is where the rest are reached
    /// from; where this device has none of those, it lists the others instead.
    pub fn others(&self, id: &str, sources: &[SourceKind], platform: &Platform) -> Vec<&Member> {
        let here = self.here(sources, platform);
        let shown = self.members.iter().find(|m| m.id.eq_ignore_ascii_case(id));
        let mut others: Vec<&Member> = Vec::new();
        for member in here {
            let same_app = shown.is_some_and(|s| s.label == member.label);
            if !same_app && !others.iter().any(|m| m.label == member.label) {
                others.push(member);
            }
        }
        if shown.is_some_and(|s| !s.official) && others.iter().any(|m| m.official) {
            others.retain(|m| m.official);
        }
        others
    }
}

impl Member {
    /// "VacuumTube · Flathub · unofficial": which version, from where, and whose.
    pub fn describe(&self) -> String {
        let source = AppId::parse(&self.id).map(|id| id.source.label()).unwrap_or("");
        let whose = if self.official { "" } else { "  ·  unofficial" };
        format!("{}  ·  {source}{whose}", self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_list_is_sound() {
        let list = families();
        let youtube = family_named(list, "You Tube").expect("YouTube is on the list");
        assert!(youtube.members.len() >= 5);
        assert!(youtube.members.iter().all(|m| !m.label.is_empty()));
        assert_eq!(youtube.members.iter().filter(|m| m.official).count(), 1, "only YouTube's own site is official");
        assert_eq!(family_of(list, "flathub:rocks.shy.vacuumtube").map(|f| f.name.as_str()), Some("YouTube"));
        assert!(family_of(list, "github:lumen-labs/lumen").is_none());
        assert!(family_named(list, "yo").is_none() && family_named(list, "tube").is_none());
    }

    #[test]
    fn a_services_own_app_lists_the_rest_and_another_makers_app_lists_the_services_own() {
        let list = families();
        let platform = Platform::preset("linux-tv").unwrap();
        let sources = [SourceKind::Web, SourceKind::Flathub, SourceKind::GitHub];
        let names = |family: &str, id: &str, sources: &[SourceKind]| -> Vec<String> {
            let family = family_named(list, family).unwrap();
            family.others(id, sources, &platform).iter().map(|m| format!("{} ({})", m.label, m.id)).collect()
        };
        // YouTube's own site: the apps others made, each once, from the first source that has it.
        assert_eq!(
            names("youtube", "web:youtube.com", &sources),
            ["VacuumTube (flathub:rocks.shy.VacuumTube)", "FreeTube (flathub:io.freetubeapp.FreeTube)"]
        );
        // One of those: YouTube's own site, and not the other players.
        assert_eq!(names("youtube", "github:shy1132/VacuumTube", &sources), ["YouTube website (web:youtube.com)"]);
        // A service with a program of its own as well as a site: each lists the other,
        // and the apps others made.
        assert_eq!(
            names("discord", "flathub:com.discordapp.Discord", &sources),
            ["Discord website (web:discord.com)", "Vesktop (flathub:dev.vencord.Vesktop)"]
        );
        assert_eq!(
            names("discord", "web:discord.com", &sources),
            ["Discord app (flathub:com.discordapp.Discord)", "Vesktop (flathub:dev.vencord.Vesktop)"]
        );
        // Where the service's own cannot be used here, the others are still reachable.
        assert_eq!(
            names("youtube", "flathub:rocks.shy.VacuumTube", &[SourceKind::Flathub]),
            ["FreeTube (flathub:io.freetubeapp.FreeTube)"]
        );
    }

    #[test]
    fn every_member_is_told_apart_from_its_service_and_the_others() {
        for family in families() {
            for member in &family.members {
                assert_ne!(member.label, family.name, "{}: a label says which way of using it this is", member.id);
                assert!(!member.note.is_empty(), "{}: a few words on what it is", member.id);
            }
        }
        // Plex's own apps for a computer, a TV and music are all reached from its website.
        let plex = family_named(families(), "plex").unwrap();
        let platform = Platform::preset("linux-tv").unwrap();
        let others: Vec<&str> = plex
            .others("web:plex.tv", &[SourceKind::Web, SourceKind::Flathub], &platform)
            .iter()
            .map(|m| m.label.as_str())
            .collect();
        assert_eq!(others, ["Plex for desktop", "Plex HTPC", "Plexamp"]);
    }

    #[test]
    fn a_family_offers_only_what_this_device_can_use() {
        let list = families();
        let youtube = family_named(list, "youtube").unwrap();
        let on = |preset: &str, sources: &[SourceKind]| -> Vec<&str> {
            let platform = Platform::preset(preset).unwrap();
            youtube.here(sources, &platform).iter().map(|m| m.id.as_str()).collect()
        };
        let windows = on("windows-x64", &[SourceKind::GitHub, SourceKind::Winget]);
        assert_eq!(windows, ["github:shy1132/VacuumTube", "github:FreeTubeApp/FreeTube"], "no Android apps on Windows");
        let linux_tv = on("linux-tv", &[SourceKind::Web, SourceKind::Flathub, SourceKind::GitHub]);
        assert_eq!(linux_tv[0], "web:youtube.com", "the service's own site comes first where it can be used");
        assert!(
            linux_tv.contains(&"flathub:rocks.shy.VacuumTube")
                && !linux_tv.iter().any(|id| id.contains("SmartTube") || id.contains("NewPipe"))
        );
        // A phone gets the phone app, not the one made for Android TVs.
        let phone = on("android-arm64", &[SourceKind::GitHub, SourceKind::FDroid]);
        assert_eq!(phone, ["fdroid:org.schabi.newpipe", "github:TeamNewPipe/NewPipe"]);
        assert_eq!(youtube.members[1].describe(), "VacuumTube  ·  Flathub  ·  unofficial");
    }

    #[test]
    fn a_broken_or_hostile_list_is_harmless() {
        assert!(parse("not json").is_empty());
        let odd = r#"{"families":[{"name":"X","members":[{"id":"github:../etc","label":"bad"},{"id":"nonsense","label":"bad"}]},
                                  {"name":"","members":[{"id":"github:a/b","label":"ok"}]}]}"#;
        assert!(parse(odd).is_empty(), "ids that could not be opened are dropped, and so are empty families");
    }
}
