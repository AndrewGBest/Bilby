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
    /// applies here.
    pub fn here(&self, sources: &[SourceKind]) -> Vec<&Member> {
        self.members.iter().filter(|m| AppId::parse(&m.id).is_some_and(|id| sources.contains(&id.source))).collect()
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
    fn a_family_offers_only_what_this_device_can_use() {
        let list = families();
        let youtube = family_named(list, "youtube").unwrap();
        let ids =
            |sources: &[SourceKind]| -> Vec<&str> { youtube.here(sources).iter().map(|m| m.id.as_str()).collect() };
        let windows = ids(&[SourceKind::GitHub, SourceKind::Winget]);
        assert!(windows.iter().all(|id| id.starts_with("github:")) && !windows.is_empty());
        let linux_tv = ids(&[SourceKind::Web, SourceKind::Flathub, SourceKind::GitHub]);
        assert_eq!(linux_tv[0], "web:youtube.com", "the service's own site comes first where it can be used");
        assert!(linux_tv.contains(&"flathub:rocks.shy.VacuumTube"));
        assert!(ids(&[SourceKind::FDroid]).contains(&"fdroid:org.schabi.newpipe"));
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
