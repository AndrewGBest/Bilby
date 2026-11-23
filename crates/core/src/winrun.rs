//! Windows programs on Linux, set up by the store so that they just run.
//!
//! A program made for Windows runs on Linux through Proton, Valve's build of Wine.
//! The store does not carry Proton itself. It fetches `umu-run`, the small launcher
//! Lutris and Heroic use (Open Wine Components, GPL-3.0, run as a program of its own),
//! and `umu-run` fetches Proton and the runtime it needs the first time it is used:
//! about 700 MB, once, shared by every Windows program.
//!
//! Each program gets a folder of its own under the store's Applications folder:
//!
//! ```text
//! Applications/Windows/.runner/umu-run      the launcher, checked against a pinned SHA-256
//! Applications/Windows/<app>/prefix/        its own Windows: nothing is shared between apps
//! Applications/Windows/<app>/run            what starts it; the app menu's entry runs this
//! Applications/Windows/.settings            how Windows programs are run, in general
//! Applications/Windows/<app>/settings       and what this one program does differently
//! ```
//!
//! so removing the program is removing its folder. Installing is: fetch the launcher
//! if it is not there, run the program's installer in its folder (without questions,
//! where the kind of installer is one that can be told to ask none), find what it
//! installed, and write `run`.
//!
//! [Run for real on Arch Linux on 2026-11-23 with umu-launcher 1.4.4 and
//! UMU-Proton 11.0: Notepad++'s installer was fetched, installed without a question,
//! and the program started from its launcher. That first time took about 670 MB of
//! downloads and 2.1 GB of disk. One program on one machine with no graphics card:
//! games, and installers that ask questions, have not been tried.]

use crate::error::{Error, Result};
use crate::http::Http;
use crate::install::Dirs;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const UMU_VERSION: &str = "1.4.4";
pub const UMU_URL: &str =
    "https://github.com/Open-Wine-Components/umu-launcher/releases/download/1.4.4/umu-launcher-1.4.4-zipapp.tar";
/// The SHA-256 GitHub publishes for that file.
pub const UMU_SHA256: &str = "eb590691841f7fad3fc3ad8fd5db4ccb87849fe7948e62b28ece7a4ee48cc851";

/// The name of what starts an installed program, inside its folder.
const RUN: &str = "run";

/// Which Proton runs a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Proton {
    /// Not said: whatever the general choice is (and in general, the standard one).
    #[default]
    Unset,
    /// UMU-Proton, built from Valve's own: the steady choice.
    Standard,
    /// GE-Proton, a community build with newer fixes: some games need it.
    Ge,
}

/// How Windows programs are run: in general, or what one program does differently.
/// What a program does not say, it takes from the general choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Choices {
    pub proton: Proton,
    /// Show MangoHud's figures (frames a second, temperatures) over the program.
    pub overlay: Option<bool>,
    /// Start it through GameMode, which asks the system for its full speed.
    pub gamemode: Option<bool>,
}

/// Where the general choices are kept.
pub fn general_choices(dirs: &Dirs) -> PathBuf {
    root(dirs).join(".settings")
}

/// Where one program's own choices are kept.
pub fn app_choices(folder: &Path) -> PathBuf {
    folder.join("settings")
}

impl Choices {
    /// The choices as lines the launcher reads: a few fixed lines, each of which the
    /// store wrote whole, so nothing a person types ever lands in them.
    fn lines(&self) -> String {
        let mut text = String::from("# Written by Bilby. Change these in Bilby's Settings.\n");
        match self.proton {
            Proton::Unset => {}
            Proton::Standard => text.push_str("unset PROTONPATH\n"), // (not shown)
            Proton::Ge => text.push_str("export PROTONPATH=GE-Proton\n"),
        }
        match self.overlay {
            None => {}
            Some(true) => text.push_str("export MANGOHUD=1\n"),
            Some(false) => text.push_str("unset MANGOHUD\n"), // (not shown)
        }
        match self.gamemode {
            None => {}
            Some(true) => text.push_str("BILBY_GAMEMODE=1\n"),
            Some(false) => text.push_str("BILBY_GAMEMODE=\n"),
        }
        text
    }

