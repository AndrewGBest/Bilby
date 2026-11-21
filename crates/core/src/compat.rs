//! Decides whether a classified asset works on a given [`Platform`], and ranks the ones
//! that do so the store can offer a single "Install" button.

use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::platform::{Arch, FormFactor, Libc, LinuxFamily, Os, Platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Cannot be used here. Hidden by default.
    Incompatible,
    /// Works, with a caveat (emulation, or the name left the architecture to a guess).
    Compatible,
    /// Built for exactly this machine.
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub level: Level,
    /// Caveat for `Compatible`, reason for `Incompatible`.
    pub note: Option<String>,
    /// Higher is better. Only meaningful between assets of the same release.
    pub score: i32,
}

impl Verdict {
    fn no(reason: impl Into<String>) -> Verdict {
        Verdict { level: Level::Incompatible, note: Some(reason.into()), score: 0 }
    }
    pub fn usable(&self) -> bool {
        self.level != Level::Incompatible
    }
}

pub fn evaluate(info: &AssetInfo, p: &Platform) -> Verdict {
    // 1. Operating system.
    match &info.os {
        TargetOs::Known(os) if *os == p.os => {}
        TargetOs::Known(os) => return Verdict::no(format!("Built for {os}")),
        TargetOs::Foreign(name) => return Verdict::no(format!("Built for {name}")),
        TargetOs::Unknown => return Verdict::no("File name does not say which system it is for"),
    }

    // 2. Can this system install that kind of package at all?
    if info.tv && p.os == Os::Android && p.form_factor != FormFactor::Tv {
        // Installs, but has no launcher icon and expects a remote: useless on a phone.
        return Verdict::no("Built for Android TV");
    }
    if info.format == Format::ApkBundle {
        return Verdict::no("Split APK bundles need a dedicated installer (not supported yet)");
    }
    if let Some(linux) = &p.linux {
        let missing = match info.format {
            Format::Deb if !linux.dpkg => Some("Debian packages need a Debian/Ubuntu based system"),
            Format::Rpm if !linux.rpm => Some("RPM packages need a Fedora/openSUSE based system"),
            Format::PacmanPkg if !linux.pacman => Some("Pacman packages need an Arch based system"),
            Format::AurBuild if linux.family != LinuxFamily::Arch => Some("The AUR is for Arch based systems"),
            Format::RepoPackage if !(linux.pacman || linux.dpkg || linux.rpm) => {
                Some("No supported package manager on this system")
            }
            Format::Flatpak | Format::FlathubApp if !linux.flatpak => Some("Flatpak is not installed"),
            Format::Snap | Format::SnapStore if !linux.snap => Some("snapd is not installed"),
            Format::AppImage if linux.libc == Libc::Musl => Some("AppImages need glibc; this system uses musl"),
            _ => None,
        };
        if let Some(reason) = missing {
            return Verdict::no(reason);
        }
        // glibc-linked programs do not start on a musl system. The reverse is fine:
        // musl builds are almost always statically linked.
        if info.libc == Some(Libc::Gnu) && linux.libc == Libc::Musl {
            return Verdict::no("Linked against glibc; this system uses musl");
        }
    }

    // 3. CPU architecture.
    let mut level = Level::Native;
    let mut note = None;
    let mut score = 0;
    match &info.arch {
        TargetArch::One(a) if *a == p.arch => {}
        TargetArch::One(a) if p.also_runs.contains(a) => {
            level = Level::Compatible;
            note = Some(emulation_note(*a, p));
        }
        TargetArch::One(a) => return Verdict::no(format!("Built for {a} processors")),
        TargetArch::Foreign(a) => return Verdict::no(format!("Built for {a} processors")),
        TargetArch::Universal | TargetArch::Any => score -= 1,
        TargetArch::Unknown => {
            // The name is silent. Use what such files overwhelmingly are in practice
            // and say that it is an assumption.
            let assumed = match (p.os, info.format) {
                // A lone APK is normally a universal one; AUR recipes build for the host.
                (Os::Android, _)
                | (Os::Ios, _)
                | (
                    _,
                    Format::AurBuild
                    | Format::RepoPackage
                    | Format::FlathubApp
                    | Format::SnapStore
                    | Format::WingetPackage
                    | Format::StoreApp,
                ) => None,
                // macOS downloads without an arch are universal or Intel; both run.
                (Os::MacOs, _) if p.can_run(Arch::X86_64) => None,
                _ => Some(Arch::X86_64),
            };
            match assumed {
                None => score -= 2,
                Some(a) if a == p.arch => {
                    score -= 5;
                }
                Some(a) if p.also_runs.contains(&a) => {
                    level = Level::Compatible;
                    note = Some(format!("Architecture not stated; assumed {a}. {}", emulation_note(a, p)));
                }
                Some(a) => {
                    return Verdict::no(format!("Architecture not stated; such files are normally {a}"));
                }
            }
        }
    }

    // 4. Platform caveats that do not block.
    if p.os == Os::Ios {
        level = Level::Compatible;
        note = Some("iOS cannot install this directly: it is handed to a sideloading tool".to_string());
    }

    // 5. Rank.
    score += format_preference(info.format, p);
    let system_libc = p.linux.as_ref().map(|l| l.libc);
    match (system_libc, info.libc, info.format) {
        // On glibc systems prefer the glibc build when both exist.
        (Some(Libc::Gnu), Some(Libc::Musl), _) => score -= 1,
        // On musl systems a prebuilt program that does not say "musl" is usually
        // glibc-linked. Offer it, but below an explicit musl build and with a warning.
        (Some(Libc::Musl), None, Format::Archive | Format::Binary) => {
            level = Level::Compatible;
            note = Some("May be linked against glibc; this system uses musl".to_string());
        }
        _ => {}
    }
    match (p.form_factor, info.tv) {
        (FormFactor::Tv, true) => score += 20,
        (FormFactor::Tv, false) => {}
        (_, true) => score -= 20,
        (_, false) => {}
    }
    if info.debug {
        score -= 15;
    }
    score += match level {
        Level::Native => 100,
        Level::Compatible => 50,
        Level::Incompatible => 0,
    };
    Verdict { level, note, score }
}

