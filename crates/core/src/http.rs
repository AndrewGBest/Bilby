//! The only place that touches the network. Sources talk to the [`Http`] trait, so tests
//! and the demo mode can swap in canned responses.

use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How much of a source's request allowance is left, as the source last reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    pub remaining: u64,
    pub limit: u64,
    /// Unix time at which the allowance refills.
    pub resets_at: u64,
}

impl Quota {
    /// "42 of 60 GitHub requests left; refills in 23 minutes".
    pub fn describe(&self, now_unix: u64) -> String {
        let minutes = self.resets_at.saturating_sub(now_unix).div_ceil(60);
        let refill = match minutes {
            0 => "refills now".to_string(),
            1 => "refills in 1 minute".to_string(),
            m => format!("refills in {m} minutes"),
        };
        format!("{} of {} GitHub requests left; {refill}", self.remaining, self.limit)
    }
}

pub trait Http: Send + Sync {
    /// GET a text (JSON) resource.
    fn get(&self, url: &str) -> Result<String>;

    /// GET a page, and say where it really came from: the address after any
    /// redirects, which is what the addresses inside the page are relative to.
    fn get_landed(&self, url: &str) -> Result<(String, String)> {
        Ok((self.get(url)?, url.to_string()))
    }

    /// Where an address leads once its redirects are followed, without fetching what
    /// is there. For finding out what a "latest" link currently points at.
    fn lands_at(&self, url: &str) -> Result<String> {
        Err(Error::Unsupported(format!("cannot follow {url}")))
    }

    /// Stream a file to `dest`, reporting `(bytes so far, total if known)`. The callback
    /// returns `false` to cancel; the partial file is removed and `Error::Cancelled` returned.
    /// Returns the SHA-256 of what was written, as lowercase hex.
    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String>;

    /// POST a JSON body and return the text answer. Only for sources whose search is a
    /// POST (Flathub); nothing here changes anything on the server.
    fn post_json(&self, url: &str, _body: &str) -> Result<String> {
        Err(Error::Unsupported(format!("cannot send a request to {url}")))
    }

    /// POST a form and return the JSON answer. Never cached: used for signing in, where
    /// the same question gets a different answer once the person has approved.
    fn post_form(&self, url: &str, _body: &str) -> Result<String> {
        Err(Error::Unsupported(format!("cannot send a request to {url}")))
    }

    /// GET something that must not be kept: the address carries a key, or the answer
    /// is one person's. Never cached, and never written anywhere.
    fn get_private(&self, url: &str) -> Result<String> {
        let _ = url;
        Err(Error::Unsupported("cannot send that request".into()))
    }

    /// GET one person's data, showing a key for their account. Never cached, and
    /// neither the key nor the answer is written anywhere.
    fn get_signed(&self, url: &str, _authorization: &str) -> Result<String> {
        Err(Error::Unsupported(format!("cannot send a request to {url}")))
    }

    /// POST JSON that changes something at the other end (a review someone wrote).
    /// Never cached and never repeated. What the server says is returned even when it
    /// refuses, since its answer says why.
    fn submit_json(&self, url: &str, _body: &str) -> Result<String> {
        Err(Error::Unsupported(format!("cannot send a request to {url}")))
    }

    /// GitHub's request allowance as of the last API response, if any was seen.
    fn quota(&self) -> Option<Quota> {
        None
    }
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Real HTTP over rustls.
#[cfg(feature = "net")]
pub struct NetHttp {
    agent: ureq::Agent,
    /// The same, but it stops at a redirect and shows it instead of following.
    stopping: ureq::Agent,
    github_token: Option<String>,
    /// Requests to URLs starting with this get GitHub's headers and the token.
    github_api: String,
    quota: std::sync::Mutex<Option<Quota>>,
}

pub const GITHUB_API: &str = "https://api.github.com";

#[cfg(feature = "net")]
impl NetHttp {
    /// `github_token` raises GitHub's limit from 60 requests/hour per IP to 5,000/hour.
    pub fn new(github_token: Option<String>) -> NetHttp {
        NetHttp::with_options(github_token, GITHUB_API, true)
    }

