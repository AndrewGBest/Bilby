//! Pictures of an app, found in its README.
//!
//! Release pages carry no screenshots, but most projects show some in their README.
//! This picks out the images that are probably screenshots (not badges, logos or
//! icons), turns them into addresses that can be fetched, and caches the files.
//!
//! Only GitHub's own hosts are fetched from. A README can embed an image from
//! anywhere, and loading it would tell that server who is looking at the project.

use crate::error::Result;
use crate::http::Http;
use crate::model::{AppId, SourceKind};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// How many pictures an app page shows at most.
pub const MAX_PICTURES: usize = 4;

/// Larger than this is not fetched: a screenshot is not worth megabytes of someone's data.
const MAX_BYTES: u64 = 6 * 1024 * 1024;

struct Candidate {
    url: String,
    alt: String,
    /// Width or height given in HTML, when it was.
    size: Option<u32>,
}

/// Addresses of likely screenshots in `markdown`, in the order they appear.
pub fn from_readme(markdown: &str, id: &AppId) -> Vec<String> {
    if id.source != SourceKind::GitHub {
        return Vec::new();
    }
    let mut found: Vec<String> = Vec::new();
    for candidate in candidates(markdown) {
        if !looks_like_a_screenshot(&candidate) {
            continue;
        }
        if let Some(url) = resolve(&candidate.url, &id.key) {
            if !found.contains(&url) {
                found.push(url);
            }
        }
        if found.len() == MAX_PICTURES {
            break;
        }
    }
    found
}

/// The project's own logo, when its README shows one: an early image that calls itself
/// a logo or an icon, or that is drawn small on purpose.
pub fn logo_from_readme(markdown: &str, id: &AppId) -> Option<String> {
    if id.source != SourceKind::GitHub {
        return None;
    }
    // A logo sits at the top. Further down, small images are diagrams and emoji.
    candidates(markdown).into_iter().take(4).find_map(|c| {
        let url = c.url.to_ascii_lowercase();
        let alt = c.alt.to_ascii_lowercase();
        let path = url.split(['?', '#']).next().unwrap_or("");
        let drawable = path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg");
        let badge = ["badge", "shields.io", "badgen", "/workflows/", "codecov", "travis", "sponsor", "button"]
            .iter()
            .any(|word| url.contains(word) || alt.contains(word));
        let named = ["logo", "icon"].iter().any(|word| url.contains(word) || alt.contains(word));
        let small = c.size.is_some_and(|s| (32..=320).contains(&s));
        (drawable && !badge && (named || small)).then(|| resolve(&c.url, &id.key)).flatten()
    })
}

/// Where projects commonly keep their icon, tried when the README shows none.
pub fn usual_logo_places(id: &AppId) -> Vec<String> {
    if id.source != SourceKind::GitHub {
        return Vec::new();
    }
    ["icon.png", "logo.png", "assets/icon.png", "assets/logo.png"]
        .iter()
        .map(|path| format!("https://raw.githubusercontent.com/{}/HEAD/{path}", id.key))
        .collect()
}

/// Every image in the text: Markdown `![alt](url)` and HTML `<img src=...>`.
fn candidates(markdown: &str) -> Vec<Candidate> {
    let mut out: Vec<(usize, Candidate)> = Vec::new();
    let bytes = markdown.as_bytes();
    let mut i = 0;
    while let Some(at) = markdown[i..].find("![") {
        let start = i + at;
        i = start + 2;
        let Some(close) = markdown[i..].find(']') else { break };
        let alt = &markdown[i..i + close];
        let after = i + close + 1;
        if bytes.get(after) != Some(&b'(') {
            continue;
        }
        let Some(end) = markdown[after..].find(')') else { break };
        // `(url "title")`: the address is the first word.
        let url = markdown[after + 1..after + end].split_whitespace().next().unwrap_or("");
        out.push((
            start,
            Candidate { url: url.trim_matches(['<', '>']).to_string(), alt: alt.to_string(), size: None },
        ));
        i = after + end;
    }
    let lower = markdown.to_ascii_lowercase();
    let mut i = 0;
    while let Some(at) = lower[i..].find("<img") {
        let start = i + at;
        let Some(end) = lower[start..].find('>') else { break };
        let tag = &markdown[start..start + end];
        i = start + end;
        let Some(url) = attribute(tag, "src") else { continue };
        let size = ["width", "height"]
            .iter()
            .filter_map(|name| attribute(tag, name))
            .filter_map(|v| v.trim_end_matches("px").parse::<u32>().ok())
            .min();
        out.push((start, Candidate { url, alt: attribute(tag, "alt").unwrap_or_default(), size }));
    }
    out.sort_by_key(|(position, _)| *position);
    out.into_iter().map(|(_, c)| c).collect()
}

