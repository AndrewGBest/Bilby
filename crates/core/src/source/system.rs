//! The package repositories this machine is already set up to use.
//!
//! Instead of talking to each distribution's web service, the store reads the package
//! databases the system's own package manager keeps on disk:
//!
//! * **pacman**: `/var/lib/pacman/sync/*.db`, one per repository in `pacman.conf`. That
//!   is `core` and `extra` on Arch, plus whatever else is configured: CachyOS's
//!   `cachyos`, `cachyos-v3`, `cachyos-core-v3`, ..., Chaotic-AUR, EndeavourOS, a
//!   personal repo.
//! * **apt**: `/var/lib/apt/lists/*_Packages`, one per configured archive, suite and
//!   component, including PPAs and vendor repositories.
//!
//! This costs no network requests, needs no per-distribution code, and is exact: it
//! lists precisely what `pacman -S` or `apt install` can install here, because it is the
//! same data they use. The price is that the lists are as fresh as the last
//! `pacman -Sy` / `apt update`, and that packages of a repository the machine does not
//! have configured are not shown (they could not be installed anyway).
//!
//! Installing is left to the package manager: the store shows the command.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::model::{valid_key, AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{Arch, Os, Platform};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemPackage {
    pub name: String,
    pub version: String,
    pub summary: String,
    /// Repository it comes from, as the user knows it: `cachyos-v3`, `extra`, `noble-updates/main`.
    pub repo: String,
    /// As the package states it: `x86_64`, `x86_64_v3`, `any`, `amd64`, `all`.
    pub arch: String,
    pub homepage: String,
    pub license: String,
    /// Download size in bytes, 0 when unknown.
    pub size: u64,
    /// `YYYY-MM-DD` build date, when the database has one.
    pub built: String,
}

/// Where one kind of package database lives, and how to read it.
#[derive(Debug, Clone)]
pub enum Backend {
    /// `sync_dir` holds `<repo>.db`; `conf` gives the repositories' priority order;
    /// `local_dir` has one `name-version` directory per installed package.
    Pacman { sync_dir: PathBuf, conf: PathBuf, local_dir: PathBuf },
    /// `status` is dpkg's record of what is installed.
    Apt { lists_dir: PathBuf, status: PathBuf },
    /// Fedora and its relatives. dnf keeps its lists in a format of its own, so it is
    /// asked for them: `dnf` is the program, `rpm` the one that says what is installed,
    /// and `cache_dirs` are watched to notice a refresh.
    Dnf { dnf: PathBuf, rpm: PathBuf, cache_dirs: Vec<PathBuf> },
    /// openSUSE. `raw_dir` holds one folder per repository, each with a
    /// `repodata/*-primary.xml` list; `rpm` says what is installed.
    Zypper { raw_dir: PathBuf, rpm: PathBuf },
    /// A fixed list, for the demo catalog and tests. `installed` is name -> version.
    Fixed { kind: SourceKind, packages: Vec<SystemPackage>, installed: Vec<(String, String)> },
}

impl Backend {
    pub fn system_pacman() -> Backend {
        Backend::Pacman {
            sync_dir: "/var/lib/pacman/sync".into(),
            conf: "/etc/pacman.conf".into(),
            local_dir: "/var/lib/pacman/local".into(),
        }
    }
    pub fn system_apt() -> Backend {
        Backend::Apt { lists_dir: "/var/lib/apt/lists".into(), status: "/var/lib/dpkg/status".into() }
    }

    pub fn system_dnf() -> Backend {
        Backend::Dnf {
            dnf: "dnf".into(),
            rpm: "rpm".into(),
            cache_dirs: vec!["/var/cache/libdnf5".into(), "/var/cache/dnf".into()],
        }
    }
    pub fn system_zypper() -> Backend {
        Backend::Zypper { raw_dir: "/var/cache/zypp/raw".into(), rpm: "rpm".into() }
    }

    fn kind(&self) -> SourceKind {
        match self {
            Backend::Pacman { .. } => SourceKind::Pacman,
            Backend::Apt { .. } => SourceKind::Apt,
            Backend::Dnf { .. } => SourceKind::Dnf,
            Backend::Zypper { .. } => SourceKind::Zypper,
            Backend::Fixed { kind, .. } => *kind,
        }
    }

