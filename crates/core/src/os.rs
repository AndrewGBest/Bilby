//! The operating system itself: what it is, whether it has updates waiting, and
//! installing them.
//!
//! Everything goes through the system's own package manager, the way a person would do
//! it in a terminal. Checking never needs a password; installing asks for one through
//! the desktop's own prompt (`pkexec`). All access to the machine goes through [`Host`],
//! so the demo and the tests run the same code against a made-up machine and can never
//! change a real one.
//!
//! | System | Check | Install |
//! | --- | --- | --- |
//! | pacman | `checkupdates` if installed, else `pacman -Qu` | `pacman -Syu --noconfirm` |
//! | apt | `apt-get -s upgrade` | `apt-get update`, then `apt-get -y upgrade` |
//! | dnf | `dnf check-update` | `dnf -y upgrade --refresh` |
//! | Flatpak | `flatpak remote-ls --updates` | `flatpak update -y` |
//! | Windows, macOS | not checked | opens the system's own update page |

use crate::error::{Error, Result};
use crate::model::human_size;
use crate::platform::{Os, Platform};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// What a command printed and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
}

/// The machine, as far as this module needs it.
pub trait Host: Send + Sync {
    fn read(&self, path: &str) -> Option<String>;
    fn exists(&self, path: &str) -> bool;
    /// When the file last changed, in seconds since 1970.
    fn modified(&self, path: &str) -> Option<u64>;
    fn env(&self, name: &str) -> Option<String>;
    /// Is this program installed?
    fn has(&self, program: &str) -> bool;
    /// Run a program to the end and collect what it printed.
    fn run(&self, program: &str, args: &[&str]) -> Result<Output>;
    /// Run a program, passing on each line as it is printed. Returns its exit code.
    fn stream(&self, program: &str, args: &[&str], line: &mut dyn FnMut(&str)) -> Result<i32>;
}

/// The computer the store is running on.
pub struct RealHost;

impl Host for RealHost {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read(path).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn modified(&self, path: &str) -> Option<u64> {
        let changed = std::fs::metadata(path).ok()?.modified().ok()?;
        Some(changed.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs())
    }

    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    fn has(&self, program: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else { return false };
        std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        // Tools are asked to answer in English so their output can be read.
        let out = Command::new(program).args(args).env("LC_ALL", "C").stdin(Stdio::null()).output()?;
        Ok(Output { code: out.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&out.stdout).into_owned() })
    }

    fn stream(&self, program: &str, args: &[&str], line: &mut dyn FnMut(&str)) -> Result<i32> {
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Errors are read on their own thread so a chatty tool cannot stall on a full pipe.
        let errors = child.stderr.take().map(|pipe| {
            std::thread::spawn(move || BufReader::new(pipe).lines().map_while(|l| l.ok()).collect::<Vec<String>>())
        });
        if let Some(pipe) = child.stdout.take() {
            for text in BufReader::new(pipe).lines().map_while(|l| l.ok()) {
                line(&text);
            }
        }
        for text in errors.and_then(|thread| thread.join().ok()).unwrap_or_default() {
            line(&text);
        }
        Ok(child.wait()?.code().unwrap_or(-1))
    }
}

/// A made-up machine: files and command results given up front. Used by the demo and by
/// tests. After any command passed to [`Host::stream`] has run, the commands named in
/// `after_update` answer with their second result, as a machine would after updating.
#[derive(Default)]
pub struct FakeHost {
    pub files: HashMap<String, String>,
    pub modified: HashMap<String, u64>,
    pub env: HashMap<String, String>,
    pub programs: Vec<String>,
    /// Results by the whole command line, e.g. `"pacman -Qu"`.
    pub commands: HashMap<String, Output>,
    pub after_update: HashMap<String, Output>,
    /// What an install prints, and the code it ends with.
    pub install_lines: Vec<String>,
    pub install_code: i32,
    /// Every command line that was streamed, in order.
    pub streamed: Mutex<Vec<String>>,
}

impl FakeHost {
    pub fn file(mut self, path: &str, text: &str) -> Self {
        self.files.insert(path.to_string(), text.to_string());
        self
    }

    pub fn program(mut self, name: &str) -> Self {
        self.programs.push(name.to_string());
        self
    }

    pub fn command(mut self, line: &str, code: i32, stdout: &str) -> Self {
        self.commands.insert(line.to_string(), Output { code, stdout: stdout.to_string() });
        self
    }

    /// What `line` answers once an update has been installed.
    pub fn then(mut self, line: &str, code: i32, stdout: &str) -> Self {
        self.after_update.insert(line.to_string(), Output { code, stdout: stdout.to_string() });
        self
    }

    pub fn var(mut self, name: &str, value: &str) -> Self {
        self.env.insert(name.to_string(), value.to_string());
        self
    }
}

