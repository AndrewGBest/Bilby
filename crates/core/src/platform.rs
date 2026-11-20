//! Describes the machine the store runs on.
//!
//! Everything the compatibility engine needs is captured in [`Platform`], a plain value.
//! [`Platform::detect`] fills it in from the running system; [`Platform::preset`] builds
//! one by name, which is what tests, the demo mode and the "preview another device"
//! switch use.

use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Windows,
    Linux,
    MacOs,
    Android,
    Ios,
}

impl Os {
    pub fn label(self) -> &'static str {
        match self {
            Os::Windows => "Windows",
            Os::Linux => "Linux",
            Os::MacOs => "macOS",
            Os::Android => "Android",
            Os::Ios => "iOS",
        }
    }

    /// Key used in the bundled index (`"os": ["linux", "windows"]`).
    pub fn key(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Linux => "linux",
            Os::MacOs => "macos",
            Os::Android => "android",
            Os::Ios => "ios",
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
    X86,
    Arm32,
    Riscv64,
}

impl Arch {
    pub fn label(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "arm64",
            Arch::X86 => "x86 (32-bit)",
            Arch::Arm32 => "arm (32-bit)",
            Arch::Riscv64 => "riscv64",
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Libc {
    Gnu,
    Musl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinuxFamily {
    Arch,
    Debian,
    Fedora,
    Suse,
    Alpine,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormFactor {
    Desktop,
    Mobile,
    Tv,
}

/// Linux-only facts: which package formats can this particular distro install?
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxInfo {
    pub distro: String,
    pub family: LinuxFamily,
    pub libc: Libc,
    pub dpkg: bool,
    pub rpm: bool,
    pub pacman: bool,
    pub flatpak: bool,
    pub snap: bool,
    /// `paru`, `yay`, ... if one is installed.
    pub aur_helper: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    /// The machine's native CPU architecture.
    pub arch: Arch,
    /// Other architectures this machine can run (emulation, multilib, extra Android ABIs).
    pub also_runs: Vec<Arch>,
    pub linux: Option<LinuxInfo>,
    pub form_factor: FormFactor,
    /// Human readable OS name, e.g. "Ubuntu 24.04" or "Windows".
    pub os_name: String,
}

pub const PRESETS: &[&str] = &[
    "windows-x64",
    "windows-arm64",
    "linux-x64-debian",
    "linux-x64-fedora",
    "linux-x64-arch",
    "linux-x64-alpine",
    "linux-arm64-debian",
    "macos-arm64",
    "macos-x64",
    "android-arm64",
    "android-arm32",
    "android-tv",
    "linux-tv",
    "ios",
];

impl Platform {
    /// Short title, e.g. `Linux · x86_64`.
    /// The device in everyday words, for people who do not know what x86_64 is.
    pub fn friendly(&self) -> &'static str {
        match (self.os, self.form_factor) {
            (_, FormFactor::Tv) => "This TV",
            (Os::Windows, _) => "This Windows PC",
            (Os::Linux, _) => "This Linux computer",
            (Os::MacOs, _) => "This Mac",
            (Os::Android, _) => "This Android device",
            (Os::Ios, _) => "This iPhone or iPad",
        }
    }

    pub fn title(&self) -> String {
        let os = match (self.os, self.form_factor) {
            (Os::Android, FormFactor::Tv) => "Android TV",
            (os, _) => os.label(),
        };
        format!("{os} · {}", self.arch.label())
    }

    /// One fact per line, shown on the "This device" page.
    pub fn facts(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("Operating system".to_string(), self.os_name.clone()),
            ("Processor".to_string(), self.arch.label().to_string()),
        ];
        if !self.also_runs.is_empty() {
            let list: Vec<_> = self.also_runs.iter().map(|a| a.label()).collect();
            out.push(("Can also run".to_string(), list.join(", ")));
        }
        if let Some(l) = &self.linux {
            out.push((
                "C library".to_string(),
                match l.libc {
                    Libc::Gnu => "glibc",
                    Libc::Musl => "musl",
                }
                .to_string(),
            ));
            let mut formats = Vec::new();
            if l.pacman {
                formats.push("pacman packages");
            }
            if l.dpkg {
                formats.push(".deb");
            }
            if l.rpm {
                formats.push(".rpm");
            }
            if l.libc == Libc::Gnu {
                formats.push("AppImage");
            }
            if l.flatpak {
                formats.push("Flatpak");
            }
            if l.snap {
                formats.push("Snap");
            }
            formats.push("portable archives");
            out.push(("Installable formats".to_string(), formats.join(", ")));
            if l.family == LinuxFamily::Arch {
                out.push((
                    "AUR".to_string(),
                    match &l.aur_helper {
                        Some(h) => format!("available (via {h})"),
                        None => "available (via makepkg)".to_string(),
                    },
                ));
            }
        }
        out.push((
            "Interface".to_string(),
            match self.form_factor {
                FormFactor::Desktop => "Desktop",
                FormFactor::Mobile => "Mobile",
                FormFactor::Tv => "TV",
            }
            .to_string(),
        ));
        out
    }

    pub fn can_run(&self, arch: Arch) -> bool {
        arch == self.arch || self.also_runs.contains(&arch)
    }

    /// A named, fully specified platform. See [`PRESETS`].
    pub fn preset(name: &str) -> Option<Platform> {
        let linux = |distro: &str, family, libc| {
            Some(LinuxInfo {
                distro: distro.to_string(),
                family,
                libc,
                dpkg: family == LinuxFamily::Debian,
                rpm: matches!(family, LinuxFamily::Fedora | LinuxFamily::Suse),
                pacman: family == LinuxFamily::Arch,
                flatpak: !matches!(family, LinuxFamily::Alpine),
                snap: false,
                aur_helper: (family == LinuxFamily::Arch).then(|| "paru".to_string()),
            })
        };
        let p = |os, arch, also_runs: &[Arch], linux: Option<LinuxInfo>, ff, os_name: &str| Platform {
            os,
            arch,
            also_runs: also_runs.to_vec(),
            os_name: linux.as_ref().map(|l| l.distro.clone()).unwrap_or_else(|| os_name.to_string()),
            linux,
            form_factor: ff,
        };
        use Arch::*;
        use FormFactor::*;
        Some(match name {
            "windows-x64" => p(Os::Windows, X86_64, &[X86], None, Desktop, "Windows 11"),
            "windows-arm64" => p(Os::Windows, Aarch64, &[X86_64, X86], None, Desktop, "Windows 11 on Arm"),
            "linux-x64-debian" => {
                p(Os::Linux, X86_64, &[], linux("Ubuntu 24.04", LinuxFamily::Debian, Libc::Gnu), Desktop, "")
            }
            "linux-x64-fedora" => {
                p(Os::Linux, X86_64, &[], linux("Fedora Linux 42", LinuxFamily::Fedora, Libc::Gnu), Desktop, "")
            }
            "linux-x64-arch" => {
                p(Os::Linux, X86_64, &[], linux("Arch Linux", LinuxFamily::Arch, Libc::Gnu), Desktop, "")
            }
            "linux-x64-alpine" => {
                p(Os::Linux, X86_64, &[], linux("Alpine Linux", LinuxFamily::Alpine, Libc::Musl), Desktop, "")
            }
            "linux-arm64-debian" => {
                p(Os::Linux, Aarch64, &[], linux("Debian 13", LinuxFamily::Debian, Libc::Gnu), Desktop, "")
            }
            "macos-arm64" => p(Os::MacOs, Aarch64, &[X86_64], None, Desktop, "macOS (Apple silicon)"),
            "macos-x64" => p(Os::MacOs, X86_64, &[], None, Desktop, "macOS (Intel)"),
            "android-arm64" => p(Os::Android, Aarch64, &[Arm32], None, Mobile, "Android"),
            "android-arm32" => p(Os::Android, Arm32, &[], None, Mobile, "Android"),
            "android-tv" => p(Os::Android, Aarch64, &[Arm32], None, Tv, "Android TV"),
            // A small computer or set-top box running Linux, used from the sofa.
            "linux-tv" => {
                p(Os::Linux, X86_64, &[], linux("Debian GNU/Linux 12", LinuxFamily::Debian, Libc::Gnu), Tv, "")
            }
            "ios" => p(Os::Ios, Aarch64, &[], None, Mobile, "iOS"),
            _ => return None,
        })
    }

    /// Inspect the running system.
    pub fn detect() -> Platform {
        let os = if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "android") {
            Os::Android
        } else if cfg!(target_os = "ios") {
            Os::Ios
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        };
        let arch = detect_arch(os);
        let linux = (os == Os::Linux).then(detect_linux);
        let also_runs = detect_also_runs(os, arch);
        let os_name = match (&linux, os) {
            (Some(l), _) => l.distro.clone(),
            (None, Os::Android) if detect_form_factor(os) == FormFactor::Tv => "Android TV".to_string(),
            (None, _) => os.label().to_string(),
        };
        Platform { os, arch, also_runs, linux, form_factor: detect_form_factor(os), os_name }
    }
}

fn arch_from_name(name: &str) -> Option<Arch> {
    Some(match name.to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => Arch::X86_64,
        "aarch64" | "arm64" | "arm64-v8a" => Arch::Aarch64,
        "x86" | "i386" | "i586" | "i686" => Arch::X86,
        "arm" | "armv7" | "armeabi-v7a" | "armeabi" => Arch::Arm32,
        "riscv64" => Arch::Riscv64,
        _ => return None,
    })
}

