//! Works out what a release file is built for, from its file name alone.
//!
//! GitHub releases carry no structured platform metadata, so the name is all there is:
//! `ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz`, `app-arm64-v8a-release.apk`,
//! `Thing-Setup-2.0.exe`. [`classify`] turns such a name into an [`AssetInfo`], or
//! `None` for files that are not installable at all (checksums, signatures, sources).

use crate::platform::{Arch, Libc, Os};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Apk,
    /// `.apks` / `.xapk` / `.aab` – split bundles that need a special installer.
    ApkBundle,
    Exe,
    Msi,
    Msix,
    Deb,
    Rpm,
    AppImage,
    Flatpak,
    Snap,
    PacmanPkg,
    Dmg,
    MacPkg,
    Ipa,
    /// `.tar.gz`, `.zip`, ... containing a prebuilt program.
    Archive,
    /// A bare executable.
    Binary,
    /// An AUR package: a build recipe, not a file from a release.
    AurBuild,
    /// A package in a repository the system's package manager is configured for.
    RepoPackage,
    /// A website opened like an app (in Longboard, on a Linux TV). Not a file at all.
    WebApp,
    /// An app in the Flathub repository, installed by Flatpak itself.
    FlathubApp,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Apk => "APK",
            Format::ApkBundle => "APK bundle",
            Format::Exe => "Installer (.exe)",
            Format::Msi => "Installer (.msi)",
            Format::Msix => "MSIX package",
            Format::Deb => "Debian package",
            Format::Rpm => "RPM package",
            Format::AppImage => "AppImage",
            Format::Flatpak => "Flatpak bundle",
            Format::Snap => "Snap",
            Format::PacmanPkg => "Pacman package",
            Format::Dmg => "Disk image (.dmg)",
            Format::MacPkg => "Installer (.pkg)",
            Format::Ipa => "iOS app (.ipa)",
            Format::Archive => "Portable archive",
            Format::Binary => "Standalone binary",
            Format::AurBuild => "AUR package",
            Format::RepoPackage => "Repository package",
            Format::WebApp => "Web app",
            Format::FlathubApp => "Flatpak",
        }
    }

    /// Formats that only exist on one OS tell us the OS even when the name does not.
    pub fn implied_os(self) -> Option<Os> {
        Some(match self {
            Format::Apk | Format::ApkBundle => Os::Android,
            Format::Exe | Format::Msi | Format::Msix => Os::Windows,
            Format::Deb
            | Format::Rpm
            | Format::AppImage
            | Format::Flatpak
            | Format::Snap
            | Format::PacmanPkg
            | Format::AurBuild
            | Format::RepoPackage
            | Format::WebApp
            | Format::FlathubApp => Os::Linux,
            Format::Dmg | Format::MacPkg => Os::MacOs,
            Format::Ipa => Os::Ios,
            Format::Archive | Format::Binary => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetOs {
    Known(Os),
    /// An OS the store does not run on (FreeBSD, WASI, ...).
    Foreign(&'static str),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetArch {
    One(Arch),
    /// Contains code for every architecture of its OS (macOS universal, universal APK).
    Universal,
    /// Architecture independent (`_all.deb`, `noarch.rpm`).
    Any,
    /// An architecture the store does not know how to run.
    Foreign(&'static str),
    /// The name does not say.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    pub format: Format,
    pub os: TargetOs,
    pub arch: TargetArch,
    pub libc: Option<Libc>,
    /// Name mentions tv / leanback / firetv.
    pub tv: bool,
    /// A debug build (only kept for APKs).
    pub debug: bool,
}

impl AssetInfo {
    /// Short description for buttons and badges: "AppImage · x86_64", "APK · arm64 · TV".
    pub fn label(&self) -> String {
        let mut label = self.format.label().to_string();
        let arch = self.arch_label();
        // A website has no processor to be built for, and Flatpak picks the right build itself.
        if !arch.is_empty() && !matches!(self.format, Format::WebApp | Format::FlathubApp) {
            label.push_str(" · ");
            label.push_str(&arch);
        }
        // Two builds of the same archive differ only in this; say which is which.
        if self.libc == Some(Libc::Musl) {
            label.push_str(" · musl");
        }
        if self.tv {
            label.push_str(" · TV");
        }
        if self.debug {
            label.push_str(" · debug");
        }
        label
    }

    pub fn arch_label(&self) -> String {
        match &self.arch {
            TargetArch::One(a) => a.label().to_string(),
            TargetArch::Universal => "universal".to_string(),
            TargetArch::Any => "any CPU".to_string(),
            TargetArch::Foreign(n) => n.to_string(),
            TargetArch::Unknown => String::new(),
        }
    }
}

/// Lower-case the name and collapse every run of non-alphanumerics into `_`, with a `_`
/// at each end, so whole tokens can be matched with `contains("_arm64_")`.
/// `App-x86-64.tar.gz` becomes `_app_x86_64_tar_gz_`.
fn normalize(name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 2);
    s.push('_');
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('_') {
            s.push('_');
        }
    }
    if !s.ends_with('_') {
        s.push('_');
    }
    s
}

/// Files that accompany a release but are never the thing to install.
const NOT_INSTALLABLE_EXT: &[&str] = &[
    ".sha256",
    ".sha256sum",
    ".sha512",
    ".sha1",
    ".md5",
    ".sig",
    ".asc",
    ".minisig",
    ".pem",
    ".crt",
    ".cert",
    ".sigstore",
    ".bundle",
    ".jsonl",
    ".json",
    ".txt",
    ".md",
    ".yml",
    ".yaml",
    ".xml",
    ".blockmap",
    ".zsync",
    ".delta",
    ".pdb",
    ".dsym",
    ".dbg",
    ".debug",
    ".sbom",
    ".spdx",
    ".cdx",
    ".whl",
    ".vsix",
    ".nupkg",
    ".jar",
    ".pdf",
    ".html",
    ".png",
    ".svg",
    ".ico",
    ".patch",
    ".diff",
    ".rb",
    ".py",
    ".ps1",
    ".sh",
    ".bat",
    ".js",
    ".wasm",
    ".crx",
    ".xpi",
    ".gem",
    ".crate",
    ".love",
    ".iso",
    ".img",
];

const NOT_INSTALLABLE_TOKENS: &[&str] = &[
    "_sha256sums_",
    "_sha512sums_",
    "_checksums_",
    "_checksum_",
    "_src_",
    "_source_",
    "_sources_",
    "_symbols_",
    "_dbgsym_",
    "_debuginfo_",
    "_debugsource_",
    "_vendor_",
    "_vendored_",
    "_docs_",
    "_manifest_",
    "_devel_",
    "_dev_",
];

/// Updater payloads that sit next to the real installers (Tauri, Electron).
const NOT_INSTALLABLE_SUFFIX: &[&str] = &[".appimage.tar.gz", ".nsis.zip", ".msi.zip", ".deb.tar.gz", ".asar.gz"];

fn detect_format(lower: &str) -> Option<Format> {
    const BY_EXT: &[(&str, Format)] = &[
        (".pkg.tar.zst", Format::PacmanPkg),
        (".pkg.tar.xz", Format::PacmanPkg),
        (".app.zip", Format::Archive),
        (".app.tar.gz", Format::Archive),
        (".msixbundle", Format::Msix),
        (".appxbundle", Format::Msix),
        (".appimage", Format::AppImage),
        (".flatpak", Format::Flatpak),
        (".tar.gz", Format::Archive),
        (".tar.xz", Format::Archive),
        (".tar.zst", Format::Archive),
        (".tar.bz2", Format::Archive),
        (".msix", Format::Msix),
        (".appx", Format::Msix),
        (".apks", Format::ApkBundle),
        (".xapk", Format::ApkBundle),
        (".snap", Format::Snap),
        (".tgz", Format::Archive),
        (".txz", Format::Archive),
        (".tbz", Format::Archive),
        (".zip", Format::Archive),
        (".aab", Format::ApkBundle),
        (".apk", Format::Apk),
        (".exe", Format::Exe),
        (".msi", Format::Msi),
        (".deb", Format::Deb),
        (".rpm", Format::Rpm),
        (".dmg", Format::Dmg),
        (".pkg", Format::MacPkg),
        (".ipa", Format::Ipa),
        (".7z", Format::Archive),
        (".gz", Format::Archive),
        (".xz", Format::Archive),
        (".zst", Format::Archive),
        (".bz2", Format::Archive),
    ];
    BY_EXT.iter().find(|(ext, _)| lower.ends_with(ext)).map(|(_, f)| *f)
}

fn any(norm: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| norm.contains(n))
}

fn detect_os(norm: &str) -> TargetOs {
    // Order matters: `x86_64-linux-android` is Android, `pc-windows-gnu` is Windows,
    // `apple-darwin` must not be read as "win".
    if any(norm, &["_android", "_androidtv_", "_firetv_"]) {
        return TargetOs::Known(Os::Android);
    }
    if any(norm, &["_ios_", "_iphoneos_", "_ipados_", "_iphone_"]) {
        return TargetOs::Known(Os::Ios);
    }
    if any(
        norm,
        &[
            "_windows",
            "_win_",
            "_win32_",
            "_win64_",
            "_win10_",
            "_win11_",
            "_msvc_",
            "_mingw",
            "_winx64_",
            "_winarm64_",
        ],
    ) {
        return TargetOs::Known(Os::Windows);
    }
    if any(norm, &["_macos", "_darwin", "_osx", "_mac_", "_apple_", "_macintosh_"]) {
        return TargetOs::Known(Os::MacOs);
    }
    for (needle, name) in [
        ("_freebsd", "FreeBSD"),
        ("_openbsd", "OpenBSD"),
        ("_netbsd", "NetBSD"),
        ("_dragonfly", "DragonFly BSD"),
        ("_illumos", "illumos"),
        ("_solaris", "Solaris"),
        ("_sunos_", "SunOS"),
        ("_haiku_", "Haiku"),
        ("_plan9_", "Plan 9"),
        ("_aix_", "AIX"),
        ("_wasi", "WASI"),
        ("_wasm", "WebAssembly"),
        ("_emscripten_", "WebAssembly"),
    ] {
        if norm.contains(needle) {
            return TargetOs::Foreign(name);
        }
    }
    if any(
        norm,
        &[
            "_linux",
            "_gnu_",
            "_musl_",
            "_ubuntu",
            "_debian",
            "_fedora",
            "_manylinux",
            "_glibc",
            "_gnueabihf_",
            "_musleabihf_",
        ],
    ) {
        return TargetOs::Known(Os::Linux);
    }
    TargetOs::Unknown
}

fn detect_arch(norm: &str) -> TargetArch {
    use TargetArch::*;
    // 64-bit names first: `_x86_64_` contains `_x86_`, `_arm64_v8a_` precedes `_arm_`.
    if any(
        norm,
        &[
            "_x86_64_",
            "_amd64_",
            "_x64_",
            "_win64_",
            "_linux64_",
            "_64bit_",
            "_64_bit_",
            "_x8664_",
            "_winx64_",
            "_x86_64v3_",
        ],
    ) {
        return One(Arch::X86_64);
    }
    if any(
        norm,
        &[
            "_aarch64_",
            "_arm64_",
            "_arm_64_",
            "_arm64v8_",
            "_arm64v8a_",
            "_armv8_",
            "_armv8a_",
            "_armv8l_",
            "_arm64ec_",
            "_winarm64_",
        ],
    ) {
        return One(Arch::Aarch64);
    }
    if any(norm, &["_universal_", "_universal2_", "_fat_"]) {
        return Universal;
    }
    if any(
        norm,
        &[
            "_armv7_",
            "_armv7l_",
            "_armv7a_",
            "_armhf_",
            "_armeabi_",
            "_arm_",
            "_arm32_",
            "_armv6_",
            "_armv6l_",
            "_armel_",
            "_gnueabihf_",
            "_musleabihf_",
            "_armv7hl_",
        ],
    ) {
        return One(Arch::Arm32);
    }
    if any(
        norm,
        &[
            "_i386_",
            "_i486_",
            "_i586_",
            "_i686_",
            "_386_",
            "_x86_",
            "_ia32_",
            "_win32_",
            "_32bit_",
            "_32_bit_",
            "_linux32_",
        ],
    ) {
        return One(Arch::X86);
    }
    if any(norm, &["_riscv64_", "_riscv64gc_"]) {
        return One(Arch::Riscv64);
    }
    for (needle, name) in [
        ("_ppc64le_", "ppc64le"),
        ("_ppc64_", "ppc64"),
        ("_powerpc64le_", "ppc64le"),
        ("_powerpc64_", "ppc64"),
        ("_powerpc_", "powerpc"),
        ("_s390x_", "s390x"),
        ("_mips64", "mips64"),
        ("_mips", "mips"),
        ("_loong64_", "loongarch64"),
        ("_loongarch64_", "loongarch64"),
        ("_sparc64_", "sparc64"),
    ] {
        if norm.contains(needle) {
            return Foreign(name);
        }
    }
    if any(norm, &["_noarch_", "_all_", "_any_"]) {
        return Any;
    }
    Unknown
}

/// Classify a release file by name. `None` means "not something a person installs".
pub fn classify(name: &str) -> Option<AssetInfo> {
    let lower = name.to_ascii_lowercase();
    let norm = normalize(name);

    if any(&norm, NOT_INSTALLABLE_TOKENS) || NOT_INSTALLABLE_SUFFIX.iter().any(|suffix| lower.ends_with(suffix)) {
        return None;
    }
    let format = match detect_format(&lower) {
        Some(f) => f,
        None => {
            if NOT_INSTALLABLE_EXT.iter().any(|ext| lower.ends_with(ext)) {
                return None;
            }
            Format::Binary
        }
    };

    let named_os = detect_os(&norm);
    let os = match (format.implied_os(), named_os) {
        // `.apk` is also Alpine's package extension; trust an explicit Linux in the name.
        (Some(Os::Android), TargetOs::Known(Os::Linux)) if !norm.contains("_android") => return None,
        (Some(os), _) => TargetOs::Known(os),
        (None, named) => named,
    };
    // A bare file with no extension and no OS in its name is almost never a program
    // we can identify (scripts, completions, zipapps): skip it rather than guess.
    if format == Format::Binary && os == TargetOs::Unknown {
        return None;
    }

    // "debug" marks debugging leftovers for desktop builds, but many Android projects
    // only publish debug-signed APKs. Keep those, ranked below a release build.
    let debug = norm.contains("_debug_");
    if debug && format != Format::Apk {
        return None;
    }

    let libc = match os {
        TargetOs::Known(Os::Linux) if any(&norm, &["_musl_", "_musleabihf_", "_static_"]) => Some(Libc::Musl),
        TargetOs::Known(Os::Linux) if any(&norm, &["_gnu_", "_glibc", "_gnueabihf_"]) => Some(Libc::Gnu),
        _ => None,
    };
    let tv = any(&norm, &["_tv_", "_androidtv_", "_leanback_", "_firetv_", "_atv_", "_googletv_"]);

    Some(AssetInfo { format, os, arch: detect_arch(&norm), libc, tv, debug })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str) -> AssetInfo {
        classify(name).unwrap_or_else(|| panic!("{name} should be installable"))
    }
    fn os(name: &str) -> TargetOs {
        c(name).os
    }
    fn arch(name: &str) -> TargetArch {
        c(name).arch
    }

    #[test]
    fn rust_target_triples() {
        let a = c("ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz");
        assert_eq!(a.format, Format::Archive);
        assert_eq!(a.os, TargetOs::Known(Os::Linux));
        assert_eq!(a.arch, TargetArch::One(Arch::X86_64));
        assert_eq!(a.libc, Some(Libc::Musl));

        assert_eq!(os("bat-v0.24.0-aarch64-apple-darwin.tar.gz"), TargetOs::Known(Os::MacOs));
        assert_eq!(arch("bat-v0.24.0-aarch64-apple-darwin.tar.gz"), TargetArch::One(Arch::Aarch64));
        assert_eq!(os("fd-v10.1.0-x86_64-pc-windows-gnu.zip"), TargetOs::Known(Os::Windows));
        assert_eq!(os("fd-v10.1.0-i686-pc-windows-msvc.zip"), TargetOs::Known(Os::Windows));
        assert_eq!(arch("fd-v10.1.0-i686-pc-windows-msvc.zip"), TargetArch::One(Arch::X86));
        assert_eq!(arch("fd-v10.1.0-arm-unknown-linux-gnueabihf.tar.gz"), TargetArch::One(Arch::Arm32));
        assert_eq!(os("tool-x86_64-linux-android.tar.gz"), TargetOs::Known(Os::Android));
        assert_eq!(os("tool-x86_64-unknown-freebsd.tar.gz"), TargetOs::Foreign("FreeBSD"));
    }

    #[test]
    fn darwin_is_not_windows() {
        assert_eq!(os("thing_darwin_arm64.tar.gz"), TargetOs::Known(Os::MacOs));
        assert_eq!(os("thing-win-x64.zip"), TargetOs::Known(Os::Windows));
    }

    #[test]
    fn go_style_names() {
        assert_eq!(os("lazygit_0.44.1_Linux_x86_64.tar.gz"), TargetOs::Known(Os::Linux));
        assert_eq!(arch("lazygit_0.44.1_Linux_arm64.tar.gz"), TargetArch::One(Arch::Aarch64));
        assert_eq!(arch("lazygit_0.44.1_Windows_32-bit.zip"), TargetArch::One(Arch::X86));
        assert_eq!(arch("tool_1.0_linux_armv6.tar.gz"), TargetArch::One(Arch::Arm32));
        assert_eq!(arch("tool_1.0_linux_ppc64le.tar.gz"), TargetArch::Foreign("ppc64le"));
    }

    #[test]
    fn android_apks() {
        let a = c("app-arm64-v8a-release.apk");
        assert_eq!(a.os, TargetOs::Known(Os::Android));
        assert_eq!(a.arch, TargetArch::One(Arch::Aarch64));
        assert_eq!(arch("app-armeabi-v7a-release.apk"), TargetArch::One(Arch::Arm32));
        assert_eq!(arch("app-x86_64-release.apk"), TargetArch::One(Arch::X86_64));
        assert_eq!(arch("app-x86-release.apk"), TargetArch::One(Arch::X86));
        assert_eq!(arch("app-universal-release.apk"), TargetArch::Universal);
        assert_eq!(arch("NewApp_v1.2.apk"), TargetArch::Unknown);
        assert!(c("player-androidtv-arm64-v8a.apk").tv);
        assert!(c("player-tv-release.apk").tv);
        assert!(!c("player-release.apk").tv);
        assert_eq!(c("app.xapk").format, Format::ApkBundle);
    }

    #[test]
    fn alpine_apk_is_not_android() {
        assert!(classify("tool-1.0-r0.x86_64-linux-musl.apk").is_none());
    }

    #[test]
    fn native_package_formats() {
        assert_eq!(c("code_1.90.0_amd64.deb").format, Format::Deb);
        assert_eq!(arch("code_1.90.0_amd64.deb"), TargetArch::One(Arch::X86_64));
        assert_eq!(arch("fonts_1.0_all.deb"), TargetArch::Any);
        assert_eq!(arch("tool-1.0-1.noarch.rpm"), TargetArch::Any);
        assert_eq!(arch("tool-1.0-1.aarch64.rpm"), TargetArch::One(Arch::Aarch64));
        assert_eq!(c("Thing-1.0-x86_64.AppImage").format, Format::AppImage);
        assert_eq!(os("Thing-1.0-x86_64.AppImage"), TargetOs::Known(Os::Linux));
        assert_eq!(c("thing-1.0-1-x86_64.pkg.tar.zst").format, Format::PacmanPkg);
        assert_eq!(c("Thing-1.0-universal.dmg").arch, TargetArch::Universal);
        assert_eq!(c("Thing-1.0.pkg").format, Format::MacPkg);
        assert_eq!(c("Thing-Setup-1.0.exe").os, TargetOs::Known(Os::Windows));
        assert_eq!(c("Thing-1.0-arm64.msi").arch, TargetArch::One(Arch::Aarch64));
        assert_eq!(c("Thing.msixbundle").format, Format::Msix);
        assert_eq!(c("Thing-1.0.ipa").os, TargetOs::Known(Os::Ios));
    }

    #[test]
    fn electron_names() {
        // "win32" here names the OS; the explicit x64 wins for the architecture.
        assert_eq!(arch("app-win32-x64-1.0.zip"), TargetArch::One(Arch::X86_64));
        assert_eq!(arch("app-win32-arm64-1.0.zip"), TargetArch::One(Arch::Aarch64));
        assert_eq!(arch("app-darwin-universal-1.0.zip"), TargetArch::Universal);
    }

    #[test]
    fn bare_binaries() {
        let a = c("yt-dlp_linux_aarch64");
        assert_eq!(a.format, Format::Binary);
        assert_eq!(a.arch, TargetArch::One(Arch::Aarch64));
        assert_eq!(c("kubectl-linux-amd64").format, Format::Binary);
        assert!(classify("yt-dlp").is_none(), "no OS, no extension: skip");
    }

    #[test]
    fn non_installables_are_skipped() {
        for name in [
            "SHA256SUMS",
            "checksums.txt",
            "app-1.0-x86_64.AppImage.zsync",
            "app-1.0.tar.gz.sha256",
            "app-1.0.exe.sig",
            "app-1.0.dmg.asc",
            "latest.yml",
            "latest-mac.yml",
            "app-1.0-src.tar.gz",
            "app-1.0-source.zip",
            "app-debug-symbols-linux-x86_64.tar.gz",
            "app_1.0_amd64-dbgsym.deb",
            "app-1.0.x86_64.debuginfo.rpm",
            "sbom.spdx.json",
            "app-1.0.intoto.jsonl",
            "completions.bash.sh",
        ] {
            assert!(classify(name).is_none(), "{name} should be skipped");
        }
    }

    #[test]
    fn unknown_os_archive_is_kept_but_unknown() {
        // Kept so the UI can explain why it is hidden.
        let a = c("project-1.0.zip");
        assert_eq!(a.os, TargetOs::Unknown);
    }
}