/// The value of `name="..."` (or single-quoted, or bare) inside an HTML tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let start = from + at;
        from = start + name.len();
        // A whole attribute name, not the tail of another one.
        if start > 0 && !lower.as_bytes()[start - 1].is_ascii_whitespace() {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let value = match rest.chars().next()? {
            quote @ ('"' | '\'') => rest[1..].split(quote).next()?,
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next()?,
        };
        return Some(value.to_string());
    }
    None
}

fn looks_like_a_screenshot(c: &Candidate) -> bool {
    let url = c.url.to_ascii_lowercase();
    let alt = c.alt.to_ascii_lowercase();
    let path = url.split(['?', '#']).next().unwrap_or("");
    let hosted_upload = url.contains("user-images.githubusercontent.com/") || url.contains("/user-attachments/assets/");
    // Only the formats the app can draw. Uploaded attachments have no extension.
    if !(path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg") || hosted_upload) {
        return false;
    }
    const NOT_SCREENSHOTS: [&str; 12] = [
        "badge",
        "shields.io",
        "badgen",
        "/workflows/",
        "codecov",
        "travis",
        "sponsor",
        "logo",
        "icon",
        "avatar",
        "banner",
        "button",
    ];
    if NOT_SCREENSHOTS.iter().any(|word| url.contains(word) || alt.contains(word)) {
        return false;
    }
    // Something drawn small on purpose is decoration.
    c.size.is_none_or(|s| s >= 200)
}

/// Where the image can be fetched from, or `None` when it is not on a GitHub host.
fn resolve(url: &str, key: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.contains("..") || url.chars().any(|c| c.is_control() || c == ' ') {
        return None;
    }
    if let Some(rest) = url.strip_prefix("https://") {
        let rest = rest.split('#').next().unwrap_or(rest);
        if rest.starts_with("raw.githubusercontent.com/") || rest.starts_with("user-images.githubusercontent.com/") {
            return Some(format!("https://{rest}"));
        }
        if let Some(path) = rest.strip_prefix("github.com/") {
            if path.starts_with("user-attachments/assets/") {
                return Some(format!("https://{rest}"));
            }
            // github.com/owner/repo/blob/branch/file and .../raw/branch/file
            let mut parts = path.splitn(4, '/');
            let (owner, repo, kind, file) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            if kind == "blob" || kind == "raw" {
                let file = file.split('?').next().unwrap_or(file);
                return Some(format!("https://raw.githubusercontent.com/{owner}/{repo}/{file}"));
            }
        }
        return None;
    }
    if url.contains("://") || url.starts_with("//") || url.starts_with("data:") {
        return None;
    }
    // A path inside the repository.
    let path = url.trim_start_matches("./").trim_start_matches('/');
    let path = path.split(['?', '#']).next().unwrap_or(path);
    (!path.is_empty()).then(|| format!("https://raw.githubusercontent.com/{key}/HEAD/{path}"))
}

