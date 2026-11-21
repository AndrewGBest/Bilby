//! What "install" means for a given file on a given machine.
//!
//! [`plan`] is pure: it describes the steps, so the UI can show them before anything is
//! downloaded. [`execute`] carries the plan out.
//!
//! The store deliberately does not run anything as root itself. Native packages are
//! handed to the system's own installer (which asks for the password); AUR builds are
//! shown as a command for the user to run after reading the PKGBUILD.

use crate::asset::{AssetInfo, Format};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::Asset;
use crate::platform::{LinuxFamily, Platform};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open the downloaded file with the system's handler (installer, package manager UI).
    OpenFile,
    /// Make executable and move to the per-user applications folder.
    PlaceAppImage,
    /// Nothing to run: show the file in the downloads folder.
    Reveal,
    /// Ask Android's package installer to install the APK.
    AndroidInstaller,
    /// Nothing is downloaded by the store; the user runs this command. Only a last
    /// resort now: when there is no way to run it for them.
    ShowCommand,
    /// The store runs the plan's command itself. One that starts with `sudo` is run
    /// through the desktop's own password prompt instead.
    RunCommand,
    /// The store opens a terminal on the plan's command, for installs that ask
    /// questions as they go (building from the AUR).
    RunInTerminal,
    /// Put a website in the app list, to open in Longboard.
    WebApp,
    /// Have Flatpak install the app from Flathub.
    Flatpak,
    /// Have Flatpak install a downloaded `.flatpak` bundle.
    FlatpakBundle,
    /// Hand over to another store's own app (Steam, the Mac App Store), which does
    /// the installing. Nothing is downloaded by this store.
    OpenStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub action: Action,
    /// Label for the button: "Install", "Download", "Build from AUR".
    pub verb: String,
    /// What will happen, one short sentence per step.
    pub steps: Vec<String>,
    /// Terminal equivalent, for people who prefer it (and required for [`Action::ShowCommand`]).
    pub command: Option<String>,
}

pub fn plan(asset: &Asset, info: &AssetInfo, p: &Platform) -> InstallPlan {
    let file = safe_file_name(&asset.name);
    let mut steps = vec![format!("Download {file}")];
    if checkable_digest(asset).is_some() {
        steps.push("Check it against the SHA-256 published with the release".to_string());
    }
    let (action, verb, command) = match info.format {
        Format::Apk => {
            steps.push("Android asks you to confirm the install".to_string());
            (Action::AndroidInstaller, "Install", None)
        }
        Format::Exe | Format::Msi | Format::Msix => {
            steps.push("Windows opens the installer".to_string());
            (Action::OpenFile, "Install", None)
        }
        Format::Dmg | Format::MacPkg => {
            steps.push("macOS opens it; drag the app to Applications or follow the installer".to_string());
            (Action::OpenFile, "Install", None)
        }
        Format::Deb => {
            steps.push("Your software installer opens the package and asks for your password".to_string());
            (Action::OpenFile, "Install", Some(format!("sudo apt install ./{file}")))
        }
        Format::Rpm => {
            steps.push("Your software installer opens the package and asks for your password".to_string());
            let tool = match p.linux.as_ref().map(|l| l.family) {
                Some(LinuxFamily::Suse) => "zypper",
                _ => "dnf",
            };
            (Action::OpenFile, "Install", Some(format!("sudo {tool} install ./{file}")))
        }
        Format::PacmanPkg => {
            steps.push("Install the package with pacman".to_string());
            (Action::OpenFile, "Install", Some(format!("sudo pacman -U ./{file}")))
        }
        Format::Flatpak => {
            steps.push("Flatpak installs the bundle for your user account; no password is needed".to_string());
            (Action::FlatpakBundle, "Install", Some(format!("flatpak install --user ./{file}")))
        }
        Format::FlathubApp => {
            return InstallPlan {
                action: Action::Flatpak,
                verb: "Install".to_string(),
                steps: vec![
                    "Flatpak downloads it from Flathub and checks it".to_string(),
                    "Your system may ask you to confirm".to_string(),
                    "From then on Flatpak keeps it up to date".to_string(),
                ],
                command: Some(format!("flatpak install flathub {}", asset.name)),
            };
        }
        Format::Snap => {
            steps.push("Install the snap".to_string());
            (Action::OpenFile, "Install", Some(format!("sudo snap install --dangerous ./{file}")))
        }
        Format::AppImage => {
            steps.push("Mark it executable and move it to ~/Applications".to_string());
            (Action::PlaceAppImage, "Install", None)
        }
        Format::Ipa => {
            steps.push("Hand it to your sideloading app (AltStore, SideStore, ...)".to_string());
            (Action::OpenFile, "Download", None)
        }
        Format::Archive | Format::Binary | Format::ApkBundle => {
            steps.push("Show it in your downloads folder; unpack and run it from there".to_string());
            (Action::Reveal, "Download", None)
        }
        Format::WebApp => {
            return InstallPlan {
                action: Action::WebApp,
                verb: if info.tv { "Add to this TV" } else { "Add to my apps" }.to_string(),
                steps: vec![
                    format!("Put {} among this device's apps", asset.name),
                    if info.tv {
                        "It opens in Longboard, a browser made for remotes".to_string()
                    } else {
                        "It opens in the browser you already use".to_string()
                    },
                    "Nothing is downloaded; remove it from your library at any time".to_string(),
                ],
                command: None,
            };
        }
        Format::RepoPackage => {
            let name = asset.name.split_whitespace().next().unwrap_or(&asset.name).to_string();
            let linux = p.linux.as_ref();
            // Each asks no questions: the store cannot answer them.
            let (manager, command) = if linux.is_some_and(|l| l.pacman) {
                ("pacman", format!("sudo pacman -S --noconfirm --needed {name}"))
            } else if linux.is_some_and(|l| l.family == crate::platform::LinuxFamily::Suse) {
                ("zypper", format!("sudo zypper --non-interactive install {name}"))
            } else if linux.is_some_and(|l| l.rpm) {
                ("dnf", format!("sudo dnf install -y {name}"))
            } else {
                ("apt", format!("sudo apt-get install -y {name}"))
            };
            return InstallPlan {
                action: Action::RunCommand,
                verb: "Install".to_string(),
                steps: vec![
                    format!("{manager} downloads it from your system's repositories and checks its signature"),
                    "Your system asks for your password".to_string(),
                    format!("From then on {manager} keeps it up to date with the rest of the system"),
                ],
                command: Some(command),
            };
        }
        Format::StoreApp => {
            let scheme = asset.url.split("://").next().unwrap_or("");
            let (store, verb) = match scheme {
                "steam" if asset.url.starts_with("steam://install/") => ("Steam", "Install with Steam"),
                "steam" => ("Steam", "View in Steam"),
                "macappstore" => ("the App Store", "Get on the App Store"),
                _ => ("its store's page", "Open its store page"),
            };
            return InstallPlan {
                action: Action::OpenStore,
                verb: verb.to_string(),
                steps: vec![
                    format!("Bilby opens {store} on this app"),
                    "That store asks for anything it needs (an account, a payment) and installs it".to_string(),
                    "Bilby downloads nothing itself".to_string(),
                ],
                command: None,
            };
        }
        Format::SnapStore => {
            let name = asset.name.split_whitespace().next().unwrap_or(&asset.name).to_string();
            return InstallPlan {
                action: Action::RunCommand,
                verb: "Install".to_string(),
                steps: vec![
                    "snapd downloads it from the Snap Store and checks its signature".to_string(),
                    "Your system asks for your password".to_string(),
                    "From then on snapd keeps it up to date by itself".to_string(),
                ],
                command: Some(format!("sudo snap install {name}")),
            };
        }
        Format::WingetPackage => {
            let name = asset.name.split_whitespace().next().unwrap_or(&asset.name).to_string();
            return InstallPlan {
                action: Action::RunCommand,
                verb: "Install".to_string(),
                steps: vec![
                    "winget downloads the installer from its publisher and checks it".to_string(),
                    "Windows may ask you to confirm".to_string(),
                ],
                command: Some(format!(
                    "winget install --id {name} --exact --source winget --silent --accept-package-agreements --accept-source-agreements --disable-interactivity"
                )),
            };
        }
        Format::AurBuild => {
            let name = asset.name.split_whitespace().next().unwrap_or(&asset.name).to_string();
            let command = match p.linux.as_ref().and_then(|l| l.aur_helper.clone()) {
                Some(helper) => format!("{helper} -S {name}"),
                None => format!("git clone https://aur.archlinux.org/{name}.git && cd {name} && makepkg -si"),
            };
            steps = vec![
                "AUR recipes are written by other users and run on your machine".to_string(),
                "A terminal opens and builds it; it asks for your password there".to_string(),
            ];
            (Action::RunInTerminal, "Build and install", Some(command))
        }
    };
    InstallPlan { action, verb: verb.to_string(), steps, command }
}

