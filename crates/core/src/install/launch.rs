//! Starting other programs: an installed app, the system's installer, a terminal, the
//! browser. Also the notifications and folder dialogs asked of the desktop.

use super::journal;
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// What an AppImage tells the program inside it about itself: where its file is and
/// where it is mounted. When the store runs from an AppImage these are about the
/// store, and a program it starts must not inherit them and take them for its own.
pub const APPIMAGE_SAYS: [&str; 4] = ["APPIMAGE", "APPDIR", "ARGV0", "OWD"];

pub(super) fn not_ours(command: &mut std::process::Command) {
    for name in APPIMAGE_SAYS {
        command.env_remove(name);
    }
    carry_activation(command);
}

/// Leave to come to the front, from the window system, for the next program started.
pub(super) static ACTIVATION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Hand the next program the store starts the right to come to the front.
///
/// A window system such as Wayland does not let a program put its own window in front
/// of the one being used. That is felt when the program is already running: started
/// again, it tells its running copy what to open, and that copy stays behind the
/// store, as if nothing had happened. The window that was just pressed can ask for a
/// token that carries its leave to do so, and pass it on; this keeps one for the next
/// program started. It is used once.
pub fn grant_activation(token: String) {
    *ACTIVATION.lock().unwrap() = Some(token).filter(|t| !t.is_empty());
}

/// Give `command` the token, if there is one, under the names programs look for it by
/// (Wayland's and, for the X Window System, the older start-up notification's).
pub(super) fn carry_activation(command: &mut std::process::Command) {
    if let Some(token) = ACTIVATION.lock().unwrap().take() {
        command.env("XDG_ACTIVATION_TOKEN", &token).env("DESKTOP_STARTUP_ID", &token);
    }
}

/// The file or folder a running program was started from, for a store that wants to
/// know where it itself lives: the AppImage, when the program really is running out
/// of the one named (a name inherited from whoever started it does not count), or the
/// `AppRun` of the folder an AppImage was unpacked into.
pub fn own_program(exe: &Path, appimage: Option<&Path>, appdir: Option<&Path>) -> Option<PathBuf> {
    if let (Some(file), Some(mounted)) = (appimage, appdir) {
        if file.is_file() && exe.starts_with(mounted) {
            return Some(file.to_path_buf());
        }
    }
    exe.ancestors()
        .find(|folder| folder.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("AppDir")))
        .map(|folder| folder.join("AppRun"))
        .filter(|run| run.is_file())
}

/// Start something, trying again for a moment if the system says its file is busy.
/// A file only just written (a download, an update) can still be held open for
/// writing for an instant, and a program cannot be started from one that is.
pub(super) fn not_while_busy<T>(mut start: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut tries = 0;
    loop {
        match start() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

/// Show one of the desktop's own notifications. True when something was found to
/// show it with. [Run for real with notify-send on KDE. The macOS and Windows forms
/// are written from their documentation.]
pub fn notify(title: &str, body: &str) -> bool {
    use std::process::{Command, Stdio};
    let quiet = |command: &mut Command| {
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
    };
    if cfg!(target_os = "macos") {
        // Passed as arguments, never spliced into the script.
        let script = "on run argv\ndisplay notification (item 2 of argv) with title (item 1 of argv)\nend run";
        return quiet(Command::new("osascript").args(["-e", script, title, body]));
    }
    if cfg!(windows) {
        // A balloon from the notification area; the texts arrive as arguments.
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
                      $n = New-Object System.Windows.Forms.NotifyIcon; \
                      $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true; \
                      $n.ShowBalloonTip(10000, $args[0], $args[1], 'Info'); Start-Sleep -Seconds 11; $n.Dispose()";
        return quiet(Command::new("powershell").args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
            title,
            body,
        ]));
    }
    quiet(Command::new("notify-send").args(["--app-name=Bilby", "--icon=bilby", "--", title, body]))
        || quiet(Command::new("gdbus").args([
            "call",
            "--session",
            "--dest=org.freedesktop.Notifications",
            "--object-path=/org/freedesktop/Notifications",
            "--method=org.freedesktop.Notifications.Notify",
            "Bilby",
            "0",
            "bilby",
            title,
            body,
            "[]",
            "{}",
            "10000",
        ]))
}

