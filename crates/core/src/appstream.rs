//! Which AppStream id an installed-from-a-repository package or a GitHub project has.
//!
//! Ratings are keyed by AppStream id (`org.mozilla.firefox`), while the store knows
//! apps as package names and repositories. Distributions ship a catalog that links
//! them, and it is already on disk: XML on Arch (`archlinux-appstream-data`), YAML
//! ("DEP-11") on Debian and Ubuntu. Reading it costs no network requests.
//!
//! Only three things are read from each entry: its id, its package name, and the
//! addresses of its homepage and source repository. The formats were read from
//! AppStream's documentation (<https://github.com/ximion/appstream>, `docs/xml`); the
//! reader has been tried on files written for the tests, not on a real system's.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Components {
    by_package: HashMap<String, String>,
    /// `github.com/owner/repo`, lower-cased.
    by_repo: HashMap<String, String>,
}

/// Where distributions keep their catalogs, current locations first.
pub fn system_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for base in [
        "/usr/share/swcatalog",
        "/var/lib/swcatalog",
        "/var/cache/swcatalog",
        "/usr/share/app-info",
        "/var/lib/app-info",
    ] {
        for kind in ["xml", "xmls", "yaml"] {
            dirs.push(PathBuf::from(base).join(kind));
        }
    }
    dirs
}

/// `github.com/owner/repo` from an address, when it is a GitHub repository's.
fn repo_key(url: &str) -> Option<String> {
    let rest = url.trim().trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
    let path = rest.strip_prefix("github.com/")?;
    let mut parts = path.split(['/', '?', '#']).filter(|p| !p.is_empty());
    let (owner, repo) = (parts.next()?, parts.next()?);
    Some(format!("github.com/{owner}/{}", repo.trim_end_matches(".git")).to_lowercase())
}

impl Components {
    pub fn load(dirs: &[PathBuf]) -> Components {
        let mut all = Components::default();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for path in entries.flatten().map(|e| e.path()) {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                let Some(text) = read_maybe_gz(&path) else { continue };
                if name.ends_with(".xml") || name.ends_with(".xml.gz") {
                    all.add_xml(&text);
                } else if name.ends_with(".yml") || name.ends_with(".yml.gz") {
                    all.add_yaml(&text);
                }
            }
        }
        all
    }

    pub fn is_empty(&self) -> bool {
        self.by_package.is_empty() && self.by_repo.is_empty()
    }

    pub fn id_of_package(&self, package: &str) -> Option<&str> {
        self.by_package.get(package).map(String::as_str)
    }

    /// `owner/repo` on GitHub.
    pub fn id_of_repo(&self, key: &str) -> Option<&str> {
        self.by_repo.get(&format!("github.com/{}", key.to_lowercase())).map(String::as_str)
    }

    fn add(&mut self, id: &str, packages: &[String], urls: &[String]) {
        let id = id.trim();
        if id.is_empty() {
            return;
        }
        for package in packages {
            self.by_package.entry(package.trim().to_string()).or_insert_with(|| id.to_string());
        }
        for key in urls.iter().filter_map(|u| repo_key(u)) {
            self.by_repo.entry(key).or_insert_with(|| id.to_string());
        }
    }

    /// AppStream catalog XML: `<component><id>..</id><pkgname>..</pkgname><url type=..>..</url>`.
    pub fn add_xml(&mut self, text: &str) {
        for component in text.split("<component").skip(1) {
            let component = component.split("</component>").next().unwrap_or(component);
            // Only applications people start; codecs, fonts and drivers have no ratings.
            let kind = component.split('>').next().unwrap_or("");
            if !(kind.contains("desktop") || kind.contains("console-application") || kind.contains("web-application")) {
                continue;
            }
            let texts = |tag: &str| -> Vec<String> {
                component
                    .split(&format!("<{tag}"))
                    .skip(1)
                    .filter_map(|rest| {
                        // `<pkgname>x</pkgname>` or `<url type="homepage">x</url>`, not `<pkgnames>`.
                        let after = rest
                            .strip_prefix('>')
                            .or_else(|| rest.strip_prefix(' ').and_then(|r| r.split_once('>').map(|(_, t)| t)))?;
                        Some(after.split('<').next()?.trim().replace("&amp;", "&"))
                    })
                    .collect()
            };
            let urls: Vec<String> = component
                .split("<url")
                .skip(1)
                .filter_map(|rest| {
                    let (attributes, after) = rest.split_once('>')?;
                    (attributes.contains("homepage") || attributes.contains("vcs-browser"))
                        .then(|| after.split('<').next().unwrap_or("").trim().to_string())
                })
                .collect();
            if let Some(id) = texts("id").first() {
                self.add(id, &texts("pkgname"), &urls);
            }
        }
    }

    /// DEP-11 YAML: documents separated by `---`, with `ID:`, `Package:` and a `Url:` block.
    pub fn add_yaml(&mut self, text: &str) {
        for document in text.split("\n---") {
            let (mut id, mut kind, mut packages, mut urls) = (String::new(), String::new(), Vec::new(), Vec::new());
            for line in document.lines() {
                let value = |key: &str| line.strip_prefix(key).map(|v| v.trim().trim_matches(['\'', '"']).to_string());
                if let Some(v) = value("ID:") {
                    id = v;
                } else if let Some(v) = value("Type:") {
                    kind = v;
                } else if let Some(v) = value("Package:") {
                    packages.push(v);
                } else if let Some((key, v)) = line.trim().split_once(": ") {
                    if key == "homepage" || key == "vcs-browser" {
                        urls.push(v.trim().trim_matches(['\'', '"']).to_string());
                    }
                }
            }
            if kind.contains("application") {
                self.add(&id, &packages, &urls);
            }
        }
    }
}

