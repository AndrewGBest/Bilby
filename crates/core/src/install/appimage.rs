//! AppImages that cannot run as they are (no FUSE 2): unpacking one, and clearing
//! away what was put in place.

use super::dirs::Dirs;
use super::launch::not_while_busy;
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

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
    // A Windows program: its folder holds its whole Windows, and goes with it.
    if let Some(folder) = crate::winrun::folder_of(dirs, program) {
        let _ = std::fs::remove_file(folder.join("prefix.log"));
        return std::fs::remove_dir_all(folder).is_ok();
    }
    match unpacked_folder(program) {
        Some(folder) => std::fs::remove_dir_all(folder).is_ok(),
        None => std::fs::remove_file(program).is_ok(),
    }
}
