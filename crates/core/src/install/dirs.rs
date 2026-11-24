//! The folders and programs an install uses on this device, in one value, so that
//! tests can point every one of them somewhere harmless.

use super::launch::find_terminal;
use std::path::{Path, PathBuf};

/// Where the store puts things.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub downloads: PathBuf,
    /// Per-user folder for AppImages.
    pub applications: PathBuf,
    /// Where the desktop looks for app-menu entries (`~/.local/share/applications`).
    pub menu: PathBuf,
    /// The Flatpak program (`flatpak`, found on the PATH).
    pub flatpak: PathBuf,
    /// The program that asks for the password and runs a command as administrator
    /// (`pkexec`, from polkit).
    pub elevate: PathBuf,
    /// A terminal to open for installs that need one, and how it takes a command to
    /// run. `None` when none was found.
    pub terminal: Option<(PathBuf, &'static str)>,
    /// Flatpak's installations: system-wide and per-user. Installed apps are in `app/<id>`.
    pub flatpak_roots: Vec<PathBuf>,
    /// The folders programs are looked for in (the PATH), for finding a browser that
    /// can show a website in a window of its own.
    pub programs: Vec<PathBuf>,
    pub cache: PathBuf,
    /// Settings and library (`state.json`).
    pub config: PathBuf,
    /// The person's desktop folder, where a web app also gets an icon. `None` where
    /// there is none to be found.
    pub desktop: Option<PathBuf>,
}

impl Dirs {
    pub fn for_current_user() -> Dirs {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty());
        let home = var("HOME").or_else(|| var("USERPROFILE")).unwrap_or_else(std::env::temp_dir);
        let cache = if cfg!(target_os = "windows") {
            var("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local"))
        } else if cfg!(target_os = "macos") {
            home.join("Library").join("Caches")
        } else {
            var("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"))
        };
        let config = if cfg!(target_os = "windows") {
            var("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"))
        } else if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support")
        } else {
            var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"))
        };
        Dirs {
            downloads: var("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home.join("Downloads")).join("bilby"),
            applications: home.join("Applications"),
            menu: var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share")).join("applications"),
            flatpak: PathBuf::from("flatpak"),
            elevate: PathBuf::from("pkexec"),
            terminal: find_terminal(),
            flatpak_roots: vec![
                PathBuf::from("/var/lib/flatpak"),
                var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share")).join("flatpak"),
            ],
            programs: std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default(),
            cache: cache.join("bilby"),
            desktop: desktop_folder(&home, &config),
            config: config.join("bilby"),
        }
    }
}

/// The desktop folder: on Linux the one the desktop itself names (it may be called
/// something else in another language, or be turned off), else `Desktop` in the home
/// folder where that exists.
pub(super) fn desktop_folder(home: &Path, config: &Path) -> Option<PathBuf> {
    let named =
        std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|text| desktop_in_user_dirs(&text, home));
    // Turned off is written as the home folder itself.
    named.or_else(|| Some(home.join("Desktop"))).filter(|folder| folder.is_dir() && folder != home)
}

/// What `user-dirs.dirs` says the desktop folder is.
pub(super) fn desktop_in_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().find_map(|line| line.trim().strip_prefix("XDG_DESKTOP_DIR="))?;
    let value = value.trim().trim_matches('"');
    match value.strip_prefix("$HOME") {
        Some(rest) => Some(home.join(rest.trim_start_matches('/'))),
        None => value.starts_with('/').then(|| PathBuf::from(value)),
    }
}
