# Bilby

An app store that only shows what runs on the device. Rust workspace, Slint interface.
Start with `docs/DEVELOPMENT.md` (code map, running, testing) and `CONTRIBUTING.md`
(how the code and comments are written). This file is only what those do not make
obvious and what has gone wrong before.

## Check before saying something is done

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
python3 scripts/i18n.py --check
```

About three minutes. `cargo test -p bilby --test ui <name>` runs one interface test.
If it can be seen or pressed, also look at it in the real app:
`scripts/try.sh out.png --demo ...` (or the `/try-it` skill), then read the picture.

## Things that have broken a release or CI

- **Android-only code is not compiled on a desktop.** `android_main` in
  `crates/app/src/lib.rs` builds `Options` and `Dirs` by hand. After changing either,
  build it: in `crates/app`, `cargo apk build --lib` (see `docs/ANDROID.md`). CI does
  this too, but only after a push.
- **Tests run on Linux, Windows and macOS.** A test that needs something only one
  system has (Flatpak, a `.desktop` file, `python3`, a colon in a file name) is marked
  `#[cfg(target_os = "linux")]` or `#[cfg(unix)]`.
- **Every text on screen exists in twelve languages.** New or changed text in a
  `.slint` file (`@tr`) or passed through `l(...)` in Rust needs
  `python3 scripts/i18n.py` and then a translation in each `crates/app/i18n/<lang>.json`.
  A one-word Rust text must also be listed in `PUT_TOGETHER` in the script. A literal
  that is not for people gets a `(not shown)` comment on its line.
- **`scripts/i18n.py` stops reading a Rust file at the first `#[cfg(test)]`.** Keep
  `#[cfg(test)] mod tests;` at the bottom of a file.
- **A new field in `Dirs` or `Options`** must be added to every literal: the tests'
  `dirs_in`, `winrun`'s tests, and `android_main`.
- **Imports of items that exist on one system only** carry the same `#[cfg]` as the
  item.
- **`/tmp` here is small and in memory.** Large downloads for a trial go under
  `~/.cache`, not the scratch folder.

## How things are done here

- `crates/core` decides, `crates/app` shows; `.slint` files hold no logic. Something
  new the interface can do: declare it in `ui/types.slint`, connect it in
  `src/wiring.rs`, write the method on `App` in the module it belongs to.
- Comments say why, in plain words, and call whoever uses Bilby "the person". Say
  what was not tried, in square brackets, where code was written without running it
  against the real thing.
- Commit subjects are a short sentence about what is different for someone using
  Bilby. A change they would notice also gets a line under "Unreleased" in
  `CHANGELOG.md`.
- Report honestly what was verified and what was not. Trying it in the real app
  counts; passing tests alone often do not.

## What needs the maintainer's word

- **Pushing, merging and releasing.** Commit locally, on a branch for anything large,
  then ask. Releases are made with the `/release` skill, only when asked for.
- Changing repository settings, secrets, or the Android signing key.
- Anything that sends the maintainer's details, or a real review or sign-in, to an
  outside service.