fn detect_arch(os: Os) -> Arch {
    // The architecture this binary was compiled for. Usually that is the machine's
    // architecture, but a 32-bit or emulated build can run on a 64-bit host, so each OS
    // gets a chance to correct it.
    let built = arch_from_name(std::env::consts::ARCH).unwrap_or(Arch::X86_64);
    match os {
        Os::Windows => {
            // PROCESSOR_ARCHITEW6432 is set for a 32-bit process on 64-bit Windows.
            // (An x64 process emulated on Arm sees AMD64 in both; the precise answer
            // needs IsWow64Process2, see docs/DESIGN.md.)
            let native =
                std::env::var("PROCESSOR_ARCHITEW6432").or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE")).ok();
            native.as_deref().and_then(arch_from_name).unwrap_or(built)
        }
        Os::Android => android::abis().first().copied().unwrap_or(built),
        _ => built,
    }
}

fn detect_also_runs(os: Os, arch: Arch) -> Vec<Arch> {
    match (os, arch) {
        // Windows 11 on Arm emulates x64 and x86; 64-bit Windows runs 32-bit via WoW64.
        (Os::Windows, Arch::Aarch64) => vec![Arch::X86_64, Arch::X86],
        (Os::Windows, Arch::X86_64) => vec![Arch::X86],
        // Rosetta 2.
        (Os::MacOs, Arch::Aarch64) => vec![Arch::X86_64],
        // The device tells us exactly which ABIs it accepts.
        (Os::Android, _) => android::abis().into_iter().filter(|a| *a != arch).collect(),
        // Linux multilib is rarely set up; 32-bit is offered only when the loader exists.
        (Os::Linux, Arch::X86_64) if Path::new("/lib/ld-linux.so.2").exists() => vec![Arch::X86],
        _ => vec![],
    }
}

