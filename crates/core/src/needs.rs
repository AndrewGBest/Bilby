//! What an app needs from the system before it will run, and where to get it.
//!
//! An AppImage is one file that mounts itself to start, and for that it leans on FUSE.
//! Most AppImages in circulation carry the older start-up code, which loads
//! `libfuse.so.2`; recent distributions ship FUSE 3 only, so the file downloads,
//! installs and then closes at once with a message few people will ever see. The
//! store can tell beforehand, say so, and install the missing piece from the
//! distribution's own repository.
//!
//! [verified 2026-11-21: Longboard's AppImage, built with electron-builder, contains
//! the text `libfuse.so.2` in its first megabyte; run on a system without that
//! library it prints "AppImages require FUSE to run" and closes.]

use crate::model::{AppId, SourceKind};
use crate::platform::{LinuxFamily, Os, Platform};
use std::path::{Path, PathBuf};

/// Something missing, in words, and the package that provides it when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    /// Short, for a button: "Install FUSE".
    pub action: String,
    /// A sentence for the person.
    pub why: String,
    /// The package in the system's repositories that provides it.
    pub package: Option<AppId>,
}

/// What this system has of FUSE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fuse {
    /// `libfuse.so.2`, which the older AppImage start-up code loads.
    pub library: bool,
    /// `fusermount` or `fusermount3`, which the newer start-up code runs.
    pub mounter: bool,
}

/// Look for FUSE where the system keeps libraries and programs.
pub fn fuse_here() -> Fuse {
    let folders = [
        "/usr/lib",
        "/usr/lib64",
        "/lib",
        "/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib/aarch64-linux-gnu",
        "/usr/lib/arm-linux-gnueabihf",
    ];
    let library = folders.iter().any(|folder| {
        std::fs::read_dir(folder)
            .is_ok_and(|entries| entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with("libfuse.so.2")))
    });
    let on_path = |program: &str| {
        std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    };
    Fuse { library, mounter: on_path("fusermount") || on_path("fusermount3") }
}

