//! The Windows Package Manager's community repository, through the `winget` command
//! that ships with Windows 10 and 11.
//!
//! winget has no public search service to ask: its catalog is a signed database the
//! command downloads for itself. So the store asks the command, and reads the table
//! it prints. Installing is left to winget as well: the store shows the command.
//!
//! On a Linux system that can run Windows programs there is no such command, so the
//! catalog is read directly ([`super::winget_index`]) and the installer it names is
//! downloaded, checked against the catalog's checksum and set up to run through
//! Proton like any other Windows installer.
//!
//! Not verified on Windows: the shape of what `winget search` and `winget show` print
//! is written from memory of using them. The table is read by where its column headings
//! stand, so wider or narrower columns do not matter; a Windows set to another language
//! prints other headings and field names, and then nothing is found.

use super::Source;
use crate::asset::{AssetInfo, Format, TargetArch, TargetOs};
use crate::error::{Error, Result};
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::{Os, Platform};
use std::sync::Arc;

/// Runs `winget` with these arguments and returns what it printed.
pub type Run = Arc<dyn Fn(&[String]) -> Result<String> + Send + Sync>;

pub struct Winget {
    run: Run,
    /// Where there is no `winget` command: the catalog itself, read over the network.
    web: Option<Web>,
}

struct Web {
    http: Arc<dyn crate::http::Http>,
    /// Where the downloaded index is kept.
    dir: std::path::PathBuf,
    packages: std::sync::Mutex<Option<Arc<Vec<super::winget_index::Package>>>>,
}

impl Web {
    fn packages(&self) -> Result<Arc<Vec<super::winget_index::Package>>> {
        let mut kept = self.packages.lock().unwrap();
        if let Some(packages) = kept.as_ref() {
            return Ok(packages.clone());
        }
        let packages = Arc::new(super::winget_index::load(self.http.as_ref(), &self.dir)?);
        *kept = Some(packages.clone());
        Ok(packages)
    }

    fn package(&self, key: &str) -> Result<super::winget_index::Package> {
        self.packages()?
            .iter()
            .find(|package| package.id.eq_ignore_ascii_case(key))
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("winget has no package called “{key}”.")))
    }
}

/// How well a package answers what was searched for; lower is better, `None` not at all.
fn fits(package: &super::winget_index::Package, words: &str) -> Option<u8> {
    let (name, id, moniker) = (package.name.to_lowercase(), package.id.to_lowercase(), package.moniker.to_lowercase());
    Some(if name == words || moniker == words {
        0
    } else if name.starts_with(words) {
        1
    } else if name.split([' ', '-']).any(|word| word == words) || name.contains(words) {
        2
    } else if id.contains(&words.replace(' ', "")) {
        3
    } else {
        return None;
    })
}

/// The file name at the end of an installer's address, when it has a usable one.
fn installer_name(url: &str, id: &str, version: &str) -> String {
    let last = url.split(['?', '#']).next().unwrap_or(url).rsplit('/').next().unwrap_or_default();
    let plain =
        !last.is_empty() && last.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'));
    match plain && last.to_lowercase().ends_with(".exe") {
        true => last.to_string(),
        false => format!("{id}-{version}-setup.exe"),
    }
}

impl Default for Winget {
    fn default() -> Self {
        Winget::new()
    }
}

impl Winget {
    pub fn new() -> Winget {
        Winget { run: Arc::new(run_winget), web: None }
    }

    /// With a stand-in for the command, for tests.
    pub fn with_runner(run: Run) -> Winget {
        Winget { run, web: None }
    }

    /// Without the command: the catalog read over the network, its index kept in `dir`.
    /// For a Linux system that runs Windows programs through Proton.
    pub fn from_the_catalog(http: Arc<dyn crate::http::Http>, dir: std::path::PathBuf) -> Winget {
        let web = Web { http, dir, packages: std::sync::Mutex::new(None) };
        Winget { run: Arc::new(run_winget), web: Some(web) }
    }