fn detect_form_factor(os: Os) -> FormFactor {
    match os {
        Os::Android => {
            // Android TV / Google TV / Fire TV images carry "tv" in ro.build.characteristics.
            // UiModeManager is the authoritative API; this property needs no JNI.
            let ch = android::prop("ro.build.characteristics").unwrap_or_default();
            if ch.split(',').any(|c| c.trim() == "tv") {
                FormFactor::Tv
            } else {
                FormFactor::Mobile
            }
        }
        Os::Ios => FormFactor::Mobile,
        _ => {
            let env = |name: &str| std::env::var(name).ok();
            match couch_session(&env, &running_commands()) {
                Some(_) => FormFactor::Tv,
                None => FormFactor::Desktop,
            }
        }
    }
}

/// The command lines of running programs (Linux only; empty elsewhere).
fn running_commands() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return vec![] };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|e| std::fs::read(e.path().join("cmdline")).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
        .filter(|line| !line.is_empty())
        .collect()
}

/// Is this desktop system set up to be used from the sofa? Returns what gave it away.
///
/// A TV is not something a computer reports, so this looks for the sessions people run
/// on one: Plasma Bigscreen, Steam's big-screen session (gamescope), Kodi. An ordinary
/// desktop shown on a television is not detected; Settings has a switch for that.
pub fn couch_session(env: &dyn Fn(&str) -> Option<String>, commands: &[String]) -> Option<&'static str> {
    let session: String = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
        .iter()
        .filter_map(|name| env(name))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty() && v != "0");
    let running = |what: &str| commands.iter().any(|c| c.to_lowercase().contains(what));
    if session.contains("bigscreen") || running("plasma.bigscreen") || running("plasma-bigscreen") {
        Some("Plasma Bigscreen")
    } else if session.contains("gamescope") || set("GAMESCOPE_WAYLAND_DISPLAY") || set("SteamTenfoot") {
        Some("Steam's big-screen mode")
    } else if session.contains("kodi") {
        Some("Kodi")
    } else {
        None
    }
}

