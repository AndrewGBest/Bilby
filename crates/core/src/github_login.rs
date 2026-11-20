//! Signing in to GitHub without typing a token: GitHub's "device flow".
//! <https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow>
//!
//! The store asks GitHub for a short code and shows it. The person opens
//! `github.com/login/device` on any device, signs in there and types the code. The store
//! meanwhile asks GitHub every few seconds whether that has happened, and is handed a
//! token when it has. No password passes through the store and there is no server of
//! ours in between.
//!
//! The token is asked for with no permissions at all: it can read what anyone can read,
//! and its only effect is GitHub's higher request allowance for signed-in use.

use crate::error::{Error, Result};
use crate::http::Http;
use serde::Deserialize;

/// Identifies the store to GitHub. Not a secret: the device flow has none, and an id
/// alone can do nothing but start a sign-in that the person must then approve.
pub const CLIENT_ID: &str = "Ov23litVVGDSwZ9TuJ5m";
pub const CODE_URL: &str = "https://github.com/login/device/code";
pub const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// What to show, and what to ask GitHub with.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DeviceCode {
    /// For asking GitHub whether the person has finished. Never shown.
    pub device_code: String,
    /// The short code the person types, e.g. `WDJB-MJHT`.
    pub user_code: String,
    /// Where they type it.
    pub verification_uri: String,
    /// Seconds to wait between questions; asking faster is refused.
    #[serde(default = "five")]
    pub interval: u64,
    /// Seconds until the code stops working.
    #[serde(default)]
    pub expires_in: u64,
}

fn five() -> u64 {
    5
}

/// Where a sign-in stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// Not approved yet: ask again after the interval.
    Pending,
    /// Asked too often: wait this many seconds between questions from now on.
    SlowDown(u64),
    /// Approved. The token.
    Done(String),
    /// The person said no.
    Denied,
    /// The code ran out before it was used.
    Expired,
}

#[derive(Deserialize)]
struct Answer {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

/// Ask GitHub for a code. No permissions are requested.
pub fn start(http: &dyn Http) -> Result<DeviceCode> {
    let body = format!("client_id={CLIENT_ID}");
    let code: DeviceCode = serde_json::from_str(&http.post_form(CODE_URL, &body)?)?;
    // Only GitHub's own page is ever shown or opened.
    if !code.verification_uri.starts_with("https://github.com/") || code.user_code.is_empty() {
        return Err(Error::Parse("GitHub's sign-in answer was not in the expected form".to_string()));
    }
    Ok(code)
}

/// Ask whether the person has entered the code yet.
pub fn poll(http: &dyn Http, code: &DeviceCode) -> Result<Poll> {
    let body = format!(
        "client_id={CLIENT_ID}&device_code={}&grant_type=urn:ietf:params:oauth:grant-type:device_code",
        crate::http::url_encode(&code.device_code)
    );
    parse_poll(&http.post_form(TOKEN_URL, &body)?, code.interval)
}

fn parse_poll(text: &str, interval: u64) -> Result<Poll> {
    let answer: Answer = serde_json::from_str(text)?;
    if let Some(token) = answer.access_token.filter(|t| !t.trim().is_empty()) {
        return Ok(Poll::Done(token));
    }
    match answer.error.as_deref() {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown(answer.interval.unwrap_or(interval + 5))),
        Some("access_denied") => Ok(Poll::Denied),
        Some("expired_token") => Ok(Poll::Expired),
        Some(other) => Err(Error::Unsupported(format!(
            "GitHub would not sign in: {}",
            answer.error_description.unwrap_or_else(|| other.to_string())
        ))),
        None => Err(Error::Parse("GitHub's sign-in answer had neither a token nor a reason".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_answer_github_documents_is_understood() {
        let poll = |text: &str| parse_poll(text, 5).unwrap();
        assert_eq!(poll(r#"{"error":"authorization_pending","error_description":"..."}"#), Poll::Pending);
        assert_eq!(poll(r#"{"error":"slow_down","interval":10}"#), Poll::SlowDown(10));
        assert_eq!(poll(r#"{"error":"slow_down"}"#), Poll::SlowDown(10), "five more when GitHub gives no figure");
        assert_eq!(poll(r#"{"error":"access_denied"}"#), Poll::Denied);
        assert_eq!(poll(r#"{"error":"expired_token"}"#), Poll::Expired);
        assert_eq!(
            poll(r#"{"access_token":"gho_abc","token_type":"bearer","scope":""}"#),
            Poll::Done("gho_abc".to_string())
        );
        let refused = parse_poll(r#"{"error":"device_flow_disabled","error_description":"Not enabled"}"#, 5);
        assert!(refused.unwrap_err().to_string().contains("Not enabled"));
        assert!(parse_poll("{}", 5).is_err() && parse_poll("<html>", 5).is_err());
    }

    #[test]
    fn the_demo_signs_in_after_one_question() {
        let http = crate::demo::DemoHttp;
        let code = start(&http).unwrap();
        assert_eq!(code.verification_uri, "https://github.com/login/device");
        assert_eq!(code.user_code.len(), 9);
        assert!(matches!(poll(&http, &code).unwrap(), Poll::Done(token) if token.starts_with("demo-")));
    }
}