fn command_line(program: &str, args: &[&str]) -> String {
    std::iter::once(program).chain(args.iter().copied()).collect::<Vec<_>>().join(" ")
}

impl Host for FakeHost {
    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    fn modified(&self, path: &str) -> Option<u64> {
        self.modified.get(path).copied()
    }

    fn env(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }

    fn has(&self, program: &str) -> bool {
        self.programs.iter().any(|p| p == program)
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        let line = command_line(program, args);
        if !self.streamed.lock().unwrap().is_empty() {
            if let Some(out) = self.after_update.get(&line) {
                return Ok(out.clone());
            }
        }
        self.commands.get(&line).cloned().ok_or_else(|| Error::NotFound(format!("{line}: no such command here")))
    }

    fn stream(&self, program: &str, args: &[&str], line: &mut dyn FnMut(&str)) -> Result<i32> {
        self.streamed.lock().unwrap().push(command_line(program, args));
        for text in &self.install_lines {
            line(text);
        }
        Ok(self.install_code)
    }
}

// ---- what the system is ----------------------------------------------------------

/// One line of the information table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub name: &'static str,
    pub value: String,
}

/// The system's own page for updates, where the store cannot install them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub label: &'static str,
    /// Handed to the system to open.
    pub target: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// "Ubuntu 24.04.1 LTS"
    pub name: String,
    /// The line under the name: "Linux 6.8.0 · x86_64".
    pub tagline: String,
    pub facts: Vec<Fact>,
    /// The package managers whose updates the store checks and installs here.
    pub managers: Vec<Manager>,
    pub handoff: Option<Handoff>,
    /// Why updates cannot be handled here, when they cannot.
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Manager {
    Pacman,
    Apt,
    Dnf,
    Flatpak,
}

impl Manager {
    pub fn label(self) -> &'static str {
        match self {
            Manager::Pacman => "pacman",
            Manager::Apt => "apt",
            Manager::Dnf => "dnf",
            Manager::Flatpak => "Flatpak",
        }
    }
}

/// A value from an `os-release` file, without its quotes.
fn os_release_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix('=')?;
        let value = value.trim().trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn first_line(text: Option<String>) -> Option<String> {
    text.and_then(|t| t.lines().next().map(|l| l.trim().to_string())).filter(|l| !l.is_empty())
}

/// "8 × Example Processor 5600" from `/proc/cpuinfo`.
fn processor(cpuinfo: &str) -> Option<String> {
    let value = |line: &str| line.split_once(':').map(|(_, v)| v.split_whitespace().collect::<Vec<_>>().join(" "));
    let threads = cpuinfo.lines().filter(|l| l.starts_with("processor")).count();
    let model = cpuinfo
        .lines()
        .find(|l| l.starts_with("model name") || l.starts_with("Model") || l.starts_with("Hardware"))
        .and_then(value)?;
    Some(if threads > 1 { format!("{threads} × {model}") } else { model })
}

/// "16 GB" from `/proc/meminfo`, rounded as it is sold.
fn memory(meminfo: &str) -> Option<String> {
    let kb: u64 = meminfo.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.split_whitespace().next()?.parse().ok()?;
    let gb = kb as f64 / 1_048_576.0;
    Some(if gb >= 1.0 { format!("{} GB", gb.ceil() as u64) } else { format!("{} MB", kb / 1024) })
}

/// "212.4 GB free of 465.6 GB" from `df -Pk /`.
fn storage(df: &str) -> Option<String> {
    let fields: Vec<&str> = df.lines().nth(1)?.split_whitespace().collect();
    let (total, free): (u64, u64) = (fields.get(1)?.parse().ok()?, fields.get(3)?.parse().ok()?);
    Some(format!("{} free of {}", human_size(free * 1024), human_size(total * 1024)))
}

fn desktop(host: &dyn Host) -> Option<String> {
    let name = host.env("XDG_CURRENT_DESKTOP")?.replace(':', ", ");
    Some(match host.env("XDG_SESSION_TYPE").as_deref() {
        Some("wayland") => format!("{name} (Wayland)"),
        Some("x11") => format!("{name} (X11)"),
        _ => name,
    })
}