    /// Read them back from such a file. Lines that are not the store's are ignored.
    pub fn read(file: &Path) -> Choices {
        let mut choices = Choices::default();
        for line in std::fs::read_to_string(file).unwrap_or_default().lines() {
            match line.trim() {
                "unset PROTONPATH" => choices.proton = Proton::Standard, // (not shown)
                "export PROTONPATH=GE-Proton" => choices.proton = Proton::Ge,
                "export MANGOHUD=1" => choices.overlay = Some(true),
                "unset MANGOHUD" => choices.overlay = Some(false), // (not shown)
                "BILBY_GAMEMODE=1" => choices.gamemode = Some(true),
                "BILBY_GAMEMODE=" => choices.gamemode = Some(false),
                _ => {}
            }
        }
        choices
    }

    /// Keep them. Choices that say nothing leave no file.
    pub fn write(&self, file: &Path) -> Result<()> {
        if *self == Choices::default() {
            let _ = std::fs::remove_file(file);
            return Ok(());
        }
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(file, self.lines())?;
        Ok(())
    }

    /// These, with whatever they leave unsaid taken from `general`.
    pub fn over(self, general: Choices) -> Choices {
        Choices {
            proton: if self.proton == Proton::Unset { general.proton } else { self.proton },
            overlay: self.overlay.or(general.overlay),
            gamemode: self.gamemode.or(general.gamemode),
        }
    }
}

/// Where Windows programs and what runs them are kept.
pub fn root(dirs: &Dirs) -> PathBuf {
    dirs.applications.join("Windows")
}

/// The folder of one program, named after where it comes from so that a newer
/// version of the same program lands in the same place.
pub fn folder_for(dirs: &Dirs, url: &str) -> PathBuf {
    let rest = url.split("://").nth(1).unwrap_or(url);
    // A release file on a code host is owner/repo; anything else, its site and file.
    let parts: Vec<&str> = rest.split(['/', '?', '#']).filter(|part| !part.is_empty()).collect();
    let named = match parts.as_slice() {
        [host, owner, repo, "releases", ..] | [host, owner, repo, "-", "releases", ..] => {
            format!("{host}-{owner}-{repo}")
        }
        [host, .., file] => format!("{host}-{}", file.rsplit_once('.').map_or(*file, |(stem, _)| stem)),
        _ => "app".to_string(),
    };
    let safe: String = named
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c.to_ascii_lowercase() } else { '-' })
        .take(80)
        .collect();
    root(dirs).join(safe.trim_matches(|c| c == '-' || c == '.'))
}

/// Is this file what starts a Windows program the store set up? Then its folder.
pub fn folder_of(dirs: &Dirs, program: &Path) -> Option<PathBuf> {
    let folder = program.parent()?;
    let ours = folder.parent()? == root(dirs)
        && program.file_name().is_some_and(|name| name == RUN)
        && !folder.file_name()?.to_string_lossy().starts_with('.');
    ours.then(|| folder.to_path_buf())
}

/// The launcher, if it is here already: the store's own copy, else one the system has.
pub fn find_runner(dirs: &Dirs) -> Option<PathBuf> {
    let own = root(dirs).join(".runner").join("umu-run");
    if own.is_file() {
        return Some(own);
    }
    dirs.programs.iter().map(|dir| dir.join("umu-run")).find(|path| path.is_file())
}

/// Can Windows programs be set up here at all? The launcher is a Python program.
pub fn python(dirs: &Dirs) -> Option<PathBuf> {
    dirs.programs.iter().map(|dir| dir.join("python3")).find(|path| path.is_file())
}