    /// The database files, with what is needed to notice that they changed.
    fn files(&self) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
        // The rpm families keep one folder per repository, with the lists inside.
        let repodata = |roots: &[PathBuf]| -> Vec<(PathBuf, u64, Option<SystemTime>)> {
            let mut files: Vec<_> = roots
                .iter()
                .flat_map(|root| std::fs::read_dir(root).into_iter().flatten().flatten())
                .flat_map(|repo| std::fs::read_dir(repo.path().join("repodata")).into_iter().flatten().flatten())
                .filter(|entry| entry.file_name().to_string_lossy().contains("primary.xml"))
                .filter_map(|entry| {
                    let meta = std::fs::metadata(entry.path()).ok()?;
                    meta.is_file().then(|| (entry.path(), meta.len(), meta.modified().ok()))
                })
                .collect();
            files.sort();
            files
        };
        let (dir, wanted): (&Path, fn(&str) -> bool) = match self {
            Backend::Dnf { cache_dirs, .. } => return repodata(cache_dirs),
            Backend::Zypper { raw_dir, .. } => return repodata(std::slice::from_ref(raw_dir)),
            Backend::Pacman { sync_dir, .. } => (sync_dir, |name| name.ends_with(".db")),
            Backend::Apt { lists_dir, .. } => (lists_dir, |name| {
                let base = name.trim_end_matches(".lz4").trim_end_matches(".gz").trim_end_matches(".zst");
                base.ends_with("_Packages")
            }),
            Backend::Fixed { .. } => return Vec::new(),
        };
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| wanted(&entry.file_name().to_string_lossy()))
            .filter_map(|entry| {
                // Follow symlinks: `core.db` is often one.
                let meta = std::fs::metadata(entry.path()).ok()?;
                meta.is_file().then(|| (entry.path(), meta.len(), meta.modified().ok()))
            })
            .collect();
        files.sort();
        files
    }

    /// What the package manager says is installed: name -> version.
    fn installed(&self) -> HashMap<String, String> {
        match self {
            Backend::Fixed { installed, .. } => installed.iter().cloned().collect(),
            Backend::Dnf { rpm, .. } | Backend::Zypper { rpm, .. } => rpm_installed(rpm),
            // Directory names are `name-version-release`; the name itself may contain dashes.
            Backend::Pacman { local_dir, .. } => std::fs::read_dir(local_dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let dir = entry.file_name().to_string_lossy().into_owned();
                    let mut parts = dir.rsplitn(3, '-');
                    let (release, version, name) = (parts.next()?, parts.next()?, parts.next()?);
                    Some((name.to_string(), format!("{version}-{release}")))
                })
                .collect(),
            Backend::Apt { status, .. } => {
                let text = std::fs::read_to_string(status).unwrap_or_default();
                text.split("\n\n")
                    .filter_map(|stanza| {
                        let field = |name: &str| {
                            stanza.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix(':')).map(str::trim)
                        };
                        // "install ok installed"; removed-but-configured packages do not count.
                        field("Status")?.ends_with(" installed").then_some(())?;
                        Some((field("Package")?.to_string(), field("Version")?.to_string()))
                    })
                    .collect()
            }
        }
    }

    fn exists(&self) -> bool {
        matches!(self, Backend::Fixed { .. }) || !self.files().is_empty()
    }

    fn load(&self, platform: &Platform) -> Vec<SystemPackage> {
        match self {
            Backend::Fixed { packages, .. } => packages.clone(),
            Backend::Dnf { dnf, .. } => dnf_available(dnf, platform),
            Backend::Zypper { .. } => {
                let mut newest: HashMap<String, SystemPackage> = HashMap::new();
                for (path, _, _) in self.files() {
                    // The folder above "repodata" is the repository's name.
                    let repo = path
                        .parent()
                        .and_then(Path::parent)
                        .and_then(Path::file_name)
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let Ok(packages) = read_rpm_primary(&path, &repo) else { continue };
                    for package in packages {
                        if !matches!(arch_of(&package.arch), TargetArch::Any)
                            && arch_of(&package.arch) != TargetArch::One(platform.arch)
                        {
                            continue;
                        }
                        // The same package in "oss" and "update": zypper takes the newer.
                        match newest.get(&package.name) {
                            Some(have) if !crate::version::is_newer(&package.version, &have.version) => {}
                            _ => {
                                newest.insert(package.name.clone(), package);
                            }
                        }
                    }
                }
                let mut out: Vec<_> = newest.into_values().collect();
                out.sort_by(|a, b| a.name.cmp(&b.name));
                out
            }
            Backend::Pacman { conf, .. } => {
                let order = pacman_repo_order(&std::fs::read_to_string(conf).unwrap_or_default());
                let mut by_repo: Vec<(String, Vec<SystemPackage>)> = self
                    .files()
                    .into_iter()
                    .filter_map(|(path, _, _)| {
                        let repo = path.file_stem()?.to_string_lossy().into_owned();
                        let packages = read_pacman_db(&path, &repo).ok()?;
                        Some((repo, packages))
                    })
                    .collect();
                // pacman installs a name from the first repository in pacman.conf that
                // has it. List them the same way, so what is shown is what is installed.
                let rank = |repo: &str| order.iter().position(|r| r == repo).unwrap_or(usize::MAX);
                by_repo.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.cmp(&b.0)));
                let mut seen = HashMap::new();
                let mut out = Vec::new();
                for package in by_repo.into_iter().flat_map(|(_, packages)| packages) {
                    if seen.insert(package.name.clone(), ()).is_none() {
                        out.push(package);
                    }
                }
                out
            }
            Backend::Apt { .. } => {
                let mut newest: HashMap<String, SystemPackage> = HashMap::new();
                for (path, _, _) in self.files() {
                    let Ok(text) = read_maybe_compressed(&path) else { continue };
                    let repo = apt_repo_label(&path.file_name().unwrap_or_default().to_string_lossy());
                    for package in parse_apt_packages(&text, &repo) {
                        // Lists for a foreign architecture (multiarch i386 on amd64) are
                        // not what "apt install name" would pick.
                        if !matches!(arch_of(&package.arch), TargetArch::Any)
                            && arch_of(&package.arch) != TargetArch::One(platform.arch)
                        {
                            continue;
                        }
                        // The same package in "noble" and "noble-updates": apt takes the newer.
                        match newest.get(&package.name) {
                            Some(have) if !crate::version::is_newer(&package.version, &have.version) => {}
                            _ => {
                                newest.insert(package.name.clone(), package);
                            }
                        }
                    }
                }
                let mut out: Vec<_> = newest.into_values().collect();
                out.sort_by(|a, b| a.name.cmp(&b.name));
                out
            }
        }
    }
}

/// What rpm says is installed: name -> version-release.
fn rpm_installed(rpm: &Path) -> HashMap<String, String> {
    let output = std::process::Command::new(rpm)
        .args(["-qa", "--qf", "%{NAME}\\t%{VERSION}-%{RELEASE}\\n"])
        .stdin(std::process::Stdio::null())
        .output();
    let text = output.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    text.lines().filter_map(|line| line.split_once('\t')).map(|(n, v)| (n.to_string(), v.to_string())).collect()
}

/// Everything dnf could install, from its own cache (`-C`: no network, no root).
/// dnf does not expand `\t` in its format, so the fields are separated by real tabs.
fn dnf_available(dnf: &Path, platform: &Platform) -> Vec<SystemPackage> {
    let arch = match platform.arch {
        Arch::Aarch64 => "aarch64",
        Arch::X86_64 => "x86_64",
        _ => return Vec::new(),
    };
    let format = "%{name}\t%{evr}\t%{arch}\t%{repoid}\t%{downloadsize}\t%{license}\t%{url}\t%{summary}\n";
    let output = std::process::Command::new(dnf)
        .args([
            "-q",
            "-C",
            "repoquery",
            "--available",
            "--latest-limit",
            "1",
            "--arch",
            &format!("{arch},noarch"),
            "--qf",
            format,
        ])
        .stdin(std::process::Stdio::null())
        .output();
    let text = output.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut packages = parse_dnf_list(&text);
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    // The same name in "fedora" and "updates": keep the newer.
    packages.dedup_by(|later, earlier| {
        if later.name != earlier.name {
            return false;
        }
        if crate::version::is_newer(&later.version, &earlier.version) {
            std::mem::swap(later, earlier);
        }
        true
    });
    packages
}