/// The published SHA-256 for this file, if there is one the store can check.
pub fn checkable_digest(asset: &Asset) -> Option<&str> {
    let hex = asset.digest.as_deref()?.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

/// A file name that cannot escape the downloads folder, whatever the release calls it.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String =
        name.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

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
    pub cache: PathBuf,
    /// Settings and library (`state.json`).
    pub config: PathBuf,
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
            cache: cache.join("bilby"),
            config: config.join("bilby"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// One sentence for the UI.
    pub message: String,
    pub file: Option<PathBuf>,
    /// The store placed the app itself (AppImage), so deleting `file` uninstalls it.
    pub store_managed: bool,
    /// The Flatpak app a downloaded bundle became (`org.example.App`): Flatpak keeps it
    /// from here on, and this is the name it is started and removed by.
    pub flatpak_app: Option<String>,
}

fn menu_entry_path(dirs: &Dirs, id: &str) -> PathBuf {
    let slug: String =
        id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    dirs.menu.join(format!("bilby-{slug}.desktop"))
}

/// Put an AppImage the store placed into the desktop's app menu, so it starts like any
/// other installed app. One entry per app: a new version replaces the entry.
///
/// The format is the freedesktop.org Desktop Entry Specification; values are escaped as
/// it requires, because names and paths come from release authors.
pub fn add_menu_entry(dirs: &Dirs, id: &str, name: &str, appimage: &Path) -> Result<PathBuf> {
    let path = write_menu_entry(dirs, id, name, appimage, &[], "Utility;")?;
    // The app's own picture, so the entry is not a blank tile. Without one the entry
    // still works.
    if let Some(icon) = appimage_icon(dirs, id, appimage) {
        if let Ok(entry) = std::fs::read_to_string(&path) {
            let line = format!("Icon={}\n", icon.display().to_string().replace('\\', "\\\\"));
            let _ = std::fs::write(&path, entry.replacen("Terminal=false\n", &format!("{line}Terminal=false\n"), 1));
        }
    }
    Ok(path)
}

/// Take an AppImage's own icon out of it and keep it beside the app. An AppImage
/// carries its icon at its root as `.DirIcon`; asking the file to extract that does
/// not start the app inside it. Anything unexpected means no icon, never an error.
fn appimage_icon(dirs: &Dirs, id: &str, appimage: &Path) -> Option<PathBuf> {
    use std::process::{Command, Stdio};
    if !cfg!(target_os = "linux") {
        return None;
    }
    // An app that was unpacked has its icon lying in its folder.
    if let Some(folder) = unpacked_folder(appimage) {
        let icon = std::fs::canonicalize(folder.join(".DirIcon"))
            .ok()
            .filter(|icon| icon.starts_with(&folder) && icon.is_file())?;
        let svg = std::fs::read(&icon).is_ok_and(|bytes| !bytes.starts_with(b"\x89PNG"));
        let slug: String =
            id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let kept = dirs.applications.join(".icons");
        std::fs::create_dir_all(&kept).ok()?;
        let to = kept.join(format!("{slug}.{}", if svg { "svg" } else { "png" }));
        std::fs::copy(&icon, &to).ok()?;
        return Some(to);
    }
    let slug: String =
        id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let work = dirs.cache.join(format!("icon-of-{slug}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).ok()?;
    let extract = |what: &str| -> bool {
        Command::new(appimage)
            .arg("--appimage-extract")
            .arg(what)
            .current_dir(&work)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let root = work.join("squashfs-root");
    let mut found = None;
    if extract(".DirIcon") {
        let mut at = root.join(".DirIcon");
        // Usually a link to the real picture, sometimes through a second link.
        for _ in 0..3 {
            let Ok(target) = std::fs::read_link(&at) else { break };
            let inside = target.to_string_lossy().trim_start_matches("./").to_string();
            if inside.starts_with('/') || inside.split('/').any(|part| part == "..") || !extract(&inside) {
                break;
            }
            at = root.join(&inside);
        }
        let is_picture = std::fs::read(&at).is_ok_and(|bytes| {
            bytes.starts_with(b"\x89PNG") || bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml")
        });
        if at.is_file() && !at.is_symlink() && is_picture {
            found = Some(at);
        }
    }
    let kept = found.and_then(|from| {
        let svg = std::fs::read(&from).is_ok_and(|bytes| !bytes.starts_with(b"\x89PNG"));
        let folder = dirs.applications.join(".icons");
        std::fs::create_dir_all(&folder).ok()?;
        let to = folder.join(format!("{slug}.{}", if svg { "svg" } else { "png" }));
        std::fs::copy(&from, &to).ok()?;
        Some(to)
    });
    let _ = std::fs::remove_dir_all(&work);
    kept
}

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

/// An app-menu entry that starts `program` with `args`.
fn write_menu_entry(
    dirs: &Dirs,
    id: &str,
    name: &str,
    appimage: &Path,
    args: &[String],
    categories: &str,
) -> Result<PathBuf> {
    // In a string value, a newline would start a new key.
    let value =
        |text: &str| -> String { text.chars().filter(|c| !c.is_control()).collect::<String>().replace('\\', "\\\\") };
    // Exec: quote the path; inside quotes `"`, `` ` ``, `$` and `\` take a backslash,
    // and the whole value is then escaped once more as a string (hence four).
    let mut exec = String::from("\"");
    for c in appimage.display().to_string().chars().filter(|c| !c.is_control()) {
        match c {
            '"' | '`' | '$' => exec.push_str(&format!("\\\\{c}")),
            '\\' => exec.push_str("\\\\\\\\"),
            '%' => exec.push_str("%%"),
            c => exec.push(c),
        }
    }
    exec.push('"');
    for arg in args {
        // Arguments here are addresses the store built; quoted the same way regardless.
        exec.push_str(" \"");
        for c in arg.chars().filter(|c| !c.is_control()) {
            match c {
                '"' | '`' | '$' => exec.push_str(&format!("\\\\{c}")),
                '\\' => exec.push_str("\\\\\\\\"),
                '%' => exec.push_str("%%"),
                c => exec.push(c),
            }
        }
        exec.push('"');
    }
    let name = value(name);
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={exec}{}\nTerminal=false\nCategories={categories}\nX-Bilby-App={}\n",
        if name.is_empty() { "App".to_string() } else { name },
        // A program takes files to open; a website launcher takes nothing.
        if args.is_empty() { " %U" } else { "" },
        value(id),
    );
    std::fs::create_dir_all(&dirs.menu)?;
    let path = menu_entry_path(dirs, id);
    std::fs::write(&path, entry)?;
    Ok(path)
}

/// Put a website among the device's apps as a shortcut that opens it in the person's
/// own browser: a menu entry on Linux, an Internet shortcut in the Start menu on
/// Windows, a web-location file in Applications on macOS. The site's own icon is used
/// where the store already has it. [Run for real on Linux. The Windows and macOS
/// forms are written from their file formats and have not been tried there.]
pub fn write_web_shortcut(dirs: &Dirs, id: &str, name: &str, url: &str) -> Result<PathBuf> {
    if !(url.starts_with("https://") && url.len() < 500 && url.chars().all(|c| c.is_ascii_graphic())) {
        return Err(Error::Unsupported("That is not an address this can open.".into()));
    }
    let label: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let label = if label.trim().is_empty() { "Web app".to_string() } else { label.trim().to_string() };
    if cfg!(target_os = "windows") {
        let programs = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| Error::Unsupported("The Start menu could not be found.".into()))?
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        std::fs::create_dir_all(&programs)?;
        let path = programs.join(format!("{label}.url"));
        std::fs::write(&path, format!("[InternetShortcut]\r\nURL={url}\r\n"))?;
        return Ok(path);
    }
    if cfg!(target_os = "macos") {
        std::fs::create_dir_all(&dirs.applications)?;
        let path = dirs.applications.join(format!("{label}.webloc"));
        let escaped = url.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>URL</key>\n\t<string>{escaped}</string>\n</dict>\n</plist>\n"
        );
        std::fs::write(&path, plist)?;
        return Ok(path);
    }
    // Linux and the like: the desktop's own opener, started on the address.
    let opener = std::env::var_os("PATH")
        .and_then(|path| std::env::split_paths(&path).map(|dir| dir.join("xdg-open")).find(|p| p.is_file()))
        .unwrap_or_else(|| PathBuf::from("/usr/bin/xdg-open"));
    let path = write_menu_entry(dirs, id, &label, &opener, &[url.to_string()], "Network;")?;
    give_entry_its_picture(dirs, id, &path);
    Ok(path)
}

/// Give a menu entry the picture the store shows for the app (a website's own icon),
/// when one has been fetched. Without this the entry gets the picture of the program
/// that opens it, or none: every website on a TV would look like the browser.
fn give_entry_its_picture(dirs: &Dirs, id: &str, entry: &Path) {
    let key = crate::model::AppId::parse(id).map(|id| crate::icons::app_key(&id)).unwrap_or_default();
    let Some(Some(icon)) = crate::icons::cached(&dirs.cache.join("icons"), &key) else { return };
    // Kept beside the apps, so clearing the cache does not blank the entry.
    let folder = dirs.applications.join(".icons");
    let kept = folder.join(icon.file_name().unwrap_or_default());
    if std::fs::create_dir_all(&folder).is_err() || std::fs::copy(&icon, &kept).is_err() {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(entry) {
        let line = format!("Icon={}\n", kept.display().to_string().replace('\\', "\\\\"));
        let _ = std::fs::write(entry, text.replacen("Terminal=false\n", &format!("{line}Terminal=false\n"), 1));
    }
}

/// What an AppImage tells the program inside it about itself: where its file is and
/// where it is mounted. When the store runs from an AppImage these are about the
/// store, and a program it starts must not inherit them and take them for its own.
pub const APPIMAGE_SAYS: [&str; 4] = ["APPIMAGE", "APPDIR", "ARGV0", "OWD"];

fn not_ours(command: &mut std::process::Command) {
    for name in APPIMAGE_SAYS {
        command.env_remove(name);
    }
    carry_activation(command);
}

/// Leave to come to the front, from the window system, for the next program started.
static ACTIVATION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Hand the next program the store starts the right to come to the front.
///
/// A window system such as Wayland does not let a program put its own window in front
/// of the one being used. That is felt when the program is already running: started
/// again, it tells its running copy what to open, and that copy stays behind the
/// store, as if nothing had happened. The window that was just pressed can ask for a
/// token that carries its leave to do so, and pass it on; this keeps one for the next
/// program started. It is used once.
pub fn grant_activation(token: String) {
    *ACTIVATION.lock().unwrap() = Some(token).filter(|t| !t.is_empty());
}

/// Give `command` the token, if there is one, under the names programs look for it by
/// (Wayland's and, for the X Window System, the older start-up notification's).
fn carry_activation(command: &mut std::process::Command) {
    if let Some(token) = ACTIVATION.lock().unwrap().take() {
        command.env("XDG_ACTIVATION_TOKEN", &token).env("DESKTOP_STARTUP_ID", &token);
    }
}

/// The file or folder a running program was started from, for a store that wants to
/// know where it itself lives: the AppImage, when the program really is running out
/// of the one named (a name inherited from whoever started it does not count), or the
/// `AppRun` of the folder an AppImage was unpacked into.
pub fn own_program(exe: &Path, appimage: Option<&Path>, appdir: Option<&Path>) -> Option<PathBuf> {
    if let (Some(file), Some(mounted)) = (appimage, appdir) {
        if file.is_file() && exe.starts_with(mounted) {
            return Some(file.to_path_buf());
        }
    }
    exe.ancestors()
        .find(|folder| folder.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("AppDir")))
        .map(|folder| folder.join("AppRun"))
        .filter(|run| run.is_file())
}

/// Start something, trying again for a moment if the system says its file is busy.
/// A file only just written (a download, an update) can still be held open for
/// writing for an instant, and a program cannot be started from one that is.
fn not_while_busy<T>(mut start: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut tries = 0;
    loop {
        match start() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

/// The folder of an app that was unpacked, when `program` is the `AppRun` inside one.
pub fn unpacked_folder(program: &Path) -> Option<PathBuf> {
    let folder = program.parent()?;
    let is_one = program.file_name().is_some_and(|name| name == "AppRun")
        && folder.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("AppDir"));
    is_one.then(|| folder.to_path_buf())
}

/// Unpack an AppImage into a folder beside it and give back the program to start.
///
/// An AppImage normally mounts itself with FUSE each time it starts. Asked to, it
/// will instead write its contents out as an ordinary folder, and the `AppRun` in
/// that folder starts the app with no FUSE involved, no package to install and no
/// password. The cost is disk: the app takes its full size rather than its packed one.
/// The AppImage itself is removed once the folder is in place. [Run for real with
/// Longboard's AppImage on Debian 13.]
pub fn unpack_appimage(appimage: &Path) -> Result<PathBuf> {
    use std::process::{Command, Stdio};
    let cannot = |why: &str| Error::Unsupported(format!("The app could not be unpacked: {why}"));
    let home = appimage.parent().ok_or_else(|| cannot("it has no folder"))?;
    let stem = appimage
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| cannot("it has no name"))?;
    let work = home.join(format!(".unpacking-{stem}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let ran = not_while_busy(|| {
        Command::new(appimage)
            .arg("--appimage-extract")
            .current_dir(&work)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    });
    let inside = work.join("squashfs-root");
    let whole = ran.is_ok_and(|status| status.success()) && inside.join("AppRun").exists();
    if !whole {
        let _ = std::fs::remove_dir_all(&work);
        return Err(cannot("it is not a kind of AppImage that can unpack itself"));
    }
    let folder = home.join(format!("{stem}.AppDir"));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::rename(&inside, &folder)?;
    let _ = std::fs::remove_dir_all(&work);
    let _ = std::fs::remove_file(appimage);
    let program = folder.join("AppRun");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&program) {
            let mut permissions = meta.permissions();
            permissions.set_mode(permissions.mode() | 0o755);
            let _ = std::fs::set_permissions(&program, permissions);
        }
    }
    Ok(program)
}

