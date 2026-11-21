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
    /// The app to start from on this device, and the apps that are its children. The
    /// parent is the service's own version when this device can use it (YouTube's
    /// website, in a browser), else the first that it can. Each child is listed once,
    /// however many sources have it, and a child is never listed under another child.
    pub fn parent_and_children(&self, sources: &[SourceKind], platform: &Platform) -> (Option<&Member>, Vec<&Member>) {
        let mut here: Vec<&Member> = Vec::new();
        for member in self.here(sources, platform) {
            if !here.iter().any(|m| m.label == member.label) {
                here.push(member);
            }
        }
        let parent = here.iter().copied().find(|m| m.official).or_else(|| here.first().copied());
        let children = here.into_iter().filter(|m| parent.is_none_or(|p| p.label != m.label)).collect();
        (parent, children)
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
    fn the_official_version_is_the_parent_and_the_rest_are_its_children() {
        let list = families();
        let youtube = family_named(list, "youtube").unwrap();
        let platform = Platform::preset("linux-tv").unwrap();
        let (parent, children) =
            youtube.parent_and_children(&[SourceKind::Web, SourceKind::Flathub, SourceKind::GitHub], &platform);
        assert_eq!(parent.map(|p| p.id.as_str()), Some("web:youtube.com"));
        let names: Vec<&str> = children.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(names, ["VacuumTube", "FreeTube"], "each child once, and none of them is the parent");
        assert_eq!(children[0].id, "flathub:rocks.shy.VacuumTube", "the source first in the list is the one kept");
        // Where the service's own site cannot be used, the first that can is the parent.
        let (parent, _) = youtube.parent_and_children(&[SourceKind::Flathub], &platform);
        assert_eq!(parent.map(|p| p.label.as_str()), Some("VacuumTube"));
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