    fn show(&self, key: &str) -> Result<Shown> {
        if !crate::model::valid_key(SourceKind::Winget, key) {
            return Err(Error::NotFound(format!("\"{key}\" is not a winget package id")));
        }
        let args = ["show", "--id", key, "--exact", "--source", "winget", "--accept-source-agreements"];
        let text = (self.run)(&args.map(String::from))?;
        parse_show(&text, key).ok_or_else(|| Error::NotFound(format!("winget has no package called “{key}”.")))
    }
}

fn run_winget(args: &[String]) -> Result<String> {
    let mut command = std::process::Command::new("winget");
    command.args(args).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this a console window flashes up behind the store.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(|e| Error::NotFound(format!("winget could not be started: {e}")))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A row of the table `winget search` prints.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    name: String,
    id: String,
    version: String,
}

/// Read the table by where its headings stand:
///
/// ```text
/// Name              Id            Version  Match        Source
/// -------------------------------------------------------------
/// VLC media player  VideoLAN.VLC  3.0.21   Moniker: vlc winget
/// ```
fn parse_search(text: &str) -> Vec<Row> {
    // A progress spinner is drawn with carriage returns; only what follows the last counts.
    let lines: Vec<&str> = text.lines().map(|line| line.rsplit('\r').next().unwrap_or(line)).collect();
    let Some(at) = lines.iter().position(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        words.first() == Some(&"Name") && words.contains(&"Id") && words.contains(&"Version")
    }) else {
        return Vec::new();
    };
    let heading: Vec<char> = lines[at].chars().collect();
    let column = |title: &str| -> Option<usize> {
        let title: Vec<char> = title.chars().collect();
        (0..heading.len()).find(|&i| {
            heading[i..].starts_with(&title)
                && (i == 0 || heading[i - 1] == ' ')
                && heading.get(i + title.len()).is_none_or(|c| *c == ' ')
        })
    };
    let (Some(id_at), Some(version_at)) = (column("Id"), column("Version")) else { return Vec::new() };
    let version_end = ["Match", "Source"].iter().filter_map(|title| column(title)).min();
    let cut = |chars: &[char], from: usize, to: Option<usize>| -> String {
        let to = to.unwrap_or(chars.len()).min(chars.len());
        if from >= to {
            return String::new();
        }
        chars[from..to].iter().collect::<String>().trim().to_string()
    };
    lines[at + 1..]
        .iter()
        .filter(|line| !line.trim().is_empty() && !line.trim().chars().all(|c| c == '-'))
        .filter_map(|line| {
            let chars: Vec<char> = line.chars().collect();
            let row = Row {
                name: cut(&chars, 0, Some(id_at)),
                id: cut(&chars, id_at, Some(version_at)),
                version: cut(&chars, version_at, version_end),
            };
            // An id cut short with "…" cannot be installed by; leave such rows out.
            (crate::model::valid_key(SourceKind::Winget, &row.id) && !row.name.is_empty()).then_some(row)
        })
        .collect()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Shown {
    name: String,
    version: String,
    publisher: String,
    description: String,
    homepage: String,
    license: String,
}

/// Read `winget show`: a `Found Name [Id]` line, then `Field: value` lines.
fn parse_show(text: &str, key: &str) -> Option<Shown> {
    let mut shown = Shown::default();
    let mut found = false;
    for line in text.lines().map(|line| line.rsplit('\r').next().unwrap_or(line)) {
        if let Some(rest) = line.strip_prefix("Found ") {
            if let Some((name, id)) = rest.rsplit_once('[') {
                if id.trim_end_matches(']').trim().eq_ignore_ascii_case(key) {
                    shown.name = name.trim().to_string();
                    found = true;
                }
            }
            continue;
        }
        // Only top-level fields: the installer's details are indented.
        let Some((field, value)) = line.split_once(':') else { continue };
        let value = value.trim().to_string();
        match field {
            "Version" => shown.version = value,
            "Publisher" => shown.publisher = value,
            "Description" => shown.description = value,
            "Homepage" => shown.homepage = value,
            "License" => shown.license = value,
            _ => {}
        }
    }
    found.then_some(shown)
}