/// Remove what the store placed for an app: the file, or the whole folder of an app
/// that was unpacked. Only ever inside the store's own Applications folder.
pub fn remove_placed(dirs: &Dirs, program: &Path) -> bool {
    if !program.starts_with(&dirs.applications) {
        return false;
    }
    match unpacked_folder(program) {
        Some(folder) => std::fs::remove_dir_all(folder).is_ok(),
        None => std::fs::remove_file(program).is_ok(),
    }
}

/// Does this step fetch a file into the downloads folder? Package managers, Flatpak,
/// other stores and web apps do not: they have their own places, or none.
pub fn saves_a_file(action: &Action) -> bool {
    matches!(action, Action::OpenFile | Action::PlaceAppImage | Action::Reveal | Action::AndroidInstaller)
}

/// Let the person choose a folder with the system's own dialog, starting at `start`.
/// `None` when they cancel, or when this system has no dialog the store can call
/// (then the folder can still be typed into Settings). [Run for real with kdialog on
/// KDE. The zenity, macOS and Windows forms are written from their documentation.]
pub fn pick_folder(start: &Path) -> Option<PathBuf> {
    use std::process::{Command, Stdio};
    let on_path = |program: &str| {
        std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    };
    let ask = |command: &mut Command| -> Option<PathBuf> {
        let said = command.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let text = String::from_utf8_lossy(&said.stdout).trim().to_string();
        let path = PathBuf::from(text);
        (said.status.success() && path.is_absolute() && path.is_dir()).then_some(path)
    };
    if cfg!(target_os = "macos") {
        return ask(Command::new("osascript")
            .args(["-e", "POSIX path of (choose folder with prompt \"Where should downloads go?\")"]));
    }
    if cfg!(target_os = "windows") {
        let script = "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Where should downloads go?'; if ($d.ShowDialog() -eq 'OK') { $d.SelectedPath }";
        return ask(Command::new("powershell").args(["-NoProfile", "-Command", script]));
    }
    if on_path("kdialog") {
        return ask(Command::new("kdialog")
            .arg("--getexistingdirectory")
            .arg(start)
            .args(["--title", "Where should downloads go?"]));
    }
    if on_path("zenity") {
        let mut from = start.display().to_string();
        from.push('/');
        return ask(Command::new("zenity")
            .args(["--file-selection", "--directory", "--title=Where should downloads go?"])
            .arg(format!("--filename={from}")));
    }
    None
}

