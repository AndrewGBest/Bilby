//! Where apps come from. Adding a store (GitLab, Codeberg/Forgejo, F-Droid, Flathub,
//! winget, ...) means implementing [`Source`]; nothing else in the app changes.

pub mod appstore;
pub mod aur;
pub mod fdroid;
pub mod flathub;
pub mod forgejo;
pub mod github;
pub mod gitlab;
pub mod homebrew;
pub mod snap;
pub mod steam;
pub mod system;
pub mod web;
pub mod winget;

use crate::error::Result;
use crate::model::{AppSummary, Release, SourceKind};
use crate::platform::Platform;

pub trait Source: Send + Sync {
    fn kind(&self) -> SourceKind;

    /// Is this source relevant on that machine at all? (The AUR only on Arch.)
    fn available_on(&self, platform: &Platform) -> bool;

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>>;

    fn summary(&self, key: &str) -> Result<AppSummary>;

    /// Newest first.
    fn releases(&self, key: &str) -> Result<Vec<Release>>;

    /// When every search hit is installable wherever the source is available, and
    /// already carries its version, return what would be installed (e.g. "AUR
    /// package"). The catalog then skips the per-hit release lookup: one request per
    /// search instead of one per result.
    fn search_hits_are_final(&self) -> Option<&'static str> {
        None
    }

    /// The project's own description of itself (its README), as Markdown, when the
    /// source has one. Costs a request, so it is asked for separately from details.
    fn about(&self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Addresses of screenshots the source itself publishes for the app.
    fn pictures(&self, _key: &str) -> Vec<String> {
        Vec::new()
    }

    /// For sources made of several named places (repositories): their names.
    /// Ready-made lists the source keeps (most popular, trending, ...), each with a title.
    /// Fetched over the network; a source without any returns nothing.
    fn collections(&self, _limit: usize) -> Vec<(String, Vec<AppSummary>)> {
        Vec::new()
    }

    fn origins(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Add up per-file download counts. `None` when no file has one, which is "not
/// counted here" and must not be read as "never downloaded".
pub(crate) fn counted(counts: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    let known: Vec<u64> = counts.flatten().collect();
    (!known.is_empty()).then(|| known.iter().sum())
}
