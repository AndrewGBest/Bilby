//! Running Flatpak, reading its progress, and knowing which apps it has installed.

use super::dirs::Dirs;
use super::launch::run_watched;
use crate::error::Result;
use std::path::PathBuf;

/// Run Flatpak and wait for it, giving up (and stopping it) when `keep_going` says so.
/// How far it has got is read from what it prints and passed on in thousandths.
pub fn run_flatpak(dirs: &Dirs, args: &[&str], keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<()> {
    run_watched(&dirs.flatpak, args, "Flatpak", Some(flatpak_progress), keep_going)
}

/// How far a Flatpak install has got, in thousandths, from a line it prints as it
/// goes: "Installing 2/5… ████  47%  30.9 MB/s" is the second of five things (the
/// app and what it needs), 47% fetched.
pub fn flatpak_progress(line: &str) -> Option<u64> {
    let rest = ["Installing ", "Updating "].iter().find_map(|start| line.trim_start().strip_prefix(start))?;
    let count: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '/').collect();
    let (at, of) = count.split_once('/')?;
    let (at, of): (u64, u64) = (at.parse().ok()?, of.parse().ok()?);
    if at == 0 || of == 0 || at > of {
        return None;
    }
    let percent = rest
        .split('%')
        .next()
        .filter(|_| rest.contains('%'))
        .map(|before| before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>())
        .and_then(|digits| digits.into_iter().rev().collect::<String>().parse::<u64>().ok())
        .unwrap_or(0)
        .min(100);
    // Never quite full while it is still working: the last step is not fetching.
    Some((((at - 1) * 1000 + percent * 10) / of).min(990))
}

/// Is this a name Flatpak would give an app: `org.example.App`, nothing that could be
/// taken for an option or a path?
pub fn flatpak_app_id_ok(id: &str) -> bool {
    id.len() <= 255
        && id.split('.').count() >= 2
        && id.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// The app Flatpak said it was installing: the id out of `app/<id>/<arch>/<branch>`.
pub(super) fn flatpak_app_named(said: &str) -> Option<String> {
    said.split_whitespace()
        .filter_map(|word| word.strip_prefix("app/"))
        .filter_map(|rest| rest.split('/').next())
        .find(|id| flatpak_app_id_ok(id))
        .map(str::to_string)
}

/// The Flatpak apps on the device, by id.
pub(super) fn flatpak_app_ids(roots: &[PathBuf]) -> Vec<String> {
    let mut ids = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join("app")) else { continue };
        ids.extend(entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()));
    }
    ids
}

/// Is this Flatpak app on the device, in any of Flatpak's installations?
pub fn flatpak_app_installed(dirs: &Dirs, id: &str) -> bool {
    flatpak_app_id_ok(id) && dirs.flatpak_roots.iter().any(|root| root.join("app").join(id).is_dir())
}