/// A game controller is plugged in or paired: a hint, not proof, that this is a TV.
pub fn controller_attached() -> bool {
    cfg!(target_os = "linux") && (0..4).any(|n| std::path::Path::new(&format!("/dev/input/js{n}")).exists())
}

/// Parse the contents of `/etc/os-release` into (pretty name, family).
pub fn parse_os_release(text: &str) -> (String, LinuxFamily) {
    let mut id = String::new();
    let mut like = String::new();
    let mut pretty = String::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim_matches('\'');
        match k.trim() {
            "ID" => id = v.to_lowercase(),
            "ID_LIKE" => like = v.to_lowercase(),
            "PRETTY_NAME" => pretty = v.to_string(),
            _ => {}
        }
    }
    let ids: Vec<&str> = std::iter::once(id.as_str()).chain(like.split_whitespace()).collect();
    let has = |names: &[&str]| ids.iter().any(|i| names.contains(i));
    let family = if has(&["arch", "archlinux"]) {
        LinuxFamily::Arch
    } else if has(&["debian", "ubuntu"]) {
        LinuxFamily::Debian
    } else if has(&["fedora", "rhel", "centos"]) {
        LinuxFamily::Fedora
    } else if has(&["suse", "opensuse"]) || id.starts_with("opensuse") {
        LinuxFamily::Suse
    } else if has(&["alpine"]) {
        LinuxFamily::Alpine
    } else {
        LinuxFamily::Other
    };
    if pretty.is_empty() {
        pretty = if id.is_empty() { "Linux".to_string() } else { id };
    }
    (pretty, family)
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

fn detect_linux() -> LinuxInfo {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let (distro, family) = parse_os_release(&text);
    // A musl system has musl's loader and no glibc one.
    let has_musl_loader = std::fs::read_dir("/lib")
        .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-")))
        .unwrap_or(false);
    let has_glibc_loader = ["/lib64/ld-linux-x86-64.so.2", "/lib/ld-linux-aarch64.so.1", "/lib/ld-linux-armhf.so.3"]
        .iter()
        .any(|p| Path::new(p).exists());
    let libc = if has_musl_loader && !has_glibc_loader { Libc::Musl } else { Libc::Gnu };
    let pacman = on_path("pacman");
    LinuxInfo {
        distro,
        family,
        libc,
        dpkg: on_path("dpkg"),
        rpm: on_path("rpm") && !on_path("dpkg"),
        pacman,
        flatpak: on_path("flatpak"),
        snap: on_path("snap"),
        aur_helper: ["paru", "yay", "pikaur", "aura"].iter().find(|h| on_path(h)).map(|h| h.to_string()),
    }
}

mod android {
    use super::Arch;

