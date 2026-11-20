//! The Windows Package Manager's community repository, through the `winget` command
//! that ships with Windows 10 and 11.
//!
//! winget has no public search service to ask: its catalog is a signed database the
//! command downloads for itself. So the store asks the command, and reads the table
//! it prints. Installing is left to winget as well: the store shows the command.
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
}

impl Default for Winget {
    fn default() -> Self {
        Winget::new()
    }
}

impl Winget {
    pub fn new() -> Winget {
        Winget { run: Arc::new(run_winget) }
    }

    /// With a stand-in for the command, for tests.
    pub fn with_runner(run: Run) -> Winget {
        Winget { run }
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

/// Read `winget show`: a "Found Name [Id]" line, then `Field: value` lines.
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
        platform.os == Os::Windows
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
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
        Some(Format::WingetPackage.label())
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
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
