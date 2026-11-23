//! Bilby in other languages.
//!
//! The interface's own text (`@tr(...)` in the `.slint` files) is translated by Slint
//! from the `.po` files under `i18n/po`. Everything the Rust side puts on screen
//! (notices, errors, what an install did, the lists Bilby carries) goes through
//! [`tr`] on its way there. Both read the same translations: `i18n/<language>.json`,
//! English text to translated text. `scripts/i18n.py` gathers the English and writes
//! the `.po` files from those.
//!
//! Text is written once, in English, where it is used. A message with something
//! filled in ("Plex is in your library") is recognised by its pattern
//! ("{} is in your library"), so the code that makes messages does not change.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// The languages Bilby comes in: code (also the folder of its `.po` file), its own
/// name for itself, and its translations.
const LANGUAGES: &[(&str, &str, &str)] = &[
    ("en", "English", ""),
    ("es", "Español", include_str!("../i18n/es.json")),
    ("fr", "Français", include_str!("../i18n/fr.json")),
    ("de", "Deutsch", include_str!("../i18n/de.json")),
    ("pt", "Português (Brasil)", include_str!("../i18n/pt.json")),
    ("it", "Italiano", include_str!("../i18n/it.json")),
    ("ru", "Русский", include_str!("../i18n/ru.json")),
    ("zh", "简体中文", include_str!("../i18n/zh.json")),
    ("ja", "日本語", include_str!("../i18n/ja.json")),
    ("ko", "한국어", include_str!("../i18n/ko.json")),
    ("hi", "हिन्दी", include_str!("../i18n/hi.json")),
    ("ar", "العربية", include_str!("../i18n/ar.json")),
];

/// The English messages that have something filled in, as patterns.
const MESSAGES: &str = include_str!("../i18n/en.json");

/// Code and own name of every language, English first.
pub fn languages() -> Vec<(&'static str, &'static str)> {
    LANGUAGES.iter().map(|(code, name, _)| (*code, *name)).collect()
}

/// The language of ours that suits a locale as systems name them ("pt_BR.UTF-8",
/// "zh-Hans-CN", "fr"), or English.
pub fn for_locale(locale: &str) -> &'static str {
    let base = locale.split(['-', '_', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
    LANGUAGES.iter().map(|(code, _, _)| *code).find(|code| *code == base).unwrap_or("en")
}

struct Catalog {
    code: &'static str,
    exact: HashMap<String, String>,
    plurals: HashMap<String, Vec<String>>,
    /// Patterns, longest fixed text first: the fixed parts around the gaps, and the
    /// translation the gaps go into.
    patterns: Vec<(Vec<String>, String)>,
}

fn current() -> &'static RwLock<Catalog> {
    static CURRENT: OnceLock<RwLock<Catalog>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(load("en")))
}

/// "{} is in your library" as the text around its gaps: ["", " is in your library"].
fn parts(pattern: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        parts.last_mut().unwrap().push_str(&rest[..open]);
        parts.push(String::new());
        rest = &rest[open + close + 1..];
    }
    parts.last_mut().unwrap().push_str(rest);
    parts
}

fn load(code: &str) -> Catalog {
    let (code, _, json) = LANGUAGES.iter().find(|(known, _, _)| *known == code).copied().unwrap_or(LANGUAGES[0]);
    let mut catalog = Catalog { code, exact: HashMap::new(), plurals: HashMap::new(), patterns: Vec::new() };
    let Ok(serde_json::Value::Object(entries)) = serde_json::from_str(json) else { return catalog };
    for (english, translated) in entries {
        match translated {
            serde_json::Value::String(text) if !text.is_empty() => {
                catalog.exact.insert(english, text);
            }
            serde_json::Value::Array(forms) => {
                let forms: Vec<String> = forms.iter().filter_map(|f| f.as_str().map(str::to_string)).collect();
                if !forms.is_empty() {
                    catalog.plurals.insert(english, forms);
                }
            }
            _ => {}
        }
    }
    // Only what the code fills in is matched by pattern, not the interface's text.
    let messages: Vec<String> = serde_json::from_str::<serde_json::Value>(MESSAGES)
        .ok()
        .and_then(|all| all["messages"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.as_str().map(str::to_string))
        .collect();
    for english in messages.iter().filter(|m| m.contains('{')) {
        let fixed: usize = parts(english).iter().map(|p| p.trim().chars().count()).sum();
        if let (true, Some(translated)) = (fixed >= 4, catalog.exact.get(english)) {
            let parts = parts(english);
            // "{}Installed to {}" begins with something that is often nothing at all
            // ("Checksum verified. ", or no checksum): known without it too.
            if let (true, Some(rest)) = (parts[0].is_empty() && parts.len() > 2, translated.strip_prefix("{}")) {
                catalog.patterns.push((parts[1..].to_vec(), rest.to_string()));
            }
            catalog.patterns.push((parts, translated.clone()));
        }
    }
    catalog.patterns.sort_by_key(|(parts, _)| std::cmp::Reverse(parts.iter().map(String::len).sum::<usize>()));
    catalog
}

/// Change language. Unknown codes mean English.
pub fn set_language(code: &str) {
    *current().write().unwrap() = load(code);
}

/// The language in use, as its code.
pub fn language() -> &'static str {
    current().read().unwrap().code
}

