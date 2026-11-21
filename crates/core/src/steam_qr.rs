//! Signing in to Steam with a code scanned by the Steam app on a phone.
//!
//! The other way to learn what an account owns (see `steam_account`): no browser at
//! all. Steam is asked for a sign-in code, the store draws it, and the person scans
//! it with the Steam app and approves there. Steam then gives the store keys to the
//! account. The store uses one of them once, to ask what the account owns, and
//! throws both away: they are never stored, never written anywhere, and never put
//! in an error message.
//!
//! That is the difference from the browser way, and it is said to the person: here
//! Steam's keys do pass through the store, for the moment it takes to ask.
//!
//! [verified 2026-11-21 against the live service, with no account: the first address
//! answers with a code address under `https://s.team/q/`, a wait of 5 seconds and no
//! key needed; the second answers `{"had_remote_interaction":false}` while nobody has
//! scanned, and `{}` for a code it does not know. Not verified, by anyone here: what
//! comes back after approval, and that the list is given for these keys. Those parts
//! follow how other programs that sign in this way describe it.]

use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use crate::steam_account::Account;
use serde::Deserialize;

pub const BEGIN_URL: &str = "https://api.steampowered.com/IAuthenticationService/BeginAuthSessionViaQR/v1/";
pub const POLL_URL: &str = "https://api.steampowered.com/IAuthenticationService/PollAuthSessionStatus/v1/";
pub const OWNED_URL: &str = "https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/";

/// A sign-in that has been started and not yet approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    client_id: String,
    request_id: String,
    /// What the code says: an address only the Steam app understands.
    pub challenge_url: String,
    /// Seconds Steam asks to be left alone between questions.
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// Not approved yet. `scanned` once the Steam app has read the code.
    Waiting {
        scanned: bool,
    },
    Done(Account),
    /// Steam no longer knows the code: it ran out, or was refused.
    Expired,
}

#[derive(Deserialize)]
struct Wrapped<T> {
    response: T,
}

/// A code address is only ever drawn if it is one of Steam's.
fn steam_code(url: &str) -> bool {
    url.starts_with("https://s.team/") && url.len() < 200 && url.chars().all(|c| c.is_ascii_graphic())
}

/// Ask Steam for a sign-in code. `device` is the name the Steam app shows the person
/// when it asks them to approve.
pub fn start(http: &dyn Http, device: &str) -> Result<Session> {
    #[derive(Deserialize)]
    struct Begun {
        client_id: serde_json::Value,
        request_id: String,
        challenge_url: String,
        #[serde(default)]
        interval: f64,
    }
    // 2 is Steam's number for "a web browser", the plainest thing this could say it is.
    let body = format!("device_friendly_name={}&platform_type=2", url_encode(device));
    let begun: Wrapped<Begun> = serde_json::from_str(&http.post_form(BEGIN_URL, &body)?)?;
    let begun = begun.response;
    let client_id = number(&begun.client_id).ok_or_else(|| Error::Parse("Steam's answer could not be read.".into()))?;
    if !steam_code(&begun.challenge_url) {
        return Err(Error::Parse("Steam's answer could not be read.".into()));
    }
    Ok(Session {
        client_id,
        request_id: begun.request_id,
        challenge_url: begun.challenge_url,
        interval: begun.interval.clamp(0.0, 30.0) as u64,
    })
}

/// A number Steam writes sometimes as a number and sometimes as text.
fn number(value: &serde_json::Value) -> Option<String> {
    let text = match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty() && text.len() <= 20 && text.chars().all(|c| c.is_ascii_digit())).then_some(text)
}

/// Ask whether the code has been approved. Steam replaces the code now and then;
/// `session.challenge_url` is then the new one to draw. Once approved, the list of
/// what the account owns is fetched and the keys are dropped.
pub fn poll(http: &dyn Http, session: &mut Session) -> Result<Poll> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Status {
        had_remote_interaction: Option<bool>,
        new_client_id: Option<serde_json::Value>,
        new_challenge_url: Option<String>,
        access_token: String,
        refresh_token: String,
    }
    let body = format!("client_id={}&request_id={}", session.client_id, url_encode(&session.request_id));
    let status: Wrapped<Status> = serde_json::from_str(&http.post_form(POLL_URL, &body)?)?;
    let status = status.response;
    if !status.access_token.is_empty() {
        return owned(http, &status.access_token).map(Poll::Done);
    }
    // Approved, but only the long-lived key was given: nothing here uses that one.
    if !status.refresh_token.is_empty() {
        return Err(Error::Unsupported(
            "Steam approved the sign-in but did not give what is needed to read your library.".into(),
        ));
    }
    if let (Some(id), Some(url)) = (status.new_client_id.as_ref().and_then(number), status.new_challenge_url) {
        if steam_code(&url) {
            session.client_id = id;
            session.challenge_url = url;
        }
    }
    Ok(match status.had_remote_interaction {
        Some(scanned) => Poll::Waiting { scanned },
        None => Poll::Expired,
    })
}