/// Where the store's own app-menu entry goes.
pub fn self_entry_path(dirs: &Dirs) -> PathBuf {
    dirs.menu.join("bilby.desktop")
}

/// Put the store itself in the app menu, so a downloaded AppImage shows up among the
/// system's apps (and on a TV launcher such as Plasma Bigscreen) after its first run.
/// `program` is the file to start and `icon` its picture as a PNG. Safe to call on every
/// start: nothing is written unless something changed, such as the file having moved.
/// Returns whether anything was written.
pub fn register_self(dirs: &Dirs, program: &Path, icon: &[u8]) -> Result<bool> {
    let quoted = |text: &str| -> String {
        let mut out = String::from("\"");
        for c in text.chars().filter(|c| !c.is_control()) {
            match c {
                '"' | '`' | '$' => out.push_str(&format!("\\\\{c}")),
                '\\' => out.push_str("\\\\\\\\"),
                '%' => out.push_str("%%"),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    };
    // Beside the menu folder, where desktops look for icons; named in full in the entry
    // so no icon cache has to notice it first.
    let icons = dirs.menu.parent().unwrap_or(&dirs.menu).join("icons").join("hicolor").join("512x512").join("apps");
    let icon_path = icons.join("bilby.png");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Bilby\nGenericName=App Store\n\
         Comment=Apps from GitHub Releases, Codeberg and other repositories that run on this device\n\
         Exec={}\nIcon={}\nTerminal=false\nCategories=System;PackageManager;\n\
         Keywords=apps;store;install;update;software;\nStartupWMClass=bilby\nX-Bilby-Self=true\n",
        quoted(&program.display().to_string()),
        icon_path.display().to_string().chars().filter(|c| !c.is_control()).collect::<String>(),
    );
    let path = self_entry_path(dirs);
    let mut changed = false;
    if std::fs::read(&icon_path).ok().as_deref() != Some(icon) {
        std::fs::create_dir_all(&icons)?;
        std::fs::write(&icon_path, icon)?;
        changed = true;
    }
    if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
        std::fs::create_dir_all(&dirs.menu)?;
        std::fs::write(&path, entry)?;
        changed = true;
    }
    Ok(changed)
}

/// Take the store's own entry out of the app menu again, if it wrote one.
pub fn unregister_self(dirs: &Dirs) {
    let path = self_entry_path(dirs);
    if std::fs::read_to_string(&path).is_ok_and(|text| text.contains("X-Bilby-Self=true")) {
        let _ = std::fs::remove_file(path);
    }
}

/// Take an app's entry out of the app menu, if the store put one there.
pub fn remove_menu_entry(dirs: &Dirs, id: &str) {
    let path = menu_entry_path(dirs, id);
    // Only files this store wrote: the name is ours, and so is the marker inside.
    if std::fs::read_to_string(&path).is_ok_and(|text| text.contains("X-Bilby-App=")) {
        let _ = std::fs::remove_file(path);
    }
}

/// Run Flatpak and wait for it, giving up (and stopping it) when `keep_going` says so.
/// Flatpak reports no usable progress, so the callback is only asked whether to go on.
pub fn run_flatpak(dirs: &Dirs, args: &[&str], keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<()> {
    run_program(&dirs.flatpak, args, "Flatpak", keep_going)
}

/// A terminal on this machine and the option that makes it run a command.
pub fn find_terminal() -> Option<(PathBuf, &'static str)> {
    const KNOWN: [(&str, &str); 9] = [
        ("konsole", "-e"),
        ("gnome-terminal", "--"),
        ("kgx", "-e"),
        ("xfce4-terminal", "-x"),
        ("alacritty", "-e"),
        ("kitty", "-e"),
        ("foot", "-e"),
        ("x-terminal-emulator", "-e"),
        ("xterm", "-e"),
    ];
    let path = std::env::var_os("PATH")?;
    KNOWN.iter().find_map(|(name, flag)| {
        std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file()).map(|p| (p, *flag))
    })
}

/// Run a program and wait for it, giving up (and stopping it) when `keep_going` says
/// so. `name` is what to call it when it fails.
pub fn run_program(
    program: &Path,
    args: &[&str],
    name: &str,
    keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this a console window flashes up behind the store.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| Error::Unsupported(format!("{name} could not be started ({e}). Is it installed?")))?;
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => return Ok(()),
            // The password prompt's own codes: it was closed, or permission was refused.
            Some(status) if matches!(status.code(), Some(126 | 127)) && program.ends_with("pkexec") => {
                return Err(Error::Unsupported(
                    "Nothing was installed: the password prompt was closed or refused.".to_string(),
                ));
            }
            Some(_) => {
                let mut said = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut said);
                }
                // The program's own last line is the most useful thing to show.
                let why =
                    said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it gave no reason").trim().to_string();
                return Err(Error::Unsupported(format!("{name} could not do it: {why}")));
            }
            None => {
                if !keep_going(0, None) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Cancelled);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// [`run_program`], also returning what the program printed. It is written to `log`
/// as the program runs (a pipe nobody reads would stop it), and the file is removed.
fn run_program_said(
    program: &Path,
    args: &[&str],
    name: &str,
    log: &Path,
    keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(std::fs::File::create(log)?).stderr(Stdio::piped());
    let finish = |result: Result<()>| {
        let said = std::fs::read_to_string(log).unwrap_or_default();
        let _ = std::fs::remove_file(log);
        result.map(|()| said)
    };
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return finish(Err(Error::Unsupported(format!("{name} could not be started ({e}). Is it installed?"))))
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return finish(Ok(())),
            Ok(Some(_)) => {
                let mut said = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut said);
                }
                let why =
                    said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it gave no reason").trim().to_string();
                return finish(Err(Error::Unsupported(format!("{name} could not do it: {why}"))));
            }
            Ok(None) => {
                if !keep_going(0, None) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return finish(Err(Error::Cancelled));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return finish(Err(e.into())),
        }
    }
}

/// Is this a name Flatpak would give an app: `org.example.App`, nothing that could be
/// taken for an option or a path?
pub fn flatpak_app_id_ok(id: &str) -> bool {
    id.len() <= 255
        && id.split('.').count() >= 2
        && id.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// The app Flatpak said it was installing: the id out of "app/<id>/<arch>/<branch>".
fn flatpak_app_named(said: &str) -> Option<String> {
    said.split_whitespace()
        .filter_map(|word| word.strip_prefix("app/"))
        .filter_map(|rest| rest.split('/').next())
        .find(|id| flatpak_app_id_ok(id))
        .map(str::to_string)
}

/// The Flatpak apps on the device, by id.
fn flatpak_app_ids(roots: &[PathBuf]) -> Vec<String> {
    let mut ids = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join("app")) else { continue };
        ids.extend(entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()));
    }
    ids
}

/// Is this Flatpak app on the device, in any of Flatpak's installations?
pub fn flatpak_app_installed(dirs: &Dirs, id: &str) -> bool {
    flatpak_app_id_ok(id) && dirs.flatpak_roots.iter().any(|root| root.join("app").join(id).is_dir())
}

/// Start a program the store placed (an AppImage), without waiting for it.
pub fn launch(path: &Path) -> Result<()> {
    launch_watched(path, &[], std::time::Duration::from_millis(2500))
}

/// Start a program and watch its first moments. One that closes straight away with an
/// error did not start, and saying "Opening…" would be untrue: what it printed is
/// turned into the reason, in words a person can act on where the cause is known.
pub fn launch_watched(path: &Path, args: &[String], watch: std::time::Duration) -> Result<()> {
    use std::process::{Command, Stdio};
    let log = std::env::temp_dir().join(format!("bilby-launch-{}.log", std::process::id()));
    let mut child = not_while_busy(|| {
        let errors = std::fs::File::create(&log).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
        let mut command = Command::new(path);
        not_ours(&mut command);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(errors).spawn()
    })?;
    let started = std::time::Instant::now();
    let failed = loop {
        match child.try_wait() {
            Ok(Some(status)) => break !status.success(),
            Ok(None) if started.elapsed() >= watch => break false,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => break false,
        }
    };
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    if failed {
        return Err(Error::Unsupported(why_it_closed(&said)));
    }
    Ok(())
}

