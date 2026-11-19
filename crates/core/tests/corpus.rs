//! The classifier against file names in the styles real projects use.
//!
//! These names follow the release-naming conventions of well-known projects (Rust
//! target triples, GoReleaser, Electron and Tauri bundlers, Android ABI splits, distro
//! packages). They were written from memory of those conventions rather than fetched
//! from live releases, so treat this as a regression suite for the conventions, not as
//! proof about any one project's current files.

use ustore_core::asset::{classify, Format, TargetArch, TargetOs};
use ustore_core::compat::{evaluate, Level};
use ustore_core::platform::{Arch, Os, Platform, PRESETS};

#[derive(Debug, PartialEq)]
enum Want {
    /// Not an installable file at all.
    Skip,
    /// Kept, but the name does not say which system it is for (so it is hidden everywhere).
    NoOs(Format),
    Is(Os, Format, A),
}

/// Architecture shorthand.
#[derive(Debug, PartialEq, Clone, Copy)]
enum A {
    X64,
    Arm64,
    X86,
    Arm32,
    Riscv,
    Universal,
    Any,
    Foreign,
    Unstated,
}

fn arch_of(arch: &TargetArch) -> A {
    match arch {
        TargetArch::One(Arch::X86_64) => A::X64,
        TargetArch::One(Arch::Aarch64) => A::Arm64,
        TargetArch::One(Arch::X86) => A::X86,
        TargetArch::One(Arch::Arm32) => A::Arm32,
        TargetArch::One(Arch::Riscv64) => A::Riscv,
        TargetArch::Universal => A::Universal,
        TargetArch::Any => A::Any,
        TargetArch::Foreign(_) => A::Foreign,
        TargetArch::Unknown => A::Unstated,
    }
}

fn actual(name: &str) -> Want {
    match classify(name) {
        None => Want::Skip,
        Some(info) => match info.os {
            TargetOs::Known(os) => Want::Is(os, info.format, arch_of(&info.arch)),
            TargetOs::Unknown => Want::NoOs(info.format),
            TargetOs::Foreign(_) => Want::Skip, // folded: never usable on any supported system
        },
    }
}

use Format::*;
use Os::{Android, Ios, Linux, MacOs, Windows};
use Want::{Is, NoOs, Skip};