/// One package per line: name, version, architecture, repository, size, licence,
/// homepage, summary, separated by tabs.
pub fn parse_dnf_list(text: &str) -> Vec<SystemPackage> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.splitn(8, '\t').collect();
            let [name, version, arch, repo, size, license, url, summary] = fields.as_slice() else { return None };
            (!name.is_empty() && !version.is_empty()).then(|| SystemPackage {
                name: name.to_string(),
                version: version.to_string(),
                summary: summary.to_string(),
                repo: repo.to_string(),
                arch: arch.to_string(),
                homepage: url.to_string(),
                license: license.to_string(),
                size: size.parse().unwrap_or(0),
                built: String::new(),
            })
        })
        .collect()
}

/// Read an rpm repository's `primary.xml`, as createrepo writes it: one element per
/// line apart from descriptions. Read as a stream; the whole list is far too big to
/// hold in memory at once (openSUSE's is 170 MB).
pub fn read_rpm_primary(path: &Path, repo: &str) -> Result<Vec<SystemPackage>> {
    use std::io::BufRead;
    let reader = std::io::BufReader::with_capacity(1 << 16, open_maybe_compressed(path)?);
    let text_of = |line: &str, tag: &str| -> Option<String> {
        let rest = line.strip_prefix('<')?.strip_prefix(tag)?.strip_prefix('>')?;
        Some(unescape_xml(rest.split("</").next().unwrap_or(rest)))
    };
    let attr = |line: &str, name: &str| -> Option<String> {
        let at = line.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(unescape_xml(&line[at..at + line[at..].find('"')?]))
    };
    let mut out = Vec::new();
    let mut current: Option<SystemPackage> = None;
    for line in reader.lines() {
        // A line that is not text (it should not happen) ends the list rather than failing it.
        let Ok(line) = line else { break };
        let line = line.trim_start();
        if line.starts_with("<package ") {
            current = Some(SystemPackage {
                name: String::new(),
                version: String::new(),
                summary: String::new(),
                repo: repo.to_string(),
                arch: String::new(),
                homepage: String::new(),
                license: String::new(),
                size: 0,
                built: String::new(),
            });
            continue;
        }
        let Some(package) = current.as_mut() else { continue };
        if line.starts_with("</package>") {
            if let Some(done) =
                current.take().filter(|p| !p.name.is_empty() && !p.version.is_empty() && p.arch != "src")
            {
                out.push(done);
            }
        } else if let Some(name) = text_of(line, "name") {
            package.name = name;
        } else if let Some(arch) = text_of(line, "arch") {
            package.arch = if arch == "noarch" { "any".to_string() } else { arch };
        } else if line.starts_with("<version ") {
            let (epoch, ver, rel) = (attr(line, "epoch"), attr(line, "ver"), attr(line, "rel"));
            let ver = [ver.unwrap_or_default(), rel.unwrap_or_default()].join("-").trim_end_matches('-').to_string();
            package.version = match epoch.as_deref() {
                Some("0") | Some("") | None => ver,
                Some(epoch) => format!("{epoch}:{ver}"),
            };
        } else if let Some(summary) = text_of(line, "summary") {
            package.summary = summary;
        } else if let Some(url) = text_of(line, "url") {
            package.homepage = url;
        } else if line.starts_with("<size ") {
            package.size = attr(line, "package").and_then(|s| s.parse().ok()).unwrap_or(0);
        } else if line.starts_with("<time ") {
            let built = attr(line, "build").and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
            package.built = date_from_unix(built);
        } else if let Some(license) = text_of(line, "rpm:license") {
            package.license = license;
        }
    }
    Ok(out)
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Repository names from `pacman.conf`, in the order pacman consults them.
pub fn pacman_repo_order(conf: &str) -> Vec<String> {
    conf.lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix('[')?.strip_suffix(']'))
        .filter(|section| *section != "options")
        .map(str::to_string)
        .collect()
}

/// Read a file that may be gzip, zstd or lz4 compressed, going by its first bytes.
fn open_maybe_compressed(path: &Path) -> Result<Box<dyn Read>> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    let read = file.read(&mut magic)?;
    let file = std::io::Cursor::new(magic[..read].to_vec()).chain(file);
    Ok(match &magic[..read] {
        [0x1f, 0x8b, ..] => Box::new(flate2::read::MultiGzDecoder::new(file)),
        [0x28, 0xb5, 0x2f, 0xfd] => Box::new(
            ruzstd::decoding::StreamingDecoder::new(file)
                .map_err(|e| Error::Parse(format!("{}: {e}", path.display())))?,
        ),
        [0x04, 0x22, 0x4d, 0x18] => Box::new(lz4_flex::frame::FrameDecoder::new(file)),
        [0xfd, b'7', b'z', b'X'] => {
            return Err(Error::Unsupported(format!("{} is xz-compressed, which is not supported yet", path.display())))
        }
        _ => Box::new(file),
    })
}

