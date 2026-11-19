//! The `ustore` binary, run as a user would run it, against the demo catalog.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A throw-away home directory, so tests never touch real settings or downloads.
fn home() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("ustore-cli-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Config locations differ per OS; look the file up instead of assuming one.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
    }
    None
}

fn run_in(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ustore"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DOWNLOAD_DIR", home.join("downloads"))
        .env("APPDATA", home.join("config"))
        .env("LOCALAPPDATA", home.join("cache"))
        .env_remove("GITHUB_TOKEN")
        .output()
        .unwrap()
}

fn out(args: &[&str]) -> String {
    let output = run_in(&home(), args);
    assert!(output.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn help_and_bad_usage() {
    let help = out(&["--help"]);
    for command in ["device", "discover", "search", "show", "install", "library", "updates", "classify"] {
        assert!(help.contains(command), "help mentions {command}");
    }
    let none = run_in(&home(), &[]);
    assert_eq!(none.status.code(), Some(2));
    let bad = run_in(&home(), &["--as", "amiga", "device"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown platform"));
    assert_eq!(out(&["--list-platforms"]).lines().count(), 13);
}

#[test]
fn device_describes_the_chosen_machine() {
    let arch = out(&["--demo", "--as", "linux-x64-arch", "device"]);
    assert!(arch.starts_with("Linux · x86_64"));
    assert!(arch.contains("pacman packages"));
    assert!(arch.contains("GitHub, AUR"));
    let windows = out(&["--demo", "--as", "windows-arm64", "device"]);
    assert!(windows.contains("x86_64, x86 (32-bit)"), "emulated architectures are listed");
    assert!(!windows.contains("AUR"));
    // Whatever this machine is, detection produces something.
    assert!(out(&["--demo", "device"]).contains("Operating system"));
}

#[test]
fn discover_and_search_are_filtered() {
    let tv = out(&["--demo", "--as", "android-tv", "discover"]);
    assert!(tv.starts_with("Made for TV"));
    assert!(!tv.contains("lumen"), "desktop apps are not listed on a TV");

    let mac = out(&["--demo", "--as", "macos-arm64", "search", "monitor"]);
    assert!(mac.contains("github:barley/barley"));
    assert!(!mac.contains("winspect"));
    assert!(mac.contains("1 more matched but have no files for this machine"));

    let none = out(&["--demo", "--as", "android-arm64", "search", "monitor"]);
    assert!(none.contains("Nothing installable on Android"));
}

#[test]
fn show_explains_offers_and_hidden_files() {
    let shown = out(&["--demo", "--as", "windows-arm64", "show", "github:lumen-labs/lumen"]);
    let first = shown.lines().find(|l| l.starts_with('*')).unwrap();
    assert!(first.contains("Lumen-0.19.2-arm64-Setup.exe"), "{first}");
    assert!(shown.contains("note: Runs through Windows' x86_64 emulation"));
    assert!(shown.contains("Built for macOS"));
    assert!(!shown.contains("checksums.txt"));

    // A link works as an id.
    let by_link = out(&["--demo", "--as", "linux-x64-debian", "show", "https://github.com/lumen-labs/lumen"]);
    assert!(by_link.contains("$ sudo apt install ./lumen_0.19.2_amd64.deb"));

    let bad = run_in(&home(), &["--demo", "show", "not an id"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("is not an app id"));
    let missing = run_in(&home(), &["--demo", "show", "github:nobody/nothing"]);
    assert!(!missing.status.success());
}

#[test]
fn install_then_library_then_updates() {
    let home = home();
    let installed = run_in(&home, &["--demo", "--as", "linux-x64-fedora", "install", "github:tidewave/tidewave", "2"]);
    assert!(installed.status.success(), "{}", String::from_utf8_lossy(&installed.stderr));
    let stdout = String::from_utf8_lossy(&installed.stdout);
    assert!(stdout.contains("Checksum verified. Installed to"), "{stdout}");
    assert!(home.join("Applications").join("Tidewave-3.4.0-x86_64.AppImage").is_file());

    let library = String::from_utf8(run_in(&home, &["--demo", "library"]).stdout).unwrap();
    assert!(library.contains("github:tidewave/tidewave") && library.contains("v3.4.0") && library.contains("AppImage"));

    let current = String::from_utf8(run_in(&home, &["--demo", "updates"]).stdout).unwrap();
    assert!(current.contains("up to date"));

    // Pretend an older version is installed.
    let state = find(&home, "demo-state.json").expect("the library was saved somewhere under the home directory");
    let text = std::fs::read_to_string(&state).unwrap().replace("v3.4.0", "v3.0.0");
    std::fs::write(&state, text).unwrap();
    let outdated = String::from_utf8(run_in(&home, &["--demo", "updates"]).stdout).unwrap();
    assert!(outdated.contains("update available: v3.4.0"), "{outdated}");

    let no_option = run_in(&home, &["--demo", "install", "github:tidewave/tidewave", "99"]);
    assert!(!no_option.status.success());
    let not_here = run_in(&home, &["--demo", "--as", "linux-x64-debian", "install", "github:winspect/winspect"]);
    assert!(!not_here.status.success());
    assert!(String::from_utf8_lossy(&not_here.stderr).contains("None of the files"));
}

#[test]
fn doctor_checks_sources_and_the_featured_list() {
    for preset in [
        "linux-x64-debian",
        "linux-x64-arch",
        "linux-x64-alpine",
        "windows-arm64",
        "macos-arm64",
        "android-arm64",
        "android-tv",
    ] {
        let report = out(&["--demo", "--as", preset, "doctor"]);
        assert!(report.contains("All checks passed."), "{preset}:\n{report}");
        assert!(!report.contains("FAIL"), "{preset}:\n{report}");
        assert!(report.contains("featured apps"));
    }
    let debian = out(&["--demo", "--as", "linux-x64-debian", "doctor"]);
    assert!(debian.contains("would install tidewave_3.4.0_amd64.deb (Debian package · x86_64)"), "{debian}");
    let tv = out(&["--demo", "--as", "android-tv", "doctor"]);
    assert!(tv.contains("APK · arm64 · TV"), "{tv}");
}

#[test]
fn an_empty_library_says_what_to_do() {
    assert!(out(&["--demo", "library"]).contains("Nothing here yet"));
}

#[test]
fn classify_explains_a_file_name() {
    let apk = out(&["--demo", "--as", "linux-x64-debian", "classify", "app-arm64-v8a.apk"]);
    assert!(apk.contains("APK") && apk.contains("Incompatible (Built for Android)"));
    let sums = out(&["--demo", "classify", "SHA256SUMS"]);
    assert!(sums.contains("not an installable file"));
}