    #[cfg(target_os = "android")]
    pub fn prop(name: &str) -> Option<String> {
        use std::ffi::{CStr, CString};
        let name = CString::new(name).ok()?;
        let mut buf = [0u8; 92]; // PROP_VALUE_MAX
                                 // SAFETY: buf is PROP_VALUE_MAX bytes, as __system_property_get requires.
        let len = unsafe { libc::__system_property_get(name.as_ptr(), buf.as_mut_ptr().cast()) };
        if len <= 0 {
            return None;
        }
        CStr::from_bytes_until_nul(&buf).ok().map(|s| s.to_string_lossy().into_owned())
    }

    #[cfg(not(target_os = "android"))]
    pub fn prop(_name: &str) -> Option<String> {
        None
    }

    /// ABIs the device accepts, most preferred first (same list as `Build.SUPPORTED_ABIS`).
    pub fn abis() -> Vec<Arch> {
        prop("ro.product.cpu.abilist")
            .unwrap_or_default()
            .split(',')
            .filter_map(|abi| super::arch_from_name(abi.trim()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_families() {
        let (name, fam) =
            parse_os_release("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n");
        assert_eq!(name, "Ubuntu 24.04.1 LTS");
        assert_eq!(fam, LinuxFamily::Debian);
        assert_eq!(parse_os_release("ID=endeavouros\nID_LIKE=arch\n").1, LinuxFamily::Arch);
        assert_eq!(parse_os_release("ID=manjaro\nID_LIKE=arch\n").1, LinuxFamily::Arch);
        assert_eq!(parse_os_release("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n").1, LinuxFamily::Suse);
        assert_eq!(parse_os_release("ID=rocky\nID_LIKE=\"rhel centos fedora\"\n").1, LinuxFamily::Fedora);
        assert_eq!(parse_os_release("ID=alpine\n").1, LinuxFamily::Alpine);
        assert_eq!(parse_os_release("ID=nixos\n").1, LinuxFamily::Other);
    }

    #[test]
    fn all_presets_exist() {
        for name in PRESETS {
            assert!(Platform::preset(name).is_some(), "{name}");
        }
        assert!(Platform::preset("amiga").is_none());
    }

    #[test]
    fn sofa_sessions_are_recognised_and_ordinary_desktops_are_not() {
        let with = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        let none: &[String] = &[];
        assert_eq!(couch_session(&with(&[("XDG_CURRENT_DESKTOP", "KDE:Bigscreen")]), none), Some("Plasma Bigscreen"));
        assert_eq!(
            couch_session(&with(&[("DESKTOP_SESSION", "plasma-bigscreen-wayland")]), none),
            Some("Plasma Bigscreen")
        );
        assert_eq!(
            couch_session(&with(&[("XDG_CURRENT_DESKTOP", "gamescope")]), none),
            Some("Steam's big-screen mode")
        );
        assert_eq!(couch_session(&with(&[("SteamTenfoot", "1")]), none), Some("Steam's big-screen mode"));
        assert_eq!(couch_session(&with(&[("XDG_SESSION_DESKTOP", "kodi")]), none), Some("Kodi"));
        // Bigscreen reporting itself as plain KDE is still found by its running shell.
        let shell = ["/usr/bin/plasmashell -p org.kde.plasma.bigscreen".to_string()];
        assert_eq!(couch_session(&with(&[("XDG_CURRENT_DESKTOP", "KDE")]), &shell), Some("Plasma Bigscreen"));
        // Ordinary desktops, and switches that are present but off.
        assert_eq!(
            couch_session(&with(&[("XDG_CURRENT_DESKTOP", "KDE")]), &["/usr/bin/plasmashell".to_string()]),
            None
        );
        assert_eq!(couch_session(&with(&[("XDG_CURRENT_DESKTOP", "GNOME"), ("SteamTenfoot", "0")]), none), None);
        assert_eq!(couch_session(&with(&[]), none), None);
    }

    #[test]
    fn detect_does_not_panic() {
        let p = Platform::detect();
        assert!(!p.title().is_empty());
        assert!(!p.facts().is_empty());
    }
}