    /// For tests and unusual setups: a different API host (GitHub Enterprise, a local
    /// test server) and whether to honour the proxy environment variables.
    pub fn with_options(github_token: Option<String>, github_api: &str, proxy_from_env: bool) -> NetHttp {
        let builder = || {
            ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(15))
                .timeout_read(Duration::from_secs(30))
                .try_proxy_from_env(proxy_from_env)
                .user_agent(concat!("bilby/", env!("CARGO_PKG_VERSION")))
        };
        let agent = builder().build();
        NetHttp {
            agent,
            stopping: builder().redirects(0).build(),
            github_token: github_token.filter(|t| !t.trim().is_empty()),
            github_api: format!("{}/", github_api.trim_end_matches('/')),
            quota: std::sync::Mutex::new(None),
        }
    }

    fn request(&self, url: &str) -> ureq::Request {
        let mut req = self.agent.get(url);
        // The Snap Store asks every client which kind of device it is; 16 is "any".
        if url.starts_with("https://api.snapcraft.io/") {
            req = req.set("Snap-Device-Series", "16");
        }
        if url.starts_with(&self.github_api) {
            req = req.set("Accept", "application/vnd.github+json").set("X-GitHub-Api-Version", "2022-11-28");
            // The token is only ever sent to the API host, never to download redirects
            // or other sources.
            if let Some(token) = &self.github_token {
                req = req.set("Authorization", &format!("Bearer {token}"));
            }
        }
        req
    }

    /// Remember the allowance GitHub reports with every API response.
    fn note_quota(&self, url: &str, resp: &ureq::Response) {
        if !url.starts_with(&self.github_api) {
            return;
        }
        let number = |name: &str| resp.header(name).and_then(|v| v.trim().parse::<u64>().ok());
        if let (Some(remaining), Some(limit)) = (number("x-ratelimit-remaining"), number("x-ratelimit-limit")) {
            let resets_at = number("x-ratelimit-reset").unwrap_or(0);
            *self.quota.lock().unwrap() = Some(Quota { remaining, limit, resets_at });
        }
    }

    fn call(&self, url: &str) -> Result<ureq::Response> {
        match self.request(url).call() {
            Ok(resp) => {
                self.note_quota(url, &resp);
                Ok(resp)
            }
            Err(ureq::Error::Status(code, resp)) => {
                self.note_quota(url, &resp);
                let github = url.starts_with(&self.github_api);
                let exhausted =
                    resp.header("x-ratelimit-remaining") == Some("0") || resp.header("retry-after").is_some();
                if github && (code == 429 || (code == 403 && exhausted)) {
                    let hint = if self.github_token.is_some() {
                        "Try again in a few minutes."
                    } else {
                        "Sign in to GitHub in Settings to raise the limit, or try again later."
                    };
                    Err(Error::RateLimited { source: "GitHub", hint: hint.to_string() })
                } else if code == 404 {
                    Err(Error::NotFound(format!("Nothing was found at {url}")))
                } else {
                    Err(Error::Status { code, url: url.to_string() })
                }
            }
            Err(e) => Err(Error::Network(describe_transport_error(url, &e))),
        }
    }
}

/// A sentence a person can act on, instead of the HTTP library's internals.
#[cfg(feature = "net")]
fn describe_transport_error(url: &str, error: &ureq::Error) -> String {
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', '?']).next().unwrap_or(url);
    let kind = match error {
        ureq::Error::Transport(t) => t.kind(),
        ureq::Error::Status(..) => return format!("{host} answered with an error"),
    };
    match kind {
        ureq::ErrorKind::Dns => format!("couldn't find {host}. Check your internet connection."),
        ureq::ErrorKind::ConnectionFailed => format!("couldn't connect to {host}. Check your internet connection."),
        ureq::ErrorKind::Io => format!("the connection to {host} was interrupted. Try again."),
        ureq::ErrorKind::ProxyConnect | ureq::ErrorKind::ProxyUnauthorized | ureq::ErrorKind::InvalidProxyUrl => {
            format!("the proxy refused the connection to {host}. Check your proxy settings.")
        }
        _ => format!("couldn't reach {host} ({kind})."),
    }
}

