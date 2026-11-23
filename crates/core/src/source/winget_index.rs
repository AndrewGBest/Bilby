//! The Windows Package Manager's catalog, read without Windows.
//!
//! On Windows the `winget` command is asked. Anywhere else there is no such command,
//! but what it reads is public: an index of every package (a zip file holding an
//! SQLite database, which the command downloads for itself), and one small text file
//! per package version in the community repository, saying where its installer is
//! and what its checksum is. That is enough to offer those Windows programs on a
//! Linux system that can run them.
//!
//! Only as much of the two file formats as that index needs is read here: one file
//! out of a zip (including the large-file form the index uses), and the rows of one
//! ordinary table of an SQLite file. Both were checked against the real index.

use crate::error::{Error, Result};
use crate::http::Http;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub const INDEX_URL: &str = "https://cdn.winget.microsoft.com/cache/source2.msix";
pub const MANIFESTS: &str = "https://raw.githubusercontent.com/microsoft/winget-pkgs/master/manifests";

/// How long a downloaded index is used before a newer one is fetched.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// One package, as the index lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// `Publisher.Name`.
    pub id: String,
    pub name: String,
    /// The short word people type for it (`vlc`), when it has one.
    pub moniker: String,
    pub version: String,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as u64)
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// One file out of a zip archive, unpacked. `limit` bounds what is unpacked, since an
/// archive can claim any size it likes.
pub fn file_in_zip(zip: &[u8], name: &str, limit: usize) -> Option<Vec<u8>> {
    // The list of files is at the end, found from the last "end of list" record.
    let end = zip.windows(4).rposition(|w| w == b"PK\x05\x06")?;
    let (mut entries, mut at) = (u16_at(zip, end + 10)? as u64, u32_at(zip, end + 16)?);
    // A large archive says "see the longer record" by filling those in with ones.
    if at == 0xFFFF_FFFF || entries == 0xFFFF {
        let locator = end.checked_sub(20).filter(|l| zip.get(*l..*l + 4) == Some(b"PK\x06\x07"))?;
        let long = usize::try_from(u64_at(zip, locator + 8)?).ok()?;
        if zip.get(long..long + 4) != Some(b"PK\x06\x06") {
            return None;
        }
        entries = u64_at(zip, long + 32)?;
        at = u64_at(zip, long + 48)?;
    }
    let mut at = usize::try_from(at).ok()?;
    for _ in 0..entries.min(100_000) {
        if zip.get(at..at + 4) != Some(b"PK\x01\x02") {
            return None;
        }
        let method = u16_at(zip, at + 10)?;
        let (mut packed, mut size) = (u32_at(zip, at + 20)?, u32_at(zip, at + 24)?);
        let (name_len, extra_len, comment_len) = (u16_at(zip, at + 28)?, u16_at(zip, at + 30)?, u16_at(zip, at + 32)?);
        let mut local = u32_at(zip, at + 42)?;
        let entry_name = zip.get(at + 46..at + 46 + name_len)?;
        // Sizes and places too large for their fields are in an extra record, in order.
        let extra = zip.get(at + 46 + name_len..at + 46 + name_len + extra_len)?;
        let mut field = 0;
        while field + 4 <= extra.len() {
            let (id, len) = (u16_at(extra, field)?, u16_at(extra, field + 2)?);
            if id == 1 {
                let mut next = field + 4;
                for value in [&mut size, &mut packed, &mut local] {
                    if *value == 0xFFFF_FFFF {
                        *value = u64_at(extra, next)?;
                        next += 8;
                    }
                }
            }
            field += 4 + len;
        }
        if entry_name == name.as_bytes() {
            let (local, packed, size) =
                (usize::try_from(local).ok()?, usize::try_from(packed).ok()?, usize::try_from(size).ok()?);
            if zip.get(local..local + 4) != Some(b"PK\x03\x04") || size > limit {
                return None;
            }
            let start = local + 30 + u16_at(zip, local + 26)? + u16_at(zip, local + 28)?;
            let data = zip.get(start..start.checked_add(packed)?)?;
            return match method {
                0 => Some(data.to_vec()),
                8 => {
                    let mut out = Vec::with_capacity(size);
                    flate2::read::DeflateDecoder::new(data).take(limit as u64 + 1).read_to_end(&mut out).ok()?;
                    (out.len() == size).then_some(out)
                }
                _ => None,
            };
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    None
}

/// A value in a row of an SQLite table.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
    Other,
}

impl Value {
    fn text(&self) -> &str {
        match self {
            Value::Text(text) => text,
            _ => "",
        }
    }
}

/// SQLite's variable-length number: seven bits a byte, most significant first, and
/// all eight of a ninth byte.
fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for n in 0..9 {
        let byte = *bytes.get(*at)? as u64;
        *at += 1;
        if n == 8 {
            return Some((value << 8) | byte);
        }
        value = (value << 7) | (byte & 0x7f);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn be(bytes: &[u8], at: usize, len: usize) -> Option<u64> {
    Some(bytes.get(at..at + len)?.iter().fold(0u64, |value, byte| (value << 8) | *byte as u64))
}

/// One row: a header saying what kind each value is, then the values.
fn record(payload: &[u8]) -> Option<Vec<Value>> {
    let mut at = 0;
    let header_end = usize::try_from(varint(payload, &mut at)?).ok()?;
    let mut kinds = Vec::new();
    while at < header_end {
        kinds.push(varint(payload, &mut at)?);
    }
    let mut at = header_end;
    let mut values = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let (len, value) = match kind {
            0 => (0, Value::Null),
            1..=6 => {
                let len = [1, 2, 3, 4, 6, 8][kind as usize - 1];
                let raw = be(payload, at, len)?;
                // Sign-extend from however many bytes it was kept in.
                let shift = 64 - 8 * len as u32;
                (len, Value::Int(((raw << shift) as i64) >> shift))
            }
            7 => (8, Value::Other),
            8 => (0, Value::Int(0)),
            9 => (0, Value::Int(1)),
            kind if kind >= 13 && kind % 2 == 1 => {
                let len = usize::try_from((kind - 13) / 2).ok()?;
                (len, Value::Text(String::from_utf8_lossy(payload.get(at..at + len)?).into_owned()))
            }
            kind if kind >= 12 => (usize::try_from((kind - 12) / 2).ok()?, Value::Other),
            _ => return None,
        };
        at += len;
        values.push(value);
    }
    Some(values)
}

/// Every row of the table whose tree starts at page `root`. A row too long for its
/// page is left out: nothing this reads has any.
fn rows_from(db: &[u8], page_size: usize, usable: usize, root: usize, depth: u8, out: &mut Vec<Vec<Value>>) {
    if root == 0 || depth > 20 {
        return;
    }
    let Some(page) = db.get((root - 1) * page_size..root * page_size) else { return };
    // The first page begins with the file's own header.
    let head = if root == 1 { 100 } else { 0 };
    let (Some(&kind), Some(cells)) = (page.get(head), be(page, head + 3, 2)) else { return };
    let pointers = head + if kind == 5 { 12 } else { 8 };
    for n in 0..cells as usize {
        let Some(mut at) = be(page, pointers + 2 * n, 2).map(|at| at as usize) else { return };
        match kind {
            // A page of pointers to pages lower in the tree.
            5 => {
                if let Some(child) = be(page, at, 4) {
                    rows_from(db, page_size, usable, child as usize, depth + 1, out);
                }
            }
            // A page of rows.
            13 => {
                let (Some(len), Some(_rowid)) = (varint(page, &mut at), varint(page, &mut at)) else { continue };
                let len = len as usize;
                if len > usable.saturating_sub(35) {
                    continue;
                }
                if let Some(row) = page.get(at..at + len).and_then(record) {
                    out.push(row);
                }
            }
            _ => return,
        }
    }
    if kind == 5 {
        if let Some(last) = be(page, head + 8, 4) {
            rows_from(db, page_size, usable, last as usize, depth + 1, out);
        }
    }
}

/// The rows of an ordinary table in an SQLite file.
pub fn table_rows(db: &[u8], table: &str) -> Option<Vec<Vec<Value>>> {
    if db.get(..16)? != b"SQLite format 3\0" {
        return None;
    }
    let page_size = match be(db, 16, 2)? as usize {
        1 => 65536,
        size => size,
    };
    if page_size < 512 || !page_size.is_power_of_two() {
        return None;
    }
    let usable = page_size - *db.get(20)? as usize;
    // The first page lists the tables: kind, name, table, first page, definition.
    let mut tables = Vec::new();
    rows_from(db, page_size, usable, 1, 0, &mut tables);
    let root = tables.iter().find_map(|row| match (row.first()?, row.get(1)?, row.get(3)?) {
        (Value::Text(kind), Value::Text(name), Value::Int(root)) if kind == "table" && name == table => Some(*root),
        _ => None,
    })?;
    let mut rows = Vec::new();
    rows_from(db, page_size, usable, usize::try_from(root).ok()?, 0, &mut rows);
    Some(rows)
}

/// The packages in an index file as downloaded.
pub fn packages_in(index: &[u8]) -> Result<Vec<Package>> {
    let unreadable = || Error::Parse("The list of Windows programs could not be read.".into());
    let db = file_in_zip(index, "Public/index.db", 200 * 1024 * 1024).ok_or_else(unreadable)?;
    // Columns: the row's number, id, name, moniker, latest version, ...
    let rows = table_rows(&db, "packages").ok_or_else(unreadable)?;
    let packages: Vec<Package> = rows
        .iter()
        .filter(|row| row.len() >= 5)
        .map(|row| Package {
            id: row[1].text().to_string(),
            name: row[2].text().to_string(),
            moniker: row[3].text().to_string(),
            version: row[4].text().to_string(),
        })
        .filter(|package| {
            crate::model::valid_key(crate::model::SourceKind::Winget, &package.id) && !package.name.is_empty()
        })
        .collect();
    if packages.is_empty() {
        return Err(unreadable());
    }
    Ok(packages)
}

/// The index, fetched at most once a day and kept in `dir`. A copy that is too old
/// is still used when a newer one cannot be had.
pub fn load(http: &dyn Http, dir: &Path) -> Result<Vec<Package>> {
    let kept = dir.join("source2.msix");
    let fresh = std::fs::metadata(&kept)
        .and_then(|meta| meta.modified())
        .is_ok_and(|at| SystemTime::now().duration_since(at).is_ok_and(|age| age < MAX_AGE));
    if !fresh {
        std::fs::create_dir_all(dir)?;
        let fetched = dir.join("source2.msix.new");
        match http.download(INDEX_URL, &fetched, &mut |_, _| true) {
            // Only a file that can be read replaces the one in use.
            Ok(_) if std::fs::read(&fetched).is_ok_and(|bytes| packages_in(&bytes).is_ok()) => {
                std::fs::rename(&fetched, &kept)?;
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&fetched);
            }
            Err(e) if !kept.is_file() => return Err(e),
            Err(_) => {}
        }
    }
    packages_in(&std::fs::read(&kept)?)
}

