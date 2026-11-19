//! Core of untitledstore: everything that is not UI.
//!
//! * [`platform`] – what machine are we running on?
//! * [`asset`]    – what is a release file (`foo-x86_64.AppImage`) built for?
//! * [`compat`]   – does that file work on this machine, and how well?
//! * [`source`]   – where apps come from (GitHub Releases, the AUR, ...)
//! * [`index`]    – the curated list behind the Discover page
//! * [`catalog`]  – glue: browse / search / details, filtered to this machine
//! * [`install`]  – what installing a given file means on this machine
//! * [`demo`]     – fictional offline catalog for demos and tests

pub mod asset;
pub mod catalog;
pub mod compat;
pub mod demo;
pub mod error;
pub mod http;
pub mod index;
pub mod install;
pub mod model;
pub mod platform;
pub mod source;

pub use catalog::Catalog;
pub use error::{Error, Result};
pub use platform::Platform;