#[rustfmt::skip]
fn corpus() -> Vec<(&'static str, Want)> {
    vec![
        // --- Rust target triples (ripgrep, bat, fd, starship style) ---
        ("ripgrep-14.1.1-x86_64-unknown-linux-musl.tar.gz", Is(Linux, Archive, A::X64)),
        ("ripgrep-14.1.1-aarch64-unknown-linux-gnu.tar.gz", Is(Linux, Archive, A::Arm64)),
        ("ripgrep-14.1.1-armv7-unknown-linux-gnueabihf.tar.gz", Is(Linux, Archive, A::Arm32)),
        ("ripgrep-14.1.1-i686-unknown-linux-gnu.tar.gz", Is(Linux, Archive, A::X86)),
        ("ripgrep-14.1.1-powerpc64-unknown-linux-gnu.tar.gz", Is(Linux, Archive, A::Foreign)),
        ("ripgrep-14.1.1-s390x-unknown-linux-gnu.tar.gz", Is(Linux, Archive, A::Foreign)),
        ("ripgrep-14.1.1-x86_64-apple-darwin.tar.gz", Is(MacOs, Archive, A::X64)),
        ("ripgrep-14.1.1-aarch64-apple-darwin.tar.gz", Is(MacOs, Archive, A::Arm64)),
        ("ripgrep-14.1.1-x86_64-pc-windows-msvc.zip", Is(Windows, Archive, A::X64)),
        ("ripgrep-14.1.1-x86_64-pc-windows-gnu.zip", Is(Windows, Archive, A::X64)),
        ("ripgrep-14.1.1-i686-pc-windows-msvc.zip", Is(Windows, Archive, A::X86)),
        ("ripgrep-14.1.1-aarch64-pc-windows-msvc.zip", Is(Windows, Archive, A::Arm64)),
        ("ripgrep_14.1.1-1_amd64.deb", Is(Linux, Deb, A::X64)),
        ("ripgrep-14.1.1-x86_64-unknown-linux-musl.tar.gz.sha256", Skip),
        ("bat-musl_0.24.0_amd64.deb", Is(Linux, Deb, A::X64)),
        ("bat_0.24.0_arm64.deb", Is(Linux, Deb, A::Arm64)),
        ("bat_0.24.0_armhf.deb", Is(Linux, Deb, A::Arm32)),
        ("bat_0.24.0_i686.deb", Is(Linux, Deb, A::X86)),
        ("starship-x86_64-unknown-freebsd.tar.gz", Skip),
        ("starship-aarch64-pc-windows-msvc.msi", Is(Windows, Msi, A::Arm64)),
        ("starship-riscv64gc-unknown-linux-gnu.tar.gz", Is(Linux, Archive, A::Riscv)),
        ("tool-x86_64-unknown-linux-gnu.tar.xz", Is(Linux, Archive, A::X64)),
        ("tool-aarch64-linux-android.tar.gz", Is(Android, Archive, A::Arm64)),
        ("tool-wasm32-wasi.tar.gz", Skip),

        // --- GoReleaser / Go names (lazygit, syncthing, gh style) ---
        ("lazygit_0.44.1_Linux_x86_64.tar.gz", Is(Linux, Archive, A::X64)),
        ("lazygit_0.44.1_Linux_arm64.tar.gz", Is(Linux, Archive, A::Arm64)),
        ("lazygit_0.44.1_Linux_armv6.tar.gz", Is(Linux, Archive, A::Arm32)),
        ("lazygit_0.44.1_Linux_32-bit.tar.gz", Is(Linux, Archive, A::X86)),
        ("lazygit_0.44.1_Darwin_arm64.tar.gz", Is(MacOs, Archive, A::Arm64)),
        ("lazygit_0.44.1_Darwin_x86_64.tar.gz", Is(MacOs, Archive, A::X64)),
        ("lazygit_0.44.1_Windows_x86_64.zip", Is(Windows, Archive, A::X64)),
        ("lazygit_0.44.1_freebsd_x86_64.tar.gz", Skip),
        ("checksums.txt", Skip),
        ("syncthing-linux-amd64-v1.27.9.tar.gz", Is(Linux, Archive, A::X64)),
        ("syncthing-linux-386-v1.27.9.tar.gz", Is(Linux, Archive, A::X86)),
        ("syncthing-linux-arm-v1.27.9.tar.gz", Is(Linux, Archive, A::Arm32)),
        ("syncthing-linux-mips64le-v1.27.9.tar.gz", Is(Linux, Archive, A::Foreign)),
        ("syncthing-linux-ppc64le-v1.27.9.tar.gz", Is(Linux, Archive, A::Foreign)),
        ("syncthing-linux-riscv64-v1.27.9.tar.gz", Is(Linux, Archive, A::Riscv)),
        ("syncthing-linux-loong64-v1.27.9.tar.gz", Is(Linux, Archive, A::Foreign)),
        ("syncthing-macos-universal-v1.27.9.zip", Is(MacOs, Archive, A::Universal)),
        ("syncthing-windows-arm64-v1.27.9.zip", Is(Windows, Archive, A::Arm64)),
        ("syncthing-windows-386-v1.27.9.zip", Is(Windows, Archive, A::X86)),
        ("syncthing-openbsd-amd64-v1.27.9.tar.gz", Skip),
        ("syncthing-illumos-amd64-v1.27.9.tar.gz", Skip),
        ("syncthing-source-v1.27.9.tar.gz", Skip),
        ("sha256sum.txt.asc", Skip),
        ("gh_2.50.0_linux_amd64.rpm", Is(Linux, Rpm, A::X64)),
        ("gh_2.50.0_macOS_universal.pkg", Is(MacOs, MacPkg, A::Universal)),
        ("gh_2.50.0_windows_amd64.msi", Is(Windows, Msi, A::X64)),
        ("kubectl-linux-amd64", Is(Linux, Binary, A::X64)),

        // --- single-file tools (yt-dlp style) ---
        ("yt-dlp", Skip),
        ("yt-dlp.exe", Is(Windows, Exe, A::Unstated)),
        ("yt-dlp_x86.exe", Is(Windows, Exe, A::X86)),
        ("yt-dlp_linux", Is(Linux, Binary, A::Unstated)),
        ("yt-dlp_linux_aarch64", Is(Linux, Binary, A::Arm64)),
        ("yt-dlp_linux_armv7l", Is(Linux, Binary, A::Arm32)),
        ("yt-dlp_macos", Is(MacOs, Binary, A::Unstated)),
        ("yt-dlp_win.zip", Is(Windows, Archive, A::Unstated)),
        ("yt-dlp.tar.gz", NoOs(Archive)),
        ("SHA2-256SUMS", Skip),
        ("SHA2-512SUMS.sig", Skip),

        // --- editors (neovim, helix, VSCodium style) ---
        ("nvim-linux-x86_64.appimage", Is(Linux, AppImage, A::X64)),
        ("nvim-linux-arm64.tar.gz", Is(Linux, Archive, A::Arm64)),
        ("nvim-macos-arm64.tar.gz", Is(MacOs, Archive, A::Arm64)),
        ("nvim-macos-x86_64.tar.gz", Is(MacOs, Archive, A::X64)),
        ("nvim-win64.msi", Is(Windows, Msi, A::X64)),
        ("nvim-win64.zip", Is(Windows, Archive, A::X64)),
        ("nvim-linux-x86_64.appimage.zsync", Skip),
        ("helix-24.07-x86_64.AppImage", Is(Linux, AppImage, A::X64)),
        ("helix-24.07-x86_64-linux.tar.xz", Is(Linux, Archive, A::X64)),
        ("helix-24.07-aarch64-macos.tar.xz", Is(MacOs, Archive, A::Arm64)),
        ("helix-24.07-x86_64-windows.zip", Is(Windows, Archive, A::X64)),
        ("helix-24.07-source.tar.xz", Skip),
        ("VSCodium-linux-x64-1.90.0.24158.tar.gz", Is(Linux, Archive, A::X64)),
        ("codium_1.90.0.24158_amd64.deb", Is(Linux, Deb, A::X64)),
        ("codium-1.90.0.24158-el8.x86_64.rpm", Is(Linux, Rpm, A::X64)),
        ("codium-1.90.0.24158-el8.aarch64.rpm", Is(Linux, Rpm, A::Arm64)),
        ("VSCodium-1.90.0.24158-glibc2.17-x86_64.AppImage", Is(Linux, AppImage, A::X64)),
        ("VSCodium-x64-1.90.0.24158.msi", Is(Windows, Msi, A::X64)),
        ("VSCodiumSetup-x64-1.90.0.24158.exe", Is(Windows, Exe, A::X64)),
        ("VSCodiumUserSetup-arm64-1.90.0.24158.exe", Is(Windows, Exe, A::Arm64)),
        ("VSCodium-win32-x64-1.90.0.24158.zip", Is(Windows, Archive, A::X64)),
        ("VSCodium-win32-arm64-1.90.0.24158.zip", Is(Windows, Archive, A::Arm64)),
        ("VSCodium.x64.1.90.0.24158.dmg", Is(MacOs, Dmg, A::X64)),
        ("VSCodium-darwin-arm64-1.90.0.24158.zip", Is(MacOs, Archive, A::Arm64)),

        // --- Electron builder ---
        ("Obsidian-1.6.7.AppImage", Is(Linux, AppImage, A::Unstated)),
        ("Obsidian-1.6.7-arm64.AppImage", Is(Linux, AppImage, A::Arm64)),
        ("Obsidian-1.6.7.dmg", Is(MacOs, Dmg, A::Unstated)),
        ("Obsidian-1.6.7.exe", Is(Windows, Exe, A::Unstated)),
        ("obsidian_1.6.7_amd64.deb", Is(Linux, Deb, A::X64)),
        ("obsidian_1.6.7_amd64.snap", Is(Linux, Snap, A::X64)),
        ("Obsidian-1.6.7.apk", Is(Android, Apk, A::Unstated)),
        ("obsidian-1.6.7.tar.gz", NoOs(Archive)),
        ("obsidian-1.6.7.asar.gz", Skip),
        ("App Setup 1.0.0.exe", Is(Windows, Exe, A::Unstated)),
        ("App-1.0.0-mac.zip", Is(MacOs, Archive, A::Unstated)),
        ("App-1.0.0-arm64-mac.zip", Is(MacOs, Archive, A::Arm64)),
        ("App-1.0.0-win.zip", Is(Windows, Archive, A::Unstated)),
        ("App-1.0.0-arm64.dmg.blockmap", Skip),
        ("latest.yml", Skip),
        ("latest-mac.yml", Skip),
        ("latest-linux-arm64.yml", Skip),
        ("app-1.0.0.x86_64.rpm", Is(Linux, Rpm, A::X64)),

        // --- Tauri bundler ---
        ("app_1.0.0_x64-setup.exe", Is(Windows, Exe, A::X64)),
        ("app_1.0.0_x64_en-US.msi", Is(Windows, Msi, A::X64)),
        ("app_1.0.0_aarch64.dmg", Is(MacOs, Dmg, A::Arm64)),
        ("app_1.0.0_amd64.AppImage", Is(Linux, AppImage, A::X64)),
        ("app_1.0.0_amd64.AppImage.tar.gz", Skip),
        ("app_1.0.0_amd64.AppImage.tar.gz.sig", Skip),
        ("app_1.0.0_x64-setup.nsis.zip", Skip),
        ("app_1.0.0_x64_en-US.msi.zip", Skip),
        ("app-1.0.0-1.x86_64.rpm", Is(Linux, Rpm, A::X64)),

        // --- Android ABI splits (LocalSend, Termux, RustDesk, Jellyfin style) ---
        ("LocalSend-1.15.4-android-arm64v8.apk", Is(Android, Apk, A::Arm64)),
        ("LocalSend-1.15.4-android-armv7.apk", Is(Android, Apk, A::Arm32)),
        ("LocalSend-1.15.4-android-x64.apk", Is(Android, Apk, A::X64)),
        ("LocalSend-1.15.4-linux-x86-64.AppImage", Is(Linux, AppImage, A::X64)),
        ("LocalSend-1.15.4-linux-x86-64.deb", Is(Linux, Deb, A::X64)),
        ("LocalSend-1.15.4-linux-arm-64.deb", Is(Linux, Deb, A::Arm64)),
        ("LocalSend-1.15.4-windows-x86-64.exe", Is(Windows, Exe, A::X64)),
        ("LocalSend-1.15.4-windows-x86-64.zip", Is(Windows, Archive, A::X64)),
        ("termux-app_v0.118.1+github-debug_arm64-v8a.apk", Is(Android, Apk, A::Arm64)),
        ("termux-app_v0.118.1+github-debug_armeabi-v7a.apk", Is(Android, Apk, A::Arm32)),
        ("termux-app_v0.118.1+github-debug_x86.apk", Is(Android, Apk, A::X86)),
        ("termux-app_v0.118.1+github-debug_x86_64.apk", Is(Android, Apk, A::X64)),
        ("termux-app_v0.118.1+github-debug_universal.apk", Is(Android, Apk, A::Universal)),
        ("NewPipe_v0.27.2.apk", Is(Android, Apk, A::Unstated)),
        ("rustdesk-1.3.1-universal-signed.apk", Is(Android, Apk, A::Universal)),
        ("rustdesk-1.3.1-aarch64-signed.apk", Is(Android, Apk, A::Arm64)),
        ("rustdesk-1.3.1-armv7-signed.apk", Is(Android, Apk, A::Arm32)),
        ("rustdesk-1.3.1-x86_64.AppImage", Is(Linux, AppImage, A::X64)),
        ("rustdesk-1.3.1-aarch64.deb", Is(Linux, Deb, A::Arm64)),
        ("rustdesk-1.3.1-0.x86_64.rpm", Is(Linux, Rpm, A::X64)),
        ("rustdesk-1.3.1-0.x86_64-suse.rpm", Is(Linux, Rpm, A::X64)),
        ("rustdesk-1.3.1-x86_64.pkg.tar.zst", Is(Linux, PacmanPkg, A::X64)),
        ("rustdesk-1.3.1-x86_64.flatpak", Is(Linux, Flatpak, A::X64)),
        ("rustdesk-1.3.1-x86_64.msi", Is(Windows, Msi, A::X64)),
        ("rustdesk-1.3.1-aarch64.dmg", Is(MacOs, Dmg, A::Arm64)),
        ("jellyfin-androidtv-v0.17.4-release.apk", Is(Android, Apk, A::Unstated)),
        ("jellyfin-androidtv-v0.17.4-debug.apk", Is(Android, Apk, A::Unstated)),
        ("app-release.aab", Is(Android, ApkBundle, A::Unstated)),
        ("App.ipa", Is(Ios, Ipa, A::Unstated)),

        // --- Windows-first projects ---
        ("PowerToysSetup-0.82.1-x64.exe", Is(Windows, Exe, A::X64)),
        ("PowerToysUserSetup-0.82.1-arm64.exe", Is(Windows, Exe, A::Arm64)),
        ("Microsoft.WindowsTerminal_1.20.11781.0_8wekyb3d8bbwe.msixbundle", Is(Windows, Msix, A::Unstated)),
        ("Microsoft.WindowsTerminal_1.20.11781.0_x64.zip", Is(Windows, Archive, A::X64)),
        ("Alacritty-v0.13.2-installer.msi", Is(Windows, Msi, A::Unstated)),
        ("Alacritty-v0.13.2-portable.exe", Is(Windows, Exe, A::Unstated)),
        ("Alacritty-v0.13.2.dmg", Is(MacOs, Dmg, A::Unstated)),
        ("alacritty.bash", Skip),
        ("Alacritty.svg", Skip),
        ("alacritty.1.gz", NoOs(Archive)),
        ("7z2407-x64.exe", Is(Windows, Exe, A::X64)),
        ("7z2407-linux-x64.tar.xz", Is(Linux, Archive, A::X64)),
        ("7z2407-mac.tar.xz", Is(MacOs, Archive, A::Unstated)),
        ("MoonlightPortable-x64-6.1.0.zip", NoOs(Archive)),

        // --- supporting files that must never be offered ---
        ("app-1.0.0.tar.gz.asc", Skip),
        ("app.exe.sig", Skip),
        ("app.dmg.sha256", Skip),
        ("app_1.0.0_amd64-dbgsym.deb", Skip),
        ("app-debuginfo-1.0.0.x86_64.rpm", Skip),
        ("app-1.0.0-windows-x64-symbols.zip", Skip),
        ("app-1.0.0-linux-x64-debug.tar.gz", Skip),
        ("sbom.spdx.json", Skip),
        ("multiple.intoto.jsonl", Skip),
        ("install.sh", Skip),
        ("install.ps1", Skip),
        ("README.md", Skip),
        ("LICENSE", Skip),
    ]
}

