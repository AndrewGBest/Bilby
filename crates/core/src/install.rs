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
    /// Nothing is downloaded by the store; the user runs this command.
    ShowCommand,
    /// Put a website in the app list, to open in Longboard.
    WebApp,
    /// Have Flatpak install the app from Flathub.
    Flatpak,
    /// Have Flatpak install a downloaded `.flatpak` bundle.
    FlatpakBundle,
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
                verb: "Add to this TV".to_string(),
                steps: vec![
                    format!("Put {} in this TV's app list", asset.name),
                    "It opens in Longboard, a browser made for remotes".to_string(),
                    "Nothing is downloaded; remove it from your library at any time".to_string(),
                ],
                command: None,
            };
        }
        Format::RepoPackage => {
            let name = asset.name.split_whitespace().next().unwrap_or(&asset.name).to_string();
            let pacman = p.linux.as_ref().is_some_and(|l| l.pacman);
            let (manager, command) = if pacman {
                ("pacman", format!("sudo pacman -S {name}"))
            } else {
                ("apt", format!("sudo apt install {name}"))
            };
            let mut steps = vec![
                format!("{manager} downloads it from your configured repositories and checks its signature"),
                "Run the command below; it asks for your password".to_string(),
                format!("From then on {manager} keeps it up to date with the rest of the system"),
            ];
            if pacman {
                // Arch-based systems do not support upgrading single packages.
                steps.push("If it is already installed, update the whole system instead: sudo pacman -Syu".to_string());
            }
            return InstallPlan {
                action: Action::ShowCommand,
                verb: format!("Install with {manager}"),
                steps,
                command: Some(command),
            };
        }
        Format::AurBuild => {
            let name = asset.name.split_whitespace().next().unwrap_or(&asset.name).to_string();
            let command = match p.linux.as_ref().and_then(|l| l.aur_helper.clone()) {
                Some(helper) => format!("{helper} -S {name}"),
                None => format!("git clone https://aur.archlinux.org/{name}.git && cd {name} && makepkg -si"),
            };
            steps = vec![
                "Read the PKGBUILD: AUR recipes are user-submitted and run on your machine".to_string(),
                "Build and install with the command below".to_string(),
            ];
            (Action::ShowCommand, "Build from AUR", Some(command))
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
    write_menu_entry(dirs, id, name, appimage, &[], "Utility;")
}

/// Longboard, when it is on this device: the copy the store placed, or one on the PATH.
pub fn find_longboard(dirs: &Dirs) -> Option<PathBuf> {
    let placed =
        std::fs::read_dir(&dirs.applications).ok().into_iter().flatten().flatten().map(|e| e.path()).find(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            name.starts_with("longboard") && name.ends_with(".appimage")
        });
    placed.or_else(|| {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).map(|dir| dir.join("longboard")).find(|p| p.is_file())
    })
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
    use std::process::{Command, Stdio};
    let mut child = Command::new(&dirs.flatpak)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Unsupported(format!("Flatpak could not be started ({e}). Is it installed?")))?;
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => {
                let mut said = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut said);
                }
                // Flatpak's own last line is the most useful thing to show.
                let why =
                    said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it gave no reason").trim().to_string();
                return Err(Error::Unsupported(format!("Flatpak could not do it: {why}")));
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

/// Start a program the store placed (an AppImage), without waiting for it.
pub fn launch(path: &Path) -> Result<()> {
    launch_with(path, &[])
}