/// Where a package version's files are in the community repository.
fn manifest_url(id: &str, version: &str, which: &str) -> Option<String> {
    let plain = |text: &str| !text.is_empty() && !text.contains("..") && !text.contains(['/', '\\', '?', '#', '%']);
    if !crate::model::valid_key(crate::model::SourceKind::Winget, id) || !plain(version) {
        return None;
    }
    let letter = id.chars().next()?.to_lowercase().to_string();
    let folders: Vec<String> = id.split('.').map(crate::http::url_encode).collect();
    let (id, version) = (crate::http::url_encode(id), crate::http::url_encode(version));
    Some(format!("{MANIFESTS}/{letter}/{}/{version}/{id}.{which}.yaml", folders.join("/")))
}

pub fn installer_manifest_url(id: &str, version: &str) -> Option<String> {
    manifest_url(id, version, "installer")
}

pub fn description_url(id: &str, version: &str) -> Option<String> {
    manifest_url(id, version, "locale.en-US")
}

/// The value of `Key: value` on a line of these files, without quotes.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim_start().trim_start_matches("- ").strip_prefix(key)?.strip_prefix(':')?;
    let value = rest.trim();
    let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
    let value = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')).unwrap_or(value);
    (!value.is_empty()).then_some(value)
}

/// One installer a package version offers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Installer {
    /// `x64`, `x86`, `arm64`, `neutral`.
    pub arch: String,
    /// `nullsoft`, `inno`, `burn`, `exe`, `portable`, `wix`, `msi`, `zip`, `msix`, ...
    pub kind: String,
    pub url: String,
    /// Lowercase hex.
    pub sha256: String,
    /// `user` or `machine`, when said.
    pub scope: String,
}

