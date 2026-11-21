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
        | SourceKind::Dnf
        | SourceKind::Zypper
        | SourceKind::Web
        | SourceKind::Flathub
        | SourceKind::GitLab
        | SourceKind::Homebrew
        | SourceKind::Winget
        | SourceKind::Snap
        | SourceKind::FDroid
        | SourceKind::Steam
        | SourceKind::MacAppStore
        | SourceKind::Own => None,
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

/// A website's own picture, from the site itself: the icon its front page names (the
/// large one made for phones' home screens where there is one), else the usual
/// addresses for one. Nothing is asked of anyone but the site. Cached like the rest,
/// and "this site has none we can show" is remembered too.
pub fn fetch_site(http: &dyn Http, dir: &Path, key: &str, host: &str) -> Result<Option<PathBuf>> {
    if let Some(known) = cached(dir, key) {
        return Ok(known);
    }
    if !crate::model::valid_key(SourceKind::Web, host) {
        return Ok(None);
    }
    std::fs::create_dir_all(dir)?;
    let base = format!("https://{host}/");
    // What the page itself says, best first; then where such pictures usually live.
    let mut candidates = http.get(&base).map(|html| site_icons(&html, &base)).unwrap_or_default();
    for usual in ["apple-touch-icon.png", "apple-touch-icon-precomposed.png", "favicon.ico"] {
        let url = format!("{base}{usual}");
        if !candidates.contains(&url) {
            candidates.push(url);
        }
    }
    let download = dir.join(format!("{key}.download"));
    let mut unreachable = None;
    for url in candidates.iter().take(6) {
        match http.download(url, &download, &mut |done, _| done < 2 * 1024 * 1024) {
            Ok(_) => {}
            Err(Error::NotFound(_)) | Err(Error::Cancelled) | Err(Error::Status { .. }) => continue,
            Err(e) => {
                unreachable = Some(e);
                continue;
            }
        }
        let bytes = std::fs::read(&download).unwrap_or_default();
        let _ = std::fs::remove_file(&download);
        // A PNG or JPEG as it is; from the older icon format, the PNG inside it.
        let picture = match sniff(&bytes) {
            Some(ext) => Some((bytes, ext)),
            None => png_in_ico(&bytes).map(|png| (png, "png")),
        };
        if let Some((bytes, ext)) = picture {
            let path = dir.join(format!("{key}.{ext}"));
            std::fs::write(&path, bytes)?;
            return Ok(Some(path));
        }
    }
    let _ = std::fs::remove_file(&download);
    // Could not be reached at all: ask again another time. Reached, with nothing usable: remember.
    match unreachable {
        Some(e) if candidates.len() <= 3 => Err(e),
        _ => {
            std::fs::write(dir.join(format!("{key}.none")), b"")?;
            Ok(None)
        }
    }
}

/// The icons a page names in its `<link>` tags, best first: the ones made for home
/// screens (large, square, opaque), then other PNG icons, larger before smaller.
/// Addresses are made whole against `base`; only https ones are kept.
pub fn site_icons(html: &str, base: &str) -> Vec<String> {
    // Cut at a character, not in the middle of one: pages are in every language.
    let mut end = html.len().min(300_000);
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    let head = &html[..end];
    let mut found: Vec<(u32, String)> = Vec::new();
    let is_link = |tag: &&str| tag.get(..5).is_some_and(|start| start.eq_ignore_ascii_case("link "));
    for tag in head.split('<').filter(is_link) {
        let tag = tag.split('>').next().unwrap_or("");
        let attribute = |name: &str| -> Option<String> {
            let lower = tag.to_ascii_lowercase();
            let at = lower.find(&format!("{name}="))? + name.len() + 1;
            let rest = &tag[at..];
            let value = match rest.chars().next()? {
                quote @ ('"' | '\'') => rest[1..].split(quote).next()?,
                _ => rest.split(|c: char| c.is_whitespace() || c == '/').next()?,
            };
            Some(value.trim().replace("&amp;", "&"))
        };
        let rel = attribute("rel").unwrap_or_default().to_ascii_lowercase();
        let Some(href) = attribute("href").filter(|h| !h.is_empty()) else { continue };
        let touch = rel.contains("apple-touch-icon");
        if !touch && !rel.split_whitespace().any(|word| word == "icon") {
            continue;
        }
        let kind = attribute("type").unwrap_or_default().to_ascii_lowercase();
        let path = href.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
        let png = path.ends_with(".png") || kind == "image/png";
        let ico = path.ends_with(".ico") || kind.contains("icon");
        if !touch && !png && !ico {
            // SVG and the like: not something this can show.
            continue;
        }
        let size: u32 = attribute("sizes").and_then(|s| s.split(['x', 'X']).next()?.parse().ok()).unwrap_or(if touch {
            180
        } else {
            32
        });
        let url = if href.starts_with("https://") {
            href
        } else if let Some(rest) = href.strip_prefix("//") {
            format!("https://{rest}")
        } else if href.starts_with('/') {
            format!("{}{}", base.trim_end_matches('/'), href)
        } else if href.contains("://") || href.starts_with("data:") {
            continue;
        } else {
            format!("{base}{href}")
        };
        // Home-screen icons first, then by size; the older format last.
        let rank = if touch {
            10_000
        } else if ico {
            0
        } else {
            1_000
        } + size.min(999);
        if !found.iter().any(|(_, seen)| *seen == url) {
            found.push((rank, url));
        }
    }
    found.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
    found.into_iter().map(|(_, url)| url).collect()
}