#[test]
fn names_are_classified_as_expected() {
    let mut wrong = Vec::new();
    for (name, want) in corpus() {
        let got = actual(name);
        if got != want {
            wrong.push(format!("  {name}\n      want {want:?}\n      got  {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{} of {} names misclassified:\n{}", wrong.len(), corpus().len(), wrong.join("\n"));
}

#[test]
fn a_release_candidate_is_preferred_over_a_debug_apk() {
    let p = Platform::preset("android-tv").unwrap();
    let score = |name: &str| evaluate(&classify(name).unwrap(), &p).score;
    assert!(score("jellyfin-androidtv-v0.17.4-release.apk") > score("jellyfin-androidtv-v0.17.4-debug.apk"));
}

#[test]
fn nothing_usable_is_ever_for_another_operating_system() {
    // The property the whole store rests on, over every name and every device preset.
    for preset in PRESETS {
        let platform = Platform::preset(preset).unwrap();
        for (name, _) in corpus() {
            let Some(info) = classify(name) else { continue };
            let verdict = evaluate(&info, &platform);
            if verdict.level == Level::Incompatible {
                assert!(verdict.note.is_some_and(|n| !n.is_empty()), "{name} hidden on {preset} without a reason");
                continue;
            }
            assert_eq!(info.os, TargetOs::Known(platform.os), "{name} offered on {preset}");
            if let TargetArch::One(arch) = info.arch {
                assert!(platform.can_run(arch), "{name} ({arch}) offered on {preset}");
            }
        }
    }
}

#[test]
fn hostile_and_odd_names_never_panic() {
    let odd = [
        "",
        " ",
        ".",
        "..",
        "...",
        "/",
        "\\",
        "a/b/c.exe",
        "../../x.deb",
        "\0",
        "\n",
        "😀.apk",
        "ｘ８６.exe",
        "ÀÉÎ-linux-x86_64.tar.gz",
        ".apk",
        ".exe",
        "-",
        "_",
        "x86_64",
        "linux",
        "a.b.c.d.e.f.g",
        "ÿ",
        "name with spaces linux amd64.AppImage",
        "UPPER-LINUX-AMD64.APPIMAGE",
        "tab\there.msi",
    ];
    for name in odd {
        let _ = classify(name);
    }
    let long = "a-".repeat(50_000) + "linux-x86_64.tar.gz";
    assert!(classify(&long).is_some());

    // A deterministic shuffle of name fragments: thousands of plausible and absurd names.
    let parts = [
        "app",
        "-",
        "_",
        ".",
        "1.2.3",
        "v",
        "linux",
        "windows",
        "darwin",
        "macos",
        "android",
        "ios",
        "x86_64",
        "amd64",
        "arm64",
        "aarch64",
        "armv7",
        "i686",
        "x86",
        "universal",
        "musl",
        "gnu",
        "tv",
        "debug",
        "setup",
        "src",
        ".tar.gz",
        ".zip",
        ".exe",
        ".msi",
        ".apk",
        ".deb",
        ".rpm",
        ".dmg",
        ".AppImage",
        ".sha256",
        "é",
        " ",
        "+",
        "(1)",
    ];
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let platforms: Vec<Platform> = PRESETS.iter().map(|p| Platform::preset(p).unwrap()).collect();
    for _ in 0..20_000 {
        let name: String = (0..(next() % 9 + 1)).map(|_| parts[(next() % parts.len() as u64) as usize]).collect();
        if let Some(info) = classify(&name) {
            for platform in &platforms {
                let verdict = evaluate(&info, platform);
                // Whatever the name, an offer is always for this OS.
                if verdict.level != Level::Incompatible {
                    assert_eq!(info.os, TargetOs::Known(platform.os), "{name:?}");
                }
            }
        }
    }
}
