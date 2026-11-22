//! One app, several sources.
//!
//! The same program is often in a distribution's repository, on Flathub and on its
//! own release page. Shown as three results it looks like three apps. This groups
//! what a search found so each app appears once, with its sources to choose between.
//!
//! No source says "this is the same app as that one over there", so it is judged from
//! names, cautiously: two entries are the same app when their names agree and nothing
//! suggests otherwise. Two unrelated people's projects that happen to share a name are
//! the case to avoid, so code hosts are held to a stricter test than curated catalogs.

use crate::model::{AppSummary, SourceKind};

/// An app as one result: the entry to show, and the same app from other sources.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub lead: AppSummary,
    pub others: Vec<AppSummary>,
}

impl Group {
    /// Every entry, the one shown first.
    pub fn members(&self) -> impl Iterator<Item = &AppSummary> {
        std::iter::once(&self.lead).chain(self.others.iter())
    }
}

/// Letters and digits only, lower-cased: "VLC media player" and "vlc" differ, but
/// "Free-Tube" and "freetube" do not.
fn squash(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The names an app goes by: what it is called, and the last part of its id
/// (`org.videolan.VLC`, `VideoLAN.VLC` and `videolan/vlc` all end in "vlc").
fn names(app: &AppSummary) -> Vec<String> {
    let tail = app.id.key.rsplit(['/', '.']).next().unwrap_or(&app.id.key);
    let mut names = vec![squash(&app.name), squash(tail)];
    names.retain(|n| n.chars().count() >= 3);
    names.dedup();
    names
}

/// Who a code-host project belongs to: the part of its id before the name.
fn owner(app: &AppSummary) -> String {
    squash(app.id.key.rsplit_once('/').map(|(owner, _)| owner).unwrap_or(""))
}

/// Could these two entries be one app from two sources?
pub fn same_app(a: &AppSummary, b: &AppSummary) -> bool {
    // A source lists each app once; two entries from it are two apps.
    if a.id.source == b.id.source {
        return false;
    }
    // A website is not a program: Plex's site and Plex's desktop app are two things to
    // choose between, not one app from two places.
    if a.id.source == SourceKind::Web || b.id.source == SourceKind::Web {
        return false;
    }
    let (names_a, names_b) = (names(a), names(b));
    let Some(shared) = names_a.iter().find(|n| names_b.contains(n)) else { return false };
    match (a.id.source.is_forge(), b.id.source.is_forge()) {
        // Catalogs are curated: one app per name. Agreement on the name is enough.
        (false, false) => true,
        // Two code hosts: the same owner's project, or one says it mirrors the other.
        (true, true) => {
            let mirror = |app: &AppSummary| app.summary.to_lowercase().contains("mirror");
            owner(a) == owner(b) || mirror(a) || mirror(b)
        }
        // A project and a catalog entry: anyone can name a repository "vlc". It counts
        // when the owner's name is part of the story, or the project is plainly the
        // well-known one.
        _ => {
            let forge = if a.id.source.is_forge() { a } else { b };
            let owner = owner(forge);
            let related = owner.len() >= 3 && (owner.contains(shared.as_str()) || shared.contains(owner.as_str()));
            related || forge.popularity.unwrap_or(0) >= 500
        }
    }
}

/// Group results, keeping their order: each group stands where its best-placed entry
/// stood, and that entry leads it.
pub fn group(apps: Vec<AppSummary>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for app in apps {
        // The same app must agree with everything already in the group, so a chain of
        // near-misses cannot pull unrelated things together.
        let home =
            groups.iter_mut().find(|g| g.members().all(|m| m.id.source != app.id.source) && same_app(&g.lead, &app));
        match home {
            Some(group) => group.others.push(app),
            None => groups.push(Group { lead: app, others: Vec::new() }),
        }
    }
    groups
}

/// How to say where an entry comes from when choosing between sources.
pub fn source_label(app: &AppSummary) -> String {
    match app.id.source {
        // A repository package says which repository.
        SourceKind::Pacman | SourceKind::Apt | SourceKind::Dnf | SourceKind::Zypper => {
            app.origin.clone().unwrap_or_else(|| app.id.source.label().to_string())
        }
        other => other.label().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppId;

    fn app(id: &str, name: &str) -> AppSummary {
        AppSummary::new(AppId::parse(id).unwrap(), name)
    }

    #[test]
    fn one_app_from_several_catalogs_is_one_result() {
        let found = vec![
            app("apt:vlc", "vlc"),
            app("flathub:org.videolan.VLC", "VLC"),
            app("snap:vlc", "VLC"),
            app("winget:VideoLAN.VLC", "VLC media player"),
            app("flathub:org.videolan.VLC.Plugin.bdj", "BDJ plugin"),
            app("apt:vlc-plugin-qt", "vlc-plugin-qt"),
        ];
        let groups = group(found);
        assert_eq!(groups.len(), 3, "the plugins are their own things");
        assert_eq!(groups[0].lead.id.to_string(), "apt:vlc", "the best-placed entry leads");
        let others: Vec<String> = groups[0].others.iter().map(|a| a.id.to_string()).collect();
        assert_eq!(others, ["flathub:org.videolan.VLC", "snap:vlc", "winget:VideoLAN.VLC"]);
    }

    #[test]
    fn a_website_is_never_one_of_a_programs_sources() {
        let site = app("web:plex.tv", "Plex");
        let program = app("flathub:tv.plex.PlexDesktop", "Plex");
        assert!(!same_app(&site, &program) && !same_app(&program, &site));
        assert_eq!(group(vec![site, program]).len(), 2, "two things to choose between, both listed");
    }

    #[test]
    fn a_repository_that_merely_shares_a_name_is_not_merged() {
        let package = app("apt:notes", "notes");
        let stranger = app("github:someone/notes", "notes");
        assert!(!same_app(&package, &stranger), "anyone can call a repository 'notes'");
        // The owner's name ties it to the app, or it is plainly the well-known project.
        let own = app("github:notes-app/notes", "notes");
        assert!(same_app(&package, &own));
        let mut famous = app("github:someone/notes", "notes");
        famous.popularity = Some(12_000);
        assert!(same_app(&package, &famous));

        // Two code hosts: the same owner, or a stated mirror.
        let gitlab = app("gitlab:veloren/veloren", "veloren");
        let mut mirror = app("github:veloren/veloren", "veloren");
        assert!(same_app(&gitlab, &mirror));
        mirror = app("github:fan/veloren", "veloren");
        assert!(!same_app(&gitlab, &mirror));
        mirror.summary = "[mirror of https://gitlab.com/veloren/veloren]".into();
        assert!(same_app(&gitlab, &mirror));
    }

    #[test]
    fn short_and_different_names_stay_apart() {
        assert!(!same_app(&app("apt:fd", "fd"), &app("flathub:org.example.fd", "fd")), "two letters say too little");
        assert!(!same_app(&app("apt:fd-find", "fd-find"), &app("brew:fd", "fd")));
        assert!(!same_app(
            &app("flathub:net.veloren.veloren", "Veloren"),
            &app("flathub:net.veloren.airshipper", "Airshipper")
        ));
        // A group takes one entry per source.
        let groups = group(vec![app("apt:vlc", "vlc"), app("snap:vlc", "VLC"), app("snap:vlc-other", "VLC")]);
        assert_eq!(groups.len(), 2);
    }
}