/// Let the person choose a folder with the system's own dialog, starting at `start`.
/// `None` when they cancel, or when this system has no dialog the store can call
/// (then the folder can still be typed into Settings). [Run for real with kdialog on
/// KDE. The zenity, macOS and Windows forms are written from their documentation.]
pub fn pick_folder(start: &Path) -> Option<PathBuf> {
    use std::process::{Command, Stdio};
    let on_path = |program: &str| {
        std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    };
    let ask = |command: &mut Command| -> Option<PathBuf> {
        let said = command.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let text = String::from_utf8_lossy(&said.stdout).trim().to_string();
        let path = PathBuf::from(text);
        (said.status.success() && path.is_absolute() && path.is_dir()).then_some(path)
    };
    if cfg!(target_os = "macos") {
        return ask(Command::new("osascript")
            .args(["-e", "POSIX path of (choose folder with prompt \"Where should downloads go?\")"]));
    }
    if cfg!(target_os = "windows") {
        let script = "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Where should downloads go?'; if ($d.ShowDialog() -eq 'OK') { $d.SelectedPath }";
        return ask(Command::new("powershell").args(["-NoProfile", "-Command", script]));
    }
    if on_path("kdialog") {
        return ask(Command::new("kdialog")
            .arg("--getexistingdirectory")
            .arg(start)
            .args(["--title", "Where should downloads go?"]));
    }
    if on_path("zenity") {
        let mut from = start.display().to_string();
        from.push('/');
        return ask(Command::new("zenity")
            .args(["--file-selection", "--directory", "--title=Where should downloads go?"])
            .arg(format!("--filename={from}")));
    }
    None
}

/// A terminal on this machine and the option that makes it run a command.
pub fn find_terminal() -> Option<(PathBuf, &'static str)> {
    const KNOWN: [(&str, &str); 9] = [
        ("konsole", "-e"),
        ("gnome-terminal", "--"),
        ("kgx", "-e"),
        ("xfce4-terminal", "-x"),
        ("alacritty", "-e"),
        ("kitty", "-e"),
        ("foot", "-e"),
        ("x-terminal-emulator", "-e"),
        ("xterm", "-e"),
    ];
    let path = std::env::var_os("PATH")?;
    KNOWN.iter().find_map(|(name, flag)| {
        std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file()).map(|p| (p, *flag))
    })
}

/// How much of what a program prints is kept for the person to read afterwards.
const KEPT_OF_OUTPUT: usize = 64 * 1024;