/// The installers in a version's installer file. What is said once at the top goes
/// for every installer that does not say otherwise.
pub fn installers_in(text: &str) -> Vec<Installer> {
    let mut overall = Installer::default();
    let mut found: Vec<Installer> = Vec::new();
    let mut listing = false;
    for line in text.lines().filter(|line| !line.trim_start().starts_with('#')) {
        let top = !line.starts_with([' ', '-']);
        if top {
            listing = line.trim_end() == "Installers:";
            if let Some(kind) = value_of(line, "InstallerType") {
                overall.kind = kind.to_lowercase();
            }
            if let Some(scope) = value_of(line, "Scope") {
                overall.scope = scope.to_lowercase();
            }
            continue;
        }
        if !listing {
            continue;
        }
        // An item of the list starts at the margin with a dash.
        if line.starts_with("- ") {
            found.push(Installer { kind: String::new(), scope: String::new(), ..Installer::default() });
        }
        // Only an installer's own lines, not those of something listed inside it.
        let own = line.starts_with("- ") || (line.starts_with("  ") && !line[2..].starts_with([' ', '-']));
        let Some(installer) = found.last_mut().filter(|_| own) else { continue };
        if let Some(arch) = value_of(line, "Architecture") {
            installer.arch = arch.to_lowercase();
        } else if let Some(kind) = value_of(line, "InstallerType") {
            installer.kind = kind.to_lowercase();
        } else if let Some(url) = value_of(line, "InstallerUrl") {
            installer.url = url.to_string();
        } else if let Some(sum) = value_of(line, "InstallerSha256") {
            installer.sha256 = sum.to_lowercase();
        } else if let Some(scope) = value_of(line, "Scope") {
            installer.scope = scope.to_lowercase();
        }
    }
    for installer in &mut found {
        if installer.kind.is_empty() {
            installer.kind = overall.kind.clone();
        }
        if installer.scope.is_empty() {
            installer.scope = overall.scope.clone();
        }
    }
    found
}

