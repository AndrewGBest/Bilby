//! Taking an app off the device with the tool that put it there.

use super::dirs::Dirs;
use super::launch::run_program;
use crate::error::{Error, Result};
use crate::model::{AppId, SourceKind};
use crate::platform::Platform;
use std::path::PathBuf;

/// How the system's own tools take an app off the device, for what they look after:
/// a package, a snap, a winget package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// What to run. One that starts with `sudo` asks for the system's password.
    pub command: String,
    /// Run first, where the tool would otherwise take other things away with it:
    /// see [`removed_with`].
    pub rehearsal: Option<String>,
}

/// The removal of one package and nothing else. Each tool is asked in the way that
/// refuses when something else needs the package (pacman, rpm), or is rehearsed
/// first (apt), so that removing one app never quietly takes others with it.
pub fn removal(id: &AppId, p: &Platform) -> Option<Removal> {
    let name = id.key.as_str();
    let plain = !name.is_empty()
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "+-._:@".contains(c));
    if !plain {
        return None;
    }
    let image = p.linux.as_ref().is_some_and(|l| l.image);
    let (command, rehearsal) = match id.source {
        SourceKind::Pacman | SourceKind::Aur => (format!("sudo pacman -R --noconfirm {name}"), None),
        // On a system that is one image, only what was added on top can come off.
        SourceKind::Dnf if image => (format!("sudo rpm-ostree uninstall {name}"), None),
        SourceKind::Dnf | SourceKind::Zypper => (format!("sudo rpm -e {name}"), None),
        SourceKind::Apt => (format!("sudo apt-get remove -y {name}"), Some(format!("apt-get -s remove {name}"))),
        SourceKind::Snap => (format!("sudo snap remove {name}"), None),
        SourceKind::Winget => (format!("winget uninstall --id {name} --exact --silent --disable-interactivity"), None),
        _ => return None,
    };
    Some(Removal { command, rehearsal })
}

/// What else a rehearsed removal (`apt-get -s remove`) would take away besides the
/// package itself.
pub fn removed_with(rehearsal: &str, name: &str) -> Vec<String> {
    rehearsal
        .lines()
        .filter_map(|line| line.strip_prefix("Remv "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|package| package.split(':').next().unwrap_or(package).to_string())
        .filter(|package| package != name)
        .collect()
}

/// Run a [`Removal`]'s command, asking for the system's password where it needs it.
pub fn remove_with(removal: &Removal, dirs: &Dirs) -> Result<()> {
    let mut words = removal.command.split_whitespace();
    let (program, rest): (PathBuf, Vec<&str>) = match words.next() {
        Some("sudo") => (dirs.elevate.clone(), words.collect()),
        Some(program) => (PathBuf::from(program), words.collect()),
        None => return Err(Error::Unsupported("There is nothing to run.".to_string())),
    };
    if let Some(rehearsal) = &removal.rehearsal {
        let mut words = rehearsal.split_whitespace();
        let tool = words.next().unwrap_or_default();
        let said = std::process::Command::new(tool).args(words).stdin(std::process::Stdio::null()).output()?;
        let name = rest.last().copied().unwrap_or_default();
        let others = removed_with(&String::from_utf8_lossy(&said.stdout), name);
        if !said.status.success() || !others.is_empty() {
            let mut list = others.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
            if others.len() > 4 {
                list.push_str(", …");
            }
            return Err(Error::Unsupported(if others.is_empty() {
                format!("{tool} could not say what removing it would do")
            } else {
                format!("other things on this device need it ({list})")
            }));
        }
    }
    run_program(&program, &rest, rest.first().copied().unwrap_or("it"), &mut |_, _| true)
}
