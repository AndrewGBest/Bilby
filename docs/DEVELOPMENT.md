# Building, running and working on Bilby

For what Bilby is, start with the [README](../README.md). For why it is built the way
it is, see the [design notes](DESIGN.md).

## What you need

- Rust, from [rustup](https://rustup.rs).
- On Linux, to build: a C toolchain, `pkg-config`, and the development files for
  fontconfig and xkbcommon. On Debian or Ubuntu:

  ```sh
  sudo apt install build-essential pkg-config libfontconfig1-dev libxkbcommon-dev
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
everything on Linux, Windows and macOS. More in [Testing](DESIGN.md#8-testing).

Before committing:

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

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

Releases are built by the `Release` workflow. Start it from the Actions page with a
version such as `v0.2.0`; it builds each system's files, creates the tag and publishes
the release. Bump `version` in `Cargo.toml` first, and wait for CI to pass.

## Where the code is

```
crates/core   bilby-core  platform detection, file classification, compatibility,
                          sources, search ranking, installing, library state,
                          system updates
crates/cli    bilby-cli   command-line front end
crates/app    bilby       Slint app: ui/desktop.slint, ui/mobile.slint, ui/tv.slint
packaging/                the icon, the Linux menu entry and AppImage script, the macOS bundle
docs/DESIGN.md            the design, its trade-offs, and what is still open
docs/COMPARISON.md        Bilby next to other stores, feature by feature
```

## Where Bilby keeps things

| | Linux | Windows | macOS |
|---|---|---|---|
| Settings and library | `~/.config/bilby/state.json` | `%APPDATA%\bilby` | `~/Library/Application Support/bilby` |
| Downloads | `~/Downloads/bilby` | same | same |
| AppImages | `~/Applications` | | |
| Cache | `~/.cache/bilby` | `%LOCALAPPDATA%\bilby` | `~/Library/Caches/bilby` |
| Its own menu entry | `~/.local/share/applications/bilby.desktop` | | |

Only the Linux column has been tried.