/// The reason a program gave for not starting, for a person. Known causes get advice;
/// anything else is the program's own last words.
pub fn why_it_closed(said: &str) -> String {
    let lower = said.to_lowercase();
    if lower.contains("fuse") {
        return "It closed straight away: this kind of app (an AppImage) needs FUSE, which this system does not have. Install the package libfuse2 (on newer systems libfuse2t64, or fuse2 on Arch), then open it again.".to_string();
    }
    if lower.contains("sandbox") {
        return "It closed straight away: its built-in browser could not start its sandbox on this system. This is a known limit of some Linux setups for apps built this way; its developer's page usually says how to allow it.".to_string();
    }
    if lower.contains("error while loading shared libraries") {
        let library = said
            .lines()
            .find(|l| l.contains("shared libraries"))
            .and_then(|l| l.split(": ").nth(2))
            .unwrap_or("a library");
        return format!("It closed straight away: this system is missing something it needs ({library}).");
    }
    let last = said.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    let last: String = last.chars().filter(|c| !c.is_control()).take(160).collect();
    if last.is_empty() {
        "It closed straight away without saying why.".to_string()
    } else {
        format!("It closed straight away. It said: {last}")
    }
}

/// Start a program with arguments, without waiting for it.
pub fn launch_with(path: &Path, args: &[String]) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(path);
    not_ours(&mut command);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(())
}

/// Open a file or folder with the desktop's default handler.
pub fn open_with_system(path: &Path) -> Result<()> {
    let mut cmd = if cfg!(target_os = "windows") {
        // Not `cmd /C start`: file names come from release authors, and cmd.exe gives
        // characters such as & and ^ a meaning even inside quotes. Explorer receives the
        // path as a plain argument and opens it with the default handler.
        std::process::Command::new("explorer")
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    carry_activation(&mut cmd);
    cmd.arg(path).spawn().map(|_| ()).map_err(|e| Error::Unsupported(format!("Could not open {}: {e}", path.display())))
}

/// Open a web page in the user's browser. Only `https://` links are opened: URLs come
/// from repository metadata, which anyone can write.
pub fn open_url(url: &str) -> Result<()> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Unsupported(format!("Refusing to open \"{url}\": not an https link")));
    }
    open_with_system(Path::new(url))
}

/// The only links handed to another store: its own scheme followed by a number, or its
/// own website. Built by this store from a checked id; checked again before opening.
pub fn store_link_ok(url: &str) -> bool {
    let number = |rest: &str| (1..=12).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_digit());
    [
        "steam://install/",
        "steam://store/",
        "steam://rungameid/",
        "macappstore://apps.apple.com/app/id",
        "https://store.steampowered.com/app/",
        "https://apps.apple.com/app/id",
    ]
    .iter()
    .any(|prefix| url.strip_prefix(prefix).is_some_and(number))
}

/// The plan for a file the store would not install here: fetch it, check it, and show
/// where it was put. Nothing is run.
pub fn save_only() -> InstallPlan {
    InstallPlan {
        action: Action::Reveal,
        verb: "Download".to_string(),
        steps: vec!["Download the file".to_string(), "Show it in your downloads folder".to_string()],
        command: None,
    }
}