/// Fetch the launcher, check it is the file that was pinned, and put it in place.
pub fn fetch_runner(
    http: &dyn Http,
    dirs: &Dirs,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<PathBuf> {
    fetch_runner_from(http, dirs, UMU_URL, UMU_SHA256, progress)
}

pub fn fetch_runner_from(
    http: &dyn Http,
    dirs: &Dirs,
    url: &str,
    sha256: &str,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<PathBuf> {
    let home = root(dirs).join(".runner");
    std::fs::create_dir_all(&home)?;
    let parcel = home.join("umu.tar");
    let actual = http.download(url, &parcel, progress)?;
    if !actual.eq_ignore_ascii_case(sha256) {
        let _ = std::fs::remove_file(&parcel);
        return Err(Error::DigestMismatch { expected: sha256.to_string(), actual });
    }
    // One file is wanted out of it, by its exact name; nothing else is written.
    let mut found = None;
    let mut archive = tar::Archive::new(std::fs::File::open(&parcel)?);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new("umu/umu-run") && entry.header().entry_type().is_file() {
            let target = home.join("umu-run");
            let mut out = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
            found = Some(target);
            break;
        }
    }
    let _ = std::fs::remove_file(&parcel);
    let runner = found.ok_or_else(|| Error::Parse("The launcher was not in the file that was fetched.".to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(runner)
}

/// The kinds of installer that can be told to ask no questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// Nullsoft's (NSIS): `/S`.
    Nullsoft,
    /// Inno Setup: `/VERYSILENT` and friends.
    Inno,
    /// Something else: it is run as it is, and may show its own windows.
    Unknown,
}

impl Installer {
    /// What tells it to install without asking.
    pub fn quiet(self) -> &'static [&'static str] {
        match self {
            Installer::Nullsoft => &["/S"],
            Installer::Inno => &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"],
            Installer::Unknown => &[],
        }
    }
}

const INNO_MARK: &[u8] = b"Inno Setup Setup Data"; // (not shown)

/// Which kind of installer a file is, from the marks each kind leaves in it. Only
/// the first part of the file is read: the marks are in the program, not its cargo.
pub fn installer_kind(file: &Path) -> Installer {
    use std::io::Read;
    let mut head = Vec::new();
    let Ok(opened) = std::fs::File::open(file) else { return Installer::Unknown };
    if opened.take(8 * 1024 * 1024).read_to_end(&mut head).is_err() {
        return Installer::Unknown;
    }
    let has = |mark: &[u8]| head.windows(mark.len()).any(|window| window == mark);
    if has(b"Nullsoft.NSIS") || has(b"NullsoftInst") {
        Installer::Nullsoft
    } else if has(INNO_MARK) || has(b"JR.Inno.Setup") {
        Installer::Inno
    } else {
        Installer::Unknown
    }
}

/// Every program in the places an installer puts one, inside a program's own Windows.
pub fn programs_in(prefix: &Path) -> HashSet<PathBuf> {
    fn walk(folder: &Path, depth: usize, found: &mut HashSet<PathBuf>) {
        // What every Windows has from the start is nobody's app.
        // Folder names (not shown).
        const NOT_APPS: &[&str] = &[
            "Internet Explorer",    // (not shown)
            "Windows Media Player", // (not shown)
            "Windows NT",           // (not shown)
            "Common Files",         // (not shown)
        ];
        let Ok(entries) = std::fs::read_dir(folder) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if depth > 0 && !NOT_APPS.contains(&name.as_str()) {
                    walk(&path, depth - 1, found);
                }
            } else if name.to_ascii_lowercase().ends_with(".exe") {
                found.insert(path);
            }
        }
    }
    let drive = prefix.join("drive_c");
    let mut found = HashSet::new();
    const PLACES: [&str; 2] = [
        "Program Files",       // (not shown)
        "Program Files (x86)", // (not shown)
    ];
    for place in PLACES {
        walk(&drive.join(place), 3, &mut found);
    }
    // Programs that install for one person only.
    if let Ok(people) = std::fs::read_dir(drive.join("users")) {
        for person in people.flatten() {
            walk(&person.path().join("AppData").join("Local").join("Programs"), 3, &mut found);
        }
    }
    found
}

