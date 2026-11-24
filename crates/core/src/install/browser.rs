//! The browsers a web app can open in: Longboard on a TV, and on a computer one that
//! can give a site a window of its own.

use super::dirs::Dirs;
use super::flatpak::flatpak_app_installed;
use crate::model::{AppId, SourceKind};
use crate::platform::Platform;
use std::path::{Path, PathBuf};

/// The name Longboard's Flatpak goes by.
pub const LONGBOARD_FLATPAK: &str = "io.github.andrewgbest.Longboard";

/// How Longboard is started on this device: the program, and what goes before
/// anything it is told (nothing for a file of its own; "run" and its name for Flatpak).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Longboard {
    pub program: PathBuf,
    pub before: Vec<String>,
}

impl Longboard {
    /// The whole argument list for starting it with `args`.
    pub fn args(&self, args: &[String]) -> Vec<String> {
        self.before.iter().chain(args).cloned().collect()
    }

    /// The same, when it has to write into `folder`: a Flatpak is shut off from the
    /// person's files unless a folder is opened to it for this one run.
    pub fn args_writing(&self, args: &[String], folder: &Path) -> Vec<String> {
        let mut all = self.args(args);
        if self.before.first().is_some_and(|first| first == "run") {
            all.insert(1, format!("--filesystem={}", folder.display()));
        }
        all
    }
}

/// A browser that can show one website in a window of its own, with no tabs or
/// address bar: what makes a website on a computer feel like an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBrowser {
    pub program: PathBuf,
    /// What comes before the address (`run <id>` for a Flatpak).
    pub before: Vec<String>,
    /// What the browser's windows of this kind are called, by which the desktop ties
    /// a window to its menu entry and so to its icon.
    family: &'static str,
}

impl AppBrowser {
    /// The whole argument list for opening `url` as an app.
    pub fn args(&self, url: &str) -> Vec<String> {
        self.before.iter().cloned().chain([format!("--app={url}")]).collect()
    }

    /// The name the browser gives the window it opens for `url`.
    pub fn window_class(&self, url: &str) -> String {
        let rest = url.trim_start_matches("https://");
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        format!("{}-{host}__{}-Default", self.family, path.replace('/', "_"))
    }
}

/// The browser links open in, as the desktop names it (`firefox`, `brave-browser`,
/// `com.google.chrome`): the id of its menu entry, in small letters and without
/// `.desktop`. Read from the lists the desktop keeps of which app opens what, the
/// person's own before the system's.
pub fn default_browser(dirs: &Dirs) -> Option<String> {
    let mut lists: Vec<PathBuf> = Vec::new();
    if let Some(config) = dirs.config.parent() {
        // A desktop's own list comes before the general one.
        let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
        lists.extend(desktops.split(':').filter(|d| !d.is_empty()).map(|d| config.join(format!("{d}-mimeapps.list"))));
        lists.push(config.join("mimeapps.list"));
    }
    lists.push(dirs.menu.join("mimeapps.list"));
    lists.push(PathBuf::from("/etc/xdg/mimeapps.list"));
    lists.push(PathBuf::from("/usr/share/applications/mimeapps.list"));
    lists.iter().filter_map(|list| std::fs::read_to_string(list).ok()).find_map(|text| browser_in_list(&text))
}

/// What a "which app opens what" list says opens web links.
pub(super) fn browser_in_list(text: &str) -> Option<String> {
    let mut in_defaults = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_defaults = line == "[Default Applications]"; // a file format's heading (not shown)
            continue;
        }
        let Some((kind, apps)) = line.split_once('=').filter(|_| in_defaults) else { continue };
        if !matches!(kind.trim(), "x-scheme-handler/https" | "x-scheme-handler/http") {
            continue;
        }
        // Several may be listed; the first is the one used.
        let first = apps.split(';').map(str::trim).find(|app| !app.is_empty())?;
        let id = first.strip_suffix(".desktop").unwrap_or(first).to_lowercase();
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
        return (!id.is_empty() && id.chars().all(plain)).then_some(id);
    }
    None
}

