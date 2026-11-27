//! Core of Bilby: everything that is not UI.
//!
//! * [`platform`] – what machine are we running on?
//! * [`asset`]    – what is a release file (`foo-x86_64.AppImage`) built for?
//! * [`compat`]   – does that file work on this machine, and how well?
//! * [`source`]   – where apps come from (GitHub Releases, the AUR, ...)
//! * [`index`]    – the starter list bundled for the Discover page
//! * [`catalog`]  – glue: browse / search / details, filtered to this machine
//! * [`categories`] – the kinds of app there are: video, music, games, office, ...
//! * [`icons`]    – app pictures, fetched from the source and cached
//! * [`install`]  – what installing a given file means on this machine
//! * [`reviewing`] – writing a review, and the places that take one
//! * [`region`]   – which country's prices to ask a store for
//! * [`os`]       – the operating system: what it is, and its own updates
//! * [`state`]    – settings, library and history, stored locally
//! * [`winrun`]   – Windows programs on Linux, set up to run through Proton
//! * [`version`]  – is this release newer than the installed one?
//! * [`demo`]     – fictional offline catalog for demos and tests

pub mod alike;
pub mod appstream;
pub mod asset;
pub mod catalog;
pub mod categories;
pub mod compat;
pub mod demo;
pub mod error;
pub mod essentials;
pub mod github_login;
pub mod gog_account;
pub mod http;
pub mod icons;
pub mod index;
pub mod install;
pub mod merge;
pub mod model;
pub mod needs;
pub mod os;
pub mod parental;
pub mod platform;
pub mod rank;
pub mod ratings;
pub mod region;
pub mod related;
pub mod reviewing;
pub mod screenshots;
pub mod source;
pub mod state;
pub mod steam_account;
pub mod steam_qr;
pub mod version;
pub mod winrun;

pub use catalog::Catalog;
pub use error::{Error, Result};
pub use platform::Platform;