/// The program an installer installed: of what is there now and was not before, the
/// one that is the app and not its helpers. Nearest the top of its folder first, then
/// the one whose name is most like the installer's, then the largest.
pub fn installed_program(before: &HashSet<PathBuf>, now: &HashSet<PathBuf>, installer: &str) -> Option<PathBuf> {
    const HELPERS: &[&str] = &["unins", "setup", "update", "crash", "helper", "redist", "install", "elevat", "report"];
    let squash =
        |text: &str| -> String { text.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_lowercase() };
    // "7z2409-x64" is "7z": the letters and digits before the version begins.
    let wanted: String = {
        let stem = installer.rsplit_once('.').map_or(installer, |(stem, _)| stem).to_lowercase();
        let lead: String = stem.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        if lead.len() >= 2 {
            lead
        } else {
            squash(&stem).chars().take(2).collect()
        }
    };
    let mut fresh: Vec<&PathBuf> = now
        .iter()
        .filter(|path| !before.contains(*path))
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            !HELPERS.iter().any(|helper| name.contains(helper))
        })
        .collect();
    fresh.sort_by_key(|path| {
        let name = squash(&path.file_stem().unwrap_or_default().to_string_lossy());
        let alike = !wanted.is_empty() && (name.starts_with(&wanted) || wanted.starts_with(&name));
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        (path.components().count(), !alike, std::cmp::Reverse(size), path.to_path_buf())
    });
    fresh.first().map(|path| path.to_path_buf())
}

/// What a run of the launcher is told: whose Windows this is, and to apply no
/// game-specific fixes (this is a program, not a game the fixes know).
fn told(prefix: &Path) -> [(&'static str, String); 2] {
    [("WINEPREFIX", prefix.display().to_string()), ("GAMEID", "umu-default".to_string())]
}

/// The Proton a folder's programs run on, as the launcher is told it: nothing for
/// the standard one. Read from the general choices and the program's own.
fn proton_for(folder: &Path) -> Option<&'static str> {
    let general = folder.parent().map(|root| Choices::read(&root.join(".settings"))).unwrap_or_default();
    match Choices::read(&app_choices(folder)).over(general).proton {
        Proton::Ge => Some("GE-Proton"),
        _ => None,
    }
}

/// Open one of Windows' own tools in a program's Windows, without waiting for it:
/// `winecfg`, its settings. Only tools the store names.
pub fn open_tool(dirs: &Dirs, folder: &Path, tool: &str) -> Result<()> {
    if tool != "winecfg" {
        return Err(Error::Unsupported(format!("\"{tool}\" is not a tool this opens")));
    }
    let runner = find_runner(dirs)
        .ok_or_else(|| Error::Unsupported("What runs Windows programs is not on this device.".to_string()))?;
    let prefix = folder.join("prefix");
    let mut command = std::process::Command::new(runner);
    command.arg(tool).envs(told(&prefix)).stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(proton) = proton_for(folder) {
        command.env("PROTONPATH", proton);
    }
    command.spawn().map(|_| ()).map_err(|e| Error::Unsupported(format!("It could not be opened: {e}")))
}