/// Start a program with arguments, without waiting for it.
pub fn launch_with(path: &Path, args: &[String]) -> Result<()> {
    use std::process::{Command, Stdio};
    Command::new(path).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
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
        return Ok(Outcome { message: format!("Run in a terminal: {command}"), file: None, store_managed: false });
    }

    if plan.action == Action::Flatpak {
        let id = asset.name.as_str();
        run_flatpak(dirs, &["install", "--noninteractive", "--assumeyes", "--or-update", "flathub", id], progress)?;
        return Ok(Outcome {
            message: "Installed from Flathub. It is in your app menu.".to_string(),
            file: None,
            store_managed: false,
        });
    }
    if plan.action == Action::WebApp {
        let host = asset.url.trim_start_matches("https://").trim_end_matches('/');
        let longboard = find_longboard(dirs).ok_or_else(|| {
            Error::Unsupported(
                "Web apps open in Longboard, a browser for TVs, and it is not installed yet. Install Longboard from this store, then add this again.".to_string(),
            )
        })?;
        let args = [format!("--url={}", asset.url)];
        let entry = write_menu_entry(dirs, &format!("web:{host}"), &asset.name, &longboard, &args, "Network;")?;
        return Ok(Outcome {
            message: format!("{} is in this TV's app list. It opens in Longboard.", asset.name),
            file: Some(entry),
            store_managed: false,
        });
    }

    let dest = dirs.downloads.join(safe_file_name(&asset.name));
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
            Ok(Outcome { message: format!("{verified}Installed to {}", target.display()), file: Some(target), store_managed: true })
        }
        Action::OpenFile => {
            // The download succeeded either way; a missing handler is not a failure.
            let message = match open(&dest) {
                Ok(()) => format!("{verified}Opened {} with your system installer", dest.display()),
                Err(_) => format!("{verified}Saved to {}. Open it to finish installing.", dest.display()),
            };
            Ok(Outcome { message, file: Some(dest), store_managed: false })
        }
        Action::Reveal => {
            // Failing to open a file manager is not a failed download.
            let _ = open(&dirs.downloads);
            Ok(Outcome { message: format!("{verified}Saved to {}", dest.display()), file: Some(dest), store_managed: false })
        }
        Action::AndroidInstaller => Err(Error::Unsupported(format!(
            "{verified}Saved to {}. Handing APKs to Android's installer is not implemented in this build yet; open the file to install it.",
            dest.display()
        ))),
        Action::FlatpakBundle => {
            let file = dest.display().to_string();
            run_flatpak(dirs, &["install", "--user", "--noninteractive", "--assumeyes", &file], progress)?;
            let _ = std::fs::remove_file(&dest);
            Ok(Outcome { message: format!("{verified}Installed with Flatpak. It is in your app menu."), file: None, store_managed: false })
        }
        Action::ShowCommand | Action::WebApp | Action::Flatpak => unreachable!("handled above"),
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
    fn the_store_puts_itself_in_the_app_menu_once_and_follows_its_file() {
        let dirs = temp_dirs("self");
        let program = Path::new("/home/someone/My Apps/bilby-v0.1.3-x86_64.AppImage");
        assert!(register_self(&dirs, program, b"png").unwrap(), "the first run writes the entry");
        let entry = std::fs::read_to_string(self_entry_path(&dirs)).unwrap();
        assert!(entry.contains("\nName=Bilby\n"));
        assert!(entry.contains("\nExec=\"/home/someone/My Apps/bilby-v0.1.3-x86_64.AppImage\"\n"), "{entry}");
        let icon = dirs.menu.parent().unwrap().join("icons/hicolor/512x512/apps/bilby.png");
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
        assert_eq!(arch.action, Action::ShowCommand);
        assert_eq!(arch.command.as_deref(), Some("sudo pacman -S linux-cachyos"));
        assert_eq!(arch.verb, "Install with pacman");
        let debian =
            plan(&asset("ripgrep (noble/universe)", None), &info, &Platform::preset("linux-x64-debian").unwrap());
        assert_eq!(debian.command.as_deref(), Some("sudo apt install ripgrep"));
        // Nothing is downloaded by the store for these.
        let dirs = temp_dirs("repo");
        let out = execute(&arch, &a, &Fake(b""), &dirs, &|_| unreachable!(), &mut |_, _| true).unwrap();
        assert_eq!(out.message, "Run in a terminal: sudo pacman -S linux-cachyos");
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