fn read_maybe_compressed(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    open_maybe_compressed(path)?.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// A pacman sync database: a (compressed) tar with one `name-version/desc` per package.
pub fn read_pacman_db(path: &Path, repo: &str) -> Result<Vec<SystemPackage>> {
    let mut archive = tar::Archive::new(open_maybe_compressed(path)?);
    let mut packages = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.path()?.to_string_lossy().ends_with("/desc") {
            continue;
        }
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_err() {
            continue;
        }
        if let Some(package) = parse_pacman_desc(&text, repo) {
            packages.push(package);
        }
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}

/// One `desc` file: `%KEY%` on a line, its values on the following lines, a blank line.
pub fn parse_pacman_desc(text: &str, repo: &str) -> Option<SystemPackage> {
    let mut fields: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut key = None;
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(name) = line.strip_prefix('%').and_then(|l| l.strip_suffix('%')) {
            key = Some(name);
        } else if line.is_empty() {
            key = None;
        } else if let Some(key) = key {
            fields.entry(key).or_default().push(line);
        }
    }
    let first = |key: &str| fields.get(key).and_then(|v| v.first()).copied().unwrap_or_default().to_string();
    let name = first("NAME");
    let version = first("VERSION");
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some(SystemPackage {
        name,
        version,
        summary: first("DESC"),
        repo: repo.to_string(),
        arch: first("ARCH"),
        homepage: first("URL"),
        license: fields.get("LICENSE").map(|l| l.join(", ")).unwrap_or_default(),
        size: first("CSIZE").parse().unwrap_or(0),
        built: first("BUILDDATE").parse::<i64>().map(date_from_unix).unwrap_or_default(),
    })
}

