//! Core of Bilby: everything that is not UI.
//!
//! * [`platform`] – what machine are we running on?
//! * [`asset`]    – what is a release file (`foo-x86_64.AppImage`) built for?
//! * [`compat`]   – does that file work on this machine, and how well?
//! * [`source`]   – where apps come from (GitHub Releases, the AUR, ...)
//! * [`index`]    – the starter list bundled for the Discover page
//! * [`catalog`]  – glue: browse / search / details, filtered to this machine
//! * [`icons`]    – app pictures, fetched from the source and cached
//! * [`install`]  – what installing a given file means on this machine
//! * [`os`]       – the operating system: what it is, and its own updates
//! * [`state`]    – settings, library and history, stored locally
//! * [`version`]  – is this release newer than the installed one?
//! * [`demo`]     – fictional offline catalog for demos and tests

pub mod appstream;
pub mod asset;
pub mod catalog;
pub mod compat;
pub mod demo;
pub mod error;
pub mod github_login;
pub mod http;
pub mod icons;
pub mod index;
pub mod install;
pub mod model;
pub mod os;
pub mod platform;
pub mod rank;
pub mod ratings;
pub mod screenshots;
pub mod source;
pub mod state;
pub mod version;

pub use catalog::Catalog;
pub use error::{Error, Result};
pub use platform::Platform;
