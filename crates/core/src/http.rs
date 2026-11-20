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

    /// Stream a file to `dest`, reporting `(bytes so far, total if known)`. The callback
    /// returns `false` to cancel; the partial file is removed and `Error::Cancelled` returned.
    /// Returns the SHA-256 of what was written, as lowercase hex.
    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String>;

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
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .try_proxy_from_env(proxy_from_env)
            .user_agent(concat!("bilby/", env!("CARGO_PKG_VERSION")))
            .build();
        NetHttp {
            agent,
            github_token: github_token.filter(|t| !t.trim().is_empty()),
            github_api: format!("{}/", github_api.trim_end_matches('/')),
            quota: std::sync::Mutex::new(None),
        }
    }

    fn request(&self, url: &str) -> ureq::Request {
        let mut req = self.agent.get(url);
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
                        "Add a GitHub token in settings to raise the limit, or try again later."
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

impl<H: Http> Http for CachedHttp<H> {
    fn get(&self, url: &str) -> Result<String> {
        let path = self.path_for(url);
        let cached = || std::fs::read_to_string(&path).ok();
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
        match self.inner.get(url) {
            Ok(body) => {
                if std::fs::create_dir_all(&self.dir).is_ok() {
                    let _ = std::fs::write(&path, &body);
                }
                Ok(body)
            }
            // Offline or rate limited: a stale answer beats no answer.
            Err(e @ (Error::Network(_) | Error::RateLimited { .. })) => cached().ok_or(e),
            Err(e) => Err(e),
        }
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

    fn get(&self, url: &str) -> Result<String> {
        (**self).get(url)
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
