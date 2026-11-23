//! Asking for the system's password on a TV.
//!
//! Changing the system (its packages, a new release of it) needs the password of
//! someone allowed to. On a desk the system's own prompt asks for it; on a TV that
//! prompt is a box nobody can type in, because a remote has no letters. So in the TV
//! layout Bilby asks itself, with its on-screen keyboard, and hands the answer to
//! `sudo`, which asks through a helper program of the caller's choosing.
//!
//! The pieces, all in a folder only this person can read:
//! * `pkexec`: stands in for the system's prompt. In the TV layout it runs the command
//!   through `sudo`; otherwise it runs the real `pkexec`.
//! * `ask`: what `sudo` runs to get the password. It reads it from a pipe.
//! * the pipe: Bilby waits at its other end, and asks on screen when `ask` turns up.
//!
//! The password goes from the screen to `sudo` and nowhere else; nothing is kept.

#![cfg_attr(not(unix), allow(dead_code))]

use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Asking {
    /// The stand-in for `pkexec`: run a command through it and the password is asked
    /// for on screen.
    pub elevate: PathBuf,
    pipe: PathBuf,
    cancelled: PathBuf,
    tv: PathBuf,
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Put the pieces in place under `dir`. `None` where it cannot work: no `sudo`, or
/// the folder cannot be made private.
#[cfg(unix)]
pub fn set_up(dir: &Path) -> Option<Asking> {
    use std::os::unix::fs::PermissionsExt;
    if !on_path("sudo") || !on_path("mkfifo") || dir.to_string_lossy().contains('\'') {
        return None;
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).ok()?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    let asking = Asking {
        elevate: dir.join("pkexec"),
        pipe: dir.join("pipe"),
        cancelled: dir.join("cancelled"),
        tv: dir.join("tv"),
    };
    let made = std::process::Command::new("mkfifo").arg("-m").arg("600").arg(&asking.pipe).status().ok()?;
    if !made.success() {
        return None;
    }
    let ask = dir.join("ask");
    let program = |path: &Path, text: String| -> Option<()> {
        std::fs::write(path, text).ok()?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).ok()
    };
    program(
        &ask,
        format!("#!/bin/sh\n# Bilby: the password, as typed on screen.\nword=$(cat '{}')\n[ -n \"$word\" ] || exit 1\nprintf '%s\\n' \"$word\"\n", asking.pipe.display()),
    )?;
    program(
        &asking.elevate,
        format!(
            "#!/bin/sh\n# Bilby: in the TV layout the password is asked for on screen; otherwise by the system.\n\
             [ -f '{tv}' ] || exec pkexec \"$@\"\n\
             rm -f '{cancelled}'\n\
             SUDO_ASKPASS='{ask}' sudo -A -- \"$@\"\n\
             code=$?\n\
             if [ -f '{cancelled}' ]; then rm -f '{cancelled}'; exit 126; fi\n\
             exit $code\n",
            tv = asking.tv.display(),
            cancelled = asking.cancelled.display(),
            ask = ask.display(),
        ),
    )?;
    Some(asking)
}

#[cfg(not(unix))]
pub fn set_up(_dir: &Path) -> Option<Asking> {
    None
}

impl Asking {
    /// Say whether the TV layout is showing: only then does Bilby ask itself.
    pub fn on_tv(&self, tv: bool) {
        let _ = if tv { std::fs::write(&self.tv, b"") } else { std::fs::remove_file(&self.tv) };
    }

    /// Wait for `sudo` to want the password, for as long as the program runs. `ask`
    /// is called each time, with whether this is a second try after a wrong one; it
    /// answers with what was typed, or `None` when the person gave up.
    pub fn serve(&self, ask: impl Fn(bool) -> Option<String> + Send + 'static) {
        let (pipe, cancelled) = (self.pipe.clone(), self.cancelled.clone());
        std::thread::spawn(move || {
            let mut answered: Option<std::time::Instant> = None;
            loop {
                // Opening waits here until the other end is opened for reading.
                let Ok(mut out) = std::fs::OpenOptions::new().write(true).open(&pipe) else { return };
                // Asked again moments after an answer: that answer was wrong.
                let again = answered.is_some_and(|then| then.elapsed() < std::time::Duration::from_secs(15));
                match ask(again) {
                    Some(word) => {
                        let _ = out.write_all(word.as_bytes());
                        let _ = out.write_all(b"\n");
                    }
                    None => {
                        let _ = std::fs::write(&cancelled, b"");
                    }
                }
                answered = Some(std::time::Instant::now());
                drop(out);
                // Let the reader finish before waiting for the next one.
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }
}

/// The system as it is, with commands that need the password sent through the
/// stand-in, so that on a TV it is asked for on screen.
pub struct AskingHost {
    pub inner: bilby_core::os::RealHost,
    pub elevate: PathBuf,
}

impl bilby_core::os::Host for AskingHost {
    fn read(&self, path: &str) -> Option<String> {
        self.inner.read(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }
    fn modified(&self, path: &str) -> Option<u64> {
        self.inner.modified(path)
    }
    fn env(&self, name: &str) -> Option<String> {
        self.inner.env(name)
    }
    fn has(&self, program: &str) -> bool {
        program == "pkexec" || self.inner.has(program)
    }
    fn run(&self, program: &str, args: &[&str]) -> bilby_core::Result<bilby_core::os::Output> {
        self.inner.run(program, args)
    }
    fn stream(&self, program: &str, args: &[&str], line: &mut dyn FnMut(&str)) -> bilby_core::Result<i32> {
        let elevate = self.elevate.to_string_lossy().into_owned();
        match (program, args.first().copied()) {
            ("pkexec", _) => self.inner.stream(&elevate, args, line),
            // Moving a whole-image system to a new release is asked about by the
            // system even of the person who set it up.
            ("rpm-ostree", Some("rebase")) => {
                let mut whole = vec![program];
                whole.extend_from_slice(args);
                self.inner.stream(&elevate, &whole, line)
            }
            _ => self.inner.stream(program, args, line),
        }
    }
}