#[cfg(feature = "net")]
impl Http for NetHttp {
    fn quota(&self) -> Option<Quota> {
        *self.quota.lock().unwrap()
    }

    fn get(&self, url: &str) -> Result<String> {
        self.call(url)?.into_string().map_err(|e| Error::Network(e.to_string()))
    }

    fn get_landed(&self, url: &str) -> Result<(String, String)> {
        let answer = self.call(url)?;
        let landed = answer.get_url().to_string();
        Ok((answer.into_string().map_err(|e| Error::Network(e.to_string()))?, landed))
    }

    fn lands_at(&self, url: &str) -> Result<String> {
        let mut here = url.to_string();
        // Each answer is asked for and dropped unread: only where it points matters.
        for _ in 0..8 {
            let answer = match self.stopping.get(&here).call() {
                Ok(answer) => answer,
                Err(ureq::Error::Status(404, _)) => {
                    return Err(Error::NotFound(format!("Nothing was found at {here}")))
                }
                Err(ureq::Error::Status(code, _)) => return Err(Error::Status { code, url: here }),
                Err(e) => return Err(Error::Network(describe_transport_error(&here, &e))),
            };
            let next = match answer.header("location") {
                Some(next) if (300..400).contains(&answer.status()) => next.to_string(),
                _ => return Ok(here),
            };
            // A redirect may be written relative to the address that gave it.
            here = if next.contains("://") {
                next
            } else {
                let origin_end = here.find("://").map(|at| at + 3).unwrap_or(0);
                let path = here[origin_end..].find('/').map(|at| origin_end + at).unwrap_or(here.len());
                match next.strip_prefix("//") {
                    Some(rest) => format!("https://{rest}"),
                    None if next.starts_with('/') => format!("{}{next}", &here[..path]),
                    None => return Err(Error::Parse(format!("{here} redirects to something that is not an address"))), // (not shown)
                }
            };
        }
        Err(Error::Network(format!("{url} redirects too many times"))) // (not shown)
    }

    fn post_json(&self, url: &str, body: &str) -> Result<String> {
        match self.agent.post(url).set("Content-Type", "application/json").send_string(body) {
            Ok(resp) => resp.into_string().map_err(|e| Error::Network(e.to_string())),
            Err(ureq::Error::Status(404, _)) => Err(Error::NotFound(format!("Nothing was found at {url}"))),
            Err(ureq::Error::Status(code, _)) => Err(Error::Status { code, url: url.to_string() }),
            Err(e) => Err(Error::Network(describe_transport_error(url, &e))),
        }
    }

    fn get_private(&self, url: &str) -> Result<String> {
        self.get(url)
    }

    fn get_signed(&self, url: &str, authorization: &str) -> Result<String> {
        match self.agent.get(url).set("Authorization", authorization).set("Accept", "application/json").call() {
            Ok(resp) => resp.into_string().map_err(|e| Error::Network(e.to_string())),
            Err(ureq::Error::Status(code, _)) => Err(Error::Status { code, url: url.to_string() }),
            Err(e) => Err(Error::Network(describe_transport_error(url, &e))),
        }
    }

    fn submit_json(&self, url: &str, body: &str) -> Result<String> {
        match self.agent.post(url).set("Content-Type", "application/json").send_string(body) {
            Ok(resp) => resp.into_string().map_err(|e| Error::Network(e.to_string())),
            // A refusal comes with its reason, which is what the person needs to see.
            Err(ureq::Error::Status(code, resp)) => match resp.into_string() {
                Ok(text) if text.trim_start().starts_with('{') => Ok(text),
                _ => Err(Error::Status { code, url: url.to_string() }),
            },
            Err(e) => Err(Error::Network(describe_transport_error(url, &e))),
        }
    }

