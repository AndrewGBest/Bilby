//! Apps of the same kind, which someone looking for one would also want to be shown.
//!
//! A person who types "chrome" may be after Chromium, Brave or Ungoogled Chromium, and
//! none of those has "chrome" in its name, so no source's search brings them up. That
//! these belong together is a human judgement, kept by hand in `data/alike.json`. A
//! search that names one member of a group lists the others straight after it.
//!
//! This is not `related`, whose families are ways to use one service and are shown as
//! one app. These are different apps, each shown as itself.

use crate::model::{AppId, AppSummary, SourceKind};
use crate::platform::{FormFactor, Platform};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
pub struct Group {
    /// What the members have in common: "Browsers built on Chromium".
    pub name: String,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Member {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    /// The ids it goes by, best first. Left out in the file for an app that
    /// `essentials` already has, and filled in from there.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Other things a person types for it: "chrome" for Google Chrome.
    #[serde(default)]
    pub words: Vec<String>,
}

/// The bundled list. A broken file means no groups, not a store that will not start.
pub fn groups() -> &'static [Group] {
    static LIST: OnceLock<Vec<Group>> = OnceLock::new();
    LIST.get_or_init(|| parse(include_str!("../data/alike.json")))
}

pub fn parse(json: &str) -> Vec<Group> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        groups: Vec<Group>,
    }
    let mut groups = serde_json::from_str::<File>(json).map(|f| f.groups).unwrap_or_default();
    for group in &mut groups {
        for member in &mut group.members {
            if member.ids.is_empty() {
                if let Some(known) = crate::essentials::all().iter().find(|app| app.name == member.name) {
                    member.ids = known.ids.clone();
                    member.summary = known.summary.clone();
                }
            }
            // Only ids the store could open are kept.
            member.ids.retain(|id| AppId::parse(id).is_some_and(|id| crate::model::valid_key(id.source, &id.key)));
        }
        group.members.retain(|member| !member.name.trim().is_empty() && !member.ids.is_empty());
    }
    // One member alone has nothing to be shown beside.
    groups.retain(|group| group.members.len() > 1);
    groups
}

/// The group a search names a member of, and which member: the words typed are that
/// member's name or one of its other words, whatever the spacing and capitals.
pub fn named<'a>(list: &'a [Group], query: &str) -> Option<(&'a Group, usize)> {
    let squash =
        |text: &str| -> String { text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect() };
    let query = squash(query);
    if query.len() < 3 {
        return None;
    }
    list.iter().find_map(|group| {
        let at = group
            .members
            .iter()
            .position(|m| squash(&m.name) == query || m.words.iter().any(|word| squash(word) == query))?;
        Some((group, at))
    })
}

impl Member {
    /// Is this app the member, at any of its sources?
    pub fn is(&self, id: &AppId) -> bool {
        let text = id.to_string();
        self.ids.iter().any(|known| known.eq_ignore_ascii_case(&text))
    }

    /// The version of it this device can use: the first id whose source applies here.
    pub fn here(&self, sources: &[SourceKind], platform: &Platform) -> Option<AppSummary> {
        // As with the essentials: a TV's build often goes by another name on Google
        // Play than the phone's, which is the one the list has.
        let on_tv = platform.form_factor == FormFactor::Tv;
        let id = self
            .ids
            .iter()
            .filter_map(|id| AppId::parse(id))
            .filter(|id| !(on_tv && id.source == SourceKind::GooglePlay))
            .find(|id| sources.contains(&id.source))?;
        let mut app = AppSummary::new(id, self.name.clone());
        app.summary = self.summary.clone();
        Some(app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_list_is_whole() {
        let raw: serde_json::Value = serde_json::from_str(include_str!("../data/alike.json")).unwrap();
        let written: usize =
            raw["groups"].as_array().unwrap().iter().map(|group| group["members"].as_array().unwrap().len()).sum();
        let kept: usize = groups().iter().map(|group| group.members.len()).sum();
        assert_eq!(kept, written, "a member has no usable id, or names an app the essentials do not have");
        for group in groups() {
            for member in &group.members {
                assert!(!member.summary.is_empty(), "{} says nothing about itself", member.name);
            }
        }
    }

    #[test]
    fn a_search_names_a_member_by_its_name_or_another_word_for_it() {
        let found = |query: &str| named(groups(), query).map(|(group, at)| group.members[at].name.as_str());
        assert_eq!(found("chrome"), Some("Google Chrome"));
        assert_eq!(found("Google  Chrome"), Some("Google Chrome"));
        assert_eq!(found("ungoogled-chromium"), Some("Ungoogled Chromium"));
        assert_eq!(found("librewolf"), Some("LibreWolf"));
        assert_eq!(found("google"), None, "part of a name is not the name");
        assert_eq!(found("browser"), None);
        assert_eq!(found("br"), None);
    }

    #[test]
    fn a_member_is_offered_from_the_first_source_this_device_has() {
        let (group, _) = named(groups(), "chromium").unwrap();
        let ungoogled = group.members.iter().find(|m| m.name == "Ungoogled Chromium").unwrap();
        let linux = Platform::preset("linux-x64-arch").unwrap();
        let app = ungoogled.here(&[SourceKind::Aur, SourceKind::Flathub], &linux).unwrap();
        assert_eq!(app.id.to_string(), "flathub:io.github.ungoogled_software.ungoogled_chromium");
        assert_eq!(ungoogled.here(&[SourceKind::Aur], &linux).unwrap().id.source, SourceKind::Aur);
        assert!(
            ungoogled.here(&[SourceKind::FDroid], &linux).is_none(),
            "nothing for a device with none of its sources"
        );
        assert!(ungoogled.is(&AppId::parse("aur:Ungoogled-Chromium-Bin").unwrap()), "whatever the capitals");
    }
}