/// The account number a key belongs to. Steam's keys are three parts joined by dots;
/// the middle one, decoded, names the account as `sub`.
pub fn account_of(key: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Claims {
        sub: String,
    }
    let middle = key.split('.').nth(1)?;
    let claims: Claims = serde_json::from_slice(&unbase64url(middle)?).ok()?;
    (5..=20).contains(&claims.sub.len()).then_some(claims.sub).filter(|id| id.chars().all(|c| c.is_ascii_digit()))
}

fn unbase64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut have) = (0u32, 0u8);
    for c in text.bytes().filter(|c| *c != b'=') {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | value as u32;
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
            bits &= (1 << have) - 1;
        }
    }
    Some(out)
}

/// Ask Steam what the account owns, with the key it just gave. An error from here
/// never repeats the address, because the key is in it.
fn owned(http: &dyn Http, key: &str) -> Result<Account> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Games {
        games: Vec<Game>,
    }
    #[derive(Deserialize)]
    struct Game {
        appid: u32,
    }
    let unreadable = || Error::Parse("Steam's answer could not be read.".into());
    let steamid = account_of(key).ok_or_else(unreadable)?;
    let url = format!(
        "{OWNED_URL}?access_token={}&steamid={steamid}&include_played_free_games=1&skip_unvetted_apps=0",
        url_encode(key)
    );
    let text = http.get_private(&url).map_err(|e| match e {
        Error::Status { code, .. } => {
            Error::Unsupported(format!("Steam would not give your library after you approved (it answered {code})."))
        }
        Error::Network(_) => Error::Network("Steam could not be reached to read your library.".into()),
        _ => Error::Unsupported("Steam would not give your library after you approved.".into()),
    })?;
    let games: Wrapped<Games> = serde_json::from_str(&text).map_err(|_| unreadable())?;
    let mut owned: Vec<u32> = games.response.games.into_iter().map(|g| g.appid).collect();
    owned.sort_unstable();
    owned.dedup();
    Ok(Account { steamid, owned })
}

/// A code to scan, as squares: `size` to a side, row by row, true where dark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub size: usize,
    pub dark: Vec<bool>,
}

