//! The real application binary, with its worker threads, event loop and software
//! renderer, driven by scripted clicks and keys under a virtual X display.
//!
//! The other UI tests run the handlers inline on a headless backend; these confirm the
//! same flows work when the work really happens on other threads. They need `xvfb-run`
//! and are skipped (with a note) where it is not installed.
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn have_xvfb() -> bool {
    let found = Command::new("xvfb-run").arg("--help").output().is_ok();
    if !found {
        eprintln!("skipped: xvfb-run is not installed");
    }
    found
}

struct Run {
    home: PathBuf,
    report: HashMap<String, String>,
    screenshot: PathBuf,
}

fn run(seed: &str, args: &[&str]) -> Run {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("bilby-bin-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let state = home.join("state.json");
    std::fs::write(&state, seed).unwrap();
    let screenshot = home.join("shot.png");
    let report = home.join("report.txt");
    let output = Command::new("xvfb-run")
        // Each run starts looking for a free display at its own number: tests run side by
        // side, and two that both find the same number free take each other's display down.
        .args(["-n", &(99 + n).to_string(), "-a", "-s", "-screen 0 1600x1000x24"])
        .arg(env!("CARGO_BIN_EXE_bilby"))
        .args(["--demo", "--theme", "dark", "--state"])
        .arg(&state)
        .args(args)
        .arg("--screenshot")
        .arg(&screenshot)
        .arg("--report")
        .arg(&report)
        .env("HOME", &home)
        .env("XDG_DOWNLOAD_DIR", home.join("Downloads"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("SLINT_BACKEND", "winit-software")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(output.status.success(), "the app exited with an error: {}", String::from_utf8_lossy(&output.stderr));
    let text = std::fs::read_to_string(&report)
        .unwrap_or_else(|_| panic!("no report written; stderr: {}", String::from_utf8_lossy(&output.stderr)));
    let report = text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Run { home, report, screenshot }
}

impl Run {
    fn get(&self, key: &str) -> &str {
        self.report.get(key).map(String::as_str).unwrap_or_else(|| panic!("report has no {key}"))
    }
}

/// A PNG that is not a single flat colour: the window really drew something.
fn drew_something(png: &Path) -> bool {
    let bytes = std::fs::read(png).unwrap();
    bytes.starts_with(&[0x89, b'P', b'N', b'G']) && bytes.len() > 20_000
}

const RETURNING: &str = r#"{"settings":{"welcomed":true}}"#;

#[test]
fn desktop_open_and_install_with_real_threads() {
    if !have_xvfb() {
        return;
    }
    // Start on an app page (loaded on a worker), then install the second option, the
    // AppImage, through the same callback its button calls. (Real clicks on real buttons
    // are covered by tests/ui.rs; this test is about the threads.)
    let run = run(
        RETURNING,
        &[
            "--as",
            "linux-x64-fedora",
            "--ui",
            "desktop",
            "--size",
            "1280x800",
            "--open",
            "tidewave/tidewave",
            "--script",
            "wait wait install:1 wait wait wait wait wait",
            "--settle",
            "1200",
        ],
    );
    assert_eq!(run.get("shell"), "desktop");
    assert_eq!(run.get("details_open"), "true");
    assert_eq!(run.get("details_loading"), "false");
    assert_eq!(run.get("app"), "github:tidewave/tidewave");
    assert_eq!(run.get("options"), "3");
    assert_eq!(run.get("first_option"), "RPM package · x86_64");
    assert_eq!(run.get("install_state"), "2", "{}", run.get("install_message"));
    assert_eq!(run.get("installed_version"), "v3.4.0");
    assert_eq!(run.get("library"), "1");
    assert_eq!(run.get("toast"), "Tidewave is in your library");
    assert!(run.home.join("Applications").join("Tidewave-3.4.0-x86_64.AppImage").is_file());
    assert!(drew_something(&run.screenshot));
}

#[test]
fn start_up_finds_updates_in_the_background() {
    if !have_xvfb() {
        return;
    }
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"},
        {"id":"github:tidewave/tidewave","name":"Tidewave","version":"v3.4.0"}]}"#;
    let run = run(seed, &["--as", "linux-x64-debian", "--ui", "desktop", "--size", "1280x800", "--page", "library"]);
    assert_eq!(run.get("page"), "2");
    assert_eq!(run.get("library"), "2");
    assert_eq!(run.get("updates"), "1");
    assert_eq!(run.get("checking_updates"), "false");
    assert_eq!(run.get("toast"), "", "the start-up check is silent");
}

#[test]
fn one_click_update_with_real_threads() {
    if !have_xvfb() {
        return;
    }
    let seed = r#"{"settings":{"welcomed":true},"library":[
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    let run = run(
        seed,
        &[
            "--as",
            "linux-x64-debian",
            "--ui",
            "desktop",
            "--size",
            "1280x800",
            "--page",
            "library",
            "--script",
            "wait wait update:github:kestrel-sh/kestrel wait wait wait wait wait wait",
            "--settle",
            "1200",
        ],
    );
    assert_eq!(run.get("page"), "2");
    assert_eq!(run.get("details_open"), "false");
    assert_eq!(run.get("updating"), "");
    assert_eq!(run.get("updates"), "0");
    assert_eq!(run.get("toast"), "Kestrel updated to v1.21.0");
    assert!(run.home.join("Downloads").join("bilby").join("kestrel-x86_64-unknown-linux-gnu.tar.gz").is_file());
}

#[test]
fn the_start_up_check_respects_the_setting() {
    if !have_xvfb() {
        return;
    }
    let seed = r#"{"settings":{"welcomed":true,"skip_update_check_on_start":true},"library":[
        {"id":"github:kestrel-sh/kestrel","name":"Kestrel","version":"v1.20.0"}]}"#;
    let run = run(seed, &["--as", "linux-x64-debian", "--ui", "desktop", "--size", "1280x800", "--page", "library"]);
    assert_eq!(run.get("library"), "1");
    assert_eq!(run.get("updates"), "0", "nothing was checked, so nothing is claimed");
}