/// Such a browser, when this device has one: Chrome, Chromium, Brave, Edge or Vivaldi,
/// as a program on the PATH or as a Flatpak. (Firefox cannot do this.) The browser the
/// person's links already open in is the one used when it can; otherwise another that
/// is installed.
pub fn find_app_browser(dirs: &Dirs) -> Option<AppBrowser> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    const PROGRAMS: [(&str, &str); 10] = [
        ("google-chrome-stable", "chrome"),
        ("google-chrome", "chrome"),
        ("chromium", "chrome"),
        ("chromium-browser", "chrome"),
        ("brave-browser", "brave"),
        ("brave", "brave"),
        ("microsoft-edge-stable", "msedge"),
        ("microsoft-edge", "msedge"),
        ("vivaldi-stable", "vivaldi"),
        ("vivaldi", "vivaldi"),
    ];
    const FLATPAKS: [(&str, &str); 6] = [
        ("com.google.Chrome", "chrome"),
        ("org.chromium.Chromium", "chrome"),
        ("com.brave.Browser", "brave"),
        ("com.microsoft.Edge", "msedge"),
        ("com.vivaldi.Vivaldi", "vivaldi"),
        ("io.github.ungoogled_software.ungoogled_chromium", "chrome"),
    ];
    // Every one that is here, each with the name the desktop would know it by.
    let mut here: Vec<(String, AppBrowser)> = Vec::new();
    for (name, family) in PROGRAMS {
        if let Some(program) = dirs.programs.iter().map(|dir| dir.join(name)).find(|p| p.is_file()) {
            here.push((name.to_string(), AppBrowser { program, before: Vec::new(), family }));
        }
    }
    for (id, family) in FLATPAKS {
        if flatpak_app_installed(dirs, id) {
            let browser =
                AppBrowser { program: dirs.flatpak.clone(), before: vec!["run".to_string(), id.to_string()], family };
            here.push((id.to_lowercase(), browser));
        }
    }
    // The usual browser first, when it is one of them. A program and its menu entry
    // do not always share a whole name (`google-chrome-stable`, `google-chrome`).
    let usual = default_browser(dirs);
    let is_usual = |name: &str| {
        usual.as_deref().is_some_and(|usual| usual == name || usual.starts_with(name) || name.starts_with(usual))
    };
    let chosen = here.iter().position(|(name, _)| is_usual(name)).unwrap_or(0);
    (!here.is_empty()).then(|| here.swap_remove(chosen).1)
}

/// A browser that can show a website in a window of its own, to install on a device
/// that has none: Chromium, from wherever this device gets its apps.
pub fn app_browser_to_install(platform: &Platform, sources: &[SourceKind]) -> Option<AppId> {
    use crate::platform::{LinuxFamily, Os};
    if platform.os != Os::Linux {
        return None;
    }
    let has = |kind: SourceKind| sources.contains(&kind);
    let suse = platform.linux.as_ref().is_some_and(|l| l.family == LinuxFamily::Suse);
    Some(if has(SourceKind::Pacman) {
        AppId::new(SourceKind::Pacman, "chromium")
    } else if has(SourceKind::Flathub) {
        AppId::new(SourceKind::Flathub, "org.chromium.Chromium")
    } else if has(SourceKind::Dnf) && !suse {
        AppId::new(SourceKind::Dnf, "chromium")
    } else if has(SourceKind::Zypper) {
        AppId::new(SourceKind::Zypper, "chromium")
    } else if has(SourceKind::Apt) {
        AppId::new(SourceKind::Apt, "chromium")
    } else {
        return None;
    })
}

/// Longboard, when it is on this device, however it got there: the AppImage the store
/// placed (or the folder it was unpacked into), its Flatpak, or one on the PATH.
pub fn find_longboard(dirs: &Dirs) -> Option<Longboard> {
    let own = |program: PathBuf| Longboard { program, before: Vec::new() };
    let placed = std::fs::read_dir(&dirs.applications)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            if !name.starts_with("longboard") {
                return None;
            }
            if name.ends_with(".appimage") && path.is_file() {
                return Some(path);
            }
            // Unpacked, on a system where AppImages cannot start as they are.
            let unpacked = path.join("AppRun");
            (name.ends_with(".appdir") && unpacked.is_file()).then_some(unpacked)
        })
        .next();
    if let Some(program) = placed {
        return Some(own(program));
    }
    if flatpak_app_installed(dirs, LONGBOARD_FLATPAK) {
        return Some(Longboard {
            program: dirs.flatpak.clone(),
            before: vec!["run".to_string(), LONGBOARD_FLATPAK.to_string()],
        });
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join("longboard")).find(|p| p.is_file()).map(own)
}
