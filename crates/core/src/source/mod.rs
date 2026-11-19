//! Where apps come from. Adding a store (GitLab, Codeberg/Forgejo, F-Droid, Flathub,
//! winget, ...) means implementing [`Source`]; nothing else in the app changes.

pub mod aur;
pub mod github;

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
}