/// What was filled into `parts`' gaps to make `text`, if it was made that way.
fn gaps<'a>(parts: &[String], text: &'a str) -> Option<Vec<&'a str>> {
    let (first, last) = (parts.first()?, parts.last()?);
    let mut rest = text.strip_prefix(first.as_str())?;
    rest = rest.strip_suffix(last.as_str())?;
    let mut found = Vec::new();
    let middle = &parts[1..parts.len() - 1];
    for part in middle {
        let at = rest.find(part.as_str())?;
        found.push(&rest[..at]);
        rest = &rest[at + part.len()..];
    }
    found.push(rest);
    found.iter().all(|gap| !gap.is_empty()).then_some(found)
}

/// Put `args` into a translation's gaps: `{}` takes the next one, `{1}` the second.
fn fill(translated: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut rest = translated;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..open + close];
        let index = name.parse::<usize>().unwrap_or(next);
        next = if name.is_empty() { next + 1 } else { next };
        out.push_str(args.get(index).map(String::as_str).unwrap_or(""));
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

fn translate(catalog: &Catalog, text: &str, depth: u8) -> Option<String> {
    if let Some(known) = catalog.exact.get(text) {
        return Some(known.clone());
    }
    if depth == 0 {
        return None;
    }
    // A message can end in a line break or a space the pattern does not have.
    let trimmed = text.trim();
    if trimmed != text {
        return translate(catalog, trimmed, depth).map(|t| text.replacen(trimmed, &t, 1));
    }
    for (parts, translated) in &catalog.patterns {
        if let Some(found) = gaps(parts, text) {
            // What was filled in may be a message itself ("Download failed: …"); a
            // single word is a name, and names stay as they are.
            let args: Vec<String> = found
                .iter()
                .map(|gap| match gap.contains(' ') {
                    true => translate(catalog, gap, depth - 1).unwrap_or_else(|| gap.to_string()),
                    false => gap.to_string(),
                })
                .collect();
            return Some(fill(translated, &args));
        }
    }
    None
}

/// `text` in the language in use; itself where there is no translation.
pub fn tr(text: &str) -> String {
    let catalog = current().read().unwrap();
    if catalog.code == "en" || text.is_empty() {
        return text.to_string();
    }
    // Several sentences, each known by itself: one after another.
    translate(&catalog, text, 3).unwrap_or_else(|| text.to_string())
}

/// The same, for text that Bilby did not write unless it is word for word one of its
/// own (an app's summary: Bilby's for the apps it lists itself, the store's otherwise).
/// Never matched by pattern: somebody else's sentence may only look like one of ours.
pub fn exact(text: &str) -> String {
    let catalog = current().read().unwrap();
    catalog.exact.get(text).cloned().unwrap_or_else(|| text.to_string())
}

/// Which of a language's forms goes with `n` things (gettext's rules).
fn form(code: &str, n: usize) -> usize {
    match code {
        "zh" | "ja" | "ko" => 0,
        "fr" | "pt" => usize::from(n > 1),
        "ru" => match (n % 10, n % 100) {
            (1, h) if h != 11 => 0,
            (2..=4, h) if !(12..=14).contains(&h) => 1,
            _ => 2,
        },
        "ar" => match (n, n % 100) {
            (0, _) => 0,
            (1, _) => 1,
            (2, _) => 2,
            (_, 3..=10) => 3,
            (_, 11..=99) => 4,
            _ => 5,
        },
        _ => usize::from(n != 1),
    }
}

/// "1 update" or "3 updates", in the language in use. `singular` and `plural` hold
/// `{n}` where the number goes.
pub fn trn(singular: &str, plural: &str, n: usize) -> String {
    let catalog = current().read().unwrap();
    let english = if n == 1 { singular } else { plural };
    let text = match catalog.plurals.get(singular) {
        Some(forms) if catalog.code != "en" => forms.get(form(catalog.code, n)).or(forms.last()).map(String::as_str),
        _ => None,
    };
    text.unwrap_or(english).replace("{n}", &n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filled_in_message_is_recognised_by_its_pattern() {
        let parts = parts("{} could not be installed: {}");
        assert_eq!(gaps(&parts, "Plex HTPC could not be installed: no room"), Some(vec!["Plex HTPC", "no room"]));
        assert_eq!(gaps(&parts, "Something else entirely"), None);
        assert_eq!(gaps(&super::parts("Version {}"), "Version "), None, "a gap is never empty");
        assert_eq!(fill("{1} ← {0}", &["a".to_string(), "b".to_string()]), "b ← a");
        assert_eq!(fill("{} et {}", &["a".to_string(), "b".to_string()]), "a et b");
    }

    #[test]
    fn locales_lead_to_a_language_of_ours_or_english() {
        assert_eq!(for_locale("pt_BR.UTF-8"), "pt");
        assert_eq!(for_locale("zh-Hans-CN"), "zh");
        assert_eq!(for_locale("fr"), "fr");
        assert_eq!(for_locale("nl_NL"), "en");
        assert_eq!(for_locale(""), "en");
    }

    #[test]
    fn each_language_counts_in_its_own_way() {
        assert_eq!([form("ru", 1), form("ru", 3), form("ru", 5), form("ru", 11), form("ru", 21)], [0, 1, 2, 2, 0]);
        assert_eq!([form("fr", 0), form("fr", 1), form("fr", 2)], [0, 0, 1]);
        assert_eq!([form("ar", 0), form("ar", 2), form("ar", 7), form("ar", 50), form("ar", 100)], [0, 2, 3, 4, 5]);
        assert_eq!(form("ja", 9), 0);
    }
}