/// Draw text as a code a phone's camera can read.
pub fn code(text: &str) -> Option<Code> {
    let made = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let dark = made.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
    Some(Code { size: made.width(), dark })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::DemoHttp;

    #[test]
    fn a_code_is_started_refreshed_and_approved() {
        let http = DemoHttp;
        let mut session = start(&http, "Bilby").unwrap();
        assert_eq!((session.challenge_url.as_str(), session.interval), ("https://s.team/q/1/1001", 0));
        // Steam swaps the code now and then: the new one is the one to draw.
        assert_eq!(poll(&http, &mut session).unwrap(), Poll::Waiting { scanned: true });
        assert_eq!(session.challenge_url, "https://s.team/q/1/1002");
        let Poll::Done(account) = poll(&http, &mut session).unwrap() else { panic!("approved the second time") };
        assert_eq!((account.steamid.as_str(), account.owned.as_slice()), ("76561198000000001", &[440, 620][..]));
    }

    /// Answers each address with what a test says, and remembers what was asked.
    struct Says(Vec<(&'static str, Result<&'static str>)>, std::sync::Mutex<Vec<String>>);
    impl Http for Says {
        fn get(&self, _: &str) -> Result<String> {
            panic!("a key must never go through the cache")
        }
        fn download(
            &self,
            _: &str,
            _: &std::path::Path,
            _: &mut dyn FnMut(u64, Option<u64>) -> bool,
        ) -> Result<String> {
            unreachable!()
        }
        fn post_form(&self, url: &str, body: &str) -> Result<String> {
            self.answer(url, &format!("{url} {body}"))
        }
        fn get_private(&self, url: &str) -> Result<String> {
            self.answer(url, url)
        }
    }
    impl Says {
        fn answer(&self, url: &str, asked: &str) -> Result<String> {
            self.1.lock().unwrap().push(asked.to_string());
            match self.0.iter().find(|(at, _)| url.starts_with(at)) {
                Some((_, Ok(text))) => Ok(text.to_string()),
                Some((_, Err(Error::Status { code, .. }))) => Err(Error::Status { code: *code, url: url.to_string() }),
                _ => Err(Error::NotFound(url.to_string())),
            }
        }
    }
    const KEY: &str = "aaa.eyJzdWIiOiI3NjU2MTE5ODAwMDAwMDAwMSJ9.secretsignature";

    fn session() -> Session {
        Session {
            client_id: "7".into(),
            request_id: "r+/=".into(),
            challenge_url: "https://s.team/q/1/7".into(),
            interval: 5,
        }
    }

    #[test]
    fn what_steam_was_seen_to_answer_is_understood() {
        // The three answers seen from the live service on 2026-11-21.
        let begun = r#"{"response":{"client_id":"12345678901234567890","challenge_url":"https://s.team/q/1/1234567890123456789","request_id":"q83vEjRWeJAAAAAAAAAAAA==","interval":5,"allowed_confirmations":[{"confirmation_type":4},{"confirmation_type":3}],"version":1}}"#;
        let http = Says(vec![(BEGIN_URL, Ok(begun))], Default::default());
        let started = start(&http, "My store & more").unwrap();
        assert_eq!((started.client_id.as_str(), started.interval), ("12345678901234567890", 5));
        assert_eq!(
            http.1.lock().unwrap()[0],
            format!("{BEGIN_URL} device_friendly_name=My%20store%20%26%20more&platform_type=2")
        );

        let waiting =
            Says(vec![(POLL_URL, Ok(r#"{"response":{"had_remote_interaction":false}}"#))], Default::default());
        assert_eq!(poll(&waiting, &mut session()).unwrap(), Poll::Waiting { scanned: false });
        assert_eq!(waiting.1.lock().unwrap()[0], format!("{POLL_URL} client_id=7&request_id=r%2B%2F%3D"));
        let unknown = Says(vec![(POLL_URL, Ok(r#"{"response":{}}"#))], Default::default());
        assert_eq!(poll(&unknown, &mut session()).unwrap(), Poll::Expired);
    }

    #[test]
    fn only_steams_own_code_addresses_are_drawn() {
        for bad in
            ["https://example.org/q/1", "http://s.team/q/1", "https://s.team.example.org/", "https://s.team/q/ 1"]
        {
            let text = format!(r#"{{"response":{{"client_id":"1","request_id":"x","challenge_url":"{bad}"}}}}"#);
            let leaked: &'static str = Box::leak(text.into_boxed_str());
            assert!(start(&Says(vec![(BEGIN_URL, Ok(leaked))], Default::default()), "x").is_err(), "{bad}");
        }
        let swapped = Says(
            vec![(
                POLL_URL,
                Ok(
                    r#"{"response":{"had_remote_interaction":false,"new_client_id":"8","new_challenge_url":"https://example.org/"}}"#,
                ),
            )],
            Default::default(),
        );
        let mut s = session();
        poll(&swapped, &mut s).unwrap();
        assert_eq!(s.challenge_url, "https://s.team/q/1/7", "a code address from elsewhere is not taken");
    }

    #[test]
    fn the_keys_are_used_once_and_never_repeated() {
        let approved: &'static str = Box::leak(
            format!(r#"{{"response":{{"refresh_token":"long.lived.key","access_token":"{KEY}"}}}}"#).into_boxed_str(),
        );
        let http = Says(
            vec![
                (POLL_URL, Ok(approved)),
                (OWNED_URL, Ok(r#"{"response":{"games":[{"appid":570},{"appid":10},{"appid":570}]}}"#)),
            ],
            Default::default(),
        );
        let Poll::Done(account) = poll(&http, &mut session()).unwrap() else { panic!() };
        assert_eq!(account.owned, [10, 570]);
        let asked = http.1.lock().unwrap().clone();
        assert_eq!(asked.len(), 2);
        assert!(asked[1].contains("steamid=76561198000000001") && !asked.iter().any(|a| a.contains("long.lived.key")));

        // Steam refusing: the message says so without the address, which has the key in it.
        let refused = Says(
            vec![(POLL_URL, Ok(approved)), (OWNED_URL, Err(Error::Status { code: 403, url: String::new() }))],
            Default::default(),
        );
        let message = poll(&refused, &mut session()).unwrap_err().to_string();
        assert!(
            message.contains("403") && !message.contains("secretsignature") && !message.contains("access_token"),
            "{message}"
        );

        assert_eq!(account_of(KEY).as_deref(), Some("76561198000000001"));
        assert!(account_of("nonsense").is_none() && account_of("a.e30.c").is_none() && account_of("a.!!.c").is_none());
    }

    #[test]
    fn a_code_is_drawn_as_squares() {
        let drawn = code("https://s.team/q/1/1234567890123456789").unwrap();
        assert!(drawn.size >= 21 && drawn.dark.len() == drawn.size * drawn.size);
        // The three corner marks every such code has: dark at the corners they sit in.
        assert!(drawn.dark[0] && drawn.dark[drawn.size - 1] && drawn.dark[drawn.size * (drawn.size - 1)]);
        assert!(!drawn.dark[drawn.size * drawn.size - 1] || drawn.size > 21);
    }
}
