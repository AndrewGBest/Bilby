# Contributing to Bilby

Thank you for wanting to help. Bilby is young and has one maintainer, so small, clear
changes land fastest. This page says how to get one in.

- [Ways to help](#ways-to-help)
- [Before you start](#before-you-start)
- [Set up](#set-up)
- [Make the change](#make-the-change)
- [How the code is written](#how-the-code-is-written)
- [Text in the interface](#text-in-the-interface)
- [Tests](#tests)
- [Commits and pull requests](#commits-and-pull-requests)
- [Using AI tools](#using-ai-tools)
- [Licence](#licence)

## Ways to help

You do not need to write Rust to be useful.

- **Try it and say what happened.** Much of Bilby is written and tested but has not
  been run on a real Windows PC, Mac, phone or TV. The
  [README](README.md#how-ready-is-it) lists what has been tried. A report that says
  "installing from winget worked on Windows 11" is as valuable as a bug.
- **Report a wrong or missing app.** Bilby works out which file fits a device from the
  file's name. When it gets one wrong, [tell us](https://github.com/AndrewGBest/Bilby/issues/new/choose);
  the fix is usually one line and one test.
- **Check a translation.** The eleven translations have not been read by native
  speakers. See [Text in the interface](#text-in-the-interface).
- **Fix something.** Issues labelled
  [good first issue](https://github.com/AndrewGBest/Bilby/labels/good%20first%20issue)
  are small and self-contained.

## Before you start

- **For a bug fix, a wrong file choice or a translation:** go ahead and open a pull
  request.
- **For a new feature, a new source or a change to how something looks:** open an
  issue or a [discussion](https://github.com/AndrewGBest/Bilby/discussions) first.
  Bilby leaves some things out on purpose (no server, no account, no tracking, no
  list of approved apps), and [the comparison](docs/COMPARISON.md) and
  [design notes](docs/DESIGN.md) explain the choices already made. It is quicker to
  agree on the idea before the code exists.
- **For a security problem:** do not open an issue. See [SECURITY.md](SECURITY.md).

## Set up

[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) has everything: what to install, how to
run the store against the demo catalog, how to see it as another device, and how to
drive it from a script. The short version:

```sh
cargo run -p bilby -- --demo          # the store, with made-up apps and no network
cargo test --workspace                # every test; none need the internet
```

## Make the change

1. Fork the repository and make a branch from `main`.
2. Make the change, with a test that fails without it.
3. Run the checks CI runs:

   ```sh
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   python3 scripts/i18n.py --check     # only matters if interface text changed
   ```

4. Try it in the real app when the change can be seen or pressed. `--demo` is enough
   for most things.
5. Open a pull request and fill in the template. Say plainly what you did not try.

## How the code is written

[docs/DESIGN.md](docs/DESIGN.md#2-architecture) describes the structure. These are the
rules a change is reviewed against.

**Where things go**

- `crates/core` decides; `crates/app` shows. The core has no interface code and must
  keep compiling for Android and iOS. Anything the interface shows, `bilby-cli` should
  be able to print.
- The `.slint` files hold layout and no logic. They read properties of the global
  `Store` (`ui/types.slint`) and call its callbacks; the Rust side fills them in.
- Every decision about a device takes a `&Platform`. Nothing asks the running system
  directly outside `platform.rs`, so every device can be tested on one machine.
- Everything on the network goes through the `Http` trait, so tests and the demo
  never touch the internet.
- A new source is one file in `crates/core/src/source/` implementing `Source`, plus
  its answers in `demo.rs` so the demo and the tests cover it.

**Style**

- `cargo fmt` decides formatting (`rustfmt.toml`: 120 columns). Clippy must be clean.
- No `unwrap()` or `expect()` on anything that comes from outside the program: a
  file, the network, another program's output. Return `bilby_core::Result` and let
  the interface show the message. They are fine in tests and for things that cannot
  fail by construction.
- Error messages are read by people. Write a sentence that says what happened and,
  where it helps, what to do about it.
- Prefer a plain function to a new trait, and a new function to a new parameter that
  only one caller uses.
- Add a dependency only when it earns its place, and say why in a comment above it in
  `Cargo.toml`, as the existing ones do. CI checks every dependency's licence against
  `deny.toml`.

**Comments**

- A comment says *why*: the constraint, the odd case, where a format was learnt from.
  It does not repeat what the code says, and it does not tell the history of the
  change (that is what the commit message is for).
- Every module starts with a `//!` paragraph saying what it is for. Public items in
  the core have a `///` line.
- Be honest about what has been tried. Where code was written from documentation and
  never run against the real thing, say so in square brackets, as the code already
  does: `[Run for real on Debian 13. Not tried on Arch.]`
- Write "the person" for whoever is using Bilby, and plain words throughout: the
  comments are read by people who do not know the code yet.

## Text in the interface

Bilby speaks twelve languages, and every sentence it can show must exist in all of
them.

- Use the words people already use: "Install", "Library", "Update". Say what a thing
  does, not how it works.
- In `.slint` files wrap text in `@tr("...")`. In Rust, pass it through `l(...)`.
- Then run `python3 scripts/i18n.py`. It gathers every text, lists the ones that have
  no translation yet, and rewrites the files in `crates/app/i18n/`. Add the missing
  translations to each `<language>.json` and run it again until
  `python3 scripts/i18n.py --check` passes.
- If you cannot translate into a language, use your best attempt and say so in the
  pull request. A reviewer or a native speaker can correct it.

To fix an existing translation, edit `crates/app/i18n/<language>.json` and run the
script.

## Tests

- Every test runs without the internet. Network code is tested against a local
  server (`crates/core/tests/http_sources.rs`); the interface is tested by pressing
  its real buttons on a headless window (`crates/app/tests/ui.rs`).
- A fix for a wrongly chosen file adds the file's name to
  `crates/core/tests/corpus.rs`.
- A change to the interface adds or extends a test in `crates/app/tests/ui.rs` that
  finds controls by their accessible label, the way a screen reader would.
- CI runs the tests on Linux, Windows and macOS, and builds the Android app. Mark a
  test that depends on one system with `#[cfg(target_os = "...")]`.

## Commits and pull requests

- One change per pull request. A fix and an unrelated tidy-up are two.
- Write the commit subject as a short sentence about what is different for someone
  using Bilby: "A web app opens in front when Longboard is already running", not
  "fix focus bug". Put the reasoning in the body.
- Add a line to the "Unreleased" section of [CHANGELOG.md](CHANGELOG.md) when the
  change is one a person using Bilby would notice.
- Pull requests are squashed when merged, so you need not tidy the commits inside
  one.
- CI must pass. If it fails for a reason that is not yours, say so and we will sort
  it out.

## Using AI tools

You may use them; much of Bilby was written with one, directed and tried by the
maintainer. The same rule applies to everyone:

- You are the author of what you submit. Read all of it, understand it, and be able
  to explain and change it when a reviewer asks.
- Run it. A change that has not been run is not ready, whoever wrote it.
- Say in the pull request that a tool was used, and for which parts.
- Do not paste in code whose origin or licence you cannot name.

Pull requests that are plainly unread tool output will be closed.

## Licence

Bilby is licensed under [GPL-3.0-only](LICENSE). By contributing you agree that your
contribution is under the same licence, and that you have the right to give it. There
is no contributor agreement to sign.

Everyone taking part is expected to follow the [code of conduct](CODE_OF_CONDUCT.md).
