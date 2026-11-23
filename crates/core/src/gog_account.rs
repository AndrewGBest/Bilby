//! What a GOG account owns, once the person has signed in.
//!
//! GOG signs people in the way its own launcher does: its sign-in page is shown in a
//! browser window, and once the person has signed in there GOG sends that window to
//! an address carrying a one-time code. The store swaps the code for a key, asks GOG
//! once which games the account owns, and throws the key away. What is kept is the
//! account's number and the ids of what it owns: nothing that could act as the
//! account.
//!
//! The addresses and the launcher's public id are the ones Heroic, Lutris and
//! Minigalaxy use. The sign-in page and the refusal of a wrong code were tried
//! against GOG; the rest could not be without an account, and follows what those
//! launchers do.

use crate::error::{Error, Result};
use crate::http::{url_encode, Http};
use serde::{Deserialize, Serialize};

/// The id and secret of GOG's own launcher, which is what GOG offers other launchers
/// to sign in with. Neither is a secret in practice: both are published.
const CLIENT_ID: &str = "46899977096215655";
const CLIENT_SECRET: &str = "9d85c43b1482497dbbce61f6e4aa173a433796eeae2ca8c5f6129f2dc4de46d9";
const REDIRECT: &str = "https://embed.gog.com/on_login_success?origin=client";

pub const TOKEN_URL: &str = "https://auth.gog.com/token";
pub const OWNED_URL: &str = "https://embed.gog.com/user/data/games";

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// The account's number, as GOG writes it.
    pub user: String,
    /// Ids of what the account owns, sorted.
    pub owned: Vec<u64>,
}

impl Account {
    /// Does the account own this game? `key` is the game's id at GOG.
    pub fn owns_key(&self, key: &str) -> bool {
        key.parse::<u64>().is_ok_and(|id| self.owned.binary_search(&id).is_ok())
    }
}

/// The one-time code in the line the sign-in window handed back.
pub fn code_of(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let code = value.get("code")?.as_str()?;
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    ((8..=400).contains(&code.len()) && code.chars().all(plain)).then(|| code.to_string())
}

/// Swap the code for a key, ask what the account owns, and forget the key.
pub fn library(http: &dyn Http, code: &str) -> Result<Account> {
    #[derive(Deserialize)]
    struct Token {
        access_token: String,
        #[serde(default)]
        user_id: serde_json::Value,
    }
    #[derive(Deserialize)]
    struct Owned {
        #[serde(default)]
        owned: Vec<u64>,
    }
    let unreadable = || Error::Parse("GOG's answer could not be read.".into());
    let asked = format!(
        "{TOKEN_URL}?client_id={CLIENT_ID}&client_secret={CLIENT_SECRET}&grant_type=authorization_code&code={}&redirect_uri={}",
        url_encode(code),
        url_encode(REDIRECT)
    );
    let token: Token = serde_json::from_str(&http.get_private(&asked)?).map_err(|_| unreadable())?;
    let user = match &token.user_id {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    if token.access_token.is_empty() || user.len() > 30 || !user.chars().all(|c| c.is_ascii_digit()) {
        return Err(unreadable());
    }
    let answer = http.get_signed(OWNED_URL, &format!("Bearer {}", token.access_token))?;
    let mut owned = serde_json::from_str::<Owned>(&answer).map_err(|_| unreadable())?.owned;
    owned.sort_unstable();
    owned.dedup();
    Ok(Account { user, owned })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Gog {
        asked: Mutex<Vec<String>>,
    }

    impl Http for Gog {
        fn get(&self, url: &str) -> Result<String> {
            Err(Error::NotFound(url.to_string()))
        }
        fn download(&self, url: &str, _: &Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
            Err(Error::NotFound(url.to_string()))
        }
        fn get_private(&self, url: &str) -> Result<String> {
            self.asked.lock().unwrap().push(url.to_string());
            if url.contains("code=good-code-1") {
                Ok(r#"{"expires_in":3600,"token_type":"bearer","access_token":"KEY","user_id":"48628349957132247","refresh_token":"R"}"#.into())
            } else {
                Err(Error::Status { code: 400, url: TOKEN_URL.to_string() })
            }
        }
        fn get_signed(&self, url: &str, authorization: &str) -> Result<String> {
            self.asked.lock().unwrap().push(format!("{url} as {authorization}"));
            Ok(r#"{"owned":[1207658924,1,1207658924,1495134320]}"#.into())
        }
    }

    #[test]
    fn a_code_becomes_the_list_of_what_the_account_owns() {
        let gog = Gog::default();
        let account = library(&gog, "good-code-1").unwrap();
        assert_eq!(account.user, "48628349957132247");
        assert_eq!(account.owned, [1, 1207658924, 1495134320]);
        assert!(account.owns_key("1207658924") && !account.owns_key("2") && !account.owns_key("x"));
        let asked = gog.asked.lock().unwrap();
        assert!(asked[0].starts_with("https://auth.gog.com/token?client_id=46899977096215655&"));
        assert!(asked[0].contains("redirect_uri=https%3A%2F%2Fembed.gog.com%2Fon_login_success%3Forigin%3Dclient"));
        assert_eq!(asked[1], "https://embed.gog.com/user/data/games as Bearer KEY");
        // Nothing that could act as the account is kept.
        let kept = serde_json::to_string(&account).unwrap();
        assert!(!kept.contains("KEY") && !kept.contains("\"R\""));
    }

    #[test]
    fn a_refused_code_is_an_error_and_an_odd_line_is_no_code() {
        assert!(library(&Gog::default(), "wrong-code").is_err());
        assert_eq!(code_of(r#"{"code":"Abc_123-xyz"}"#).as_deref(), Some("Abc_123-xyz"));
        assert!(code_of(r#"{"code":"a b"}"#).is_none());
        assert!(code_of(r#"{"code":"short"}"#).is_none());
        assert!(code_of("not json").is_none());
    }
}
