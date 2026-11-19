//! The only place that touches the network. Sources talk to the [`Http`] trait, so tests
//! and the demo mode can swap in canned responses.

use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub trait Http: Send + Sync {
    /// GET a text (JSON) resource.
    fn get(&self, url: &str) -> Result<String>;

    /// Stream a file to `dest`, reporting `(bytes so far, total if known)`.
    /// Returns the SHA-256 of what was written, as lowercase hex.
    fn download(
        &self,
        url: &str,
        dest: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<String>;
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
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
}

#[cfg(feature = "net")]
impl NetHttp {
    /// `github_token` raises GitHub's limit from 60 requests/hour per IP to 5,000/hour.
    pub fn new(github_token: Option<String>) -> NetHttp {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .try_proxy_from_env(true)
            .user_agent(concat!("untitledstore/", env!("CARGO_PKG_VERSION")))
            .build();
        NetHttp {
            agent,
            github_token: github_token.filter(|t| !t.trim().is_empty()),
        }
    }

    fn request(&self, url: &str) -> ureq::Request {
        let mut req = self.agent.get(url);
        if url.starts_with("https://api.github.com/") {
            req = req
                .set("Accept", "application/vnd.github+json")
                .set("X-GitHub-Api-Version", "2022-11-28");
            // The token is only ever sent to the API host, never to download redirects
            // or other sources.
            if let Some(token) = &self.github_token {
                req = req.set("Authorization", &format!("Bearer {token}"));
            }
        }
        req
    }

    fn call(&self, url: &str) -> Result<ureq::Response> {
        match self.request(url).call() {
            Ok(resp) => Ok(resp),
            Err(ureq::Error::Status(code, resp)) => {
                let github = url.starts_with("https://api.github.com/");
                let exhausted = resp.header("x-ratelimit-remaining") == Some("0")
                    || resp.header("retry-after").is_some();
                if github && (code == 429 || (code == 403 && exhausted)) {
                    let hint = if self.github_token.is_some() {
                        "Try again in a few minutes."
                    } else {
                        "Add a GitHub token in settings to raise the limit, or try again later."
                    };
                    Err(Error::RateLimited {
                        source: "GitHub",
                        hint: hint.to_string(),
                    })
                } else if code == 404 {
                    Err(Error::NotFound(url.to_string()))
                } else {
                    Err(Error::Status {
                        code,
                        url: url.to_string(),
                    })
                }
            }
            Err(e) => Err(Error::Network(e.to_string())),
        }
    }
}

#[cfg(feature = "net")]
impl Http for NetHttp {
    fn get(&self, url: &str) -> Result<String> {
        self.call(url)?
            .into_string()
            .map_err(|e| Error::Network(e.to_string()))
    }

    fn download(
        &self,
        url: &str,
        dest: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<String> {
        let resp = self.call(url)?;
        let total = resp
            .header("content-length")
            .and_then(|v| v.parse::<u64>().ok());
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
            let n = reader
                .read(&mut buf)
                .map_err(|e| Error::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(done, total);
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
        self.dir.join(format!(
            "{}.json",
            &hex(&Sha256::digest(url.as_bytes()))[..32]
        ))
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

    fn download(
        &self,
        url: &str,
        dest: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<String> {
        self.inner.download(url, dest, progress)
    }
}

impl<H: Http + ?Sized> Http for std::sync::Arc<H> {
    fn get(&self, url: &str) -> Result<String> {
        (**self).get(url)
    }
    fn download(
        &self,
        url: &str,
        dest: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<String> {
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
        fn download(
            &self,
            _: &str,
            _: &Path,
            _: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<String> {
            unreachable!()
        }
    }

    #[test]
    fn encoding() {
        assert_eq!(
            url_encode("visual studio+code/bin"),
            "visual%20studio%2Bcode%2Fbin"
        );
    }

    #[test]
    fn cache_serves_repeats_and_survives_outages() {
        let dir = std::env::temp_dir().join(format!("ustore-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let inner = std::sync::Arc::new(Counting {
            calls: AtomicUsize::new(0),
            fail: false.into(),
        });

        let cache = CachedHttp::new(inner.clone(), dir.clone(), Duration::from_secs(3600));
        assert_eq!(cache.get("https://x/a").unwrap(), "{\"ok\":true}");
        assert_eq!(cache.get("https://x/a").unwrap(), "{\"ok\":true}");
        assert_eq!(
            inner.calls.load(Ordering::SeqCst),
            1,
            "second call is served from disk"
        );

        // Expired entry + network down: stale copy is returned.
        let expired = CachedHttp::new(inner.clone(), dir.clone(), Duration::ZERO);
        inner.fail.store(true, Ordering::SeqCst);
        assert_eq!(expired.get("https://x/a").unwrap(), "{\"ok\":true}");
        assert!(expired.get("https://x/never-seen").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