/// Does this AppImage carry the older start-up code, which needs `libfuse.so.2`?
/// `None` when the file cannot be read.
pub fn wants_fuse_library(appimage: &Path) -> Option<bool> {
    use std::io::Read;
    let mut start = vec![0u8; 2 * 1024 * 1024];
    let mut file = std::fs::File::open(appimage).ok()?;
    let mut filled = 0;
    while filled < start.len() {
        match file.read(&mut start[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return None,
        }
    }
    let needle = b"libfuse.so.2";
    Some(start[..filled].windows(needle.len()).any(|window| window == needle))
}

/// The names the FUSE 2 library's package goes by on this kind of system, best first.
pub fn fuse_packages(platform: &Platform) -> Vec<AppId> {
    let Some(linux) = platform.linux.as_ref().filter(|_| platform.os == Os::Linux) else { return Vec::new() };
    let (source, names): (SourceKind, &[&str]) = match linux.family {
        // Debian 13 and Ubuntu 24.04 renamed it; older ones have the plain name.
        LinuxFamily::Debian => (SourceKind::Apt, &["libfuse2t64", "libfuse2"]),
        LinuxFamily::Arch => (SourceKind::Pacman, &["fuse2"]),
        LinuxFamily::Fedora => (SourceKind::Dnf, &["fuse-libs"]),
        LinuxFamily::Suse => (SourceKind::Zypper, &["libfuse2"]),
        LinuxFamily::Alpine | LinuxFamily::Other => return Vec::new(),
    };
    names.iter().map(|name| AppId::new(source, *name)).collect()
}

/// Will an AppImage start here? `file` is the AppImage when it is already on the
/// device (then its own start-up code is looked at); before that, the answer goes by
/// what most AppImages need. `offered` says whether the system's repositories have a
/// package, so only one that can really be installed is proposed.
pub fn for_appimage(
    platform: &Platform,
    fuse: Fuse,
    file: Option<&Path>,
    offered: &dyn Fn(&AppId) -> bool,
) -> Option<Need> {
    if platform.os != Os::Linux {
        return None;
    }
    let wants_library = file.and_then(wants_fuse_library);
    let (missing, sure) = match wants_library {
        Some(true) => (!fuse.library, true),
        Some(false) => (!fuse.mounter, true),
        // Not downloaded yet: most need the library; a few get by with the newer FUSE.
        None => (!fuse.library, false),
    };
    if !missing {
        return None;
    }
    let package = fuse_packages(platform).into_iter().find(|id| offered(id));
    let _ = sure;
    let why = match &package {
        // The store takes care of it: say what will happen, not what to go and do.
        Some(_) => "This app needs FUSE 2 to run, which this system does not have. Bilby will install it for you along with the app; your system will ask for your password. If that cannot be done, Bilby unpacks the app so that it runs without it.".to_string(),
        // No package to be had: the app is unpacked instead, which needs no FUSE at all.
        None => "This app usually needs FUSE 2 to run, which this system does not have. Bilby will unpack it instead, so that it runs without it. Nothing for you to do.".to_string(),
    };
    Some(Need { action: "Install FUSE".to_string(), why, package })
}

/// The AppImage the store placed for an app, when its path says it is one.
pub fn as_appimage(path: &Path) -> Option<PathBuf> {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage")).then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &AppId) -> bool {
        true
    }

    #[test]
    fn a_system_without_fuse_two_is_told_before_and_after_downloading() {
        let debian = Platform::preset("linux-x64-debian").unwrap();
        let modern = Fuse { library: false, mounter: true };
        // Before downloading: a warning in "most of those" words, with the package to install.
        let before = for_appimage(&debian, modern, None, &all).unwrap();
        assert!(before.why.contains("needs FUSE 2") && before.why.contains("Bilby will install it for you"));
        assert_eq!(before.package.unwrap().to_string(), "apt:libfuse2t64");
        // An older Debian has it under the plain name.
        let older = for_appimage(&debian, modern, None, &|id| id.key == "libfuse2").unwrap();
        assert_eq!(older.package.unwrap().to_string(), "apt:libfuse2");
        // No package to be had: say what to look for instead of offering a button.
        let none = for_appimage(&debian, modern, None, &|_| false).unwrap();
        assert!(
            none.package.is_none() && none.why.contains("Bilby will unpack it"),
            "nothing is left to the person: {}",
            none.why
        );

        // With the library, nothing is said; nor on Windows.
        assert!(for_appimage(&debian, Fuse { library: true, mounter: false }, None, &all).is_none());
        assert!(for_appimage(&Platform::preset("windows-x64").unwrap(), modern, None, &all).is_none());
        assert_eq!(fuse_packages(&Platform::preset("linux-x64-arch").unwrap())[0].to_string(), "pacman:fuse2");
        assert_eq!(fuse_packages(&Platform::preset("linux-x64-fedora").unwrap())[0].to_string(), "dnf:fuse-libs");
    }

    #[test]
    fn a_downloaded_appimage_is_looked_at_for_what_it_really_needs() {
        let dir = std::env::temp_dir().join(format!("bilby-needs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("Old.AppImage");
        std::fs::write(&old, [b"\x7fELF....".as_slice(), b"dlopen libfuse.so.2 failed", &[0u8; 64]].concat()).unwrap();
        let new = dir.join("New.AppImage");
        std::fs::write(&new, b"\x7fELF.... fusermount3 static runtime").unwrap();
        assert_eq!((wants_fuse_library(&old), wants_fuse_library(&new)), (Some(true), Some(false)));
        assert_eq!(wants_fuse_library(&dir.join("gone")), None);

        let debian = Platform::preset("linux-x64-debian").unwrap();
        let modern = Fuse { library: false, mounter: true };
        let need = for_appimage(&debian, modern, Some(&old), &all).unwrap();
        assert!(need.why.contains("needs FUSE 2"), "{}", need.why);
        assert!(for_appimage(&debian, modern, Some(&new), &all).is_none(), "the newer kind runs on FUSE 3");
        assert!(for_appimage(&debian, Fuse { library: false, mounter: false }, Some(&new), &all).is_some());
        assert!(as_appimage(&old).is_some() && as_appimage(&dir.join("app.deb")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// `rpm-ostree`, on a system that is one read-only image (Fedora Kinoite, Silverblue
/// and their kin). Such a system has no package manager in the usual sense: a package
/// is added on top of the image with `rpm-ostree install`, and is there from the next
/// start. Fedora took FUSE 2 out of these images on purpose and names this as the way
/// to put it back (<https://fedoraproject.org/wiki/Changes/AtomicDesktopDropFuse2>).
pub fn layering_here() -> Option<PathBuf> {
    if !Path::new("/run/ostree-booted").exists() {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join("rpm-ostree")).find(|tool| tool.is_file())
}

/// Add FUSE 2 on top of the system's image. It takes effect at the next restart.
///
/// `fuse-libs` is the library older AppImages load and `fuse` the program it runs to
/// mount; the library alone is tried if the pair cannot be had. `--idempotent` makes
/// asking twice harmless. rpm-ostree asks for the password itself where one is
/// needed. [Written from Fedora's and rpm-ostree's documentation; not run on such a
/// system by the author.]
pub fn layer_fuse(rpm_ostree: &Path) -> crate::Result<()> {
    use std::process::{Command, Stdio};
    let mut said = String::new();
    for packages in [&["fuse", "fuse-libs"][..], &["fuse-libs"][..]] {
        let ran = Command::new(rpm_ostree)
            .args(["install", "-y", "--idempotent"])
            .args(packages)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        match ran {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                said = String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            }
            Err(e) => said = e.to_string(),
        }
    }
    Err(crate::Error::Unsupported(format!("FUSE could not be added to this system. {said}")))
}

#[cfg(all(test, unix))]
mod layering_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn fuse_is_added_on_top_of_a_read_only_system_and_a_refusal_is_reported() {
        let dir = std::env::temp_dir().join(format!("bilby-layer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tool = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\necho \"$@\" >> {}/asked\n{body}\n", dir.display())).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        // Tried a few times: another test starting a program at the same instant can
        // make the system call this file busy for a moment.
        let run = |tool: &Path| {
            (0..40)
                .find_map(|_| match layer_fuse(tool) {
                    Err(e) if e.to_string().contains("busy") => {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        None
                    }
                    other => Some(other),
                })
                .unwrap()
        };
        assert!(run(&tool("works", "exit 0")).is_ok());
        assert_eq!(
            std::fs::read_to_string(dir.join("asked")).unwrap().lines().last(),
            Some("install -y --idempotent fuse fuse-libs")
        );
        // Without the pair, the library alone.
        std::fs::remove_file(dir.join("asked")).unwrap();
        assert!(run(&tool("one", "case \"$*\" in *' fuse '*) exit 1;; esac; exit 0")).is_ok());
        assert_eq!(
            std::fs::read_to_string(dir.join("asked")).unwrap().lines().last(),
            Some("install -y --idempotent fuse-libs")
        );
        let refused = run(&tool("refuses", "echo 'error: Not authorized' >&2; exit 1")).unwrap_err().to_string();
        assert!(refused.contains("could not be added") && refused.contains("Not authorized"), "{refused}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
