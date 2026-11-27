//! What an install did, step by step, for the person to read when they ask.
//!
//! The page shows one sentence about how an install went. When it went wrong that is
//! often not enough to tell why: what was fetched, which command was run and what it
//! printed are what someone needs to put it right, or to report it. So the steps are
//! written down as they happen and handed to whoever started the install.
//!
//! The notes are kept for the thread doing the work: [`keep`] wraps one install, and
//! [`note`] anywhere underneath adds a line. With nobody keeping, a note goes nowhere.
//! Nothing here leaves the device.

use std::cell::RefCell;

/// No more lines than this are kept of one install; the oldest in the middle go.
const MOST: usize = 600;

thread_local! {
    static KEPT: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Do `work`, and return with it everything noted while it ran.
pub fn keep<T>(work: impl FnOnce() -> T) -> (T, Vec<String>) {
    let before = KEPT.with(|kept| kept.borrow_mut().replace(Vec::new()));
    let done = work();
    let lines = KEPT.with(|kept| std::mem::replace(&mut *kept.borrow_mut(), before)).unwrap_or_default();
    (done, lines)
}

/// Add a line about what is being done, if anyone is keeping them.
pub fn note(line: impl AsRef<str>) {
    KEPT.with(|kept| {
        if let Some(lines) = kept.borrow_mut().as_mut() {
            // What a program prints can carry colour codes and the like: only what
            // can be read is kept.
            let clean: String = line.as_ref().chars().filter(|c| !c.is_control() || *c == '\t').collect();
            if lines.len() == MOST {
                lines[MOST / 2] = "[…lines left out…]".to_string(); // (not shown)
                lines.remove(MOST / 2 + 1);
            }
            lines.push(clean);
        }
    });
}

/// Note what a program printed, each line set in a little under `title`.
pub(super) fn note_said(title: &str, said: &str) {
    let lines: Vec<&str> = said.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    if lines.is_empty() {
        return;
    }
    note(format!("{title}:"));
    for line in lines {
        note(format!("  {line}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_kept_only_while_someone_keeps_them() {
        note("nobody is listening");
        let (answer, lines) = keep(|| {
            note("first");
            note("second \u{1b}[31mred\u{1b}[0m");
            note_said("It said", "one\n\n  two  \n");
            7
        });
        assert_eq!(answer, 7);
        assert_eq!(lines, ["first", "second [31mred[0m", "It said:", "  one", "    two"]);
        let (_, after) = keep(|| ());
        assert!(after.is_empty(), "each install starts with nothing");
    }

    #[test]
    fn a_very_long_install_keeps_its_beginning_and_its_end() {
        let (_, lines) = keep(|| (0..MOST * 2).for_each(|n| note(format!("line {n}"))));
        assert_eq!(lines.len(), MOST);
        assert_eq!(lines[0], "line 0");
        assert_eq!(lines.last().unwrap(), &format!("line {}", MOST * 2 - 1));
        assert!(lines.iter().any(|line| line.contains("left out")));
    }
}