/// Describe the system. Nothing here touches the network.
pub fn describe(host: &dyn Host, platform: &Platform) -> Report {
    let mut report = Report { name: platform.os_name.clone(), ..Report::default() };
    let mut fact = |name: &'static str, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            report.facts.push(Fact { name, value });
        }
    };
    let arch = platform.arch.label().to_string();
    let mut kernel = None;
    match platform.os {
        Os::Linux => {
            let release = host.read("/etc/os-release").or_else(|| host.read("/usr/lib/os-release")).unwrap_or_default();
            let pretty = os_release_value(&release, "PRETTY_NAME");
            // A rolling distribution has no version: it is always the newest there is.
            let rolling = os_release_value(&release, "BUILD_ID").as_deref() == Some("rolling");
            let version = os_release_value(&release, "VERSION")
                .or_else(|| os_release_value(&release, "VERSION_ID"))
                .or_else(|| rolling.then(|| "Rolling release: always the newest".to_string()));
            kernel = first_line(host.read("/proc/sys/kernel/osrelease"));
            fact("Operating system", pretty.clone().or_else(|| Some(platform.os_name.clone())));
            fact("Version", version);
            fact("Supported until", os_release_value(&release, "SUPPORT_END"));
            fact("Linux kernel", kernel.clone());
            fact("Processor type", Some(arch.clone()));
            fact("Desktop", desktop(host));
            fact("Processor", host.read("/proc/cpuinfo").as_deref().and_then(processor));
            fact("Memory", host.read("/proc/meminfo").as_deref().and_then(memory));
            fact("Storage", host.run("df", &["-Pk", "/"]).ok().and_then(|out| storage(&out.stdout)));
            fact("Device name", first_line(host.read("/proc/sys/kernel/hostname")));
            if let Some(pretty) = pretty {
                report.name = pretty;
            }
        }
        Os::Windows => {
            // "Microsoft Windows [Version 10.0.22631.4460]"
            let version = host.run("cmd", &["/C", "ver"]).ok().and_then(|out| {
                let text = out.stdout.trim().to_string();
                text.split_once("[Version ").map(|(_, v)| v.trim_end_matches(']').to_string())
            });
            fact("Operating system", Some(platform.os_name.clone()));
            fact("Version", version);
            fact("Processor type", Some(arch.clone()));
            fact("Processor", host.env("PROCESSOR_IDENTIFIER"));
            fact("Device name", host.env("COMPUTERNAME"));
        }
        Os::MacOs => {
            let ask = |program: &str, args: &[&str]| first_line(host.run(program, args).ok().map(|out| out.stdout));
            let version = ask("sw_vers", &["-productVersion"]);
            if let Some(version) = &version {
                report.name = format!("macOS {version}");
            }
            let bytes = ask("sysctl", &["-n", "hw.memsize"]).and_then(|t| t.parse::<u64>().ok());
            fact("Operating system", Some("macOS".to_string()));
            fact("Version", version);
            fact("Build", ask("sw_vers", &["-buildVersion"]));
            fact("Processor type", Some(arch.clone()));
            fact("Processor", ask("sysctl", &["-n", "machdep.cpu.brand_string"]));
            fact("Memory", bytes.map(|b| format!("{} GB", b / 1_073_741_824)));
        }
        Os::Android | Os::Ios => {
            fact("Operating system", Some(platform.os_name.clone()));
            fact("Processor type", Some(arch.clone()));
        }
    }
    report.tagline = match kernel {
        Some(kernel) => format!("Linux {kernel}  ·  {arch}"),
        None => arch,
    };

    match platform.os {
        Os::Linux => {
            let linux = platform.linux.as_ref();
            // An image-based system (Fedora Atomic, SteamOS) replaces itself as a whole;
            // its package manager must not be used to update it.
            let image = host.exists("/run/ostree-booted") || host.has("steamos-update");
            if !image {
                if linux.is_some_and(|l| l.pacman) && host.has("pacman") {
                    report.managers.push(Manager::Pacman);
                }
                if linux.is_some_and(|l| l.dpkg) && host.has("apt-get") {
                    report.managers.push(Manager::Apt);
                }
                if linux.is_some_and(|l| l.rpm) && host.has("dnf") {
                    report.managers.push(Manager::Dnf);
                }
            }
            if host.has("flatpak") {
                report.managers.push(Manager::Flatpak);
            }
            if image {
                report.note = "This system updates as one whole image. Use its own updater for the system itself; \
                               apps from Flathub are updated here."
                    .to_string();
            } else if report.managers.iter().all(|m| *m == Manager::Flatpak) {
                report.note =
                    "Bilby does not know how to update this kind of Linux yet. Use its own updater.".to_string();
            }
        }
        Os::Windows => {
            report.handoff = Some(Handoff { label: "Open Windows Update", target: "ms-settings:windowsupdate" });
            report.note = "Windows updates itself. Windows Update shows what is waiting.".to_string();
        }
        Os::MacOs => {
            report.handoff = Some(Handoff {
                label: "Open Software Update",
                target: "x-apple.systempreferences:com.apple.preferences.softwareupdate",
            });
            report.note = "macOS updates itself. Software Update shows what is waiting.".to_string();
        }
        Os::Android | Os::Ios => {
            report.note = "System updates are in this device's own Settings.".to_string();
        }
    }
    let names: Vec<&str> = report.managers.iter().map(|m| m.label()).collect();
    if !names.is_empty() {
        report.facts.push(Fact { name: "Updates through", value: names.join(", ") });
    }
    report
}