/// An apt `Packages` index: stanzas of `Field: value` separated by blank lines.
pub fn parse_apt_packages(text: &str, repo: &str) -> Vec<SystemPackage> {
    let mut packages = Vec::new();
    for stanza in text.split("\n\n") {
        let mut package = SystemPackage { repo: repo.to_string(), ..Default::default() };
        for line in stanza.lines() {
            // Continuation lines (long descriptions, multi-line fields) start with a space.
            if line.starts_with([' ', '\t']) {
                continue;
            }
            let Some((field, value)) = line.split_once(':') else { continue };
            let value = value.trim();
            match field {
                "Package" => package.name = value.to_string(),
                "Version" => package.version = value.to_string(),
                "Architecture" => package.arch = value.to_string(),
                "Description" => package.summary = value.to_string(),
                "Homepage" => package.homepage = value.to_string(),
                "Size" => package.size = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        if !package.name.is_empty() && !package.version.is_empty() {
            packages.push(package);
        }
    }
    packages
}

/// `archive.ubuntu.com_ubuntu_dists_noble-updates_main_binary-amd64_Packages.lz4`
/// becomes `noble-updates/main`; anything unusual falls back to the host name.
pub fn apt_repo_label(file_name: &str) -> String {
    let parts: Vec<&str> = file_name.split('_').collect();
    match parts.iter().position(|p| *p == "dists") {
        Some(i) if parts.len() > i + 2 && !parts[i + 2].starts_with("binary-") => {
            format!("{}/{}", parts[i + 1], parts[i + 2])
        }
        Some(i) if parts.len() > i + 1 => parts[i + 1].to_string(),
        _ => parts.first().copied().unwrap_or("apt").to_string(),
    }
}

/// What a package's architecture string means for this store. CachyOS's optimised
/// repositories label packages `x86_64_v3` / `x86_64_v4`; they are x86_64 code for newer
/// CPUs, and a machine only has such a repository configured if its CPU qualifies.
fn arch_of(arch: &str) -> TargetArch {
    match arch {
        "any" | "all" | "" => TargetArch::Any,
        "x86_64" | "amd64" | "x86_64_v2" | "x86_64_v3" | "x86_64_v4" | "znver4" | "znver5" => {
            TargetArch::One(Arch::X86_64)
        }
        "aarch64" | "arm64" => TargetArch::One(Arch::Aarch64),
        "i686" | "i386" | "pentium4" => TargetArch::One(Arch::X86),
        "armv7h" | "armhf" | "armel" => TargetArch::One(Arch::Arm32),
        "riscv64" => TargetArch::One(Arch::Riscv64),
        _ => TargetArch::Foreign("another architecture"),
    }
}

fn date_from_unix(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Libraries, headers, documentation and debug symbols: real packages, rarely what
/// someone searching a store is after. They are listed after everything else.
fn is_supporting_package(name: &str) -> bool {
    const PREFIXES: [&str; 5] = ["lib", "lib32-", "python3-", "gir1.2-", "golang-"];
    const SUFFIXES: [&str; 12] = [
        "-dev", "-devel", "-doc", "-docs", "-dbg", "-dbgsym", "-debug", "-data", "-common", "-headers", "-bin", "-l10n",
    ];
    let lower = name.to_lowercase();
    // "libreoffice" and "librewolf" are applications that happen to start with "lib".
    let library = PREFIXES.iter().any(|p| lower.starts_with(p)) && !lower.starts_with("libre");
    // Translations and plug-ins come by the dozen ("firefox-esr-l10n-af",
    // "vlc-plugin-qt") and would bury the application they belong to.
    const PARTS: [&str; 8] = ["l10n", "i18n", "locale", "locales", "langpack", "plugin", "plugins", "help"];
    let part = lower.split('-').skip(1).any(|word| PARTS.contains(&word));
    library || part || SUFFIXES.iter().any(|s| lower.ends_with(s))
}

type Fingerprint = Vec<(PathBuf, u64, Option<SystemTime>)>;

pub struct SystemRepos {
    backend: Backend,
    platform: Platform,
    /// Parsed databases, kept until the files on disk change.
    cache: Mutex<Option<(Fingerprint, Arc<Vec<SystemPackage>>)>>,
}

impl SystemRepos {
    pub fn new(backend: Backend, platform: Platform) -> SystemRepos {
        SystemRepos { backend, platform, cache: Mutex::new(None) }
    }

    /// Every package the system's package manager could install, parsed once and
    /// re-read when the database files change (after `pacman -Sy` / `apt update`).
    pub fn packages(&self) -> Arc<Vec<SystemPackage>> {
        let fingerprint = self.backend.files();
        let mut cache = self.cache.lock().unwrap();
        if let Some((seen, packages)) = cache.as_ref() {
            if *seen == fingerprint {
                return packages.clone();
            }
        }
        let packages = Arc::new(self.backend.load(&self.platform));
        *cache = Some((fingerprint, packages.clone()));
        packages
    }

    /// Repository names that have at least one package, in listing order.
    pub fn repos(&self) -> Vec<String> {
        let mut repos: Vec<String> = Vec::new();
        for package in self.packages().iter() {
            if !repos.contains(&package.repo) {
                repos.push(package.repo.clone());
            }
        }
        repos
    }

    fn find(&self, name: &str) -> Result<SystemPackage> {
        self.packages().iter().find(|p| p.name == name).cloned().ok_or_else(|| {
            Error::NotFound(format!("None of this system's repositories has a package called “{name}”."))
        })
    }

    fn summary_of(&self, package: &SystemPackage) -> AppSummary {
        let mut summary = self.summary_of_without_lookup(package);
        summary.installed_version = self.backend.installed().get(&package.name).cloned();
        summary
    }

    /// Without reading the installed-package records (callers that list many do it once).
    fn summary_of_without_lookup(&self, package: &SystemPackage) -> AppSummary {
        AppSummary {
            id: AppId::new(self.backend.kind(), package.name.clone()),
            name: package.name.clone(),
            author: package.repo.clone(),
            summary: package.summary.clone(),
            popularity: None,
            homepage: package.homepage.clone(),
            license: (!package.license.is_empty()).then(|| package.license.clone()),
            category: None,
            version: Some(package.version.clone()),
            install_label: None,
            warning: None,
            origin: Some(package.repo.clone()),
            installed_version: None,
            rating: None,
            icon_url: None,
            downloads: None,
            verified: false,
            momentum: None,
            price: None,
            adult: false,
        }
    }
}

impl Source for SystemRepos {
    fn kind(&self) -> SourceKind {
        self.backend.kind()
    }

    fn installed_names(&self) -> Vec<String> {
        self.backend.installed().into_keys().collect()
    }

    fn available_on(&self, platform: &Platform) -> bool {
        let Some(linux) = platform.linux.as_ref().filter(|_| platform.os == Os::Linux) else { return false };
        let has_manager = match self.backend.kind() {
            SourceKind::Pacman => linux.pacman,
            SourceKind::Apt => linux.dpkg,
            // Both use rpm; which one is this system's is a matter of family.
            SourceKind::Zypper => linux.rpm && linux.family == crate::platform::LinuxFamily::Suse,
            SourceKind::Dnf => linux.rpm && linux.family != crate::platform::LinuxFamily::Suse,
            _ => false,
        };
        has_manager && self.backend.exists()
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.iter().map(String::len).sum::<usize>() < 2 {
            return Ok(Vec::new());
        }
        let whole = words.join(" ");
        let packages = self.packages();
        // There is no popularity to rank by, so rank by how well the name fits.
        let mut hits: Vec<(u8, &SystemPackage)> = packages
            .iter()
            .filter_map(|p| {
                let name = p.name.to_lowercase();
                let rank = if name == whole || name == words.join("-") {
                    0
                } else if words.iter().all(|w| name.starts_with(w.as_str())) {
                    1
                } else if words.iter().all(|w| name.contains(w.as_str())) {
                    2
                } else {
                    let summary = p.summary.to_lowercase();
                    if words.iter().all(|w| name.contains(w.as_str()) || summary.contains(w.as_str())) {
                        3
                    } else {
                        return None;
                    }
                };
                // An exact name is what was asked for, whatever kind of package it is.
                let rank = if rank > 0 && is_supporting_package(&p.name) { rank + 4 } else { rank };
                Some((rank, p))
            })
            .collect();
        hits.sort_by(|a, b| {
            a.0.cmp(&b.0).then_with(|| a.1.name.len().cmp(&b.1.name.len())).then_with(|| a.1.name.cmp(&b.1.name))
        });
        let installed = self.backend.installed();
        Ok(hits
            .into_iter()
            .take(limit)
            .map(|(_, p)| {
                let mut summary = self.summary_of_without_lookup(p);
                summary.installed_version = installed.get(&p.name).cloned();
                summary
            })
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        if !valid_key(self.backend.kind(), key) {
            return Err(Error::NotFound(format!("“{key}” is not a package name.")));
        }
        Ok(self.summary_of(&self.find(key)?))
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let package = self.find(key)?;
        let manager = match self.backend.kind() {
            SourceKind::Pacman => "pacman",
            SourceKind::Dnf => "dnf",
            SourceKind::Zypper => "zypper",
            _ => "apt",
        };
        let mut notes = format!(
            "From the “{}” repository configured on this system. Installed and updated by {manager}.",
            package.repo
        );
        if !package.homepage.is_empty() {
            notes.push_str(&format!("\nUpstream: {}", package.homepage));
        }
        Ok(vec![Release {
            version: package.version.clone(),
            published: package.built.clone(),
            prerelease: false,
            downloads: None,
            notes,
            assets: vec![Asset {
                name: format!("{} ({})", package.name, package.repo),
                size: package.size,
                url: String::new(),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::RepoPackage,
                    os: TargetOs::Known(Os::Linux),
                    arch: arch_of(&package.arch),
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some(Format::RepoPackage.label())
    }

    fn origins(&self) -> Vec<String> {
        self.repos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bilby-sysrepo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn desc(name: &str, version: &str, summary: &str, arch: &str) -> String {
        format!(
            "%FILENAME%\n{name}-{version}-{arch}.pkg.tar.zst\n\n%NAME%\n{name}\n\n%BASE%\n{name}\n\n%VERSION%\n{version}\n\n%DESC%\n{summary}\n\n%CSIZE%\n1048576\n\n%ISIZE%\n4194304\n\n%SHA256SUM%\nabc\n\n%URL%\nhttps://example.invalid/{name}\n\n%LICENSE%\nMIT\nApache-2.0\n\n%ARCH%\n{arch}\n\n%BUILDDATE%\n1700000000\n\n%PACKAGER%\nSomeone <someone@example.invalid>\n\n%DEPENDS%\nglibc\ngcc-libs\n\n"
        )
    }

    /// A tar of `name-version/desc` entries, as `repo-add` produces.
    fn tarball(packages: &[(&str, &str, &str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, version, summary, arch) in packages {
            let body = desc(name, version, summary, arch);
            let mut dir_header = tar::Header::new_gnu();
            dir_header.set_entry_type(tar::EntryType::Directory);
            dir_header.set_size(0);
            dir_header.set_mode(0o755);
            builder.append_data(&mut dir_header, format!("{name}-{version}/"), std::io::empty()).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, format!("{name}-{version}/desc"), body.as_bytes()).unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn arch_system(tag: &str, conf: &str, dbs: &[(&str, Vec<u8>)]) -> SystemRepos {
        let d = dir(tag);
        for (repo, bytes) in dbs {
            std::fs::write(d.join(format!("{repo}.db")), bytes).unwrap();
            // The companion file-list database must be ignored.
            std::fs::write(d.join(format!("{repo}.files")), b"not a database").unwrap();
        }
        std::fs::write(d.join("pacman.conf"), conf).unwrap();
        SystemRepos::new(
            Backend::Pacman { sync_dir: d.clone(), conf: d.join("pacman.conf"), local_dir: d.join("local") },
            Platform::preset("linux-x64-arch").unwrap(),
        )
    }

    const CACHY_CONF: &str = "[options]\nArchitecture = x86_64 x86_64_v3\n\n[cachyos-v3]\nInclude = /etc/pacman.d/cachyos-v3-mirrorlist\n\n[cachyos-core-v3]\nInclude = /etc/pacman.d/cachyos-v3-mirrorlist\n\n[cachyos]\nInclude = /etc/pacman.d/cachyos-mirrorlist\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n\n[extra]\nInclude = /etc/pacman.d/mirrorlist\n";

    #[test]
    fn a_desc_file_is_understood() {
        let p = parse_pacman_desc(&desc("ripgrep", "14.1.1-1", "A search tool", "x86_64"), "extra").unwrap();
        assert_eq!(
            p,
            SystemPackage {
                name: "ripgrep".into(),
                version: "14.1.1-1".into(),
                summary: "A search tool".into(),
                repo: "extra".into(),
                arch: "x86_64".into(),
                homepage: "https://example.invalid/ripgrep".into(),
                license: "MIT, Apache-2.0".into(),
                size: 1_048_576,
                built: "2023-11-14".into(),
            }
        );
        assert!(parse_pacman_desc("%NAME%\nx\n", "r").is_none(), "no version, no package");
        assert!(parse_pacman_desc("", "r").is_none());
        assert!(parse_pacman_desc("garbage\n%%\n%NAME%", "r").is_none());
    }

    #[test]
    fn repositories_are_read_in_pacman_conf_order() {
        assert_eq!(pacman_repo_order(CACHY_CONF), ["cachyos-v3", "cachyos-core-v3", "cachyos", "core", "extra"]);
        assert_eq!(pacman_repo_order("# [commented]\n  [core]  \n[extra]\n"), ["core", "extra"]);
    }

    #[test]
    fn cachyos_and_arch_repositories_side_by_side() {
        let repos = arch_system(
            "cachy",
            CACHY_CONF,
            &[
                // Written to disk in a different order than pacman.conf, and with mixed
                // compression, to make sure neither matters.
                (
                    "extra",
                    gz(&tarball(&[
                        ("firefox", "131.0-1", "Web browser", "x86_64"),
                        ("ripgrep", "14.1.1-1", "A search tool", "x86_64"),
                    ])),
                ),
                (
                    "core",
                    tarball(&[
                        ("linux", "6.11.1-1", "The Linux kernel", "x86_64"),
                        ("bash", "5.2-1", "The GNU shell", "x86_64"),
                    ]),
                ),
                (
                    "cachyos",
                    gz(&tarball(&[
                        ("linux-cachyos", "6.11.2-1", "Linux kernel with CachyOS patches", "x86_64"),
                        ("paru", "2.0.4-1", "AUR helper", "x86_64"),
                    ])),
                ),
                (
                    "cachyos-v3",
                    gz(&tarball(&[
                        ("firefox", "131.0-1.1", "Web browser", "x86_64_v3"),
                        ("fonts-any", "1-1", "Fonts", "any"),
                    ])),
                ),
            ],
        );
        assert!(repos.available_on(&Platform::preset("linux-x64-arch").unwrap()));
        assert!(!repos.available_on(&Platform::preset("linux-x64-debian").unwrap()));
        assert!(!repos.available_on(&Platform::preset("windows-x64").unwrap()));
        assert_eq!(repos.repos(), ["cachyos-v3", "cachyos", "core", "extra"]);

        // firefox exists in cachyos-v3 and extra; pacman would take the first, so do we.
        let firefox = repos.summary("firefox").unwrap();
        assert_eq!(firefox.origin.as_deref(), Some("cachyos-v3"));
        assert_eq!(firefox.version.as_deref(), Some("131.0-1.1"));
        assert_eq!(firefox.id.to_string(), "pacman:firefox");

        let names = |q: &str| -> Vec<String> { repos.search(q, 10).unwrap().into_iter().map(|a| a.name).collect() };
        assert_eq!(names("linux"), ["linux", "linux-cachyos"], "exact name first");
        assert_eq!(names("kernel"), ["linux", "linux-cachyos"], "descriptions are searched");
        assert_eq!(names("cachyos"), ["linux-cachyos"]);
        assert_eq!(names("fire"), ["firefox"]);
        assert!(names("x").is_empty());
        assert!(names("no-such-thing").is_empty());

        let release = &repos.releases("linux-cachyos").unwrap()[0];
        assert_eq!(release.version, "6.11.2-1");
        assert_eq!(release.published, "2023-11-14");
        assert_eq!(release.assets[0].name, "linux-cachyos (cachyos)");
        assert_eq!(release.assets[0].size, 1_048_576);
        assert!(release.notes.contains("“cachyos” repository"));
        assert!(matches!(repos.summary("nope"), Err(Error::NotFound(_))));
        assert!(repos.summary("../etc").is_err());
    }

    #[test]
    fn rpm_package_lists_are_read_as_fedora_and_opensuse_write_them() {
        // Both samples are cut from the real lists of Fedora 45 and openSUSE Tumbleweed.
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures");
        let suse = read_rpm_primary(&fixtures.join("rpm_primary.xml"), "repo-oss").unwrap();
        let names: Vec<&str> = suse.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["0ad", "ripgrep", "vlc", "yast2-trans-af"]);
        let ripgrep = &suse[1];
        assert_eq!((ripgrep.arch.as_str(), ripgrep.repo.as_str()), ("x86_64", "repo-oss"));
        assert!(ripgrep.version.starts_with("15.") && ripgrep.size > 1_000_000 && ripgrep.built.len() == 10);
        assert!(!ripgrep.summary.is_empty() && ripgrep.homepage.starts_with("https://"));
        assert_eq!(suse[3].arch, "any", "noarch packages run everywhere");

        let fedora = parse_dnf_list(&std::fs::read_to_string(fixtures.join("dnf_list.tsv")).unwrap());
        let vlc = fedora.iter().find(|p| p.name == "vlc").unwrap();
        assert_eq!(vlc.version, "1:3.0.24-3.fc45", "the epoch is kept: it decides which version is newer");
        assert_eq!((vlc.repo.as_str(), vlc.homepage.as_str()), ("fedora", "https://www.videolan.org"));
        assert!(parse_dnf_list("not a package line\n\n").is_empty());

        // Each family's source appears only on its own systems.
        let on = |backend: Backend, preset: &str| {
            let platform = Platform::preset(preset).unwrap();
            let fixed = Backend::Fixed { kind: backend.kind(), packages: vec![], installed: vec![] };
            SystemRepos::new(fixed, platform.clone()).available_on(&platform)
        };
        assert!(on(Backend::system_dnf(), "linux-x64-fedora"));
        assert!(!on(Backend::system_dnf(), "linux-x64-debian") && !on(Backend::system_zypper(), "linux-x64-fedora"));
    }

    #[test]
    fn installed_pacman_packages_are_recognised() {
        let repos = arch_system(
            "installed",
            "[extra]\n",
            &[(
                "extra",
                tarball(&[
                    ("xdg-desktop-portal-gtk", "1.15.1-1", "Portal", "x86_64"),
                    ("ripgrep", "14.1.1-1", "Search", "x86_64"),
                ]),
            )],
        );
        let Backend::Pacman { local_dir, .. } = &repos.backend else { unreachable!() };
        // Names with dashes, and an epoch in the version.
        std::fs::create_dir_all(local_dir.join("xdg-desktop-portal-gtk-1.15.0-2")).unwrap();
        // An epoch puts a colon in the directory name, which Windows cannot create
        // (and where pacman does not exist anyway).
        #[cfg(not(windows))]
        {
            std::fs::create_dir_all(local_dir.join("something-else-1:2.0-1")).unwrap();
            assert_eq!(repos.backend.installed().get("something-else").map(String::as_str), Some("1:2.0-1"));
        }
        std::fs::write(local_dir.join("ALPM_DB_VERSION"), "9").unwrap();
        assert_eq!(repos.summary("xdg-desktop-portal-gtk").unwrap().installed_version.as_deref(), Some("1.15.0-2"));
        assert_eq!(repos.summary("ripgrep").unwrap().installed_version, None);
    }

    #[test]
    fn applications_are_listed_before_their_libraries() {
        let repos = arch_system(
            "ranking",
            "[extra]\n",
            &[(
                "extra",
                tarball(&[
                    ("libtexteditor", "1-1", "text editor library", "x86_64"),
                    ("texteditor-dev", "1-1", "text editor headers", "x86_64"),
                    ("texteditor-docs", "1-1", "text editor manual", "x86_64"),
                    ("gnome-text-editor", "46-1", "simple text editor", "x86_64"),
                    ("libreoffice-fresh", "24-1", "office suite with a text editor", "x86_64"),
                    ("python3-editor", "1-1", "text editor bindings", "x86_64"),
                ]),
            )],
        );
        let names: Vec<String> = repos.search("text editor", 10).unwrap().into_iter().map(|a| a.name).collect();
        assert_eq!(&names[..2], ["gnome-text-editor", "libreoffice-fresh"], "{names:?}");
        assert_eq!(names.len(), 6, "supporting packages are still there, after the applications");
        // Asking for a library by its exact name still puts it first.
        assert_eq!(repos.search("libtexteditor", 10).unwrap()[0].name, "libtexteditor");
        // Seen on a real Debian: a search for a browser listed its language packs first.
        for part in ["firefox-esr-l10n-af", "vlc-plugin-qt", "libreoffice-help-en-gb", "thunderbird-locale-de"] {
            assert!(is_supporting_package(part), "{part}");
        }
        for app in ["firefox-esr", "vlc", "plugin-manager", "help2man", "localepurge"] {
            assert!(!is_supporting_package(app), "{app}");
        }
    }

    #[test]
    fn optimised_architectures_count_as_x86_64() {
        for arch in ["x86_64", "x86_64_v3", "x86_64_v4", "znver4", "amd64"] {
            assert_eq!(arch_of(arch), TargetArch::One(Arch::X86_64), "{arch}");
        }
        assert_eq!(arch_of("any"), TargetArch::Any);
        assert_eq!(arch_of("all"), TargetArch::Any);
        assert_eq!(arch_of("aarch64"), TargetArch::One(Arch::Aarch64));
        assert!(matches!(arch_of("s390x"), TargetArch::Foreign(_)));
    }

    #[test]
    fn zstd_databases_and_broken_ones() {
        // A zstd-compressed database, as some repositories publish.
        let tar = tarball(&[("thing", "1-1", "A thing", "x86_64")]);
        let zst = ruzstd::encoding::compress_to_vec(tar.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
        let repos = arch_system(
            "zstd",
            "[good]\n[broken]\n[xz]\n",
            &[
                ("good", zst),
                ("broken", b"this is not an archive at all".to_vec()),
                ("xz", vec![0xfd, b'7', b'z', b'X', b'Z', 0, 0]),
            ],
        );
        // The readable repository is listed; the others are skipped rather than fatal.
        let names: Vec<String> = repos.packages().iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, ["thing"]);
        assert_eq!(repos.repos(), ["good"]);
    }

    #[test]
    fn the_cache_notices_a_refreshed_database() {
        let repos = arch_system("refresh", "[core]\n", &[("core", tarball(&[("a", "1-1", "A", "any")]))]);
        assert_eq!(repos.packages().len(), 1);
        let Backend::Pacman { sync_dir, .. } = &repos.backend else { unreachable!() };
        // "pacman -Sy" replaces the file.
        std::fs::write(
            sync_dir.join("core.db"),
            tarball(&[("a", "1-1", "A", "any"), ("b-longer-name", "2-1", "B", "any")]),
        )
        .unwrap();
        assert_eq!(repos.packages().len(), 2);
    }

    const APT_MAIN: &str = "Package: ripgrep\nArchitecture: amd64\nVersion: 14.1.0-1\nPriority: optional\nSection: universe/utils\nInstalled-Size: 5000\nDepends: libc6 (>= 2.34)\nFilename: pool/universe/r/rust-ripgrep/ripgrep_14.1.0-1_amd64.deb\nSize: 1300000\nSHA256: abc\nHomepage: https://github.com/BurntSushi/ripgrep\nDescription: Recursively searches directories for a regex pattern\n ripgrep is a line-oriented search tool.\n .\n Second paragraph.\nDescription-md5: 0123\n\nPackage: fonts-thing\nArchitecture: all\nVersion: 1.0-2\nDescription: Some fonts\n\nPackage: only-i386\nArchitecture: i386\nVersion: 1.0\nDescription: Foreign architecture\n\nPackage: broken-no-version\nDescription: skipped\n";
    const APT_UPDATES: &str = "Package: ripgrep\nArchitecture: amd64\nVersion: 14.1.1-1ubuntu0.1\nDescription: Recursively searches directories for a regex pattern\n";

    #[test]
    fn apt_lists_plain_and_compressed() {
        let d = dir("apt");
        std::fs::write(d.join("archive.ubuntu.com_ubuntu_dists_noble_universe_binary-amd64_Packages"), APT_MAIN)
            .unwrap();
        // The updates pocket is lz4-compressed, as on systems that keep lists compressed.
        let mut lz4 = lz4_flex::frame::FrameEncoder::new(Vec::new());
        lz4.write_all(APT_UPDATES.as_bytes()).unwrap();
        std::fs::write(
            d.join("archive.ubuntu.com_ubuntu_dists_noble-updates_universe_binary-amd64_Packages.lz4"),
            lz4.finish().unwrap(),
        )
        .unwrap();
        std::fs::write(d.join("archive.ubuntu.com_ubuntu_dists_noble_InRelease"), "not a package list").unwrap();
        std::fs::write(d.join("lock"), "").unwrap();

        // dpkg's record: ripgrep is installed (an older version), fonts-thing was removed.
        std::fs::write(
            d.join("status"),
            "Package: ripgrep\nStatus: install ok installed\nVersion: 14.1.0-1\n\nPackage: fonts-thing\nStatus: deinstall ok config-files\nVersion: 1.0-1\n",
        )
        .unwrap();
        let repos = SystemRepos::new(
            Backend::Apt { lists_dir: d.clone(), status: d.join("status") },
            Platform::preset("linux-x64-debian").unwrap(),
        );
        assert!(repos.available_on(&Platform::preset("linux-x64-debian").unwrap()));
        assert!(!repos.available_on(&Platform::preset("linux-x64-arch").unwrap()));

        let names: Vec<String> = repos.packages().iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, ["fonts-thing", "ripgrep"], "foreign-architecture and incomplete entries are left out");

        // The newer version from noble-updates wins, as it would with apt.
        let rg = repos.summary("ripgrep").unwrap();
        assert_eq!(rg.version.as_deref(), Some("14.1.1-1ubuntu0.1"));
        assert_eq!(rg.origin.as_deref(), Some("noble-updates/universe"));
        assert_eq!(rg.id.to_string(), "apt:ripgrep");
        assert_eq!(rg.installed_version.as_deref(), Some("14.1.0-1"), "what dpkg has, not what the archive offers");
        assert_eq!(repos.summary("fonts-thing").unwrap().installed_version, None, "removed packages are not installed");
        assert_eq!(repos.search("ripgrep", 5).unwrap()[0].installed_version.as_deref(), Some("14.1.0-1"));
        assert_eq!(repos.search("regex", 5).unwrap()[0].name, "ripgrep");
        assert_eq!(repos.releases("fonts-thing").unwrap()[0].assets[0].name, "fonts-thing (noble/universe)");
    }

    #[test]
    fn apt_repository_labels() {
        assert_eq!(
            apt_repo_label("archive.ubuntu.com_ubuntu_dists_noble-updates_main_binary-amd64_Packages.lz4"),
            "noble-updates/main"
        );
        assert_eq!(apt_repo_label("deb.debian.org_debian_dists_trixie_main_binary-arm64_Packages"), "trixie/main");
        assert_eq!(
            apt_repo_label("download.docker.com_linux_ubuntu_dists_noble_stable_binary-amd64_Packages"),
            "noble/stable"
        );
        assert_eq!(
            apt_repo_label("ppa.launchpadcontent.net_someone_ppa_ubuntu_dists_noble_main_binary-amd64_Packages"),
            "noble/main"
        );
        assert_eq!(apt_repo_label("example.org_flat_._Packages"), "example.org");
    }

    #[test]
    fn a_machine_without_the_package_manager_has_no_such_source() {
        let empty = dir("empty");
        let repos = SystemRepos::new(
            Backend::Pacman { sync_dir: empty.clone(), conf: empty.join("none"), local_dir: empty.join("none") },
            Platform::preset("linux-x64-arch").unwrap(),
        );
        assert!(!repos.available_on(&Platform::preset("linux-x64-arch").unwrap()), "pacman but no databases yet");
        assert!(repos.packages().is_empty());
        let missing = SystemRepos::new(
            Backend::Apt { lists_dir: "/definitely/not/here".into(), status: "/definitely/not/here".into() },
            Platform::preset("linux-x64-debian").unwrap(),
        );
        assert!(!missing.available_on(&Platform::preset("linux-x64-debian").unwrap()));
    }
}
