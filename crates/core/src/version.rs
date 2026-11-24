//! Just enough version understanding to answer "is this release newer than what I have?"
//!
//! Release tags are free text (`v1.2.3`, `1.2`, `2024.05.1`, `release-3.4`, `0.19.2-1`,
//! `v2.0.0-beta.1`). The numeric components are compared left to right; a pre-release
//! suffix sorts before the plain version. When nothing numeric can be found the answer
//! is "don't know", and callers fall back to "different means newer".

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Parsed {
    numbers: Vec<u64>,
    prerelease: bool,
    /// Numbers after the core version: `rc.2` -> `[2]`, an Arch `-3` package release -> `[3]`.
    suffix: Vec<u64>,
}

fn parse(version: &str) -> Option<Parsed> {
    // Skip any leading label ("v", "release-", "app-").
    let start = version.find(|c: char| c.is_ascii_digit())?;
    let rest = &version[start..];
    let lower = rest.to_ascii_lowercase();
    let prerelease = ["alpha", "beta", "rc", "pre", "dev", "nightly", "snapshot"].iter().any(|m| lower.contains(m));
    // The core version ends at the first character that is neither a digit nor a dot.
    let core_end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
    let numbers: Vec<u64> =
        rest[..core_end].split('.').filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect();
    let mut suffix = Vec::new();
    let mut digits = String::new();
    for c in rest[core_end..].chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if !digits.is_empty() {
            // Absurdly long digit runs (hashes) are not counters; ignore them.
            if let Ok(n) = digits.parse::<u64>() {
                suffix.push(n);
            }
            digits.clear();
        }
    }
    (!numbers.is_empty()).then_some(Parsed { numbers, prerelease, suffix })
}

/// `Some(Ordering)` when both versions could be understood.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (pa, pb) = (parse(a)?, parse(b)?);
    let len = pa.numbers.len().max(pb.numbers.len());
    for i in 0..len {
        let (x, y) = (pa.numbers.get(i).copied().unwrap_or(0), pb.numbers.get(i).copied().unwrap_or(0));
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return Some(other),
        }
    }
    // Same numbers: 1.0-beta < 1.0. Between two of a kind, the counters in the suffix
    // decide: rc.1 < rc.2, and an Arch package release -1 < -2.
    Some(match (pa.prerelease, pb.prerelease) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => {
            let len = pa.suffix.len().max(pb.suffix.len());
            (0..len)
                .map(|i| pa.suffix.get(i).copied().unwrap_or(0).cmp(&pb.suffix.get(i).copied().unwrap_or(0)))
                .find(|o| *o != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        }
    })
}

/// Is `candidate` an update over `installed`? Unknown formats count as an update when
/// the strings differ, because the source lists newest first.
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    match compare(candidate, installed) {
        Some(ordering) => ordering == Ordering::Greater,
        None => candidate.trim() != installed.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering() {
        let newer = [
            ("v1.2.4", "v1.2.3"),
            ("1.10.0", "1.9.9"),
            ("v2", "1.99.99"),
            ("1.2.1", "1.2"),
            ("2024.05.1", "2023.12.9"),
            ("release-3.4", "release-3.3"),
            ("v2.0.0", "v2.0.0-beta.1"),
            ("v2.0.0-rc.1", "v1.9.0"),
            ("0.19.2-1", "0.19.1-3"),
            ("21.1", "21.0"),
            ("v2.0.0-rc.2", "v2.0.0-rc.1"),
            ("1.0-beta.10", "1.0-beta.9"),
            ("0.19.2-2", "0.19.2-1"),
            ("2.0.4-10", "2.0.4-9"),
            ("1.2.3", "1.2.3-rc.5"),
        ];
        for (a, b) in newer {
            assert!(is_newer(a, b), "{a} should be newer than {b}");
            assert!(!is_newer(b, a), "{b} should not be newer than {a}");
        }
    }

    #[test]
    fn equal_versions_are_not_updates() {
        assert!(!is_newer("v1.2.3", "1.2.3"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("0.19.2-1", "0.19.2-1"));
        assert!(!is_newer("v2.0.0-rc.1", "v2.0.0-rc.1"));
        assert!(!is_newer("nightly", "nightly"));
    }

    #[test]
    fn unparseable_versions_fall_back_to_inequality() {
        assert_eq!(compare("nightly", "1.0"), None);
        assert!(is_newer("build-b", "build-a") || compare("build-b", "build-a").is_none());
        assert!(is_newer("continuous", "stable"));
    }
}