/// Run a program and wait for it, giving up (and stopping it) when `keep_going` says
/// so. `name` is what to call it when it fails.
pub fn run_program(
    program: &Path,
    args: &[&str],
    name: &str,
    keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<()> {
    run_watched(program, args, name, None, keep_going)
}

/// [`run_program`], reading how far the program has got from what it prints: `read`
/// turns a line into thousandths done, and `keep_going` is told the latest.
pub(super) fn run_watched(
    program: &Path,
    args: &[&str],
    name: &str,
    read: Option<fn(&str) -> Option<u64>>,
    keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<()> {
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    journal::note(format!("Running {} {}", program.display(), args.join(" "))); // (not shown)
                                                                                // Thousandths done, plus one; nothing while it has not said.
    let done = std::sync::Arc::new(AtomicU64::new(0));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this a console window flashes up behind the store.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| Error::Unsupported(format!("{name} could not be started ({e}). Is it installed?")))?;
    // Both of the things it writes to are read as it goes: a program that is not
    // listened to stops when it has said enough to fill the pipe.
    let printed = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let mut readers = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let (done, printed) = (done.clone(), printed.clone());
        // It redraws one line over and over: what ends a line is either kind of return.
        readers.push(std::thread::spawn(move || {
            let (mut line, mut buffer) = (Vec::new(), [0u8; 4096]);
            while let Ok(count @ 1..) = std::io::Read::read(&mut out, &mut buffer) {
                for byte in &buffer[..count] {
                    if matches!(byte, b'\r' | b'\n') {
                        let text = String::from_utf8_lossy(&line).to_string();
                        if let Some(so_far) = read.and_then(|read| read(&text)) {
                            done.store(so_far + 1, Ordering::Relaxed);
                        }
                        // A line that is drawn over is how far it has got, not
                        // something it said: only finished lines are kept.
                        if *byte == b'\n' {
                            let mut printed = printed.lock().unwrap();
                            if printed.len() < KEPT_OF_OUTPUT {
                                printed.push_str(&text);
                                printed.push('\n');
                            }
                        }
                        line.clear();
                    } else if line.len() < 4096 {
                        line.push(*byte);
                    }
                }
            }
        }));
    }
    let errors = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(mut err) = child.stderr.take() {
        let errors = errors.clone();
        readers.push(std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(count @ 1..) = std::io::Read::read(&mut err, &mut buffer) {
                let mut errors = errors.lock().unwrap();
                if errors.len() < KEPT_OF_OUTPUT {
                    errors.push_str(&String::from_utf8_lossy(&buffer[..count]));
                }
            }
        }));
    }
    // Once it has ended: everything it said, for whoever asks to see it.
    let mut finished = |how: String| -> String {
        for reader in readers.drain(..) {
            let _ = reader.join();
        }
        let said = errors.lock().unwrap().clone();
        journal::note_said("It printed", &printed.lock().unwrap()); // (not shown)
        journal::note_said("It reported", &said); // (not shown)
        journal::note(how);
        said
    };
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => {
                finished("It finished without an error".to_string()); // (not shown)
                return Ok(());
            }
            // The password prompt's own codes: it was closed, or permission was refused.
            Some(status) if matches!(status.code(), Some(126 | 127)) && program.ends_with("pkexec") => {
                finished(format!("It ended with {status}")); // (not shown)
                return Err(Error::Unsupported(
                    "Nothing was installed: the password prompt was closed or refused.".to_string(),
                ));
            }
            Some(status) => {
                let said = finished(format!("It ended with {status}")); // (not shown)
                                                                        // The program's own last line is the most useful thing to show.
                let why =
                    said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it gave no reason").trim().to_string();
                return Err(Error::Unsupported(format!("{name} could not do it: {why}")));
            }
            None => {
                let going = match done.load(Ordering::Relaxed) {
                    0 => keep_going(0, None),
                    so_far => keep_going(so_far - 1, Some(1000)),
                };
                if !going {
                    let _ = child.kill();
                    let _ = child.wait();
                    finished("It was stopped before it had finished".to_string()); // (not shown)
                    return Err(Error::Cancelled);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// [`run_program`], also returning what the program printed. It is written to `log`
/// as the program runs (a pipe nobody reads would stop it), and the file is removed.
pub(super) fn run_program_said(
    program: &Path,
    args: &[&str],
    name: &str,
    log: &Path,
    keep_going: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(std::fs::File::create(log)?).stderr(Stdio::piped());
    journal::note(format!("Running {} {}", program.display(), args.join(" "))); // (not shown)
    let finish = |result: Result<()>| {
        let said = std::fs::read_to_string(log).unwrap_or_default();
        let _ = std::fs::remove_file(log);
        journal::note_said("It printed", &said); // (not shown)
        if let Err(e) = &result {
            journal::note(format!("It did not finish: {e}")); // (not shown)
        }
        result.map(|()| said)
    };
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return finish(Err(Error::Unsupported(format!("{name} could not be started ({e}). Is it installed?"))))
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return finish(Ok(())),
            Ok(Some(_)) => {
                let mut said = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut said);
                }
                let why =
                    said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it gave no reason").trim().to_string();
                return finish(Err(Error::Unsupported(format!("{name} could not do it: {why}"))));
            }
            Ok(None) => {
                if !keep_going(0, None) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return finish(Err(Error::Cancelled));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return finish(Err(e.into())),
        }
    }
}

/// Start a program the store placed (an AppImage), without waiting for it.
pub fn launch(path: &Path) -> Result<()> {
    launch_watched(path, &[], std::time::Duration::from_millis(2500))
}