// ---- updates -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub name: String,
    /// Installed version; empty when the tool does not say.
    pub from: String,
    pub to: String,
    pub manager: Manager,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Check {
    pub pending: Vec<Pending>,
    /// Something already installed only takes effect after a restart.
    pub restart_needed: bool,
    /// Managers that could not be asked, in words.
    pub problems: Vec<String>,
    /// When the list of available packages was last fetched, where checking relies on
    /// it (apt, and pacman without `checkupdates`).
    pub lists_fetched: Option<u64>,
}

/// `name old -> new`, as `checkupdates` and `pacman -Qu` print.
fn parse_pacman(text: &str) -> Vec<Pending> {
    let parse = |line: &str| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            // Packages the user told pacman to hold back are not coming.
            [.., "[ignored]"] => None,
            [name, from, "->", to, ..] => Some(Pending {
                name: name.to_string(),
                from: from.to_string(),
                to: to.to_string(),
                manager: Manager::Pacman,
            }),
            _ => None,
        }
    };
    text.lines().filter_map(parse).collect()
}

/// `Inst name [old] (new suite [arch])`, as `apt-get -s upgrade` prints.
fn parse_apt(text: &str) -> Vec<Pending> {
    let parse = |line: &str| {
        let rest = line.strip_prefix("Inst ")?;
        let name = rest.split_whitespace().next()?;
        let from = rest.split_once('[').and_then(|(_, r)| r.split_once(']')).map(|(v, _)| v).unwrap_or("");
        let to = rest.split_once('(')?.1.split_whitespace().next()?;
        // A brand-new dependency has no old version in brackets before the parenthesis.
        let from = if rest.find('[') < rest.find('(') { from } else { "" };
        Some(Pending { name: name.to_string(), from: from.to_string(), to: to.to_string(), manager: Manager::Apt })
    };
    text.lines().filter_map(parse).collect()
}

