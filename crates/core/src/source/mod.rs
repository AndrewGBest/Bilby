//! Where apps come from. Adding a store (GitLab, Codeberg/Forgejo, F-Droid, Flathub,
//! winget, ...) means implementing [`Source`]; nothing else in the app changes.

pub mod aur;
pub mod forgejo;
pub mod github;
pub mod system;

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

    /// For sources made of several named places (repositories): their names.
    fn origins(&self) -> Vec<String> {
        Vec::new()
    }
}