/// Start a program and watch its first moments. One that closes straight away with an
/// error did not start, and saying "Opening…" would be untrue: what it printed is
/// turned into the reason, in words a person can act on where the cause is known.
pub fn launch_watched(path: &Path, args: &[String], watch: std::time::Duration) -> Result<()> {
    use std::process::{Command, Stdio};
    let log = std::env::temp_dir().join(format!("bilby-launch-{}.log", std::process::id()));
    let mut child = not_while_busy(|| {
        let errors = std::fs::File::create(&log).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
        let mut command = Command::new(path);
        not_ours(&mut command);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(errors).spawn()
    })?;
    let started = std::time::Instant::now();
    let failed = loop {
        match child.try_wait() {
            Ok(Some(status)) => break !status.success(),
            Ok(None) if started.elapsed() >= watch => break false,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => break false,
        }
    };
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    if failed {
        return Err(Error::Unsupported(why_it_closed(&said)));
    }
    Ok(())
}

/// The reason a program gave for not starting, for a person. Known causes get advice;
/// anything else is the program's own last words.
pub fn why_it_closed(said: &str) -> String {
    let lower = said.to_lowercase();
    if lower.contains("fuse") {
        return "It closed straight away: this kind of app (an AppImage) needs FUSE, which this system does not have. Install the package libfuse2 (on newer systems libfuse2t64, or fuse2 on Arch), then open it again.".to_string();
    }
    if lower.contains("sandbox") {
        return "It closed straight away: its built-in browser could not start its sandbox on this system. This is a known limit of some Linux setups for apps built this way; its developer's page usually says how to allow it.".to_string();
    }
    if lower.contains("error while loading shared libraries") {
        let library = said
            .lines()
            .find(|l| l.contains("shared libraries"))
            .and_then(|l| l.split(": ").nth(2))
            .unwrap_or("a library");
        return format!("It closed straight away: this system is missing something it needs ({library}).");
    }
    let last = said.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    let last: String = last.chars().filter(|c| !c.is_control()).take(160).collect();
    if last.is_empty() {
        "It closed straight away without saying why.".to_string()
    } else {
        format!("It closed straight away. It said: {last}")
    }
}

/// Start a program with arguments, without waiting for it.
pub fn launch_with(path: &Path, args: &[String]) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(path);
    not_ours(&mut command);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(())
}

/// Open a file or folder with the desktop's default handler.
pub fn open_with_system(path: &Path) -> Result<()> {
    let mut cmd = if cfg!(target_os = "windows") {
        // Not `cmd /C start`: file names come from release authors, and cmd.exe gives
        // characters such as & and ^ a meaning even inside quotes. Explorer receives the
        // path as a plain argument and opens it with the default handler.
        std::process::Command::new("explorer")
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    carry_activation(&mut cmd);
    cmd.arg(path).spawn().map(|_| ()).map_err(|e| Error::Unsupported(format!("Could not open {}: {e}", path.display())))
}

/// Open a web page in the person's browser. Only `https://` links are opened: URLs come
/// from repository metadata, which anyone can write.
pub fn open_url(url: &str) -> Result<()> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Unsupported(format!("Refusing to open \"{url}\": not an https link")));
    }
    open_with_system(Path::new(url))
}

/// The only links handed to another store: its own scheme followed by a number, or its
/// own website. Built by this store from a checked id; checked again before opening.
pub fn store_link_ok(url: &str) -> bool {
    let number = |rest: &str| (1..=12).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_digit());
    // The Microsoft Store's ids are capitals and digits, not a number.
    let product = |rest: &str| crate::model::valid_key(crate::model::SourceKind::MicrosoftStore, rest);
    [
        "steam://install/",
        "steam://store/",
        "steam://rungameid/",
        "macappstore://apps.apple.com/app/id",
        "https://store.steampowered.com/app/",
        "https://apps.apple.com/app/id",
    ]
    .iter()
    .any(|prefix| url.strip_prefix(prefix).is_some_and(number))
        || ["ms-windows-store://pdp/?ProductId=", "https://apps.microsoft.com/detail/"]
            .iter()
            .any(|prefix| url.strip_prefix(prefix).is_some_and(product))
        // GOG's pages go by a name in small letters.
        || url.strip_prefix("https://www.gog.com/en/game/").is_some_and(crate::source::gog::slug_ok)
        // Google Play's go by the app's Android name.
        || url
            .strip_prefix("https://play.google.com/store/apps/details?id=")
            .is_some_and(|name| crate::model::valid_key(crate::model::SourceKind::GooglePlay, name))
}