/// `name.arch  version  repository`, as `dnf check-update` prints.
fn parse_dnf(text: &str) -> Vec<Pending> {
    let mut pending = Vec::new();
    for line in text.lines() {
        // What follows this heading is a list of replacements, not of updates.
        if line.starts_with("Obsoleting") {
            break;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if let [name, version, _repository] = fields.as_slice() {
            if let Some((name, _arch)) = name.rsplit_once('.') {
                let starts_with_digit = version.chars().next().is_some_and(|c| c.is_ascii_digit());
                if starts_with_digit {
                    pending.push(Pending {
                        name: name.to_string(),
                        from: String::new(),
                        to: version.to_string(),
                        manager: Manager::Dnf,
                    });
                }
            }
        }
    }
    pending
}

/// `application<TAB>version`, as `flatpak remote-ls --updates --columns=application,version` prints.
fn parse_flatpak(text: &str) -> Vec<Pending> {
    let parse = |line: &str| {
        let mut fields = line.split('\t');
        let name = fields.next()?.trim();
        (name.contains('.') && !name.contains(' ')).then(|| Pending {
            name: name.to_string(),
            from: String::new(),
            to: fields.next().unwrap_or("").trim().to_string(),
            manager: Manager::Flatpak,
        })
    };
    let mut pending: Vec<Pending> = text.lines().filter_map(parse).collect();
    // The same app can be listed once for the system and once for the user.
    pending.dedup_by(|a, b| a.name == b.name);
    pending
}

const FLATPAK_LIST: &[&str] = &["remote-ls", "--updates", "--columns=application,version"];

fn pending_for(host: &dyn Host, manager: Manager) -> Result<Vec<Pending>> {
    let failed = |tool: &str, out: &Output| {
        Error::Unsupported(format!("{tool} could not check for updates (it ended with code {}).", out.code))
    };
    match manager {
        Manager::Pacman if host.has("checkupdates") => {
            // 0: updates listed. 2: none. Anything else: it could not check.
            let out = host.run("checkupdates", &[])?;
            match out.code {
                0 | 2 => Ok(parse_pacman(&out.stdout)),
                _ => Err(failed("checkupdates", &out)),
            }
        }
        // Prints what is newer in the package lists already on disk; ends with 1 when
        // nothing is.
        Manager::Pacman => Ok(parse_pacman(&host.run("pacman", &["-Qu"])?.stdout)),
        Manager::Apt => {
            let out = host.run("apt-get", &["-s", "-o", "Debug::NoLocking=true", "upgrade"])?;
            if out.code != 0 {
                return Err(failed("apt", &out));
            }
            Ok(parse_apt(&out.stdout))
        }
        Manager::Dnf => {
            // 100: updates listed. 0: none.
            let out = host.run("dnf", &["-q", "check-update"])?;
            match out.code {
                0 | 100 => Ok(parse_dnf(&out.stdout)),
                _ => Err(failed("dnf", &out)),
            }
        }
        Manager::Flatpak => {
            let out = host.run("flatpak", FLATPAK_LIST)?;
            if out.code != 0 {
                return Err(failed("Flatpak", &out));
            }
            Ok(parse_flatpak(&out.stdout))
        }
    }
}

/// Is the running kernel one that has since been replaced or removed?
fn restart_needed(host: &dyn Host) -> bool {
    // Debian and Ubuntu leave a note.
    if host.exists("/var/run/reboot-required") || host.exists("/run/reboot-required") {
        return true;
    }
    // Elsewhere: the running kernel's own files are gone once a newer one replaced it.
    match first_line(host.read("/proc/sys/kernel/osrelease")) {
        Some(kernel) => host.exists("/usr/lib/modules") && !host.exists(&format!("/usr/lib/modules/{kernel}")),
        None => false,
    }
}

/// Look for updates. Never asks for a password and never changes the system.
pub fn check(host: &dyn Host, report: &Report) -> Check {
    let mut check = Check::default();
    for manager in &report.managers {
        match pending_for(host, *manager) {
            Ok(found) => check.pending.extend(found),
            Err(e) => check.problems.push(e.to_string()),
        }
    }
    check.restart_needed = restart_needed(host);
    check.lists_fetched = if report.managers.contains(&Manager::Apt) {
        host.modified("/var/lib/apt/periodic/update-success-stamp").or_else(|| host.modified("/var/lib/apt/lists"))
    } else if report.managers.contains(&Manager::Pacman) && !host.has("checkupdates") {
        host.modified("/var/lib/pacman/sync")
    } else {
        None
    };
    check
}

/// The commands that install everything waiting, in order. Those that change the system
/// run through `pkexec`, which shows the desktop's own password prompt.
pub fn update_commands(report: &Report) -> Vec<(Manager, &'static str, Vec<&'static str>)> {
    // One command, so one password prompt. The text is fixed; nothing is spliced into it.
    const APT: &str = "apt-get update && DEBIAN_FRONTEND=noninteractive apt-get -y \
                       -o Dpkg::Options::=--force-confold --with-new-pkgs upgrade";
    let command = |manager: &Manager| match manager {
        Manager::Pacman => (*manager, "pkexec", vec!["pacman", "-Syu", "--noconfirm"]),
        Manager::Apt => (*manager, "pkexec", vec!["sh", "-c", APT]),
        Manager::Dnf => (*manager, "pkexec", vec!["dnf", "-y", "upgrade", "--refresh"]),
        Manager::Flatpak => (*manager, "flatpak", vec!["update", "-y", "--noninteractive"]),
    };
    report.managers.iter().map(command).collect()
}

/// Install every waiting update. `line` receives what the tools print. Stops at the
/// first tool that fails.
pub fn update(host: &dyn Host, report: &Report, line: &mut dyn FnMut(&str)) -> Result<()> {
    if report.managers.is_empty() {
        return Err(Error::Unsupported("There is nothing here that Bilby can update.".to_string()));
    }
    for (manager, program, args) in update_commands(report) {
        if program == "pkexec" && !host.has("pkexec") {
            return Err(Error::Unsupported(
                "Updating the system needs a way to ask for your password (pkexec, from polkit), which is not installed."
                    .to_string(),
            ));
        }
        line(&format!("Updating through {}…", manager.label()));
        match host.stream(program, &args, line)? {
            0 => {}
            // pkexec's own codes: the prompt was closed, or permission was refused.
            126 | 127 if program == "pkexec" => {
                return Err(Error::Unsupported(
                    "Nothing was changed: the password prompt was closed or refused.".to_string(),
                ))
            }
            code => {
                return Err(Error::Unsupported(format!(
                    "{} stopped with an error (code {code}). The details show what it said.",
                    manager.label()
                )))
            }
        }
    }
    Ok(())
}

/// Fetch fresh package lists where a check relies on the ones on disk (apt). Asks for a
/// password. Other systems fetch what they need while checking, or must not refresh
/// without updating (pacman).
pub fn refresh_lists(host: &dyn Host, report: &Report, line: &mut dyn FnMut(&str)) -> Result<()> {
    if !report.managers.contains(&Manager::Apt) {
        return Ok(());
    }
    match host.stream("pkexec", &["apt-get", "update"], line)? {
        0 => Ok(()),
        126 | 127 => Err(Error::Unsupported(
            "The password prompt was closed, so the list of updates was not refreshed.".to_string(),
        )),
        code => Err(Error::Unsupported(format!("apt could not refresh its lists (code {code})."))),
    }
}

/// A made-up machine for the demo and for previews of other devices: it has the shape of
/// `platform` and a few updates waiting, which "installing" clears.
pub fn demo_host(platform: &Platform) -> FakeHost {
    use crate::platform::LinuxFamily;
    let mut host = FakeHost {
        install_lines: vec![
            "Fetching the newest packages…".to_string(),
            "Installing 5 updates…".to_string(),
            "Done. (Nothing was changed: this is the demo.)".to_string(),
        ],
        ..FakeHost::default()
    };
    match platform.os {
        Os::Linux => {}
        Os::Windows => {
            return host
                .command("cmd /C ver", 0, "\nMicrosoft Windows [Version 10.0.26100.2314]\n")
                .var("PROCESSOR_IDENTIFIER", "Example Family 25 Model 80")
                .var("COMPUTERNAME", "DEMO-PC")
        }
        Os::MacOs => {
            return host
                .command("sw_vers -productVersion", 0, "15.1\n")
                .command("sw_vers -buildVersion", 0, "24B83\n")
                .command("sysctl -n machdep.cpu.brand_string", 0, "Example M-series\n")
                .command("sysctl -n hw.memsize", 0, "17179869184\n")
        }
        Os::Android | Os::Ios => return host,
    }
    let family = platform.linux.as_ref().map(|l| l.family);
    let (id, pretty, version) = match family {
        Some(LinuxFamily::Arch) => ("arch", platform.os_name.clone(), "BUILD_ID=rolling\n".to_string()),
        Some(LinuxFamily::Fedora) => (
            "fedora",
            platform.os_name.clone(),
            "VERSION=\"41 (Workstation Edition)\"\nSUPPORT_END=2025-12-15\n".to_string(),
        ),
        _ if platform.os_name.starts_with("Ubuntu") => {
            ("ubuntu", format!("{} LTS", platform.os_name), "VERSION=\"24.04.1 LTS (Noble Numbat)\"\n".to_string())
        }
        _ => ("debian", platform.os_name.clone(), "VERSION=\"12 (bookworm)\"\n".to_string()),
    };
    host = host
        .file("/etc/os-release", &format!("ID={id}\nPRETTY_NAME=\"{pretty}\"\n{version}"))
        .file("/proc/sys/kernel/osrelease", "6.11.5-demo\n")
        .file("/proc/sys/kernel/hostname", "demo-computer\n")
        .file("/proc/cpuinfo", &"processor\t: 0\nmodel name\t: Example Processor 5600\n\n".repeat(8))
        .file("/proc/meminfo", "MemTotal:       16303428 kB\n")
        .file("/usr/lib/modules", "")
        .file("/usr/lib/modules/6.11.5-demo", "")
        .command("df -Pk /", 0, "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/demo 488245288 265400000 222845288 55% /\n")
        .var("XDG_CURRENT_DESKTOP", "KDE")
        .var("XDG_SESSION_TYPE", "wayland")
        .program("pkexec");
    host = match family {
        Some(LinuxFamily::Arch) => host
            .program("pacman")
            .program("checkupdates")
            .command(
                "checkupdates",
                0,
                "linux 6.11.5.arch1-1 -> 6.11.6.arch1-1\nmesa 1:24.2.5-1 -> 1:24.2.6-1\nfirefox 131.0.3-1 -> 132.0-1\n\
                 systemd 256.7-1 -> 256.8-1\nopenssl 3.3.2-1 -> 3.4.0-1\n",
            )
            .then("checkupdates", 2, ""),
        Some(LinuxFamily::Fedora) => host
            .program("dnf")
            .command(
                "dnf -q check-update",
                100,
                "\nkernel.x86_64  6.11.6-300.fc41  updates\nmesa-dri-drivers.x86_64  24.2.6-1.fc41  updates\n\
                 firefox.x86_64  132.0-2.fc41  updates\nsystemd.x86_64  256.8-1.fc41  updates\nopenssl-libs.x86_64  1:3.2.2-9.fc41  updates\n",
            )
            .then("dnf -q check-update", 0, ""),
        Some(LinuxFamily::Debian) => host
            .program("apt-get")
            .command(
                "apt-get -s -o Debug::NoLocking=true upgrade",
                0,
                "Reading package lists...\nInst linux-image-generic [6.8.0-47.47] (6.8.0-48.48 Ubuntu:24.04/noble-updates [amd64])\n\
                 Inst libgl1-mesa-dri [24.0.9-0ubuntu0.1] (24.0.9-0ubuntu0.2 Ubuntu:24.04/noble-updates [amd64])\n\
                 Inst firefox [131.0.3] (132.0 Ubuntu:24.04/noble-updates [amd64])\n\
                 Inst systemd [255.4-1ubuntu8.4] (255.4-1ubuntu8.5 Ubuntu:24.04/noble-updates [amd64])\n\
                 Inst openssl [3.0.13-0ubuntu3.4] (3.0.13-0ubuntu3.5 Ubuntu:24.04/noble-security [amd64])\n\
                 Conf openssl (3.0.13-0ubuntu3.5 Ubuntu:24.04/noble-security [amd64])\n",
            )
            .then("apt-get -s -o Debug::NoLocking=true upgrade", 0, "Reading package lists...\n"),
        _ => host,
    };
    if platform.linux.as_ref().is_some_and(|l| l.flatpak) {
        let line = command_line("flatpak", FLATPAK_LIST);
        host = host.program("flatpak").command(&line, 0, "io.quillpad.Quillpad\t4.2.1\n").then(&line, 0, "");
    }
    host
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform(name: &str) -> Platform {
        Platform::preset(name).unwrap()
    }

    #[test]
    fn each_tool_s_list_of_updates_is_read() {
        let pacman = parse_pacman("linux 6.11.5-1 -> 6.11.6-1\nheld 1-1 -> 2-1 [ignored]\n:: noise\n");
        assert_eq!(pacman.len(), 1, "a package the user holds back is not coming");
        assert_eq!(
            (pacman[0].name.as_str(), pacman[0].from.as_str(), pacman[0].to.as_str()),
            ("linux", "6.11.5-1", "6.11.6-1")
        );

        let apt = parse_apt(
            "Reading package lists...\nInst openssl [3.0.13-3.4] (3.0.13-3.5 Ubuntu:24.04/noble-security [amd64])\n\
             Inst libnew1 (1.2-1 Ubuntu:24.04/noble [amd64])\nConf openssl (3.0.13-3.5 Ubuntu:24.04/noble-security [amd64])\n",
        );
        assert_eq!(apt.len(), 2, "only the Inst lines are updates");
        assert_eq!((apt[0].from.as_str(), apt[0].to.as_str()), ("3.0.13-3.4", "3.0.13-3.5"));
        assert_eq!((apt[1].name.as_str(), apt[1].from.as_str(), apt[1].to.as_str()), ("libnew1", "", "1.2-1"));

        let dnf = parse_dnf(
            "\nkernel.x86_64  6.11.6-300.fc41  updates\nLast metadata expiration check\n\
             Obsoleting Packages\nold.noarch  1.0-1  updates\n",
        );
        assert_eq!(dnf.len(), 1);
        assert_eq!((dnf[0].name.as_str(), dnf[0].to.as_str()), ("kernel", "6.11.6-300.fc41"));

        let flatpak =
            parse_flatpak("org.example.App\t2.0\norg.example.App\t2.0\nApplication ID\tVersion\norg.other.Thing\t\n");
        assert_eq!(flatpak.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["org.example.App", "org.other.Thing"]);
    }

    #[test]
    fn a_linux_computer_is_described_from_its_own_files() {
        let host = demo_host(&platform("linux-x64-debian"));
        let report = describe(&host, &platform("linux-x64-debian"));
        assert_eq!(report.name, "Ubuntu 24.04 LTS");
        assert_eq!(report.tagline, "Linux 6.11.5-demo  ·  x86_64");
        let fact = |name: &str| report.facts.iter().find(|f| f.name == name).map(|f| f.value.as_str());
        assert_eq!(fact("Version"), Some("24.04.1 LTS (Noble Numbat)"));
        assert_eq!(fact("Processor"), Some("8 × Example Processor 5600"));
        assert_eq!(fact("Memory"), Some("16 GB"));
        assert_eq!(fact("Storage"), Some("213 GB free of 466 GB"));
        assert_eq!(fact("Desktop"), Some("KDE (Wayland)"));
        assert_eq!(fact("Updates through"), Some("apt, Flatpak"));
        assert_eq!(report.managers, [Manager::Apt, Manager::Flatpak]);
        assert_eq!(report.handoff, None);

        // A rolling distribution says so instead of showing an empty version.
        let arch = describe(&demo_host(&platform("linux-x64-arch")), &platform("linux-x64-arch"));
        assert!(arch.facts.iter().any(|f| f.name == "Version" && f.value.starts_with("Rolling release")));
        let fedora = describe(&demo_host(&platform("linux-x64-fedora")), &platform("linux-x64-fedora"));
        assert!(fedora.facts.iter().any(|f| f.name == "Supported until" && f.value == "2025-12-15"));
    }

    #[test]
    fn updates_are_found_installed_and_then_gone() {
        for (preset, command) in [
            ("linux-x64-arch", "pkexec pacman -Syu --noconfirm"),
            ("linux-x64-fedora", "pkexec dnf -y upgrade --refresh"),
        ] {
            let host = demo_host(&platform(preset));
            let report = describe(&host, &platform(preset));
            let before = check(&host, &report);
            assert_eq!(before.pending.len(), 6, "{preset}: five packages and one Flatpak app");
            assert!(before.problems.is_empty() && !before.restart_needed);

            let mut said = Vec::new();
            update(&host, &report, &mut |line| said.push(line.to_string())).unwrap();
            assert_eq!(*host.streamed.lock().unwrap(), [command, "flatpak update -y --noninteractive"]);
            assert!(said.iter().any(|l| l.starts_with("Updating through")));
            assert_eq!(check(&host, &report).pending, [], "{preset}: nothing is waiting afterwards");
        }
    }

    #[test]
    fn apt_refreshes_and_upgrades_behind_one_password_prompt() {
        let host = demo_host(&platform("linux-x64-debian"));
        let report = describe(&host, &platform("linux-x64-debian"));
        let commands = update_commands(&report);
        assert_eq!((commands[0].1, commands[0].2[0], commands[0].2[1]), ("pkexec", "sh", "-c"));
        assert!(commands[0].2[2].starts_with("apt-get update && ") && commands[0].2[2].contains("--force-confold"));
        refresh_lists(&host, &report, &mut |_| {}).unwrap();
        assert_eq!(*host.streamed.lock().unwrap(), ["pkexec apt-get update"]);
    }

    #[test]
    fn a_closed_password_prompt_and_a_failing_tool_are_told_apart() {
        let mut host = demo_host(&platform("linux-x64-arch"));
        let report = describe(&host, &platform("linux-x64-arch"));
        host.install_code = 126;
        let refused = update(&host, &report, &mut |_| {}).unwrap_err().to_string();
        assert!(refused.contains("Nothing was changed"), "{refused}");
        assert_eq!(host.streamed.lock().unwrap().len(), 1, "the next tool is not tried");
        host.install_code = 1;
        assert!(update(&host, &report, &mut |_| {}).unwrap_err().to_string().contains("pacman stopped with an error"));

        // Without a way to ask for a password nothing is attempted.
        host.programs.retain(|p| p != "pkexec");
        host.streamed.lock().unwrap().clear();
        assert!(update(&host, &report, &mut |_| {}).unwrap_err().to_string().contains("pkexec"));
        assert!(host.streamed.lock().unwrap().is_empty());
    }

    #[test]
    fn a_tool_that_cannot_check_is_reported_and_the_others_still_are() {
        let host = demo_host(&platform("linux-x64-arch")).command("checkupdates", 1, "");
        let report = describe(&host, &platform("linux-x64-arch"));
        let found = check(&host, &report);
        assert_eq!(found.pending.len(), 1, "Flatpak still answered");
        assert_eq!(found.problems.len(), 1);

        // Without checkupdates, pacman's own lists are used and their age is given.
        let mut host = demo_host(&platform("linux-x64-arch")).command("pacman -Qu", 1, "");
        host.programs.retain(|p| p != "checkupdates");
        host.modified.insert("/var/lib/pacman/sync".to_string(), 1_700_000_000);
        let found = check(&host, &report);
        assert_eq!((found.pending.len(), found.lists_fetched), (1, Some(1_700_000_000)));
    }

    #[test]
    fn a_replaced_kernel_or_the_system_s_own_note_means_a_restart() {
        let host = demo_host(&platform("linux-x64-arch"));
        let report = describe(&host, &platform("linux-x64-arch"));
        assert!(!check(&host, &report).restart_needed);
        let mut replaced = demo_host(&platform("linux-x64-arch"));
        replaced.files.remove("/usr/lib/modules/6.11.5-demo");
        assert!(check(&replaced, &report).restart_needed, "the running kernel's files are gone");
        let noted = demo_host(&platform("linux-x64-debian")).file("/var/run/reboot-required", "");
        assert!(check(&noted, &report).restart_needed);
    }

    #[test]
    fn image_based_systems_and_other_devices_are_left_to_their_own_updaters() {
        let host = demo_host(&platform("linux-x64-fedora")).file("/run/ostree-booted", "");
        let report = describe(&host, &platform("linux-x64-fedora"));
        assert_eq!(report.managers, [Manager::Flatpak], "dnf must not be used on an image-based system");
        assert!(report.note.contains("whole image"));

        let windows = describe(&demo_host(&platform("windows-x64")), &platform("windows-x64"));
        assert_eq!(windows.handoff.as_ref().map(|h| h.label), Some("Open Windows Update"));
        assert!(windows.facts.iter().any(|f| f.name == "Version" && f.value == "10.0.26100.2314"));
        assert!(windows.managers.is_empty());
        assert!(update(&demo_host(&platform("windows-x64")), &windows, &mut |_| {}).is_err());

        let mac = describe(&demo_host(&platform("macos-arm64")), &platform("macos-arm64"));
        assert_eq!(mac.name, "macOS 15.1");
        assert!(mac.facts.iter().any(|f| f.name == "Memory" && f.value == "16 GB"));
        let phone = describe(&demo_host(&platform("android-arm64")), &platform("android-arm64"));
        assert!(phone.handoff.is_none() && phone.note.contains("Settings"));
    }
}
