//! One Bilby at a time.
//!
//! With the store kept in the system tray, opening it again from the app menu must
//! bring back the one that is running, not start a second. The first to start listens
//! on a port of this machine's own (127.0.0.1, which nothing outside the machine can
//! reach) and writes the port's number beside its cache; a later start finds the
//! number, says what it was asked to do, and leaves.
//!
//! What can be said is small and harmless: show the window, open an app's page, or
//! search. Anything else is ignored.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

const HELLO: &str = "bilby 1 ";
const HEARD: &str = "bilby ok";

/// Whether this start is the one that stays.
pub enum Claim {
    /// Nothing was running: this one listens for the next.
    First(TcpListener),
    /// Another is running and has been told; this one has nothing left to do.
    Second,
}

/// The file that says where the running store listens.
pub fn note(cache: &Path) -> std::path::PathBuf {
    cache.join("instance")
}

/// Tell the running store `message`, or become the running store.
pub fn claim(cache: &Path, message: &str) -> Claim {
    let note = note(cache);
    let port = std::fs::read_to_string(&note).ok().and_then(|text| text.trim().parse::<u16>().ok());
    if port.is_some_and(|port| tell(port, message)) {
        return Claim::Second;
    }
    // Nobody answered (or something else has that port now): this is the first.
    match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(listener) => {
            if let Ok(address) = listener.local_addr() {
                let _ = std::fs::create_dir_all(cache);
                let _ = std::fs::write(&note, address.port().to_string());
            }
            Claim::First(listener)
        }
        // No way to listen: start anyway, as before there was any of this.
        Err(_) => match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
            Ok(listener) => Claim::First(listener),
            Err(_) => Claim::Second,
        },
    }
}

fn tell(port: u16, message: &str) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(400)) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(1500)));
    let line: String = message.chars().filter(|c| !c.is_control()).take(900).collect();
    if stream.write_all(format!("{HELLO}{line}\n").as_bytes()).is_err() {
        return false;
    }
    let mut answer = String::new();
    let _ = BufReader::new(stream).read_line(&mut answer);
    answer.trim() == HEARD
}

/// Hear what later starts say, for as long as the program runs.
pub fn listen(listener: TcpListener, heard: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(1500)));
            let mut line = String::new();
            let mut reader = BufReader::new(std::io::Read::take(&stream, 1024));
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            // Only another Bilby says this; whatever else knocks gets no answer.
            let Some(message) = line.trim_end().strip_prefix(HELLO) else { continue };
            let mut stream = &stream;
            if stream.write_all(format!("{HEARD}\n").as_bytes()).is_ok() {
                heard(message.to_string());
            }
        }
    });
}

/// What a later start says to the one that is running: open this, search for that,
/// or only show the window.
pub fn message(open: Option<&str>, search: Option<&str>) -> String {
    match (open, search) {
        (Some(target), _) => format!("open {target}"),
        (None, Some(query)) => format!("search {query}"),
        (None, None) => "show".to_string(),
    }
}

/// Does this desktop show tray icons at all? Where it does not (GNOME without its
/// extension), a store "in the tray" would be running with no way to reach it.
pub fn has_a_tray() -> bool {
    if !cfg!(target_os = "linux") {
        return cfg!(any(windows, target_os = "macos"));
    }
    use std::process::{Command, Stdio};
    let owned = |name: &str| {
        Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.DBus",
                "--type=method_call",
                "--print-reply",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus.NameHasOwner",
                &format!("string:{name}"),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|said| String::from_utf8_lossy(&said.stdout).contains("boolean true"))
    };
    owned("org.kde.StatusNotifierWatcher") || owned("org.freedesktop.StatusNotifierWatcher")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_second_start_tells_the_first_and_leaves() {
        let cache = std::env::temp_dir().join(format!("bilby-instance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let Claim::First(listener) = claim(&cache, "show") else { panic!("nothing was running") };
        let (said, heard) = mpsc::channel();
        listen(listener, move |message| said.send(message).unwrap());
        assert!(matches!(claim(&cache, "open github:kestrel-sh/kestrel"), Claim::Second));
        assert_eq!(heard.recv_timeout(Duration::from_secs(5)).unwrap(), "open github:kestrel-sh/kestrel");

        // A note left by a store that is gone, or pointing at something that is not
        // Bilby, does not stop the next one starting.
        let other = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        std::fs::write(note(&cache), other.local_addr().unwrap().port().to_string()).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = other.accept() {
                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
            }
        });
        assert!(matches!(claim(&cache, "show"), Claim::First(_)));
        std::fs::write(note(&cache), "not a number").unwrap();
        assert!(matches!(claim(&cache, "show"), Claim::First(_)));
        let _ = std::fs::remove_dir_all(&cache);
    }
}