/// Run a Windows program and wait for it, giving up (and stopping it) when
/// `keep_going` says so. What it printed last is kept for the reason it failed.
pub fn run(
    runner: &Path,
    prefix: &Path,
    program: &Path,
    args: &[&str],
    keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<()> {
    run_until(runner, prefix, program, args, keep_going, &mut || false)
}

/// Ask what is running Windows programs to stop, and make it if it will not: asked,
/// it takes everything it started with it.
fn stop(child: &mut std::process::Child) {
    let asked = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(if asked { 10 } else { 0 });
    while std::time::Instant::now() < until {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The same, and also done when `finished` says the program's work is over although
/// something it started is still running (an installer that leaves a window of
/// Windows' own open behind it would otherwise be waited for forever).
pub fn run_until(
    runner: &Path,
    prefix: &Path,
    program: &Path,
    args: &[&str],
    keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool,
    finished: &mut dyn FnMut() -> bool,
) -> Result<()> {
    std::fs::create_dir_all(prefix)?;
    let log = prefix.with_extension("log");
    let said = std::fs::File::create(&log)?;
    let mut child = std::process::Command::new(runner)
        .arg(program)
        .args(args)
        .envs(told(prefix))
        // The same Proton it will run on afterwards.
        .envs(prefix.parent().and_then(proton_for).map(|proton| ("PROTONPATH", proton)))
        .stdin(std::process::Stdio::null())
        .stdout(said.try_clone()?)
        .stderr(said)
        .spawn()
        .map_err(|e| Error::Unsupported(format!("What runs Windows programs could not be started: {e}")))?;
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => {
                let text = std::fs::read_to_string(&log).unwrap_or_default();
                let last = text.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("it said nothing");
                let last: String = last.chars().take(200).collect();
                return Err(Error::Unsupported(format!("The Windows program did not finish: {last}")));
            }
            None if !keep_going(0, None) => {
                stop(&mut child);
                return Err(Error::Cancelled);
            }
            None if finished() => {
                stop(&mut child);
                return Ok(());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
}

/// Write what starts an installed program: a small script, so that the app menu, the
/// store's Open button and a terminal all start it the same way.
pub fn write_launcher(folder: &Path, runner: &Path, program: &Path) -> Result<PathBuf> {
    // Inside single quotes nothing has a meaning but the quote itself.
    let quoted = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
    let prefix = folder.join("prefix");
    let mut script = String::from("#!/bin/sh\n# Written by Bilby: starts a Windows program through Proton.\n");
    // How it is run: the general choices first, then this program's own over them.
    // Each time it starts, so that a change in Settings needs nothing rewritten.
    script.push_str("BILBY_GAMEMODE=\n");
    let general = folder.parent().map(|root| root.join(".settings")).unwrap_or_default();
    for choices in [general, app_choices(folder)] {
        script.push_str(&format!("[ -r {0} ] && . {0}\n", quoted(&choices)));
    }
    for (name, value) in told(&prefix) {
        script.push_str(&format!("export {name}={}\n", quoted(Path::new(&value))));
    }
    let start = format!("{} {} \"$@\"", quoted(runner), quoted(program));
    script.push_str("if [ \"$BILBY_GAMEMODE\" = 1 ] && command -v gamemoderun >/dev/null 2>&1; then\n");
    script.push_str(&format!("    exec gamemoderun {start}\nfi\n"));
    script.push_str(&format!("exec {start}\n"));
    let path = folder.join(RUN);
    std::fs::write(&path, script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// How often an installer's folder is looked at, and how long nothing may change in
/// it before the installing is taken to be over.
const SETTLE_LOOK: std::time::Duration = std::time::Duration::from_secs(5);
const SETTLED_AFTER: std::time::Duration = std::time::Duration::from_secs(45);

/// How much is in a folder: the sizes of its files and how many there are, as one
/// number that changes when anything is written. Links are not followed.
fn written(folder: &Path, depth: usize) -> u64 {
    if depth > 24 {
        return 0;
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            total = total.wrapping_add(written(&entry.path(), depth + 1));
        } else if kind.is_file() {
            total = total.wrapping_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0)).wrapping_add(1);
        }
    }
    total
}

/// Set a downloaded Windows installer up as an app: the whole of it, start to finish.
/// Returns what starts the app.
pub fn install(
    http: &dyn Http,
    dirs: &Dirs,
    url: &str,
    installer: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<PathBuf> {
    if python(dirs).is_none() && find_runner(dirs).is_none() {
        return Err(Error::Unsupported(
            "Windows programs need Python 3 to be set up here, and it is not on this device.".to_string(),
        ));
    }
    let runner = match find_runner(dirs) {
        Some(runner) => runner,
        None => fetch_runner(http, dirs, progress)?,
    };
    let folder = folder_for(dirs, url);
    let prefix = folder.join("prefix");
    std::fs::create_dir_all(&prefix)?;
    let before = programs_in(&prefix);
    let kind = installer_kind(installer);
    let name = installer.file_name().unwrap_or_default().to_string_lossy().to_string();
    // Some installers finish their work and leave something of Windows' own running,
    // which would be waited for forever. Once a program has appeared and nothing has
    // been written for a while, the installing is over.
    let mut last_look = std::time::Instant::now();
    let mut still: Option<(u64, std::time::Instant)> = None;
    let mut finished = || {
        if last_look.elapsed() < SETTLE_LOOK {
            return false;
        }
        last_look = std::time::Instant::now();
        if installed_program(&before, &programs_in(&prefix), &name).is_none() {
            still = None;
            return false;
        }
        let size = written(&prefix.join("drive_c"), 0);
        match still {
            Some((same, since)) if same == size => since.elapsed() >= SETTLED_AFTER,
            _ => {
                still = Some((size, std::time::Instant::now()));
                false
            }
        }
    };
    run_until(&runner, &prefix, installer, kind.quiet(), progress, &mut finished)?;
    let program = match installed_program(&before, &programs_in(&prefix), &name) {
        Some(program) => program,
        // Nothing was installed: the file is the program itself. It is kept in its
        // folder, since the downloads folder is the person's to tidy.
        None => {
            let kept = folder.join(&name);
            std::fs::copy(installer, &kept)?;
            kept
        }
    };
    write_launcher(&folder, &runner, &program)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("bilby-winrun-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Dirs {
            downloads: root.join("dl"),
            applications: root.join("apps"),
            menu: root.join("menu"),
            flatpak: root.join("flatpak-program"),
            elevate: root.join("elevate-program"),
            terminal: None,
            flatpak_roots: vec![],
            programs: Vec::new(),
            cache: root.join("cache"),
            config: root.join("config"),
            desktop: None,
        }
    }

    #[test]
    fn a_program_has_one_folder_whatever_its_version() {
        let dirs = dirs("folders");
        let one = folder_for(
            &dirs,
            "https://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v8.9/npp.8.9.Installer.x64.exe",
        );
        let next = folder_for(
            &dirs,
            "https://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v9.0/npp.9.0.Installer.x64.exe",
        );
        assert_eq!(one, next);
        assert_eq!(one, root(&dirs).join("github.com-notepad-plus-plus-notepad-plus-plus"));
        // Nothing in an address can lead out of the folder.
        let odd = folder_for(&dirs, "https://example.org/../../etc/Some App (1).exe?x=../y");
        assert_eq!(odd.parent().unwrap(), root(&dirs));
        assert!(!odd.file_name().unwrap().to_string_lossy().contains(".."), "{odd:?}");
        assert_eq!(folder_of(&dirs, &one.join("run")), Some(one.clone()));
        assert_eq!(folder_of(&dirs, &one.join("prefix").join("run")), None);
        assert_eq!(folder_of(&dirs, &root(&dirs).join(".runner").join("run")), None);
        assert_eq!(folder_of(&dirs, &dirs.applications.join("run")), None);
    }

    #[test]
    #[cfg(unix)]
    fn a_program_runs_as_the_general_choices_say_unless_it_says_otherwise() {
        use std::os::unix::fs::PermissionsExt;
        let dirs = dirs("choices");
        let folder = root(&dirs).join("thing");
        std::fs::create_dir_all(&folder).unwrap();
        // A stand-in for the launcher that writes down how it was run.
        let fake = root(&dirs).join("umu-run");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho \"proton=${PROTONPATH:-standard} hud=${MANGOHUD:-0} $@\" > \"$WINEPREFIX/../ran\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(folder.join("prefix")).unwrap();
        let launcher = write_launcher(&folder, &fake, &folder.join("prefix").join("thing.exe")).unwrap();
        let ran = || {
            // Whatever the person's own shell has set is not what decides.
            let status = std::process::Command::new(&launcher).env("PROTONPATH", "something-else").status().unwrap();
            assert!(status.success());
            std::fs::read_to_string(folder.join("ran")).unwrap()
        };
        let (general, own) = (general_choices(&dirs), app_choices(&folder));

        // Nothing chosen: no file is left, and it runs as it always did.
        Choices::default().write(&general).unwrap();
        assert!(!general.exists());
        assert!(ran().starts_with("proton=something-else hud=0"), "nothing said, nothing changed");
        // In general: GE-Proton, with the figures shown.
        let wanted = Choices { proton: Proton::Ge, overlay: Some(true), gamemode: None };
        wanted.write(&general).unwrap();
        assert_eq!(Choices::read(&general), wanted);
        assert!(ran().starts_with("proton=GE-Proton hud=1"));
        // This program alone: the standard Proton and no figures.
        let mine = Choices { proton: Proton::Standard, overlay: Some(false), gamemode: Some(true) };
        mine.write(&own).unwrap();
        assert_eq!(Choices::read(&own), mine);
        assert!(ran().starts_with("proton=standard hud=0"));
        assert_eq!(Choices::default().over(wanted), wanted);
        assert_eq!(mine.over(wanted), mine);
        assert_eq!(proton_for(&folder), None);
        std::fs::remove_file(&own).unwrap();
        assert_eq!(proton_for(&folder), Some("GE-Proton"));
        // A line that is not the store's changes nothing when read back.
        std::fs::write(&own, "rm -rf /\nexport PROTONPATH=/somewhere\nexport MANGOHUD=1\n").unwrap();
        assert_eq!(Choices::read(&own), Choices { overlay: Some(true), ..Choices::default() });
        assert!(open_tool(&dirs, &folder, "regedit; rm -rf /").is_err());
    }

    #[test]
    fn installers_that_can_be_quiet_are_known_by_their_marks() {
        let dirs = dirs("kinds");
        std::fs::create_dir_all(&dirs.cache).unwrap();
        let file = |name: &str, body: &[u8]| {
            let path = dirs.cache.join(name);
            std::fs::write(&path, body).unwrap();
            path
        };
        assert_eq!(installer_kind(&file("a.exe", b"MZ....Nullsoft.NSIS.exehead....")), Installer::Nullsoft);
        assert_eq!(installer_kind(&file("b.exe", b"MZ....Inno Setup Setup Data (6.2.0)....")), Installer::Inno);
        assert_eq!(installer_kind(&file("c.exe", b"MZ....just a program....")), Installer::Unknown);
        assert_eq!(installer_kind(&dirs.cache.join("missing.exe")), Installer::Unknown);
        assert_eq!(Installer::Nullsoft.quiet(), ["/S"]);
        assert!(Installer::Inno.quiet().contains(&"/VERYSILENT") && Installer::Unknown.quiet().is_empty());
    }

    #[test]
    fn the_app_is_told_from_its_helpers() {
        let dirs = dirs("programs");
        let prefix = dirs.cache.join("prefix");
        let put = |path: &str, size: usize| {
            let full = prefix.join("drive_c").join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, vec![0u8; size]).unwrap();
            full
        };
        put("Program Files/Internet Explorer/iexplore.exe", 10);
        put("windows/notepad.exe", 10);
        let before = programs_in(&prefix);
        assert!(before.is_empty(), "what Windows starts with is nobody's app: {before:?}");
        let app = put("Program Files/7-Zip/7zFM.exe", 900);
        put("Program Files/7-Zip/7z.exe", 500);
        put("Program Files/7-Zip/7zG.exe", 600);
        put("Program Files/7-Zip/Uninstall.exe", 5000);
        put("Program Files/7-Zip/Lang/extra/deep/tool.exe", 9000);
        let now = programs_in(&prefix);
        assert_eq!(installed_program(&before, &now, "7z2409-x64.exe"), Some(app));
        // One that installs for one person, under a name unlike its installer's: the largest.
        let own = put("users/steamuser/AppData/Local/Programs/Thing/Thing.exe", 700);
        put("users/steamuser/AppData/Local/Programs/Thing/crashpad_handler.exe", 9000);
        let later = programs_in(&prefix);
        assert_eq!(installed_program(&now, &later, "setup-v2.exe"), Some(own));
        assert_eq!(installed_program(&later, &later, "setup.exe"), None);
    }

    #[test]
    #[cfg(unix)]
    fn an_installer_is_run_in_its_own_folder_and_given_a_launcher() {
        use std::os::unix::fs::PermissionsExt;
        let mut dirs = dirs("install");
        // A stand-in for the launcher, on the PATH: it "installs" by making the app,
        // and writes down what it was told.
        let bin = dirs.cache.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("umu-run");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho \"$GAMEID $@\" >> \"$WINEPREFIX/../told\"\nmkdir -p \"$WINEPREFIX/drive_c/Program Files/7-Zip\"\nprintf app > \"$WINEPREFIX/drive_c/Program Files/7-Zip/7zFM.exe\"\nprintf u > \"$WINEPREFIX/drive_c/Program Files/7-Zip/Uninstall.exe\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        dirs.programs = vec![bin];
        std::fs::create_dir_all(&dirs.downloads).unwrap();
        let installer = dirs.downloads.join("7z2409-x64.exe");
        std::fs::write(&installer, b"MZ..Nullsoft.NSIS.exehead..").unwrap();

        struct NoNet;
        impl Http for NoNet {
            fn get(&self, _: &str) -> Result<String> {
                unreachable!("the launcher is here already: nothing is fetched")
            }
            fn download(&self, _: &str, _: &Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
                unreachable!("the launcher is here already: nothing is fetched")
            }
        }
        let url = "https://www.7-zip.org/a/7z2409-x64.exe";
        let launcher = install(&NoNet, &dirs, url, &installer, &mut |_, _| true).unwrap();
        let folder = folder_for(&dirs, url);
        assert_eq!(launcher, folder.join("run"));
        assert_eq!(folder_of(&dirs, &launcher), Some(folder.clone()));
        // The installer was told to ask nothing, in the app's own Windows.
        let told = std::fs::read_to_string(folder.join("told")).unwrap();
        assert_eq!(told.trim(), format!("umu-default {} /S", installer.display()));
        // What starts it runs the app, not its uninstaller, in the same Windows.
        let script = std::fs::read_to_string(&launcher).unwrap();
        assert!(script.contains(&format!("export WINEPREFIX='{}'", folder.join("prefix").display())), "{script}");
        assert!(script.contains("7-Zip/7zFM.exe' \"$@\"") && !script.contains("Uninstall"), "{script}");
        assert!(std::fs::metadata(&launcher).unwrap().permissions().mode() & 0o111 != 0);
        // Started, it runs the launcher on the app.
        assert!(std::process::Command::new(&launcher).arg("--flag").status().unwrap().success());
        let told = std::fs::read_to_string(folder.join("told")).unwrap();
        assert!(told.lines().last().unwrap().ends_with("7-Zip/7zFM.exe --flag"), "{told}");

        // Without the launcher and without Python, it says so before fetching anything.
        dirs.programs = Vec::new();
        let none = install(&NoNet, &dirs, url, &installer, &mut |_, _| true).unwrap_err();
        assert!(none.to_string().contains("Python 3"), "{none}");
    }

    #[test]
    fn the_launcher_is_only_taken_if_it_is_the_file_that_was_pinned() {
        use sha2::{Digest, Sha256};
        // A parcel like the real one: the launcher, and something that is not wanted.
        let mut parcel = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut parcel);
            for (name, body) in [("umu/umu-run", &b"#!/usr/bin/env python3\n"[..]), ("umu/../../evil", &b"x"[..])] {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o755);
                header.set_entry_type(tar::EntryType::Regular);
                // Set the name as bytes: a parcel may name anything.
                header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
                header.set_cksum();
                builder.append(&header, body).unwrap();
            }
            builder.finish().unwrap();
        }
        struct Serves(Vec<u8>);
        impl Http for Serves {
            fn get(&self, _: &str) -> Result<String> {
                unreachable!()
            }
            fn download(&self, _: &str, dest: &Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
                std::fs::write(dest, &self.0)?;
                Ok(crate::http::hex(&Sha256::digest(&self.0)))
            }
        }
        let sum = crate::http::hex(&Sha256::digest(&parcel));
        let dirs = dirs("fetch");
        let http = Serves(parcel);
        let wrong =
            fetch_runner_from(&http, &dirs, "https://example.invalid/umu.tar", &"0".repeat(64), &mut |_, _| true);
        assert!(matches!(wrong, Err(Error::DigestMismatch { .. })));
        assert!(find_runner(&dirs).is_none(), "a file that is not the pinned one is not kept");
        let runner =
            fetch_runner_from(&http, &dirs, "https://example.invalid/umu.tar", &sum, &mut |_, _| true).unwrap();
        assert_eq!(find_runner(&dirs), Some(runner.clone()));
        assert_eq!(std::fs::read(&runner).unwrap(), b"#!/usr/bin/env python3\n");
        // Only the launcher was written: nothing else from the parcel, nowhere else.
        let written: Vec<_> =
            std::fs::read_dir(runner.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(written, ["umu-run"]);
        assert!(!dirs.applications.join("evil").exists() && !root(&dirs).join("evil").exists());
    }
}