fn summary_of(id: &str, name: &str, version: &str) -> AppSummary {
    let mut app = AppSummary::new(AppId::new(SourceKind::Winget, id), name);
    // Publisher.Package: the first part is the nearest thing to an author a search gives.
    app.author = id.split('.').next().unwrap_or("").to_string();
    app.version = (!version.is_empty() && version != "Unknown").then(|| version.to_string());
    app.homepage = format!("https://winget.run/pkg/{}", id.replacen('.', "/", 1));
    app
}

impl Source for Winget {
    fn kind(&self) -> SourceKind {
        SourceKind::Winget
    }

    fn available_on(&self, platform: &Platform) -> bool {
        match self.web {
            // Read from the catalog: only where its Windows programs can be run.
            Some(_) => platform.os == Os::Linux && platform.runs_windows(),
            None => platform.os == Os::Windows,
        }
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        if let Some(web) = &self.web {
            let words = query.trim().to_lowercase();
            if words.chars().count() < 2 {
                return Ok(Vec::new());
            }
            // No index, no such source for now: nothing found, no error.
            let Ok(packages) = web.packages() else { return Ok(Vec::new()) };
            let mut hits: Vec<(u8, &super::winget_index::Package)> =
                packages.iter().filter_map(|package| fits(package, &words).map(|rank| (rank, package))).collect();
            hits.sort_by_key(|(rank, package)| (*rank, package.name.len(), package.id.len()));
            return Ok(hits
                .into_iter()
                .take(limit.clamp(1, 20))
                .map(|(_, package)| summary_of(&package.id, &package.name, &package.version))
                .collect());
        }
        // One argument, never a command line; a leading dash would read as an option.
        let query = query.trim().trim_start_matches(['-', '/']).to_string();
        if query.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let count = limit.clamp(1, 20).to_string();
        let args =
            ["search", query.as_str(), "--source", "winget", "--count", count.as_str(), "--accept-source-agreements"];
        // A Windows without winget simply has no such source: nothing found, no error.
        let Ok(text) = (self.run)(&args.map(String::from)) else { return Ok(Vec::new()) };
        Ok(parse_search(&text)
            .into_iter()
            .take(limit)
            .map(|row| summary_of(&row.id, &row.name, &row.version))
            .collect())
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        if let Some(web) = &self.web {
            let package = web.package(key)?;
            let mut app = summary_of(&package.id, &package.name, &package.version);
            // Its description is a file of its own; without it the name and version do.
            let said = super::winget_index::description_url(&package.id, &package.version)
                .and_then(|url| web.http.get(&url).ok())
                .map(|text| super::winget_index::description_in(&text));
            if let Some((publisher, summary, homepage, license)) = said {
                if !publisher.is_empty() {
                    app.author = publisher;
                }
                app.summary = summary;
                app.license = (!license.is_empty()).then_some(license);
                if !homepage.is_empty() {
                    app.homepage = homepage;
                }
            }
            return Ok(app);
        }
        let shown = self.show(key)?;
        let mut app = summary_of(key, &shown.name, &shown.version);
        if !shown.publisher.is_empty() {
            app.author = shown.publisher;
        }
        app.summary = shown.description;
        app.license = (!shown.license.is_empty()).then_some(shown.license);
        if shown.homepage.starts_with("https://") {
            app.homepage = shown.homepage;
        }
        Ok(app)
    }

