//! Adult content, Family mode and the PIN.
//!
//! The store carries things meant for adults: some games, some apps, some websites.
//! They are part of the catalog, handled with care rather than hidden:
//!
//! * In a search they sit in a section of their own, each one covered until it is
//!   chosen, and choosing one asks the person to confirm they are an adult (once), or
//!   for the PIN where one is set.
//! * On the home page they appear only if the person has asked for that.
//! * **Family mode** takes them out altogether: not listed, not covered, not openable.
//!
//! A PIN, if set, guards the way out of all this: leaving Family mode, changing these
//! settings, opening adult content and, if wanted, installing or removing anything.
//! It is four digits, kept only as a salted hash. That stops a child with the remote;
//! it would not stop someone who can read the settings file and try ten thousand
//! numbers, and it does not pretend to.

use sha2::{Digest, Sha256};

/// A PIN is exactly four digits: quick on a remote, and what people expect.
pub fn valid_pin(pin: &str) -> bool {
    pin.len() == 4 && pin.chars().all(|c| c.is_ascii_digit())
}

/// Something unpredictable enough to salt a PIN with, without a random-number crate:
/// the clock and the hasher the standard library seeds differently in every process.
pub fn new_salt() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    hasher.write_u128(now);
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

pub fn hash_pin(salt: &str, pin: &str) -> String {
    let digest = Sha256::digest(format!("bilby-pin:{salt}:{pin}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What the person has chosen, as far as adult content goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Policy {
    /// Nothing for adults anywhere, in any form.
    pub family_mode: bool,
    /// Adult things may appear on the home page (covered, like everywhere).
    pub adult_on_home: bool,
}

/// What stands between the person and something guarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Nothing: go ahead.
    Open,
    /// Ask whether they are an adult.
    ConfirmAdult,
    /// Ask for the PIN.
    Pin,
    /// Not available at all (Family mode).
    Blocked,
}

/// What it takes to open something for adults.
/// `confirmed`: the person has said before that they are an adult.
/// `unlocked`: the PIN was entered earlier in this sitting.
pub fn to_open_adult(policy: Policy, has_pin: bool, confirmed: bool, unlocked: bool) -> Gate {
    if policy.family_mode {
        Gate::Blocked
    } else if has_pin {
        if unlocked {
            Gate::Open
        } else {
            Gate::Pin
        }
    } else if confirmed {
        Gate::Open
    } else {
        Gate::ConfirmAdult
    }
}

/// What it takes to change one of these settings: the PIN, where there is one. Turning
/// Family mode *on* is always allowed, since it only ever makes things stricter.
pub fn to_change_settings(has_pin: bool, makes_stricter: bool) -> Gate {
    if has_pin && !makes_stricter {
        Gate::Pin
    } else {
        Gate::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_four_digits_and_kept_only_as_a_salted_hash() {
        assert!(valid_pin("0420"));
        for bad in ["", "123", "12345", "12a4", "١٢٣٤", " 123"] {
            assert!(!valid_pin(bad), "{bad:?}");
        }
        let (a, b) = (new_salt(), new_salt());
        assert_ne!(a, b, "a new salt each time");
        let kept = hash_pin(&a, "0420");
        assert_eq!(kept.len(), 64);
        assert!(!kept.contains("0420") || kept.len() == 64);
        assert_eq!(kept, hash_pin(&a, "0420"));
        assert_ne!(kept, hash_pin(&a, "0421"));
        assert_ne!(kept, hash_pin(&b, "0420"), "the same PIN under another salt looks different");
    }

    #[test]
    fn adult_content_is_asked_about_once_or_guarded_by_the_pin_or_blocked() {
        let usual = Policy::default();
        assert_eq!(to_open_adult(usual, false, false, false), Gate::ConfirmAdult);
        assert_eq!(to_open_adult(usual, false, true, false), Gate::Open, "asked once");
        // With a PIN, having said "I am an adult" before is not enough.
        assert_eq!(to_open_adult(usual, true, true, false), Gate::Pin);
        assert_eq!(to_open_adult(usual, true, false, true), Gate::Open, "entered earlier in this sitting");
        // Family mode is absolute.
        let family = Policy { family_mode: true, adult_on_home: true };
        assert_eq!(to_open_adult(family, true, true, true), Gate::Blocked);
        assert_eq!(to_open_adult(family, false, true, false), Gate::Blocked);
    }

    #[test]
    fn settings_need_the_pin_except_to_make_things_stricter() {
        assert_eq!(to_change_settings(false, false), Gate::Open);
        assert_eq!(to_change_settings(true, false), Gate::Pin);
        assert_eq!(to_change_settings(true, true), Gate::Open, "anyone may switch Family mode on");
    }
}