/// The installer to use on an x86-64 Linux system running it through Proton: a
/// program that installs itself or is the app itself, for this kind of processor,
/// with a checksum to hold it to. Packages in Windows' own formats cannot be used.
pub fn for_proton(installers: &[Installer]) -> Option<&Installer> {
    let usable = |i: &&Installer| {
        matches!(i.kind.as_str(), "nullsoft" | "inno" | "burn" | "exe" | "portable")
            && i.url.starts_with("https://")
            && i.url.len() < 600
            && !i.url.chars().any(|c| c.is_whitespace() || c.is_control())
            && i.sha256.len() == 64
            && i.sha256.chars().all(|c| c.is_ascii_hexdigit())
    };
    // For one person rather than the whole machine where there is a choice: there is
    // no administrator to ask inside the program's own little Windows.
    let rank = |i: &&Installer| {
        let arch = match i.arch.as_str() {
            "x64" => 0,
            "x86" => 2,
            "neutral" => 4,
            _ => return None,
        };
        Some(arch + usize::from(i.scope != "user"))
    };
    installers.iter().filter(usable).filter_map(|i| rank(&i).map(|r| (r, i))).min_by_key(|(r, _)| *r).map(|(_, i)| i)
}

/// What a version's description file says: (publisher, summary, homepage, licence).
pub fn description_in(text: &str) -> (String, String, String, String) {
    let top = |key: &str| {
        text.lines()
            .filter(|line| !line.starts_with([' ', '-', '#']))
            .find_map(|line| value_of(line, key))
            .unwrap_or_default()
            .to_string()
    };
    let homepage = [top("PackageUrl"), top("PublisherUrl")].into_iter().find(|u| u.starts_with("https://"));
    (top("Publisher"), top("ShortDescription"), homepage.unwrap_or_default(), top("License"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLERS: &str = "# Created using wingetcreate\nPackageIdentifier: WinMerge.WinMerge\nPackageVersion: 2.16.60\nInstallerType: inno\nFileExtensions:\n- winmerge\nInstallers:\n- Architecture: x86\n  Scope: machine\n  InstallerUrl: https://example.invalid/WinMerge-Setup.exe\n  InstallerSha256: 2FBC886BE8B28D8D542AAFE430E4440B449D2E4107BD13CDB5C9238D165A2329\n- Architecture: x64\n  Scope: user\n  InstallerUrl: https://example.invalid/WinMerge-x64-PerUser-Setup.exe\n  InstallerSha256: 35217E6B3BB981E37B7CE3858E686E4EE3AFE5CB7820FF5960258829FEA0BA03\n  Dependencies:\n    PackageDependencies:\n    - PackageIdentifier: Other.Thing\n      InstallerUrl: https://example.invalid/not-this.exe\n- Architecture: x64\n  Scope: machine\n  InstallerUrl: https://example.invalid/WinMerge-x64-Setup.exe\n  InstallerSha256: ED01097F60B63B3E7D58B3036064C40F05504A47C23C231A1081142295815F7F\n- Architecture: arm64\n  InstallerType: msix\n  InstallerUrl: https://example.invalid/WinMerge-ARM64.msix\n  InstallerSha256: 6BD632957D3A89FA8D7BC724D509E000A12BFA8141000A8925C0847BD6457FDE\nManifestType: installer\n";

    #[test]
    fn the_installer_for_this_kind_of_machine_is_chosen_with_its_checksum() {
        let installers = installers_in(INSTALLERS);
        assert_eq!(installers.len(), 4);
        assert_eq!((installers[0].arch.as_str(), installers[0].kind.as_str()), ("x86", "inno"), "the kind said once");
        assert_eq!(installers[3].kind, "msix", "unless an installer says its own");
        let chosen = for_proton(&installers).unwrap();
        assert_eq!(chosen.url, "https://example.invalid/WinMerge-x64-PerUser-Setup.exe", "64-bit, for one person");
        assert_eq!(chosen.sha256, "35217e6b3bb981e37b7ce3858e686e4ee3afe5cb7820ff5960258829fea0ba03");

        // Only Windows' own package formats, or nothing with a checksum: nothing to use.
        let store_only = "InstallerType: msix\nInstallers:\n- Architecture: x64\n  InstallerUrl: https://example.invalid/a.msix\n  InstallerSha256: 35217E6B3BB981E37B7CE3858E686E4EE3AFE5CB7820FF5960258829FEA0BA03\n";
        assert!(for_proton(&installers_in(store_only)).is_none());
        let unchecked =
            "InstallerType: exe\nInstallers:\n- Architecture: x64\n  InstallerUrl: https://example.invalid/a.exe\n";
        assert!(for_proton(&installers_in(unchecked)).is_none());
        let plain_http = "InstallerType: exe\nInstallers:\n- Architecture: x64\n  InstallerUrl: http://example.invalid/a.exe\n  InstallerSha256: 35217E6B3BB981E37B7CE3858E686E4EE3AFE5CB7820FF5960258829FEA0BA03\n";
        assert!(for_proton(&installers_in(plain_http)).is_none());
    }

    #[test]
    fn a_packages_files_are_found_by_its_id_and_version() {
        assert_eq!(
            installer_manifest_url("Notepad++.Notepad++", "8.9.9.1").unwrap(),
            "https://raw.githubusercontent.com/microsoft/winget-pkgs/master/manifests/n/Notepad%2B%2B/Notepad%2B%2B/8.9.9.1/Notepad%2B%2B.Notepad%2B%2B.installer.yaml"
        );
        assert!(description_url("WinMerge.WinMerge.Beta", "2.16.53.0")
            .unwrap()
            .ends_with("/manifests/w/WinMerge/WinMerge/Beta/2.16.53.0/WinMerge.WinMerge.Beta.locale.en-US.yaml"));
        assert!(installer_manifest_url("WinMerge.WinMerge", "../../x").is_none());
        let said = description_in("PackageIdentifier: A.B\nPublisher: \"Thingy Ltd\"\nPublisherUrl: https://thingy.example/\nLicense: GPL-2.0\nShortDescription: Compares files.\nTags:\n- diff\n");
        assert_eq!(
            said,
            ("Thingy Ltd".into(), "Compares files.".into(), "https://thingy.example/".into(), "GPL-2.0".into())
        );
    }

    /// A zip holding one stored file, and the smallest SQLite file with one table of
    /// two rows, both made by hand: the real index is too large to keep here.
    #[test]
    fn a_file_is_read_out_of_a_zip_and_rows_out_of_a_table() {
        let stored = b"hello";
        let mut zip = Vec::new();
        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&5u32.to_le_bytes());
        zip.extend_from_slice(&5u32.to_le_bytes());
        zip.extend_from_slice(&[5, 0, 0, 0]);
        zip.extend_from_slice(b"a.txt");
        zip.extend_from_slice(stored);
        let list = zip.len() as u32;
        zip.extend_from_slice(b"PK\x01\x02");
        zip.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&5u32.to_le_bytes());
        zip.extend_from_slice(&5u32.to_le_bytes());
        zip.extend_from_slice(&[5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(b"a.txt");
        let list_len = zip.len() as u32 - list;
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        zip.extend_from_slice(&list_len.to_le_bytes());
        zip.extend_from_slice(&list.to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        assert_eq!(file_in_zip(&zip, "a.txt", 100).as_deref(), Some(&stored[..]));
        assert!(file_in_zip(&zip, "b.txt", 100).is_none());
        assert!(file_in_zip(&zip, "a.txt", 3).is_none(), "larger than allowed");
        assert!(file_in_zip(b"not a zip", "a.txt", 100).is_none());

        assert!(table_rows(b"not a database", "packages").is_none());
        // A number kept in one byte keeps its sign; text is as long as its header says.
        let row = record(&[4, 1, 15, 0, 0xFE, b'x', 0, 0]).unwrap();
        assert_eq!(row[0], Value::Int(-2));
        assert_eq!(row[1], Value::Text("x".into()));
        assert_eq!(row[2], Value::Null);
    }

    /// Against the real index, when a copy is beside the tests (it is not kept in the
    /// repository: four megabytes that change daily).
    #[test]
    fn the_real_index_is_read_when_there_is_a_copy_to_read() {
        let Ok(path) = std::env::var("BILBY_WINGET_INDEX") else { return };
        let packages = packages_in(&std::fs::read(path).unwrap()).unwrap();
        assert!(packages.len() > 5000, "{}", packages.len());
        let winmerge = packages.iter().find(|p| p.id == "WinMerge.WinMerge").unwrap();
        assert_eq!((winmerge.name.as_str(), winmerge.moniker.as_str()), ("WinMerge", "winmerge"));
        assert!(!winmerge.version.is_empty());
    }
}