/// The text of a file, unpacked when it is gzip. Catalogs are tens of megabytes at most.
fn read_maybe_gz(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut text = String::new();
        flate2::read::GzDecoder::new(&bytes[..]).take(256 * 1024 * 1024).read_to_string(&mut text).ok()?;
        Some(text)
    } else {
        String::from_utf8(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<components version="0.16" origin="archlinux-arch-extra">
  <component type="desktop-application">
    <id>org.mozilla.firefox</id>
    <pkgname>firefox</pkgname>
    <name>Firefox</name>
    <url type="homepage">https://www.mozilla.org/firefox/</url>
  </component>
  <component type="desktop-application">
    <id>io.example.Notes.desktop</id>
    <pkgname>notes</pkgname>
    <pkgname>notes-git</pkgname>
    <url type="bugtracker">https://github.com/wrong/place/issues</url>
    <url type="vcs-browser">https://github.com/Example/Notes</url>
  </component>
  <component type="codec">
    <id>org.example.codec</id>
    <pkgname>some-codec</pkgname>
  </component>
</components>"#;

    const YAML: &str = "---\nFile: DEP-11\nVersion: '1.0'\nOrigin: ubuntu-noble-main\n---\nType: desktop-application\nID: org.gnome.Calculator\nPackage: gnome-calculator\nName:\n  C: Calculator\nUrl:\n  homepage: https://github.com/GNOME/gnome-calculator\n---\nType: font\nID: org.example.font\nPackage: fonts-example\n";

    #[test]
    fn packages_and_repositories_are_linked_to_their_ids() {
        let mut c = Components::default();
        c.add_xml(XML);
        c.add_yaml(YAML);
        assert_eq!(c.id_of_package("firefox"), Some("org.mozilla.firefox"));
        assert_eq!(c.id_of_package("notes-git"), Some("io.example.Notes.desktop"));
        assert_eq!(c.id_of_repo("example/notes"), Some("io.example.Notes.desktop"), "whatever its case");
        assert_eq!(c.id_of_repo("wrong/place"), None, "a bug tracker is not the project's home");
        assert_eq!(c.id_of_package("gnome-calculator"), Some("org.gnome.Calculator"));
        assert_eq!(c.id_of_repo("GNOME/gnome-calculator"), Some("org.gnome.Calculator"));
        assert_eq!(c.id_of_package("some-codec"), None, "only applications");
        assert_eq!(c.id_of_package("fonts-example"), None);
        assert_eq!(c.id_of_package("unknown"), None);
    }

    #[test]
    fn catalogs_are_read_from_disk_packed_or_not() {
        let dir = std::env::temp_dir().join(format!("bilby-appstream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("xml")).unwrap();
        std::fs::create_dir_all(dir.join("yaml")).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, XML.as_bytes()).unwrap();
        std::fs::write(dir.join("xml").join("extra.xml.gz"), gz.finish().unwrap()).unwrap();
        std::fs::write(dir.join("yaml").join("main_Components-amd64.yml"), YAML).unwrap();
        std::fs::write(dir.join("yaml").join("junk.gz"), [0x1f, 0x8b, 0, 1, 2]).unwrap();
        let c = Components::load(&[dir.join("xml"), dir.join("yaml"), dir.join("missing")]);
        assert_eq!(c.id_of_package("firefox"), Some("org.mozilla.firefox"));
        assert_eq!(c.id_of_package("gnome-calculator"), Some("org.gnome.Calculator"));
        assert!(Components::load(&[dir.join("missing")]).is_empty());
    }
}
