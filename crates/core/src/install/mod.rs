//! What "install" means for a given file on a given machine.
//!
//! [`plan`] is pure: it describes the steps, so the UI can show them before anything is
//! downloaded. [`execute`] carries the plan out.
//!
//! The store deliberately does not run anything as root itself. Native packages are
//! handed to the system's own installer (which asks for the password); AUR builds are
//! shown as a command for the person to run after reading the PKGBUILD.
//!
//! The rest of the module is what an install is made of: the folders it uses
//! ([`Dirs`]), menu entries, starting programs, Flatpak, AppImages without FUSE, and
//! removing what was installed.

use crate::asset::{AssetInfo, Format};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::Asset;
use crate::platform::{LinuxFamily, Platform};
use std::path::{Path, PathBuf};

mod appimage;
mod browser;
mod dirs;
mod flatpak;
mod launch;
mod menu;
mod remove;

pub use appimage::*;
pub use browser::*;
pub use dirs::*;
pub use flatpak::*;
pub use launch::*;
pub use menu::*;
pub use remove::*;

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
    /// Nothing is downloaded by the store; the person runs this command. Only a last
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
    /// Set a Windows installer up to run on Linux through Proton (see `winrun`).
    WindowsApp,
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
        Format::Exe if p.runs_windows() => {
            steps.push(
                "The first time, Bilby fetches what Windows programs need to run here (about 700 MB, and 2 GB of space)".to_string(),
            );
            steps.push("Bilby runs the installer for you, in a folder of the app's own".to_string());
            steps.push("The app is added to your apps, and removing it removes everything it brought".to_string());
            (Action::WindowsApp, "Install", None)
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
                "ms-windows-store" => ("the Microsoft Store", "Get from the Microsoft Store"),
                "https" if asset.url.starts_with("https://www.gog.com/") => ("GOG's website", "Get it on GOG"),
                "https" if asset.url.starts_with("https://play.google.com/") => {
                    ("Google Play", "Get it on Google Play")
                }
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

/// Does this step fetch a file into the downloads folder? Package managers, Flatpak,
/// other stores and web apps do not: they have their own places, or none.
pub fn saves_a_file(action: &Action) -> bool {
    matches!(
        action,
        Action::OpenFile | Action::PlaceAppImage | Action::Reveal | Action::AndroidInstaller | Action::WindowsApp
    )
}

/// Can this step finish with nobody there? Putting an AppImage in place, Flatpak, a
/// web app, and a command that needs no password can. An installer that opens, a
/// terminal, another store and anything run as administrator need the person.
pub fn unattended(plan: &InstallPlan) -> bool {
    match plan.action {
        Action::PlaceAppImage | Action::Flatpak | Action::FlatpakBundle | Action::WebApp => true,
        Action::RunCommand => {
            plan.command.as_deref().is_some_and(|command| command.split_whitespace().next() != Some("sudo"))
        }
        _ => false,
    }
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
        } else if asset.url.starts_with("ms-windows-store://") {
            "The Microsoft Store"
        } else if asset.url.starts_with("https://play.google.com/") {
            "Google Play"
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
        let install = |place: &[&str], progress: &mut dyn FnMut(u64, Option<u64>) -> bool| {
            let mut args = vec!["install"];
            args.extend_from_slice(place);
            // Not "--noninteractive": that also silences how far it has got. Nothing
            // is asked all the same: every question is answered with yes.
            args.extend_from_slice(&["--assumeyes", "--or-update", "flathub", id]);
            run_flatpak(dirs, &args, progress)
        };
        match install(&[], progress) {
            // Flathub is set up twice on this device, for everyone and for this person
            // alone (installing a Flatpak file for one person adds the second), and
            // Flatpak will not choose between them unasked. For this person, then:
            // that never needs a password.
            Err(Error::Unsupported(why)) if why.contains("multiple installations") => install(&["--user"], progress)?,
            done => done?,
        }
        return Ok(Outcome {
            message: "Installed from Flathub. It is in your app menu.".to_string(),
            file: None,
            store_managed: false,
            flatpak_app: None,
        });
    }
    // On a computer or phone a web app is a shortcut to the site in the usual browser.
    if plan.action == Action::WebApp && !asset.info.as_ref().is_some_and(|info| info.tv) {
        // The site itself, whichever of its pages the app opens on.
        let host = asset.url.trim_start_matches("https://").split('/').next().unwrap_or_default();
        let entry = write_web_shortcut(dirs, &format!("web:{host}"), &asset.name, &asset.url)?;
        let how = match find_app_browser(dirs) {
            Some(_) => "It opens in a window of its own.",
            None => "It opens in your browser.",
        };
        return Ok(Outcome {
            message: format!("{} is among your apps. {how}", asset.name),
            file: Some(entry),
            store_managed: false,
            flatpak_app: None,
        });
    }
    if plan.action == Action::WebApp {
        // The site itself, whichever of its pages the app opens on.
        let host = asset.url.trim_start_matches("https://").split('/').next().unwrap_or_default();
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
        Action::WindowsApp => {
            let launcher = crate::winrun::install(http, dirs, &asset.url, &dest, progress);
            // The installer has done its work, or could not: either way it is not kept.
            let _ = std::fs::remove_file(&dest);
            Ok(Outcome {
                message: format!("{verified}Set up to run through Proton."),
                file: Some(launcher?),
                store_managed: true,
                flatpak_app: None,
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
mod tests;