fn emulation_note(asset_arch: Arch, p: &Platform) -> String {
    match (p.os, p.arch, asset_arch) {
        (Os::Windows, Arch::Aarch64, _) => format!("Runs through Windows' {asset_arch} emulation"),
        (Os::Windows, Arch::X86_64, Arch::X86) => "32-bit program".to_string(),
        (Os::MacOs, Arch::Aarch64, Arch::X86_64) => "Intel build; runs through Rosetta 2".to_string(),
        (Os::Android, _, _) => format!("{asset_arch} build; supported as a secondary ABI"),
        (Os::Linux, _, Arch::X86) => "32-bit program; needs 32-bit libraries".to_string(),
        _ => format!("{asset_arch} build"),
    }
}

/// How natural a format is on this platform, 0..=30. This is what makes an Ubuntu user
/// get the `.deb` and an Arch user the pacman package from the same release.
fn format_preference(format: Format, p: &Platform) -> i32 {
    use Format::*;
    match p.os {
        Os::Windows => match format {
            Msi => 30,
            Exe => 28,
            Msix => 24,
            Archive => 15,
            _ => 5,
        },
        Os::MacOs => match format {
            Dmg => 30,
            MacPkg => 26,
            Archive => 15,
            _ => 5,
        },
        Os::Android => match format {
            Apk => 30,
            _ => 5,
        },
        Os::Ios => match format {
            Ipa => 30,
            _ => 0,
        },
        Os::Linux => {
            let family = p.linux.as_ref().map(|l| l.family).unwrap_or(LinuxFamily::Other);
            let native = match family {
                LinuxFamily::Debian => Some(Deb),
                LinuxFamily::Fedora | LinuxFamily::Suse => Some(Rpm),
                LinuxFamily::Arch => Some(PacmanPkg),
                LinuxFamily::Alpine | LinuxFamily::Other => None,
            };
            if Some(format) == native {
                return 30;
            }
            // Maintained and updated by the distribution: the first choice when offered.
            if format == RepoPackage {
                return 32;
            }
            match format {
                AppImage => 24,
                Flatpak => 22,
                // Building from the AUR is slower and runs a build script, so a
                // prebuilt package from the same project wins when both exist.
                AurBuild => 20,
                Archive => 14,
                Binary => 12,
                Snap | SnapStore => 10,
                _ => 5,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::classify;

    fn verdict(name: &str, preset: &str) -> Verdict {
        evaluate(&classify(name).unwrap(), &Platform::preset(preset).unwrap())
    }
    fn level(name: &str, preset: &str) -> Level {
        verdict(name, preset).level
    }

    /// Pick the winner the way the catalog does.
    fn best<'a>(names: &[&'a str], preset: &str) -> Option<&'a str> {
        names
            .iter()
            .map(|n| (*n, verdict(n, preset)))
            .filter(|(_, v)| v.usable())
            .max_by_key(|(_, v)| v.score)
            .map(|(n, _)| n)
    }

    const DESKTOP_APP: &[&str] = &[
        "Thing-2.0-x64-Setup.exe",
        "Thing-2.0-arm64-Setup.exe",
        "Thing-2.0-x64.msi",
        "Thing-2.0-win-x64.zip",
        "Thing-2.0-universal.dmg",
        "Thing-2.0-x86_64.AppImage",
        "Thing-2.0-aarch64.AppImage",
        "thing_2.0_amd64.deb",
        "thing_2.0_arm64.deb",
        "thing-2.0-1.x86_64.rpm",
        "thing-2.0-1-x86_64.pkg.tar.zst",
        "thing-2.0-linux-x86_64.tar.gz",
        "thing-2.0-linux-x86_64-musl.tar.gz",
        "Thing-2.0-arm64-v8a.apk",
        "Thing-2.0-armeabi-v7a.apk",
        "Thing-2.0-universal.apk",
        "Thing-2.0.ipa",
    ];

    #[test]
    fn each_platform_gets_its_natural_package() {
        let cases = [
            ("windows-x64", "Thing-2.0-x64.msi"),
            ("windows-arm64", "Thing-2.0-arm64-Setup.exe"),
            ("linux-x64-debian", "thing_2.0_amd64.deb"),
            ("linux-arm64-debian", "thing_2.0_arm64.deb"),
            ("linux-x64-fedora", "thing-2.0-1.x86_64.rpm"),
            ("linux-x64-arch", "thing-2.0-1-x86_64.pkg.tar.zst"),
            ("linux-x64-alpine", "thing-2.0-linux-x86_64-musl.tar.gz"),
            ("macos-arm64", "Thing-2.0-universal.dmg"),
            ("macos-x64", "Thing-2.0-universal.dmg"),
            ("android-arm64", "Thing-2.0-arm64-v8a.apk"),
            ("android-arm32", "Thing-2.0-armeabi-v7a.apk"),
            ("android-tv", "Thing-2.0-arm64-v8a.apk"),
            ("ios", "Thing-2.0.ipa"),
        ];
        for (preset, expected) in cases {
            assert_eq!(best(DESKTOP_APP, preset), Some(expected), "{preset}");
        }
    }

    #[test]
    fn wrong_os_is_incompatible() {
        assert_eq!(level("Thing-2.0-x64.msi", "linux-x64-debian"), Level::Incompatible);
        assert_eq!(level("Thing-2.0-universal.dmg", "windows-x64"), Level::Incompatible);
        assert_eq!(level("thing_2.0_amd64.deb", "android-arm64"), Level::Incompatible);
        // A Linux binary is not an Android app even on the same CPU.
        assert_eq!(level("thing-2.0-linux-aarch64.tar.gz", "android-arm64"), Level::Incompatible);
        assert_eq!(level("thing-x86_64-unknown-freebsd.tar.gz", "linux-x64-debian"), Level::Incompatible);
    }

    #[test]
    fn wrong_cpu_is_incompatible() {
        assert_eq!(level("thing_2.0_arm64.deb", "linux-x64-debian"), Level::Incompatible);
        assert_eq!(level("Thing-2.0-x86_64.apk", "android-arm64"), Level::Incompatible);
        assert_eq!(level("Thing-2.0-arm64-v8a.apk", "android-arm32"), Level::Incompatible);
        assert_eq!(level("thing-linux-ppc64le.tar.gz", "linux-x64-debian"), Level::Incompatible);
    }

    #[test]
    fn linux_package_formats_follow_the_distro() {
        assert_eq!(level("thing_2.0_amd64.deb", "linux-x64-fedora"), Level::Incompatible);
        assert_eq!(level("thing-2.0-1.x86_64.rpm", "linux-x64-debian"), Level::Incompatible);
        assert_eq!(level("thing-2.0-1-x86_64.pkg.tar.zst", "linux-x64-debian"), Level::Incompatible);
        assert_eq!(level("Thing-2.0-x86_64.AppImage", "linux-x64-fedora"), Level::Native);
    }

    #[test]
    fn musl_and_glibc() {
        assert_eq!(level("t-x86_64-unknown-linux-gnu.tar.gz", "linux-x64-alpine"), Level::Incompatible);
        assert_eq!(level("t-x86_64-unknown-linux-musl.tar.gz", "linux-x64-alpine"), Level::Native);
        assert_eq!(level("t-x86_64-unknown-linux-musl.tar.gz", "linux-x64-debian"), Level::Native);
        assert_eq!(level("Thing-2.0-x86_64.AppImage", "linux-x64-alpine"), Level::Incompatible);
        // Both offered on glibc: the glibc build is preferred.
        assert_eq!(
            best(&["t-x86_64-unknown-linux-musl.tar.gz", "t-x86_64-unknown-linux-gnu.tar.gz"], "linux-x64-debian"),
            Some("t-x86_64-unknown-linux-gnu.tar.gz")
        );
    }

    #[test]
    fn emulation_is_offered_with_a_note_and_ranked_below_native() {
        let v = verdict("Thing-2.0-x64.msi", "windows-arm64");
        assert_eq!(v.level, Level::Compatible);
        assert!(v.note.unwrap().contains("emulation"));
        let v = verdict("thing-x86_64-apple-darwin.tar.gz", "macos-arm64");
        assert_eq!(v.level, Level::Compatible);
        assert!(v.note.unwrap().contains("Rosetta"));
        assert_eq!(level("thing-aarch64-apple-darwin.tar.gz", "macos-x64"), Level::Incompatible);
        assert_eq!(
            best(&["t-x86_64-apple-darwin.tar.gz", "t-aarch64-apple-darwin.tar.gz"], "macos-arm64"),
            Some("t-aarch64-apple-darwin.tar.gz")
        );
        // 32-bit APK on a 64-bit phone that still accepts 32-bit ABIs.
        assert_eq!(level("Thing-2.0-armeabi-v7a.apk", "android-arm64"), Level::Compatible);
    }

    #[test]
    fn unstated_architecture() {
        // Lone APK: treated as universal.
        assert_eq!(level("Thing.apk", "android-arm64"), Level::Native);
        // Windows setup without arch: assumed x64 -> emulated on Arm, fine on x64.
        assert_eq!(level("ThingSetup.exe", "windows-x64"), Level::Native);
        assert_eq!(level("ThingSetup.exe", "windows-arm64"), Level::Compatible);
        // Linux AppImage without arch on an Arm board: hidden, we will not guess wrong.
        assert_eq!(level("Thing.AppImage", "linux-arm64-debian"), Level::Incompatible);
        assert_eq!(level("Thing.dmg", "macos-arm64"), Level::Native);
        // A file that names its arch beats one that does not.
        assert_eq!(
            best(&["Thing.AppImage", "Thing-x86_64.AppImage"], "linux-x64-fedora"),
            Some("Thing-x86_64.AppImage")
        );
    }

    #[test]
    fn tv_builds() {
        let both = ["player-arm64-v8a.apk", "player-androidtv-arm64-v8a.apk"];
        assert_eq!(best(&both, "android-tv"), Some("player-androidtv-arm64-v8a.apk"));
        assert_eq!(best(&both, "android-arm64"), Some("player-arm64-v8a.apk"));
        assert_eq!(level("player-androidtv-arm64-v8a.apk", "android-arm64"), Level::Incompatible);
        // The phone build still works on a TV; it just ranks below the TV build.
        assert_eq!(level("player-arm64-v8a.apk", "android-tv"), Level::Native);
    }

    #[test]
    fn unknown_os_and_bundles_are_hidden() {
        assert_eq!(level("project-1.0.zip", "linux-x64-debian"), Level::Incompatible);
        assert_eq!(level("app.xapk", "android-arm64"), Level::Incompatible);
    }

    #[test]
    fn ios_is_flagged_as_sideload() {
        let v = verdict("Thing-2.0.ipa", "ios");
        assert_eq!(v.level, Level::Compatible);
        assert!(v.note.unwrap().contains("sideloading"));
    }
}