#[test]
fn tv_remote_and_typed_search_with_real_key_events() {
    if !have_xvfb() {
        return;
    }
    // Up reaches Search in the top bar, OK starts typing, OK searches (on a worker),
    // Right moves, OK opens the result.
    let run = run(
        RETURNING,
        &[
            "--as",
            "android-tv",
            "--ui",
            "tv",
            "--size",
            "1920x1080",
            "--script",
            "up ok text:play ok wait right ok wait",
        ],
    );
    assert_eq!(run.get("shell"), "tv");
    // Three apps, and the web apps that are for playing.
    assert!(run.get("results").parse::<usize>().unwrap() >= 3, "{}", run.get("results"));
    assert_eq!(run.get("details_open"), "true");
    // Tidewave is far more widely used and says it is a player, so it is listed first;
    // Right moves on to Couchplay, which has "play" inside its name.
    assert_eq!(run.get("app"), "github:couchplay/couchplay");
    assert_eq!(run.get("first_option"), "APK · universal · TV");
    assert!(drew_something(&run.screenshot));
}

#[test]
fn a_narrow_window_gets_the_mobile_interface() {
    if !have_xvfb() {
        return;
    }
    let run = run(RETURNING, &["--as", "linux-x64-debian", "--size", "420x800"]);
    assert_eq!(run.get("shell"), "mobile");
    let run = run_wide();
    assert_eq!(run.get("shell"), "desktop");
}

fn run_wide() -> Run {
    run(RETURNING, &["--as", "linux-x64-debian", "--size", "1200x800"])
}

#[test]
fn typing_in_the_search_field_suggests_apps() {
    if !have_xvfb() {
        return;
    }
    let run = run(
        RETURNING,
        &[
            "--as",
            "linux-x64-debian",
            "--ui",
            "desktop",
            "--size",
            "1280x800",
            "--script",
            "click:500,38 text:lum wait",
        ],
    );
    assert_eq!(run.get("page"), "1");
    assert_eq!(run.get("suggestions"), "1");
    assert_eq!(run.get("searching"), "false");
}