/// Make sure the picture at `url` is in `dir` and say where. `Ok(None)` when it cannot
/// be shown (missing, too large, not a PNG or JPEG), which is remembered.
pub fn fetch(http: &dyn Http, dir: &Path, url: &str) -> Result<Option<PathBuf>> {
    let key: String = Sha256::digest(url.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect();
    crate::icons::fetch_picture(http, dir, &key, url, MAX_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(markdown: &str) -> Vec<String> {
        from_readme(markdown, &AppId::new(SourceKind::GitHub, "owner/app"))
    }

    #[test]
    fn screenshots_are_found_and_decoration_is_not() {
        let readme = r#"
<p align="center"><img src="assets/logo.png" width="120"></p>

# App

[![build](https://github.com/owner/app/workflows/ci/badge.svg)](ci)
![License badge](https://img.shields.io/badge/license-MIT-blue.png)

![Main window](docs/screenshots/main.png)
<img alt="Settings" src="./docs/screenshots/settings.jpg" width="600">
<img src="docs/small-detail.png" width="48" height="48">
![Animated demo](docs/demo.gif)
![Diagram](docs/architecture.svg)
![Dark mode](https://github.com/owner/app/blob/main/docs/dark.png?raw=true "Dark")
![From an issue](https://user-images.githubusercontent.com/1/abc.png)
![Fifth](docs/fifth.png)
"#;
        assert_eq!(
            found(readme),
            [
                "https://raw.githubusercontent.com/owner/app/HEAD/docs/screenshots/main.png",
                "https://raw.githubusercontent.com/owner/app/HEAD/docs/screenshots/settings.jpg",
                "https://raw.githubusercontent.com/owner/app/main/docs/dark.png",
                "https://user-images.githubusercontent.com/1/abc.png",
            ],
            "in order, at most four, without the logo, badges, the tiny image, the GIF and the SVG"
        );
    }

    #[test]
    fn a_logo_is_an_early_image_that_says_so_or_is_small() {
        let id = AppId::new(SourceKind::GitHub, "owner/app");
        let logo = |markdown: &str| logo_from_readme(markdown, &id);
        assert_eq!(
            logo("<p align=\"center\"><img src=\"assets/logo.png\" width=\"120\"></p>\n# App\n![Shot](docs/main.png)")
                .as_deref(),
            Some("https://raw.githubusercontent.com/owner/app/HEAD/assets/logo.png")
        );
        assert_eq!(
            logo("![App icon](art/app.png)").as_deref(),
            Some("https://raw.githubusercontent.com/owner/app/HEAD/art/app.png")
        );
        assert_eq!(
            logo("<img src=\"art/mark.png\" height=\"96\">").as_deref(),
            Some("https://raw.githubusercontent.com/owner/app/HEAD/art/mark.png")
        );
        assert_eq!(logo("![Main window](docs/main.png)"), None, "a screenshot is not a logo");
        assert_eq!(logo("![logo](assets/logo.svg)"), None, "a format the app cannot draw");
        assert_eq!(logo("![logo badge](https://img.shields.io/badge/logo-x.png)"), None);
        assert_eq!(logo("![logo](https://example.com/logo.png)"), None, "not from a third party");
        assert_eq!(
            logo("![a](1.png)\n![b](2.png)\n![c](3.png)\n![d](4.png)\n![logo](logo.png)"),
            None,
            "only near the top"
        );
        assert_eq!(usual_logo_places(&id).len(), 4);
        assert!(usual_logo_places(&AppId::new(SourceKind::Aur, "x")).is_empty());
    }

    #[test]
    fn only_github_hosts_are_ever_fetched_from() {
        assert!(found("![Shot](https://example.com/shot.png)").is_empty(), "a third party would learn who looked");
        assert!(found("![Shot](http://raw.githubusercontent.com/o/r/main/a.png)").is_empty(), "no plain http");
        assert!(found("![Shot](//evil.example/a.png)").is_empty());
        assert!(found("![Shot](../../other/repo/secret.png)").is_empty());
        assert!(found("![Shot](data:image/png;base64,AAAA.png)").is_empty());
        assert_eq!(
            found("![Shot](https://github.com/user-attachments/assets/1234-abcd)"),
            ["https://github.com/user-attachments/assets/1234-abcd"]
        );
        let codeberg = AppId::new(SourceKind::Codeberg, "owner/app");
        assert!(from_readme("![Shot](docs/a.png)", &codeberg).is_empty(), "other hosts are not guessed at");
    }

    #[test]
    fn odd_markup_does_not_break_it() {
        assert!(found("![unclosed](docs/a.png").is_empty());
        assert!(found("![no address]").is_empty());
        assert!(found("<img src=>").is_empty());
        assert_eq!(found("<IMG SRC='docs/a.png'>"), ["https://raw.githubusercontent.com/owner/app/HEAD/docs/a.png"]);
        assert_eq!(
            found("<img data-src=x.png src=docs/b.png>"),
            ["https://raw.githubusercontent.com/owner/app/HEAD/docs/b.png"]
        );
        assert!(found("").is_empty());
    }
}