/// The largest PNG picture inside a `.ico` file, when it holds one. Modern icon files
/// do; the older ones hold bitmaps, which are left alone.
pub fn png_in_ico(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 6 || bytes[..4] != [0, 0, 1, 0] {
        return None;
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let mut best: Option<&[u8]> = None;
    for entry in 0..count.min(64) {
        let at = 6 + entry * 16;
        let field = |offset: usize| -> Option<usize> {
            let b = bytes.get(at + offset..at + offset + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        };
        let (size, start) = (field(8)?, field(12)?);
        let Some(data) = bytes.get(start..start.checked_add(size)?) else { continue };
        if sniff(data) == Some("png") && best.is_none_or(|b| data.len() > b.len()) {
            best = Some(data);
        }
    }
    best.map(<[u8]>::to_vec)
}

#[cfg(test)]
mod site_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A made-up web: each address answers with what the test gave it.
    struct Web(HashMap<&'static str, Vec<u8>>, Mutex<Vec<String>>);
    impl Http for Web {
        fn get(&self, url: &str) -> Result<String> {
            self.1.lock().unwrap().push(url.to_string());
            self.0
                .get(url)
                .map(|b| String::from_utf8_lossy(b).to_string())
                .ok_or_else(|| Error::NotFound(url.to_string()))
        }
        fn download(&self, url: &str, dest: &Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
            self.1.lock().unwrap().push(url.to_string());
            let body = self.0.get(url).ok_or_else(|| Error::NotFound(url.to_string()))?;
            std::fs::write(dest, body)?;
            Ok(String::new())
        }
    }
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 13, 10, 26, 10, 1, 2, 3];

    fn folder(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bilby-site-icons-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_pages_own_icons_are_read_best_first() {
        let html = r#"<html><head>
            <link rel="icon" href="/favicon.ico">
            <LINK REL="icon" type="image/png" sizes="32x32" href="/img/fav-32.png?v=2&amp;x=1">
            <link rel='icon' sizes='192x192' href='https://cdn.example.net/fav-192.png'>
            <link rel="apple-touch-icon" href="touch.png">
            <link rel="icon" type="image/svg+xml" href="/logo.svg">
            <link rel="stylesheet" href="/site.css">
            <link rel="icon" href="http://insecure.example/x.png">
            <link rel="icon" href="//static.example.org/i.png" sizes="64x64">
        </head></html>"#;
        assert_eq!(
            site_icons(html, "https://example.org/"),
            [
                "https://example.org/touch.png",
                "https://cdn.example.net/fav-192.png",
                "https://static.example.org/i.png",
                "https://example.org/img/fav-32.png?v=2&x=1",
                "https://example.org/favicon.ico",
            ]
        );
        assert!(site_icons("<p>no head</p>", "https://example.org/").is_empty());
    }

    #[test]
    fn a_sites_picture_comes_from_the_site_and_is_remembered() {
        let dir = folder("named");
        let mut pages = HashMap::new();
        pages.insert("https://example.org/", br#"<link rel="apple-touch-icon" href="/touch.png">"#.to_vec());
        pages.insert("https://example.org/touch.png", PNG.to_vec());
        let web = Web(pages, Mutex::new(Vec::new()));
        let path = fetch_site(&web, &dir, "app-web-example-org", "example.org").unwrap().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
        assert!(
            web.1.lock().unwrap().iter().all(|url| url.starts_with("https://example.org/")),
            "only the site itself is asked"
        );
        // The second time nothing is asked at all.
        let asked = web.1.lock().unwrap().len();
        assert!(fetch_site(&web, &dir, "app-web-example-org", "example.org").unwrap().is_some());
        assert_eq!(web.1.lock().unwrap().len(), asked);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_usual_places_are_tried_and_an_old_style_icon_gives_up_its_png() {
        // No page to read (it refuses), no touch icon, but a favicon.ico holding a PNG.
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend([0u8; 8]);
        ico.extend((PNG.len() as u32).to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        ico.extend(PNG);
        assert_eq!(png_in_ico(&ico).as_deref(), Some(PNG));
        assert!(png_in_ico(b"not an icon").is_none() && png_in_ico(&[0, 0, 1, 0, 1, 0, 9, 9]).is_none());

        let dir = folder("usual");
        let mut pages = HashMap::new();
        pages.insert("https://old.example/favicon.ico", ico);
        let web = Web(pages, Mutex::new(Vec::new()));
        let path = fetch_site(&web, &dir, "app-web-old-example", "old.example").unwrap().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), PNG);

        // A site with nothing usable is remembered as such, and not asked again.
        let none = Web(HashMap::new(), Mutex::new(Vec::new()));
        assert!(fetch_site(&none, &dir, "app-web-bare-example", "bare.example").unwrap().is_none());
        let asked = none.1.lock().unwrap().len();
        assert!(fetch_site(&none, &dir, "app-web-bare-example", "bare.example").unwrap().is_none());
        assert_eq!(none.1.lock().unwrap().len(), asked);
        assert!(fetch_site(&none, &dir, "k", "not a host").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_page_in_any_language_is_read_without_falling_over() {
        // A tag of a few bytes that starts with a character several bytes long, and a
        // long page whose cut would land inside one.
        assert!(site_icons("<星星 x><link rel=\"icon\" href=\"/a.png\">", "https://example.org/").len() <= 1);
        let long = format!("<head>{}</head>", "星".repeat(120_000));
        assert!(site_icons(&long, "https://example.org/").is_empty());
    }

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