    fn search_hits_are_final(&self) -> Option<&'static str> {
        Some(match self.web {
            Some(_) => Format::Exe.label(),
            None => Format::WingetPackage.label(),
        })
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        if let Some(web) = &self.web {
            use super::winget_index::{for_proton, installer_manifest_url, installers_in};
            let package = web.package(key)?;
            let listed = installer_manifest_url(&package.id, &package.version)
                .and_then(|url| web.http.get(&url).ok())
                .map(|text| installers_in(&text))
                .unwrap_or_default();
            let asset = match for_proton(&listed) {
                // An installer that runs under Proton: fetched and checked like any file.
                Some(installer) => Asset {
                    name: installer_name(&installer.url, &package.id, &package.version),
                    size: 0,
                    url: installer.url.clone(),
                    digest: Some(format!("sha256:{}", installer.sha256)),
                    info: Some(AssetInfo {
                        format: Format::Exe,
                        os: TargetOs::Known(Os::Windows),
                        arch: match installer.arch.as_str() {
                            "x64" => TargetArch::One(crate::platform::Arch::X86_64),
                            "x86" => TargetArch::One(crate::platform::Arch::X86),
                            _ => TargetArch::Unknown,
                        },
                        libc: None,
                        tv: false,
                        debug: false,
                    }),
                },
                // Only in one of Windows' own package formats: shown as what it is,
                // which this device cannot install.
                None => Asset {
                    name: key.to_string(),
                    size: 0,
                    url: format!("https://winget.run/pkg/{}", key.replacen('.', "/", 1)),
                    digest: None,
                    info: Some(AssetInfo {
                        format: Format::WingetPackage,
                        os: TargetOs::Known(Os::Windows),
                        arch: TargetArch::Unknown,
                        libc: None,
                        tv: false,
                        debug: false,
                    }),
                },
            };
            return Ok(vec![Release {
                version: package.version,
                published: String::new(),
                prerelease: false,
                downloads: None,
                notes: String::new(),
                assets: vec![asset],
            }]);
        }
        let shown = self.show(key)?;
        Ok(vec![Release {
            version: if shown.version.is_empty() { "latest".to_string() } else { shown.version },
            published: String::new(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets: vec![Asset {
                name: key.to_string(),
                size: 0,
                url: format!("https://winget.run/pkg/{}", key.replacen('.', "/", 1)),
                digest: None,
                info: Some(AssetInfo {
                    format: Format::WingetPackage,
                    os: TargetOs::Known(Os::Windows),
                    // winget picks the installer for this machine.
                    arch: TargetArch::Unknown,
                    libc: None,
                    tv: false,
                    debug: false,
                }),
            }],
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = "   -\r   \\\r                                                             \rName                                  Id                           Version   Match          Source\n--------------------------------------------------------------------------------------------------\nVLC media player                      VideoLAN.VLC                 3.0.21    Moniker: vlc   winget\nVLC media player nightly              VideoLAN.VLC.Nightly         4.0.0     Tag: vlc       winget\nA very long name that gets cut sho…   SomePublisher.AVeryLongIde…  1.0                      winget\nPart of the footer, not a row\n";

    const SHOW: &str = "Found VLC media player [VideoLAN.VLC]\nVersion: 3.0.21\nPublisher: VideoLAN\nPublisher Url: https://www.videolan.org/\nDescription: VLC is a free and open source cross-platform multimedia player.\nHomepage: https://www.videolan.org/vlc/\nLicense: GPL-2.0\nInstaller:\n  Installer Type: nullsoft\n  Installer Url: https://example.invalid/vlc.exe\n";

    #[test]
    fn read_from_the_catalog_it_is_for_linux_that_runs_windows_programs_and_ranks_by_name() {
        use crate::source::winget_index::Package;
        let package = |id: &str, name: &str, moniker: &str| Package {
            id: id.into(),
            name: name.into(),
            moniker: moniker.into(),
            version: "1.0".into(),
        };
        let vlc = package("VideoLAN.VLC", "VLC media player", "vlc");
        assert_eq!(fits(&vlc, "vlc"), Some(0), "the word people type for it");
        assert_eq!(fits(&vlc, "vlc media"), Some(1));
        assert_eq!(fits(&vlc, "media"), Some(2));
        assert_eq!(fits(&vlc, "videolan"), Some(3));
        assert_eq!(fits(&vlc, "paint"), None);

        assert_eq!(
            installer_name("https://example.invalid/a/WinMerge-2.16-Setup.exe?x=1", "A.B", "1"),
            "WinMerge-2.16-Setup.exe"
        );
        assert_eq!(installer_name("https://example.invalid/download?product=thing", "A.B", "1.2"), "A.B-1.2-setup.exe");
        assert_eq!(installer_name("https://example.invalid/a/setup%20x.exe", "A.B", "1.2"), "A.B-1.2-setup.exe");

        struct NoWeb;
        impl crate::http::Http for NoWeb {
            fn get(&self, url: &str) -> Result<String> {
                Err(Error::NotFound(url.to_string()))
            }
            fn download(
                &self,
                url: &str,
                _: &std::path::Path,
                _: &mut dyn FnMut(u64, Option<u64>) -> bool,
            ) -> Result<String> {
                Err(Error::NotFound(url.to_string()))
            }
        }
        let dir = std::env::temp_dir().join(format!("bilby-winget-none-{}", std::process::id()));
        let winget = Winget::from_the_catalog(Arc::new(NoWeb), dir.clone());
        let mut linux = Platform::preset("linux-x64-arch").unwrap();
        assert!(!winget.available_on(&Platform::preset("windows-x64").unwrap()), "Windows asks its own winget");
        linux.linux.as_mut().unwrap().windows = false;
        assert!(!winget.available_on(&linux), "nothing to run them with");
        linux.linux.as_mut().unwrap().windows = true;
        assert!(winget.available_on(&linux));
        // Without the index there is simply nothing found.
        assert!(winget.search("vlc", 5).unwrap().is_empty());
        assert!(Winget::new().available_on(&Platform::preset("windows-x64").unwrap()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_search_table_is_read_by_where_its_headings_stand() {
        let rows = parse_search(SEARCH);
        assert_eq!(rows.len(), 2, "a row whose id was cut short is left out, and so is the footer: {rows:?}");
        assert_eq!(
            rows[0],
            Row { name: "VLC media player".into(), id: "VideoLAN.VLC".into(), version: "3.0.21".into() }
        );
        assert_eq!(rows[1].id, "VideoLAN.VLC.Nightly");
        // Narrower columns and no Match or Source column.
        let narrow = "Name   Id         Version\n-------------------------\nfd     sharkdp.fd 10.2.0\n";
        assert_eq!(
            parse_search(narrow),
            [Row { name: "fd".into(), id: "sharkdp.fd".into(), version: "10.2.0".into() }]
        );
        assert!(parse_search("No package found matching input criteria.\n").is_empty());
        assert!(parse_search("").is_empty());
    }

    #[test]
    fn a_package_is_described_and_offered_as_a_winget_install() {
        let asked: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
        let log = asked.clone();
        let source = Winget::with_runner(Arc::new(move |args: &[String]| {
            log.lock().unwrap().push(args.to_vec());
            Ok(if args[0] == "search" { SEARCH } else { SHOW }.to_string())
        }));
        let hits = source.search("-vlc", 5).unwrap();
        assert_eq!(hits[0].id, AppId::new(SourceKind::Winget, "VideoLAN.VLC"));
        assert_eq!((hits[0].name.as_str(), hits[0].version.as_deref()), ("VLC media player", Some("3.0.21")));
        assert_eq!(
            asked.lock().unwrap()[0][..2],
            ["search", "vlc"],
            "the query is one argument and never starts with a dash"
        );

        let app = source.summary("VideoLAN.VLC").unwrap();
        assert_eq!((app.author.as_str(), app.license.as_deref()), ("VideoLAN", Some("GPL-2.0")));
        assert_eq!(app.homepage, "https://www.videolan.org/vlc/");
        let release = &source.releases("VideoLAN.VLC").unwrap()[0];
        assert_eq!(release.version, "3.0.21");
        assert_eq!(release.assets[0].info.as_ref().unwrap().format, Format::WingetPackage);

        assert!(
            matches!(source.summary("../x"), Err(Error::NotFound(_))),
            "ids are checked before they reach the command"
        );
        assert!(
            matches!(source.summary("Other.App"), Err(Error::NotFound(_))),
            "an answer about another package is not used"
        );
        assert!(source.available_on(&Platform::preset("windows-x64").unwrap()));
        assert!(!source.available_on(&Platform::preset("linux-x64-debian").unwrap()));
    }

    #[test]
    fn a_windows_without_winget_finds_nothing_and_says_nothing() {
        let source = Winget::with_runner(Arc::new(|_: &[String]| Err(Error::NotFound("no winget".into()))));
        assert!(source.search("vlc", 5).unwrap().is_empty());
    }
}
