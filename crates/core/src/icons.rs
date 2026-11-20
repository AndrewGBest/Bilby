//! App icons, fetched from the source and kept on disk.
//!
//! GitHub projects have no icon as such; the owner's avatar is what GitHub itself shows
//! next to a repository, and for organisations it is normally the project's logo. It is
//! served from `github.com/<owner>.png`, which is an ordinary web request and does not
//! count against the API allowance. AUR packages have no picture at all.
//!
//! Results are cached in both directions: a picture is stored under the owner's name,
//! and "this owner has none" is remembered too, so a missing icon is not asked for again
//! on every start.

use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::{AppId, SourceKind};
use std::path::{Path, PathBuf};

/// Edge length requested, in pixels. Enough for the largest tile at 2x.
const SIZE: u32 = 128;

/// What identifies an app's picture: the lower-cased owner. Apps of one owner share it.
pub fn icon_key(id: &AppId) -> Option<String> {
    match id.source {
        SourceKind::GitHub => id.key.split('/').next().filter(|o| !o.is_empty()).map(str::to_lowercase),
        SourceKind::Codeberg => {
            id.key.split('/').next().filter(|o| !o.is_empty()).map(|o| format!("codeberg-{}", o.to_lowercase()))
        }
        // The rest give each app its own picture, or none.
        SourceKind::Aur
        | SourceKind::Pacman
        | SourceKind::Apt
        | SourceKind::Web
        | SourceKind::Flathub
        | SourceKind::GitLab
        | SourceKind::Homebrew
        | SourceKind::Winget
        | SourceKind::Snap
        | SourceKind::FDroid
        | SourceKind::Steam
        | SourceKind::MacAppStore => None,
    }
}

/// What identifies a picture that belongs to one app (its own logo), as a file name.
pub fn app_key(id: &AppId) -> String {
    let safe: String =
        id.to_string().chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    format!("app-{safe}")
}

pub fn icon_url(id: &AppId) -> Option<String> {
    let owner = id.key.split('/').next()?;
    if owner.is_empty() {
        return None;
    }
    match id.source {
        SourceKind::GitHub => Some(format!("https://github.com/{owner}.png?size={SIZE}")),
        SourceKind::Codeberg => Some(format!("https://codeberg.org/{owner}.png?size={SIZE}")),
        _ => None,
    }
}

/// The kind of image, from its first bytes. The URL's `.png` is not to be trusted:
/// avatars are frequently JPEGs.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else {
        None
    }
}

/// `Some(Some(path))`: cached picture. `Some(None)`: known to have none. `None`: never asked.
pub fn cached(dir: &Path, key: &str) -> Option<Option<PathBuf>> {
    for ext in ["png", "jpg"] {
        let path = dir.join(format!("{key}.{ext}"));
        if path.is_file() {
            return Some(Some(path));
        }
    }
    dir.join(format!("{key}.none")).is_file().then_some(None)
}

/// Make sure the app's picture is on disk and say where; `Ok(None)` when it has none.
/// Network trouble is an `Err` and is not remembered, so it is tried again next time.
pub fn fetch(http: &dyn Http, dir: &Path, id: &AppId) -> Result<Option<PathBuf>> {
    let (Some(key), Some(url)) = (icon_key(id), icon_url(id)) else { return Ok(None) };
    fetch_picture(http, dir, &key, &url, 2 * 1024 * 1024)
}