    fn post_form(&self, url: &str, body: &str) -> Result<String> {
        let request = self
            .agent
            .post(url)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .set("Accept", "application/json");
        match request.send_string(body) {
            Ok(resp) => resp.into_string().map_err(|e| Error::Network(e.to_string())),
            Err(ureq::Error::Status(code, _)) => Err(Error::Status { code, url: url.to_string() }),
            Err(e) => Err(Error::Network(describe_transport_error(url, &e))),
        }
    }

    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
        let resp = self.call(url)?;
        let total = resp.header("content-length").and_then(|v| v.parse::<u64>().ok());
        let mut reader = resp.into_reader();
        // Write next to the destination and rename, so a cancelled download never
        // leaves something that looks complete.
        let partial = dest.with_extension("part");
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::io::{Read, Write};
        let mut file = std::fs::File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(|e| Error::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            if !progress(done, total) {
                drop(file);
                let _ = std::fs::remove_file(&partial);
                return Err(Error::Cancelled);
            }
        }
        file.flush()?;
        drop(file);
        std::fs::rename(&partial, dest)?;
        Ok(hex(&hasher.finalize()))
    }
}

/// Remembers GET responses on disk for `ttl`. With 60 unauthenticated GitHub requests an
/// hour, not asking twice is the difference between a usable store and an error page.
pub struct CachedHttp<H> {
    inner: H,
    dir: PathBuf,
    ttl: Duration,
}

impl<H: Http> CachedHttp<H> {
    pub fn new(inner: H, dir: PathBuf, ttl: Duration) -> Self {
        CachedHttp { inner, dir, ttl }
    }

    fn path_for(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{}.json", &hex(&Sha256::digest(url.as_bytes()))[..32]))
    }
}

impl<H: Http> CachedHttp<H> {
    /// Answer from the cache while it is fresh; otherwise ask, remember, and fall back
    /// to a stale answer when the network or the allowance fails.
    fn cached(&self, key: &str, ask: impl FnOnce() -> Result<String>) -> Result<String> {
        let path = self.path_for(key);
        // An empty file is not an answer: it is what a device that lost power in the
        // middle of writing one leaves behind.
        let cached = || std::fs::read_to_string(&path).ok().filter(|body| !body.is_empty());
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < self.ttl);
        if fresh {
            if let Some(body) = cached() {
                return Ok(body);
            }
        }
        match ask() {
            Ok(body) => {
                // Written beside it and moved into place, so that what is there is
                // always a whole answer or the one before it.
                if !body.is_empty() && std::fs::create_dir_all(&self.dir).is_ok() {
                    let partial = path.with_extension("part");
                    if std::fs::write(&partial, &body).is_ok() && std::fs::rename(&partial, &path).is_err() {
                        let _ = std::fs::remove_file(&partial);
                    }
                }
                Ok(body)
            }
            // Offline or rate limited: a stale answer beats no answer.
            Err(e @ (Error::Network(_) | Error::RateLimited { .. })) => cached().ok_or(e),
            Err(e) => Err(e),
        }
    }
}

impl<H: Http> Http for CachedHttp<H> {
    fn get(&self, url: &str) -> Result<String> {
        self.cached(url, || self.inner.get(url))
    }

    fn get_landed(&self, url: &str) -> Result<(String, String)> {
        self.inner.get_landed(url)
    }

    fn lands_at(&self, url: &str) -> Result<String> {
        self.cached(&format!("lands at\n{url}"), || self.inner.lands_at(url))
    }

    fn post_json(&self, url: &str, body: &str) -> Result<String> {
        self.cached(&format!("{url}\n{body}"), || self.inner.post_json(url, body))
    }

    fn post_form(&self, url: &str, body: &str) -> Result<String> {
        self.inner.post_form(url, body)
    }