/// Carry out `plan` for `asset`. `open` is how files are handed to the system
/// ([`open_with_system`] in the app; a recorder in tests).
pub fn execute(
    plan: &InstallPlan,
    asset: &Asset,
    http: &dyn Http,
    dirs: &Dirs,
    open: &dyn Fn(&Path) -> Result<()>,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<Outcome> {
    if plan.action == Action::ShowCommand {
        let command = plan.command.clone().unwrap_or_default();
        return Ok(Outcome {
            message: format!("Run in a terminal: {command}"),
            file: None,
            store_managed: false,
            flatpak_app: None,
        });
    }
    if plan.action == Action::RunCommand {
        // The command is one this store built from a checked name: words, no quoting.
        let command = plan.command.clone().unwrap_or_default();
        let mut words = command.split_whitespace();
        let (program, rest): (PathBuf, Vec<&str>) = match words.next() {
            // "sudo" stands for "as administrator": the desktop's own prompt does that here.
            Some("sudo") => (dirs.elevate.clone(), words.collect()),
            Some(program) => (PathBuf::from(program), words.collect()),
            None => return Err(Error::Unsupported("There is nothing to run.".to_string())),
        };
        let what = rest.first().copied().unwrap_or("it");
        run_program(&program, &rest, if command.starts_with("sudo ") { what } else { "The installer" }, progress)?;
        return Ok(Outcome { message: "Installed.".to_string(), file: None, store_managed: false, flatpak_app: None });
    }
    if plan.action == Action::RunInTerminal {
        let command = plan.command.clone().unwrap_or_default();
        let Some((terminal, flag)) = &dirs.terminal else {
            // No terminal to open: the command is all that can be offered.
            return Ok(Outcome {
                message: format!("No terminal was found. Run this in one: {command}"),
                file: None,
                store_managed: false,
                flatpak_app: None,
            });
        };
        // Keep the window open afterwards, so the result can be read.
        let script = format!("{command}; echo; echo 'Done. Press Enter to close.'; read _");
        std::process::Command::new(terminal)
            .arg(flag)
            .args(["sh", "-c", &script])
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|e| Error::Unsupported(format!("The terminal could not be opened ({e}). Run: {command}")))?;
        return Ok(Outcome {
            message: "A terminal has opened to build it. Come back when it has finished.".to_string(),
            file: None,
            store_managed: false,
            flatpak_app: None,
        });
    }

    if plan.action == Action::OpenStore {
        if !store_link_ok(&asset.url) {
            return Err(Error::Unsupported(format!("Refusing to open \"{}\": not a store link", asset.url)));
        }
        open(Path::new(&asset.url))?;
        let store = if asset.url.starts_with("steam://") {
            "Steam"
        } else if asset.url.starts_with("macappstore://") {
            "The App Store"
        } else {
            "Your browser"
        };
        return Ok(Outcome {
            message: format!("{store} is taking it from here. Come back when it has finished."),
            file: None,
            store_managed: false,
            flatpak_app: None,
        });
    }

    if plan.action == Action::Flatpak {
        let id = asset.name.as_str();
        run_flatpak(dirs, &["install", "--noninteractive", "--assumeyes", "--or-update", "flathub", id], progress)?;
        return Ok(Outcome {
            message: "Installed from Flathub. It is in your app menu.".to_string(),
            file: None,
            store_managed: false,
            flatpak_app: None,
        });
    }
    // On a computer or phone a web app is a shortcut to the site in the usual browser.
    if plan.action == Action::WebApp && !asset.info.as_ref().is_some_and(|info| info.tv) {
        let host = asset.url.trim_start_matches("https://").trim_end_matches('/');
        let entry = write_web_shortcut(dirs, &format!("web:{host}"), &asset.name, &asset.url)?;
        return Ok(Outcome {
            message: format!("{} is among your apps. It opens in your browser.", asset.name),
            file: Some(entry),
            store_managed: false,
            flatpak_app: None,
        });
    }
    if plan.action == Action::WebApp {
        let host = asset.url.trim_start_matches("https://").trim_end_matches('/');
        // The store installs Longboard itself before it gets here; this is for a caller
        // that did not.
        let longboard = find_longboard(dirs).ok_or_else(|| {
            Error::Unsupported(
                "Web apps open in Longboard, a browser for TVs, which is not on this device.".to_string(),
            )
        })?;
        let args = longboard.args(&[format!("--url={}", asset.url)]);
        let entry = write_menu_entry(dirs, &format!("web:{host}"), &asset.name, &longboard.program, &args, "Network;")?;
        // The site's own picture, not Longboard's, in the TV's list of apps.
        give_entry_its_picture(dirs, &format!("web:{host}"), &entry);
        return Ok(Outcome {
            message: format!("{} is in this TV's app list. It opens in Longboard.", asset.name),
            file: Some(entry),
            store_managed: false,
            flatpak_app: None,
        });
    }

    // A Flatpak bundle is only a parcel: Flatpak copies the app out of it into its own
    // place, so it is kept out of the downloads folder and thrown away afterwards.
    let folder = if plan.action == Action::FlatpakBundle { dirs.cache.join("bundles") } else { dirs.downloads.clone() };
    let dest = folder.join(safe_file_name(&asset.name));
    let actual = http.download(&asset.url, &dest, progress)?;
    let expected = checkable_digest(asset);
    if let Some(expected) = expected {
        if !expected.eq_ignore_ascii_case(&actual) {
            let _ = std::fs::remove_file(&dest);
            return Err(Error::DigestMismatch { expected: expected.to_string(), actual });
        }
    }
    // Only claim what was done: a digest in a format we cannot check is not a check.
    let verified = if expected.is_some() { "Checksum verified. " } else { "" };

    match plan.action {
        Action::PlaceAppImage => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
            }
            std::fs::create_dir_all(&dirs.applications)?;
            let target = dirs.applications.join(dest.file_name().unwrap_or_default());
            // rename() fails across filesystems; fall back to copy + delete.
            if std::fs::rename(&dest, &target).is_err() {
                std::fs::copy(&dest, &target)?;
                std::fs::remove_file(&dest)?;
            }
            Ok(Outcome { message: format!("{verified}Installed to {}", target.display()), file: Some(target), store_managed: true, flatpak_app: None })
        }
        Action::OpenFile => {
            // The download succeeded either way; a missing handler is not a failure.
            let message = match open(&dest) {
                Ok(()) => format!("{verified}Opened {} with your system installer", dest.display()),
                Err(_) => format!("{verified}Saved to {}. Open it to finish installing.", dest.display()),
            };
            Ok(Outcome { message, file: Some(dest), store_managed: false, flatpak_app: None })
        }
        Action::Reveal => {
            // Failing to open a file manager is not a failed download.
            let _ = open(&dirs.downloads);
            Ok(Outcome { message: format!("{verified}Saved to {}", dest.display()), file: Some(dest), store_managed: false, flatpak_app: None })
        }
        Action::AndroidInstaller => Err(Error::Unsupported(format!(
            "{verified}Saved to {}. Handing APKs to Android's installer is not implemented in this build yet; open the file to install it.",
            dest.display()
        ))),
        Action::FlatpakBundle => {
            let file = dest.display().to_string();
            let before = flatpak_app_ids(&dirs.flatpak_roots);
            // "--reinstall" so that installing again, or a newer version over an older
            // one, goes through: without it Flatpak refuses what is already installed.
            let args = ["install", "--user", "--noninteractive", "--assumeyes", "--reinstall", &file];
            let said = run_program_said(&dirs.flatpak, &args, "Flatpak", &dest.with_extension("log"), progress);
            // Whether or not it went through, the bundle has done its work.
            let _ = std::fs::remove_file(&dest);
            let said = said?;
            // Which app it was: Flatpak names it as it installs ("app/<id>/<arch>/<branch>").
            // Failing that, the one app that is there now and was not before.
            let flatpak_app = flatpak_app_named(&said).or_else(|| {
                let mut new = flatpak_app_ids(&dirs.flatpak_roots);
                new.retain(|id| !before.contains(id));
                (new.len() == 1).then(|| new.remove(0))
            });
            Ok(Outcome {
                message: format!("{verified}Installed with Flatpak. It is in your app menu."),
                file: None,
                store_managed: false,
                flatpak_app,
            })
        }
        Action::ShowCommand
        | Action::RunCommand
        | Action::RunInTerminal
        | Action::WebApp
        | Action::Flatpak
        | Action::OpenStore => unreachable!("handled above"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::classify;
    use crate::http::hex;
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;

    struct Fake(&'static [u8]);
    impl Http for Fake {
        fn get(&self, _: &str) -> Result<String> {
            unreachable!()
        }
        fn download(&self, _: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
            std::fs::create_dir_all(dest.parent().unwrap())?;
            std::fs::write(dest, self.0)?;
            assert!(progress(self.0.len() as u64, Some(self.0.len() as u64)));
            Ok(hex(&Sha256::digest(self.0)))
        }
    }

    fn asset(name: &str, digest: Option<String>) -> Asset {
        Asset { name: name.to_string(), size: 4, url: format!("https://example.invalid/{name}"), digest, info: None }
    }

    #[test]
    fn handing_over_to_another_store_opens_its_link_and_downloads_nothing() {
        let dirs = temp_dirs("handoff");
        let link = |url: &str| Asset {
            name: "Install with Steam".into(),
            size: 0,
            url: url.into(),
            digest: None,
            info: Some(AssetInfo {
                format: Format::StoreApp,
                os: crate::asset::TargetOs::Known(crate::platform::Os::Linux),
                arch: crate::asset::TargetArch::Unknown,
                libc: None,
                tv: false,
                debug: false,
            }),
        };
        let platform = Platform::preset("linux-x64-debian").unwrap();
        let asset = link("steam://install/620");
        let made = plan(&asset, asset.info.as_ref().unwrap(), &platform);
        assert!(made.action == Action::OpenStore && made.verb == "Install with Steam");
        let opened = RefCell::new(Vec::new());
        let open = |path: &Path| {
            opened.borrow_mut().push(path.to_path_buf());
            Ok(())
        };
        let outcome = execute(&made, &asset, &Fake(b""), &dirs, &open, &mut |_, _| true).unwrap();
        assert_eq!(opened.borrow()[0].to_str(), Some("steam://install/620"));
        assert!(outcome.file.is_none() && outcome.message.starts_with("Steam is taking it from here"));
        assert!(!dirs.downloads.exists(), "nothing was downloaded");

        // Only a store's own link followed by a number is ever opened.
        for bad in [
            "steam://install/620/../x",
            "steam://run/620",
            "file:///etc/passwd",
            "https://example.com/app/620",
            "steam://install/",
        ] {
            assert!(!store_link_ok(bad), "{bad}");
            let asset = link(bad);
            assert!(execute(&made, &asset, &Fake(b""), &dirs, &open, &mut |_, _| true).is_err(), "{bad}");
        }
        assert!(store_link_ok("macappstore://apps.apple.com/app/id6746662575"));
        assert_eq!(opened.borrow().len(), 1);
        let mac = link("macappstore://apps.apple.com/app/id1");
        assert_eq!(plan(&mac, mac.info.as_ref().unwrap(), &platform).verb, "Get on the App Store");
    }

    #[test]
    fn the_store_puts_itself_in_the_app_menu_once_and_follows_its_file() {
        let dirs = temp_dirs("self");
        let program = Path::new("/home/someone/My Apps/bilby-v0.1.3-x86_64.AppImage");
        assert!(register_self(&dirs, program, b"png").unwrap(), "the first run writes the entry");
        let entry = std::fs::read_to_string(self_entry_path(&dirs)).unwrap();
        assert!(entry.contains("\nName=Bilby\n"));
        assert!(entry.contains("\nExec=\"/home/someone/My Apps/bilby-v0.1.3-x86_64.AppImage\"\n"), "{entry}");
        // Joined piece by piece, as the code does, so the separators agree on every system.
        let icon = ["icons", "hicolor", "512x512", "apps", "bilby.png"]
            .iter()
            .fold(dirs.menu.parent().unwrap().to_path_buf(), |path, part| path.join(part));
        assert!(entry.contains(&format!("\nIcon={}\n", icon.display())));
        assert_eq!(std::fs::read(&icon).unwrap(), b"png");

        // Starting again changes nothing; starting from somewhere else updates it.
        assert!(!register_self(&dirs, program, b"png").unwrap());
        assert!(register_self(&dirs, Path::new("/opt/bilby.AppImage"), b"png").unwrap());
        assert!(std::fs::read_to_string(self_entry_path(&dirs)).unwrap().contains("Exec=\"/opt/bilby.AppImage\""));

        // Only its own entry is ever removed.
        unregister_self(&dirs);
        assert!(!self_entry_path(&dirs).exists());
        std::fs::write(self_entry_path(&dirs), "[Desktop Entry]\nName=Someone else's\n").unwrap();
        unregister_self(&dirs);
        assert!(self_entry_path(&dirs).exists());
    }

    fn temp_dirs(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("bilby-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Dirs {
            downloads: root.join("dl"),
            applications: root.join("apps"),
            menu: root.join("menu"),
            flatpak: root.join("flatpak-program"),
            elevate: root.join("elevate-program"),
            terminal: None,
            flatpak_roots: vec![root.join("flatpak")],
            cache: root.join("cache"),
            config: root.join("config"),
        }
    }

    fn plan_for(name: &str, preset: &str) -> InstallPlan {
        plan(&asset(name, None), &classify(name).unwrap(), &Platform::preset(preset).unwrap())
    }

    #[test]
    fn plans_match_the_format() {
        assert_eq!(
            plan_for("a_1.0_amd64.deb", "linux-x64-debian").command.as_deref(),
            Some("sudo apt install ./a_1.0_amd64.deb")
        );
        assert_eq!(
            plan_for("a-1.0.x86_64.rpm", "linux-x64-fedora").command.as_deref(),
            Some("sudo dnf install ./a-1.0.x86_64.rpm")
        );
        assert_eq!(plan_for("A-x86_64.AppImage", "linux-x64-fedora").action, Action::PlaceAppImage);
        assert_eq!(plan_for("a-arm64-v8a.apk", "android-arm64").action, Action::AndroidInstaller);
        assert_eq!(plan_for("a-x64.msi", "windows-x64").action, Action::OpenFile);
        assert_eq!(plan_for("a-linux-x86_64.tar.gz", "linux-x64-debian").verb, "Download");
    }

    #[test]
    fn only_https_links_are_opened() {
        for bad in
            ["file:///etc/passwd", "javascript:alert(1)", "http://example.com", "", "https://a b", "/bin/sh", "-rf"]
        {
            assert!(open_url(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn repository_packages_are_left_to_the_package_manager() {
        use crate::asset::{TargetArch, TargetOs};
        use crate::platform::Os;
        let info = AssetInfo {
            format: Format::RepoPackage,
            os: TargetOs::Known(Os::Linux),
            arch: TargetArch::Any,
            libc: None,
            tv: false,
            debug: false,
        };
        let a = asset("linux-cachyos (cachyos-v3)", None);
        let arch = plan(&a, &info, &Platform::preset("linux-x64-arch").unwrap());
        assert_eq!(arch.action, Action::RunCommand);
        assert_eq!(arch.command.as_deref(), Some("sudo pacman -S --noconfirm --needed linux-cachyos"));
        assert_eq!(arch.verb, "Install");
        let on = |preset: &str| {
            plan(&asset("ripgrep (repo)", None), &info, &Platform::preset(preset).unwrap()).command.unwrap()
        };
        assert_eq!(on("linux-x64-debian"), "sudo apt-get install -y ripgrep");
        assert_eq!(on("linux-x64-fedora"), "sudo dnf install -y ripgrep");

        // The store runs it: "sudo" becomes the desktop's own password prompt, and
        // nothing is downloaded by the store itself.
        let dirs = temp_dirs("repo");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(dirs.elevate.parent().unwrap()).unwrap();
            let log = dirs.elevate.with_extension("log");
            let script = |code: u8| {
                format!("#!/bin/sh\necho \"$@\" >> '{}'\necho 'it said why' >&2\nexit {code}\n", log.display())
            };
            std::fs::write(&dirs.elevate, script(0)).unwrap();
            std::fs::set_permissions(&dirs.elevate, std::fs::Permissions::from_mode(0o755)).unwrap();
            let out = execute(&arch, &a, &Fake(b""), &dirs, &|_| unreachable!(), &mut |_, _| true).unwrap();
            assert_eq!(out.message, "Installed.");
            assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "pacman -S --noconfirm --needed linux-cachyos");
            // A failure passes on what the package manager said.
            std::fs::write(&dirs.elevate, script(1)).unwrap();
            let failed = execute(&arch, &a, &Fake(b""), &dirs, &|_| unreachable!(), &mut |_, _| true).unwrap_err();
            assert_eq!(failed.to_string(), "pacman could not do it: it said why");
        }
        // Without a way to ask for the password, it says so instead of pretending.
        let nowhere = Dirs { elevate: dirs.elevate.with_extension("missing"), ..temp_dirs("repo2") };
        let failed = execute(&arch, &a, &Fake(b""), &nowhere, &|_| unreachable!(), &mut |_, _| true).unwrap_err();
        assert!(failed.to_string().contains("could not be started"), "{failed}");
        assert!(!dirs.downloads.exists());
    }

    #[test]
    fn file_names_cannot_escape() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("..\\evil.exe"), "_evil.exe");
        assert_eq!(safe_file_name(".."), "download");
        assert_eq!(safe_file_name("Thing-1.0.AppImage"), "Thing-1.0.AppImage");
    }

    #[test]
    fn appimage_is_verified_made_executable_and_moved() {
        let dirs = temp_dirs("appimage");
        let good = format!("sha256:{}", hex(&Sha256::digest(b"data")));
        let a = asset("Thing-x86_64.AppImage", Some(good));
        let p = plan_for("Thing-x86_64.AppImage", "linux-x64-fedora");
        let out = execute(&p, &a, &Fake(b"data"), &dirs, &|_| Ok(()), &mut |_, _| true).unwrap();
        let file = out.file.unwrap();
        assert_eq!(file, dirs.applications.join("Thing-x86_64.AppImage"));
        assert!(out.message.starts_with("Checksum verified."));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o111, 0o111);
        }
    }

    #[test]
    fn only_a_digest_that_can_be_checked_counts_as_one() {
        let dirs = temp_dirs("digest-kinds");
        let p = plan_for("a-linux-x86_64.tar.gz", "linux-x64-debian");
        for odd in ["sha512:abcdef", "md5:900150983cd24fb0d6963f7d28e17f72", "sha256:", "sha256:xyz", "garbage"] {
            let a = asset("a-linux-x86_64.tar.gz", Some(odd.to_string()));
            assert_eq!(checkable_digest(&a), None, "{odd}");
            let planned = plan(&a, &classify(&a.name).unwrap(), &Platform::preset("linux-x64-debian").unwrap());
            assert!(!planned.steps.iter().any(|s| s.contains("SHA-256")), "{odd}: no check is promised");
            let out = execute(&p, &a, &Fake(b"data"), &dirs, &|_| Ok(()), &mut |_, _| true).unwrap();
            assert!(!out.message.contains("verified"), "{odd}: no check is claimed: {}", out.message);
        }
        // Upper-case hex is the same digest.
        let upper = format!("sha256:{}", hex(&Sha256::digest(b"data")).to_uppercase());
        let a = asset("a-linux-x86_64.tar.gz", Some(upper));
        let out = execute(&p, &a, &Fake(b"data"), &dirs, &|_| Ok(()), &mut |_, _| true).unwrap();
        assert!(out.message.starts_with("Checksum verified."));
    }

    #[test]
    fn tampered_download_is_rejected_and_deleted() {
        let dirs = temp_dirs("tamper");
        let a = asset("a_1.0_amd64.deb", Some(format!("sha256:{}", "0".repeat(64))));
        let p = plan_for("a_1.0_amd64.deb", "linux-x64-debian");
        let opened = RefCell::new(Vec::new());
        let err = execute(
            &p,
            &a,
            &Fake(b"data"),
            &dirs,
            &|path| {
                opened.borrow_mut().push(path.to_path_buf());
                Ok(())
            },
            &mut |_, _| true,
        )
        .unwrap_err();
        assert!(matches!(err, Error::DigestMismatch { .. }));
        assert!(opened.borrow().is_empty(), "a file that fails verification is never opened");
        assert!(!dirs.downloads.join("a_1.0_amd64.deb").exists());
    }

    #[test]
    fn native_packages_are_handed_to_the_system() {
        let dirs = temp_dirs("deb");
        let a = asset("a_1.0_amd64.deb", None);
        let p = plan_for("a_1.0_amd64.deb", "linux-x64-debian");
        let opened = RefCell::new(Vec::new());
        execute(
            &p,
            &a,
            &Fake(b"data"),
            &dirs,
            &|path| {
                opened.borrow_mut().push(path.to_path_buf());
                Ok(())
            },
            &mut |_, _| true,
        )
        .unwrap();
        assert_eq!(*opened.borrow(), vec![dirs.downloads.join("a_1.0_amd64.deb")]);
    }

    #[test]
    fn an_appimage_gets_one_menu_entry_that_cannot_be_broken_out_of() {
        let dirs = temp_dirs("menu");
        let file = dirs.applications.join("My $HOME `app` \"x\" 100%.AppImage");
        let path = add_menu_entry(&dirs, "github:Owner/Thing", "Thing\nExec=evil", &file).unwrap();
        assert_eq!(path, dirs.menu.join("bilby-github-owner-thing.desktop"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("Exec=")).count(), 1, "a name cannot add keys: {text}");
        assert!(text.contains("Name=ThingExec=evil\n"));
        let exec = text.lines().find(|l| l.starts_with("Exec=")).unwrap();
        assert!(exec.contains(r#"\\$HOME"#) && exec.contains(r#"\\`app\\`"#) && exec.contains(r#"\\"x\\""#), "{exec}");
        assert!(exec.contains("100%%") && exec.ends_with("\" %U"), "{exec}");

        // A new version of the same app replaces the entry instead of adding one.
        add_menu_entry(&dirs, "github:Owner/Thing", "Thing", &dirs.applications.join("Thing-2.AppImage")).unwrap();
        assert_eq!(std::fs::read_dir(&dirs.menu).unwrap().count(), 1);
        assert!(std::fs::read_to_string(&path).unwrap().contains("Thing-2.AppImage"));

        // Removing only ever touches entries the store wrote.
        let foreign = menu_entry_path(&dirs, "github:someone/else");
        std::fs::write(&foreign, "[Desktop Entry]\nName=Not ours\n").unwrap();
        remove_menu_entry(&dirs, "github:someone/else");
        assert!(foreign.exists());
        remove_menu_entry(&dirs, "github:Owner/Thing");
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod launch_tests {
    use super::*;

    #[test]
    fn a_program_that_closes_at_once_gives_its_reason() {
        assert!(
            why_it_closed("dlopen(): error loading libfuse.so.2\nAppImages require FUSE to run.").contains("libfuse2")
        );
        assert!(why_it_closed("FATAL:setuid_sandbox_host.cc(163)] The SUID sandbox helper binary was found, but is not configured correctly.").contains("sandbox"));
        let missing = why_it_closed(
            "app: error while loading shared libraries: libwebkit2gtk-4.1.so.0: cannot open shared object file",
        );
        assert!(missing.contains("libwebkit2gtk-4.1.so.0"), "{missing}");
        assert_eq!(why_it_closed("first\nsecond line\n\n"), "It closed straight away. It said: second line");
        assert_eq!(why_it_closed(""), "It closed straight away without saying why.");
    }

    #[cfg(unix)]
    #[test]
    fn starting_is_watched_for_a_moment() {
        use std::os::unix::fs::PermissionsExt;
        let _one_at_a_time = super::SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("bilby-launch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        let watch = std::time::Duration::from_millis(1500);
        let broken = script("broken", "echo 'AppImages require FUSE to run.' >&2; exit 127");
        assert!(launch_watched(&broken, &[], watch).unwrap_err().to_string().contains("FUSE"));
        // One that stays up, and one that finishes cleanly, both count as started.
        assert!(launch_watched(&script("stays", "sleep 3"), &[], std::time::Duration::from_millis(300)).is_ok());
        assert!(launch_watched(&script("done", "exit 0"), &[], watch).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Tests that write a script and start it take turns. A script still open for writing
/// in one test, when another test starts a program, cannot be started ("text file
/// busy"): the new process holds the open file for a moment.
#[cfg(all(test, unix))]
static SCRIPTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(all(test, unix))]
mod unpack_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in AppImage: a script that unpacks itself the way a real one does.
    fn stand_in(dir: &Path, name: &str, works: bool) -> PathBuf {
        let body = if works {
            "#!/bin/sh\nif [ \"$1\" = \"--appimage-extract\" ]; then mkdir -p squashfs-root/usr/share; printf '#!/bin/sh\\necho started\\n' > squashfs-root/AppRun; chmod 644 squashfs-root/AppRun; printf '\\211PNG....' > squashfs-root/usr/share/app.png; ln -s usr/share/app.png squashfs-root/.DirIcon; exit 0; fi\nexit 1\n"
        } else {
            "#!/bin/sh\nexit 1\n"
        };
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_program_knows_where_it_lives_and_is_not_told_by_whoever_started_it() {
        let root = std::env::temp_dir().join(format!("bilby-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join("Downloads").join("bilby-old.AppImage");
        let mounted = root.join("mount_old");
        let unpacked = root.join("Applications").join("bilby-new.AppDir");
        for folder in [old.parent().unwrap(), &mounted.join("usr/bin"), &unpacked.join("usr/bin")] {
            std::fs::create_dir_all(folder).unwrap();
        }
        for file in [&old, &mounted.join("usr/bin/bilby"), &unpacked.join("usr/bin/bilby"), &unpacked.join("AppRun")] {
            std::fs::write(file, b"x").unwrap();
        }
        // Running out of the AppImage that is named: that file.
        assert_eq!(own_program(&mounted.join("usr/bin/bilby"), Some(&old), Some(&mounted)), Some(old.clone()));
        // An unpacked copy started by the old AppImage, which left its own name behind:
        // it lives in its folder, not in the file that started it.
        let exe = unpacked.join("usr/bin/bilby");
        assert_eq!(own_program(&exe, Some(&old), Some(&mounted)), Some(unpacked.join("AppRun")));
        assert_eq!(own_program(&exe, None, None), Some(unpacked.join("AppRun")));
        // A plain program somewhere: nowhere in particular.
        assert_eq!(own_program(&root.join("bin/bilby"), None, None), None);
        assert_eq!(own_program(&root.join("bin/bilby"), Some(&old), Some(&mounted)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_is_started_is_not_told_it_is_the_stores_appimage() {
        let _one_at_a_time = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("bilby-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("says");
        std::fs::write(&script, format!("#!/bin/sh\nenv > '{}/env'\n", dir.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = std::process::Command::new(&script);
        command.env("APPIMAGE", "/somewhere/store.AppImage").env("APPDIR", "/tmp/.mount_store").env("KEPT", "yes");
        not_ours(&mut command);
        assert!(not_while_busy(|| command.status()).unwrap().success());
        let env = std::fs::read_to_string(dir.join("env")).unwrap();
        assert!(!env.contains("APPIMAGE=") && !env.contains("APPDIR="), "{env}");
        assert!(env.contains("KEPT=yes"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_appimage_is_unpacked_into_a_folder_that_runs_without_fuse() {
        let _one_at_a_time = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("bilby-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = Dirs {
            applications: root.join("Applications"),
            cache: root.join("cache"),
            menu: root.join("menu"),
            ..Dirs::for_current_user()
        };
        std::fs::create_dir_all(&dirs.applications).unwrap();
        let appimage = stand_in(&dirs.applications, "Gauge-2.1.0-x86_64.AppImage", true);

        let program = unpack_appimage(&appimage).unwrap();
        assert_eq!(program, dirs.applications.join("Gauge-2.1.0-x86_64.AppDir").join("AppRun"));
        assert!(program.is_file() && !appimage.exists(), "the folder takes the file's place");
        assert_ne!(std::fs::metadata(&program).unwrap().permissions().mode() & 0o111, 0, "and can be started");
        assert!(!dirs.applications.join(".unpacking-Gauge-2.1.0-x86_64").exists(), "nothing is left lying about");
        assert_eq!(unpacked_folder(&program), Some(dirs.applications.join("Gauge-2.1.0-x86_64.AppDir")));
        assert_eq!(unpacked_folder(&dirs.applications.join("x.AppImage")), None);
        // It starts like any program.
        assert!(launch_watched(&program, &[], std::time::Duration::from_millis(800)).is_ok());

        // Its menu entry starts the folder's program and carries its icon.
        let entry = add_menu_entry(&dirs, "own:workshop/gauge", "Gauge", &program).unwrap();
        let text = std::fs::read_to_string(entry).unwrap();
        assert!(text.contains("Gauge-2.1.0-x86_64.AppDir/AppRun"), "{text}");
        // Icons are taken out of the app only on Linux, where menu entries are used.
        assert!(text.contains("Icon=") || !cfg!(target_os = "linux"), "{text}");

        // Removing it removes the whole folder, and nothing outside Applications.
        assert!(!remove_placed(&dirs, &root.join("elsewhere").join("AppRun")));
        assert!(remove_placed(&dirs, &program));
        assert!(!dirs.applications.join("Gauge-2.1.0-x86_64.AppDir").exists());

        // One that cannot unpack itself is left as it was, with the reason.
        let stubborn = stand_in(&dirs.applications, "Old.AppImage", false);
        assert!(unpack_appimage(&stubborn).unwrap_err().to_string().contains("could not be unpacked"));
        assert!(stubborn.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(all(test, unix))]
mod unpack_for_real {
    /// Not a check that runs by itself: unpacks a real AppImage named in
    /// `BILBY_APPIMAGE` and prints the program to start.
    #[test]
    #[ignore]
    fn unpack_the_named_appimage() {
        let Ok(path) = std::env::var("BILBY_APPIMAGE") else { return };
        let program = super::unpack_appimage(std::path::Path::new(&path)).unwrap();
        println!("PROGRAM {}", program.display());
    }
}