/// Fetch any PNG or JPEG into `dir` under `key`, giving up past `max_bytes`.
pub fn fetch_picture(http: &dyn Http, dir: &Path, key: &str, url: &str, max_bytes: u64) -> Result<Option<PathBuf>> {
    if let Some(known) = cached(dir, key) {
        return Ok(known);
    }
    std::fs::create_dir_all(dir)?;
    let remember_missing = || std::fs::write(dir.join(format!("{key}.none")), b"").map_err(Error::from);
    let download = dir.join(format!("{key}.download"));
    match http.download(url, &download, &mut |done, _| done < max_bytes) {
        Ok(_) => {}
        // No such picture, or something far too large to be one.
        Err(Error::NotFound(_)) | Err(Error::Cancelled) | Err(Error::Status { .. }) => {
            let _ = std::fs::remove_file(&download);
            remember_missing()?;
            return Ok(None);
        }
        Err(e) => return Err(e),
    }
    let mut head = [0u8; 8];
    let read = std::fs::File::open(&download).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    match sniff(&head[..read]) {
        Some(ext) => {
            let path = dir.join(format!("{key}.{ext}"));
            std::fs::rename(&download, &path)?;
            Ok(Some(path))
        }
        None => {
            // An error page or an unsupported format: not a picture we can show.
            let _ = std::fs::remove_file(&download);
            remember_missing()?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Serves {
        body: &'static [u8],
        status: Option<Error>,
        calls: AtomicUsize,
    }
    impl Serves {
        fn new(body: &'static [u8]) -> Serves {
            Serves { body, status: None, calls: AtomicUsize::new(0) }
        }
    }
    impl Http for Serves {
        fn get(&self, _: &str) -> Result<String> {
            unreachable!()
        }
        fn download(
            &self,
            url: &str,
            dest: &Path,
            progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
        ) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(url, "https://github.com/Octo-Org.png?size=128");
            match &self.status {
                Some(Error::NotFound(u)) => return Err(Error::NotFound(u.clone())),
                Some(_) => return Err(Error::Network("offline".into())),
                None => {}
            }
            std::fs::write(dest, self.body)?;
            if !progress(self.body.len() as u64, None) {
                std::fs::remove_file(dest)?;
                return Err(Error::Cancelled);
            }
            Ok(String::new())
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bilby-icons-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }
    fn id() -> AppId {
        AppId::new(SourceKind::GitHub, "Octo-Org/thing")
    }

    #[test]
    fn keys_and_urls() {
        assert_eq!(icon_key(&id()).as_deref(), Some("octo-org"));
        assert_eq!(icon_url(&id()).as_deref(), Some("https://github.com/Octo-Org.png?size=128"));
        let aur = AppId::new(SourceKind::Aur, "paru");
        assert_eq!((icon_key(&aur), icon_url(&aur)), (None, None));
    }

    #[test]
    fn a_picture_is_fetched_once_and_named_for_what_it_is() {
        let d = dir("jpeg");
        // Served from a ".png" URL, but actually a JPEG.
        let http = Serves::new(&[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]);
        let path = fetch(&http, &d, &id()).unwrap().unwrap();
        assert_eq!(path, d.join("octo-org.jpg"));
        assert_eq!(fetch(&http, &d, &id()).unwrap(), Some(path.clone()));
        // Another app of the same owner reuses it.
        assert_eq!(fetch(&http, &d, &AppId::new(SourceKind::GitHub, "octo-org/other")).unwrap(), Some(path));
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
        assert!(!d.join("octo-org.download").exists());
    }

    #[test]
    fn things_that_are_not_pictures_are_remembered_as_missing() {
        for (tag, body) in [("html", &b"<html>Not Found</html>"[..]), ("empty", &b""[..])] {
            let d = dir(tag);
            let http = Serves::new(body);
            assert_eq!(fetch(&http, &d, &id()).unwrap(), None);
            assert_eq!(fetch(&http, &d, &id()).unwrap(), None);
            assert_eq!(http.calls.load(Ordering::SeqCst), 1, "{tag}: not asked twice");
            assert_eq!(cached(&d, "octo-org"), Some(None));
        }
        let d = dir("404");
        let mut http = Serves::new(b"");
        http.status = Some(Error::NotFound("x".into()));
        assert_eq!(fetch(&http, &d, &id()).unwrap(), None);
        assert_eq!(cached(&d, "octo-org"), Some(None));
    }

    #[test]
    fn network_trouble_is_not_remembered() {
        let d = dir("offline");
        let mut http = Serves::new(b"");
        http.status = Some(Error::Network("offline".into()));
        assert!(fetch(&http, &d, &id()).is_err());
        assert_eq!(cached(&d, "octo-org"), None, "it will be tried again");
    }

    #[test]
    fn oversized_downloads_are_abandoned() {
        static BIG: [u8; 3 * 1024 * 1024] = [0x89; 3 * 1024 * 1024];
        let d = dir("big");
        assert_eq!(fetch(&Serves::new(&BIG), &d, &id()).unwrap(), None);
        assert!(!d.join("octo-org.download").exists());
    }

    #[test]
    fn aur_packages_have_no_picture_and_cost_nothing() {
        let http = Serves::new(b"");
        assert_eq!(fetch(&http, &dir("aur"), &AppId::new(SourceKind::Aur, "paru")).unwrap(), None);
        assert_eq!(http.calls.load(Ordering::SeqCst), 0);
    }
}