    fn get_private(&self, url: &str) -> Result<String> {
        self.inner.get_private(url)
    }

    fn get_signed(&self, url: &str, authorization: &str) -> Result<String> {
        self.inner.get_signed(url, authorization)
    }

    fn submit_json(&self, url: &str, body: &str) -> Result<String> {
        self.inner.submit_json(url, body)
    }

    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
        self.inner.download(url, dest, progress)
    }

    fn quota(&self) -> Option<Quota> {
        self.inner.quota()
    }
}

impl<H: Http + ?Sized> Http for std::sync::Arc<H> {
    fn quota(&self) -> Option<Quota> {
        (**self).quota()
    }

    fn post_json(&self, url: &str, body: &str) -> Result<String> {
        (**self).post_json(url, body)
    }

    fn post_form(&self, url: &str, body: &str) -> Result<String> {
        (**self).post_form(url, body)
    }

    fn get_private(&self, url: &str) -> Result<String> {
        (**self).get_private(url)
    }

    fn get_signed(&self, url: &str, authorization: &str) -> Result<String> {
        (**self).get_signed(url, authorization)
    }

    fn submit_json(&self, url: &str, body: &str) -> Result<String> {
        (**self).submit_json(url, body)
    }

    fn get(&self, url: &str) -> Result<String> {
        (**self).get(url)
    }
    fn get_landed(&self, url: &str) -> Result<(String, String)> {
        (**self).get_landed(url)
    }
    fn lands_at(&self, url: &str) -> Result<String> {
        (**self).lands_at(url)
    }
    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
        (**self).download(url, dest, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        calls: AtomicUsize,
        fail: std::sync::atomic::AtomicBool,
    }
    impl Http for Counting {
        fn get(&self, _url: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                Err(Error::Network("offline".into()))
            } else {
                Ok("{\"ok\":true}".to_string())
            }
        }
        fn download(&self, _: &str, _: &Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
            unreachable!()
        }
    }

    #[test]
    fn encoding() {
        assert_eq!(url_encode("visual studio+code/bin"), "visual%20studio%2Bcode%2Fbin");
    }

    #[test]
    fn cache_serves_repeats_and_survives_outages() {
        let dir = std::env::temp_dir().join(format!("bilby-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let inner = std::sync::Arc::new(Counting { calls: AtomicUsize::new(0), fail: false.into() });

        let cache = CachedHttp::new(inner.clone(), dir.clone(), Duration::from_secs(3600));
        assert_eq!(cache.get("https://x/a").unwrap(), "{\"ok\":true}");
        assert_eq!(cache.get("https://x/a").unwrap(), "{\"ok\":true}");
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1, "second call is served from disk");

        // Expired entry + network down: stale copy is returned.
        let expired = CachedHttp::new(inner.clone(), dir.clone(), Duration::ZERO);
        inner.fail.store(true, Ordering::SeqCst);
        assert_eq!(expired.get("https://x/a").unwrap(), "{\"ok\":true}");
        assert!(expired.get("https://x/never-seen").is_err());

        // A device that lost power while writing leaves an empty file: that is not an
        // answer, however new it is. It is asked for again, and a whole one kept.
        inner.fail.store(false, Ordering::SeqCst);
        let before = inner.calls.load(Ordering::SeqCst);
        let written: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|entry| entry.path()).collect();
        assert_eq!(written.len(), 1, "nothing half-written is left beside the answer: {written:?}");
        std::fs::write(&written[0], "").unwrap();
        assert_eq!(cache.get("https://x/a").unwrap(), "{\"ok\":true}");
        assert_eq!(inner.calls.load(Ordering::SeqCst), before + 1);
        assert_eq!(std::fs::read_to_string(&written[0]).unwrap(), "{\"ok\":true}");
        // And with the network down, an empty file is no stale answer to fall back on.
        std::fs::write(&written[0], "").unwrap();
        inner.fail.store(true, Ordering::SeqCst);
        assert!(expired.get("https://x/a").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
