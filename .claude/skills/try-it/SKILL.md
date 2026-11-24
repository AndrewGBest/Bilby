---
name: try-it
description: Run the real Bilby app without a display and look at what it shows - to check a change to the interface, reproduce a report, or make a screenshot. Use after any change that can be seen or pressed.
---

# Try it in the real app

Tests press buttons by name; they do not show that a page looks right. Use
`scripts/try.sh`, which runs the built program under a virtual display with a
throw-away home, so nothing touches real settings or installs.

```sh
cargo build -p bilby
scripts/try.sh <out.png> [options for bilby]
```

Put the picture in the scratch folder, then **read the picture** and the `.txt`
report written beside it (what was on screen, as `key=value` lines).

## Useful options

- `--demo`: the made-up catalog, no network. Leave it out to use the real sources
  (then pass `GITHUB_TOKEN=$(gh auth token)` in the environment to avoid GitHub's limit).
- `--ui desktop|mobile|tv` and `--size 1280x800` / `412x892` / `1920x1080`.
- `--as <platform>`: be another device; `cargo run -q -p bilby-cli -- --list-platforms`.
- `--page discover|search|library|settings|system`, `--open <app id>`, `--search <words>`.
- `--lang <code>`, `--theme light|dark`.
- `--script "<steps>"`: `up down left right ok back tab wait click:X,Y text:WORD install:N`,
  one step every quarter second. A page that loads from the network needs waiting
  for: put about forty `wait` before the first press.
- `--settle <ms>`: how long to wait after the script before the picture (default 600).

## Check all three layouts when a shared piece changed

Desktop, phone and TV share `types.slint` and most of what `App` does. A change
there is tried on each: `--ui desktop`, `--ui mobile --size 412x892`,
`--ui tv --size 1920x1080 --as android-tv`.

## What it cannot show

Sound, the system tray, a real remote control, the sign-in window, and anything
that needs a real desktop session (the password prompt, desktop icons). Say so in
the report instead of implying they were seen.
