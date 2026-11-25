//! Entries in the app menu and icons on the desktop: for an AppImage, for a web app,
//! and for the store itself.

use super::appimage::unpacked_folder;
use super::browser::find_app_browser;
use super::dirs::Dirs;
use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Where an app's icon on the desktop is kept.
pub(super) fn desktop_icon_path(dirs: &Dirs, id: &str) -> Option<PathBuf> {
    let name = menu_entry_path(dirs, id).file_name()?.to_os_string();
    Some(dirs.desktop.as_ref()?.join(name))
}

/// Put a copy of an app's menu entry on the desktop, where the desktop shows it as
/// an icon to start the app from. Desktops only start such a file when it may be run,
/// and GNOME's when it has also been marked as trusted.
pub(super) fn put_on_desktop(dirs: &Dirs, id: &str, entry: &Path) {
    let Some(icon) = desktop_icon_path(dirs, id) else { return };
    if std::fs::copy(entry, &icon).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&icon, std::fs::Permissions::from_mode(0o755));
        if let Some(gio) = dirs.programs.iter().map(|dir| dir.join("gio")).find(|p| p.is_file()) {
            let _ = std::process::Command::new(gio)
                .args(["set"])
                .arg(&icon)
                .args(["metadata::trusted", "true"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

pub fn menu_entry_path(dirs: &Dirs, id: &str) -> PathBuf {
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
pub(super) fn appimage_icon(dirs: &Dirs, id: &str, appimage: &Path) -> Option<PathBuf> {
    // What starts a Windows program is a script, not an AppImage: asking it for its
    // icon would start the program.
    if crate::winrun::folder_of(dirs, appimage).is_some() {
        return None;
    }
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

/// An app-menu entry that starts `program` with `args`.
pub(super) fn write_menu_entry(
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
    // Linux and the like. With a browser that can do it, the site gets a window of
    // its own; the entry says what that window is called, so it wears the site's icon.
    if let Some(browser) = find_app_browser(dirs) {
        let path = write_menu_entry(dirs, id, &label, &browser.program, &browser.args(url), "Network;")?;
        if let Ok(entry) = std::fs::read_to_string(&path) {
            let _ = std::fs::write(&path, format!("{entry}StartupWMClass={}\n", browser.window_class(url)));
        }
        give_entry_its_picture(dirs, id, &path);
        put_on_desktop(dirs, id, &path);
        return Ok(path);
    }
    // Without one: the desktop's own opener, started on the address.
    let opener = std::env::var_os("PATH")
        .and_then(|path| std::env::split_paths(&path).map(|dir| dir.join("xdg-open")).find(|p| p.is_file()))
        .unwrap_or_else(|| PathBuf::from("/usr/bin/xdg-open"));
    let path = write_menu_entry(dirs, id, &label, &opener, &[url.to_string()], "Network;")?;
    give_entry_its_picture(dirs, id, &path);
    put_on_desktop(dirs, id, &path);
    Ok(path)
}

/// Give a menu entry the picture the store shows for the app (a website's own icon),
/// when one has been fetched. Without this the entry gets the picture of the program
/// that opens it, or none: every website on a TV would look like the browser.
pub(super) fn give_entry_its_picture(dirs: &Dirs, id: &str, entry: &Path) {
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
    // The file is named after the picture in it. A launcher remembers a picture by its
    // file's name, so a new logo written over the old file kept showing the old logo
    // (seen on Plasma Bigscreen). A new name also changes the entry, which the desktop
    // does notice. [Not tried on a Bigscreen itself.]
    let mark: String = Sha256::digest(icon).iter().take(4).map(|b| format!("{b:02x}")).collect();
    let icon_path = icons.join(format!("bilby-{mark}.png"));
    // The plain name stays as well: notifications ask for the icon called "bilby".
    let named = icons.join("bilby.png");
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
    for file in [&icon_path, &named] {
        if std::fs::read(file).ok().as_deref() != Some(icon) {
            std::fs::create_dir_all(&icons)?;
            std::fs::write(file, icon)?;
            changed = true;
        }
    }
    // Earlier logos are not left behind.
    for old in std::fs::read_dir(&icons).into_iter().flatten().flatten().map(|file| file.path()) {
        let name = old.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.starts_with("bilby-") && name.ends_with(".png") && old != icon_path {
            let _ = std::fs::remove_file(old);
        }
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
    // And its icon on the desktop, by the same test.
    if let Some(icon) = desktop_icon_path(dirs, id) {
        if std::fs::read_to_string(&icon).is_ok_and(|text| text.contains("X-Bilby-App=")) {
            let _ = std::fs::remove_file(icon);
        }
    }
}
