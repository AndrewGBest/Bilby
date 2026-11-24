# Building, running and working on Bilby

For what Bilby is, start with the [README](../README.md). For why it is built the way
it is, see the [design notes](DESIGN.md).

## What you need

- Rust, from [rustup](https://rustup.rs).
- On Linux, to build: a C toolchain, `pkg-config`, and the development files for
  fontconfig and xkbcommon. On Debian or Ubuntu:

  ```sh
  sudo apt install build-essential pkg-config libfontconfig1-dev libxkbcommon-dev libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev
  ```

- On Linux, to run: `libxkbcommon` (and `libxkbcommon-x11` on X11).

## Run it

```sh
# The store, against the real sources
cargo run --release -p bilby

# The store, with the offline demo catalog (made-up apps, nothing is installed)
cargo run -p bilby -- --demo
```

### Try it as another device

`--as <platform>` makes Bilby behave as another device, which is the quickest way to
see what a phone, a TV or a Windows-on-Arm laptop would be offered.
`bilby-cli --list-platforms` lists the choices.

```sh
cargo run -p bilby -- --demo --as android-tv --ui tv
cargo run -p bilby -- --demo --as windows-arm64
cargo run -p bilby -- --demo --as android-arm64 --ui mobile --size 412x892
```

A demo or an `--as` run never looks at or changes the real system: the device page and
system updates use a made-up machine.

### From a terminal

```sh
cargo run -p bilby-cli -- device                 # what was detected about this machine
cargo run -p bilby-cli -- search ripgrep         # search the real sources
cargo run -p bilby-cli -- show github:sharkdp/fd # the files this machine can install
cargo run -p bilby-cli -- classify ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz
cargo run -p bilby-cli -- doctor                 # check that the sources answer as expected
```

Add `--demo` to any of them to use the demo catalog instead of the network.

### Keys

| Layout | Keys |
|---|---|
| Desktop | `Ctrl+K` or `/` starts a search. `Esc` or `Alt+Left` goes back |
| Phone | A window narrower than 700 px becomes the phone layout |
| TV | Arrow keys, Enter and Escape, which is what a remote sends |

### Settings from the environment

- `GITHUB_TOKEN` (or a token saved in Settings) raises GitHub's limit from 60 to 5,000
  requests an hour.
- `HTTPS_PROXY` / `ALL_PROXY` are honoured (untested behind a real proxy).

## Test it

```sh
cargo test --workspace
```

The tests need no internet. Network tests start their own local server, and interface
tests drive the real window without a display. On Linux, a handful more run the
finished program under `xvfb-run` and are skipped if it is not installed. CI runs
everything on Linux, Windows and macOS, and builds the Android app. More in [Testing](DESIGN.md#8-testing).

Before committing, run what CI runs:

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
python3 scripts/i18n.py --check
```

[CONTRIBUTING.md](../CONTRIBUTING.md) says how changes are reviewed and how the code
is written.

## Drive it with a script

The app can be driven by a script, which is how the screenshots are made:

```sh
cargo run -p bilby -- --demo --as android-tv --ui tv --size 1920x1080 \
    --script "down right ok" --screenshot tv.png --report tv.txt
```

- `--script` replays keys (`up down left right ok back tab`), `click:X,Y`, `text:WORD`,
  `install:N` and `wait`, one step every quarter second.
- `--report` writes what ended up on screen as `key=value` lines.
- `--page discover|search|library|settings|system` and `--open <app id>` start
  somewhere other than the home page.
- Without a display, run it under `xvfb-run` with `SLINT_BACKEND=winit-software`.

In the demo catalog the made-up authors have no pictures, so tiles show initials until
an app's page has been opened once and its logo cached.

## Release it

Releases are built by the `Release` workflow.

1. Move the "Unreleased" lines of [CHANGELOG.md](../CHANGELOG.md) under a heading for
   the new version, with today's date. That section becomes the release's notes.
2. Change `version` in `Cargo.toml`, run `cargo update --workspace --offline` so
   `Cargo.lock` follows, and commit both.
3. Push, and wait for CI to pass.
4. Start the workflow from the Actions page (or `gh workflow run release.yml -f
   version=v0.2.0`). It checks that the version matches `Cargo.toml`, builds each
   system's files, creates the tag and publishes the release with a `SHA256SUMS` file
   and a signed record of where the files were built.

Published releases and their tags cannot be changed afterwards. A release that went
wrong is followed by a new one.

## Where the code is

```
crates/core     bilby-core    everything Bilby knows and decides, with no interface
  src/platform.rs               what this device is
  src/asset.rs, compat.rs       what a file is, and whether it runs here
  src/source/                   one file per place apps come from
  src/catalog.rs                search, browse and details across the sources
  src/merge.rs, rank.rs         one app from several sources; search order
  src/install/, needs.rs        planning an install and carrying it out
  src/winrun.rs                 Windows programs on Linux
  src/os.rs                     the operating system's own updates
  src/state.rs                  settings and the library, one file on disk
  src/demo.rs                   the made-up catalog behind --demo and the tests
  data/                         the lists the store carries (see data/SOURCES.md)
crates/app      bilby         the window
  ui/types.slint                `Store`: everything the layouts show and can ask for
  ui/desktop.slint, mobile.slint, tv.slint    the three layouts
  ui/widgets.slint, lists.slint, details.slint, library.slint, settings.slint,
  ui/overlays.slint             what the desktop and phone layouts share
  src/lib.rs                    `App`, the controller, and its state
  src/wiring.rs                 connects each callback of `Store` to a method of `App`
  src/search.rs, details.rs, installing.rs, library.rs, updates.rs, settings.rs, ...
                                what `App` does, one file per part of the store
  i18n/                         the translations (see CONTRIBUTING.md)
  tests/                        the interface tests
crates/cli      bilby-cli     the same store from a terminal
crates/signin   bilby-signin  the small browser window for signing in to a store
packaging/                    icons, the Linux menu entry and AppImage script, the macOS bundle
scripts/i18n.py               gathers the interface's text for translating
```

To add a source, start from a small one such as `crates/core/src/source/snap.rs`. To
add something the interface can do: declare the property or callback in
`ui/types.slint`, use it in the layouts, connect it in `src/wiring.rs`, and write the
method in the module it belongs to.

More to read:

| | |
|---|---|
| [DESIGN.md](DESIGN.md) | The design, its trade-offs, and what is still open |
| [COMPARISON.md](COMPARISON.md) | Bilby next to other stores, feature by feature |
| [ANDROID.md](ANDROID.md) | Building and running the Android package |
| [LONGBOARD.md](LONGBOARD.md) | What Bilby asks of the Longboard browser on a TV |
| [OWN-SOURCES.md](OWN-SOURCES.md) | The format of a list of apps on your own server |

## Where Bilby keeps things

| | Linux | Windows | macOS |
|---|---|---|---|
| Settings and library | `~/.config/bilby/state.json` | `%APPDATA%\bilby` | `~/Library/Application Support/bilby` |
| Downloads | `~/Downloads/bilby` | same | same |
| AppImages | `~/Applications` | | |
| Cache | `~/.cache/bilby` | `%LOCALAPPDATA%\bilby` | `~/Library/Caches/bilby` |
| Its own menu entry | `~/.local/share/applications/bilby.desktop` | | |

Only the Linux column has been tried.
