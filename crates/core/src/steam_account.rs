//! What a Steam account owns, once the person has signed in.
//!
//! Steam tells a signed-in browser which games the account owns. The store never
//! signs in itself: a browser window on Steam's own page does (a small one the store
//! opens, or Longboard on a TV), and hands back one line: the account's number and
//! the ids of what it owns. That line is all that is kept. No password, cookie or
//! token reaches the store, so there is nothing here that could act as the account.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};

/// Steam's sign-in page. Signed in already, Steam sends the browser on to its store.
pub const SIGN_IN_PAGE: &str = "https://store.steampowered.com/login/?redir=&redir_ssl=1";

/// What Longboard is asked for, by name. Longboard decides what the name means; the
/// store cannot ask it for anything else.
pub const HAND_BACK: &str = "steam-library";

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// The account's number, as Steam writes it.
    pub steamid: String,
    /// Ids of what the account owns, sorted.
    pub owned: Vec<u32>,
}

impl Account {
    pub fn owns(&self, app: u32) -> bool {
        self.owned.binary_search(&app).is_ok()
    }

    /// Does the account own this app? `key` is the app's id at Steam.
    pub fn owns_key(&self, key: &str) -> bool {
        key.parse().is_ok_and(|app| self.owns(app))
    }
}

/// The browser said no, in so many words.
pub fn declined(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .is_ok_and(|v| v.get("declined").and_then(|d| d.as_bool()) == Some(true))
}

/// Read the line a browser handed back. Anything that is not an account number and a
/// list of ids is refused.
pub fn parse(line: &str) -> Option<Account> {
    #[derive(Deserialize)]
    struct Line {
        steamid: String,
        owned: Vec<u32>,
    }
    let line: Line = serde_json::from_str(line.trim()).ok()?;
    if !(5..=20).contains(&line.steamid.len()) || !line.steamid.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut owned = line.owned;
    owned.sort_unstable();
    owned.dedup();
    Some(Account { steamid: line.steamid, owned })
}

/// What to pass Longboard so it shows Steam's sign-in page and, with the person's
/// say-so, writes the answer to `to`. A Longboard too old to know how simply opens
/// the page.
pub fn longboard_args(to: &Path) -> Vec<String> {
    vec![
        format!("--hand-back={HAND_BACK}"),
        format!("--hand-back-to={}", to.display()),
        format!("--url={SIGN_IN_PAGE}"),
    ]
}

/// Wait for Longboard to write its answer. `going` is asked now and then whether to
/// keep waiting.
pub fn await_hand_back(file: &Path, limit: Duration, going: &dyn Fn() -> bool) -> Result<Option<Account>> {
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(file) {
            if declined(&text) {
                return Ok(None);
            }
            if let Some(account) = parse(&text) {
                return Ok(Some(account));
            }
            // Something else was written there: not an answer, and not worth waiting on.
            if !text.trim().is_empty() {
                return Err(Error::Parse("Longboard's answer could not be read.".into()));
            }
        }
        if !going() {
            return Ok(None);
        }
        if started.elapsed() >= limit {
            return Err(Error::Unsupported(
                "Longboard did not answer. Sharing your Steam library needs a newer Longboard; update it and try again.".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handed_back_line_is_read_strictly() {
        let account = parse(r#"{"steamid":"76561198000000001","owned":[570,440,440]}"#).unwrap();
        assert_eq!(account.owned, [440, 570]);
        assert!(account.owns(440) && account.owns_key("570") && !account.owns(10) && !account.owns_key("x"));
        for bad in [
            "",
            "{}",
            r#"{"steamid":"me","owned":[]}"#,
            r#"{"steamid":"76561198000000001"}"#,
            r#"{"steamid":"76561198000000001","owned":["440"]}"#,
            r#"{"steamid":"76561198000000001","owned":[-1]}"#,
        ] {
            assert!(parse(bad).is_none(), "{bad}");
        }
        assert!(declined(r#"{"declined":true}"#) && !declined(r#"{"declined":false}"#) && !declined("no"));
    }

    #[test]
    fn longboard_is_asked_by_name_and_waited_for() {
        let dir = std::env::temp_dir().join(format!("bilby-steam-account-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("answer.json");
        let _ = std::fs::remove_file(&file);
        let args = longboard_args(&file);
        assert_eq!(args[0], "--hand-back=steam-library");
        assert!(args[1].ends_with("answer.json") && args[2].starts_with("--url=https://store.steampowered.com/login/"));

        // Nothing written and told to stop: no account, no error.
        assert_eq!(await_hand_back(&file, Duration::from_secs(5), &|| false).unwrap(), None);
        // Nothing written in time: says what is probably wrong.
        let late = await_hand_back(&file, Duration::ZERO, &|| true).unwrap_err().to_string();
        assert!(late.contains("newer Longboard"), "{late}");
        std::fs::write(&file, r#"{"declined":true}"#).unwrap();
        assert_eq!(await_hand_back(&file, Duration::from_secs(5), &|| true).unwrap(), None);
        std::fs::write(&file, r#"{"steamid":"76561198000000001","owned":[440]}"#).unwrap();
        assert_eq!(await_hand_back(&file, Duration::from_secs(5), &|| true).unwrap().unwrap().owned, [440]);
        std::fs::write(&file, "<html>").unwrap();
        assert!(await_hand_back(&file, Duration::from_secs(5), &|| true).is_err());
    }
}
