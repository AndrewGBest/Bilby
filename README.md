<p align="center"><img src="crates/app/ui/icons/logo.svg" width="96" alt="Bilby logo: a long-eared bilby face"></p>

# Bilby

An app store for GitHub Releases, Codeberg and other repositories that only shows you
what runs on your device.

Open-source apps usually publish a page of download files and leave you to work out
which one is yours. Bilby works it out for you. A release with 18 files becomes one
**Install** button, and the other 17 stay out of the way unless you ask for them.

There is no account, no Bilby server and no tracking. The app talks to the sources
directly and keeps your library in one file on your device.

| Desktop | Mobile | TV |
|---|---|---|
| ![Desktop](docs/screenshots/desktop-discover.png) | ![Mobile](docs/screenshots/mobile-details.png) | ![TV](docs/screenshots/tv-home.png) |

More screenshots are in [`docs/screenshots/`](docs/screenshots). They show the built-in
demo catalog; the apps in it are fictional.

## Where apps come from

- **GitHub Releases** and **Codeberg**.
- **Your Linux distribution's repositories**: everything pacman is set up for (Arch,
  CachyOS, Chaotic-AUR, your own) or everything apt is set up for (Debian, Ubuntu,
  PPAs).
- **The AUR**, on Arch-based systems only.
- **Websites**, on Linux TVs only: add a site and it opens like an app in
  [Longboard](https://github.com/AndrewGBest/Longboard), a browser made for remotes.

A source only appears on devices it applies to. A Windows user never sees the AUR.

## What it does

**Finding apps**

- Discover lists hand-picked apps by category and works offline.
- Search puts the best match first: the app with the name you typed, then the apps most
  people use. You can also paste a project link.
- Each app's page shows pictures and a description taken from the project itself.
- Lists have an Install button on every app.

**Installing**

- Bilby picks the right file for your system, processor and device type.
- Downloads are checked against the checksum the source publishes, when it publishes one.
- It never asks for your password or runs anything as administrator. Packages are handed
  to your system's installer; AUR and repository packages give you the command to run.
- On Linux, AppImages are added to your app menu.

**Keeping up to date**

- The Library lists what you installed and tells you when something newer is out.
- Update one app or all of them. Skip an update you do not want.
- Older versions can still be installed from an app's page.
- Bilby looks for its own updates the same way.

**Familiar, and not in your face**

An app's page is laid out like the app stores people already use: name and one button
at the top, a row of key facts (popularity, size, last updated), what the app is, what
is new, and an Information table at the bottom. File names, commands, other install
options, older versions and the reason each hidden file was hidden are one press away
under *More options and details*.

**Also**

- One app, three interfaces: desktop (mouse and keyboard), mobile (touch) and TV
  (remote only). Light and dark themes.
- Favourites: star an app to keep it in your Library without installing it.
- `bilby-cli` does everything from a terminal, and can export your library to move it
  to another device.

## Status

Bilby works, but it is early: **there is no release to download yet**, and it has only
ever been run on Linux.

| | State |
|---|---|
| Linux | Builds, runs, all 196 automated tests pass |
| Windows, macOS | Builds and passes the tests on GitHub's machines. Nobody has seen the window on either yet |
| Android (phone, tablet, TV) | The engine compiles. There is no installable package and APKs cannot be installed yet |
| iOS | Can only ever browse, not install: see [the design notes](docs/DESIGN.md#ios) |
| GitHub, AUR, Codeberg | Tested against local stand-ins built from their documentation, never against the real services |
| apt repositories | Read from real Ubuntu 24.04 package lists |
| pacman repositories | Tested against databases built in the tests; never read on a real Arch or CachyOS system |
| Fedora, openSUSE, Flathub, winget, Homebrew | Not yet |

Nothing in the catalog is reviewed by anyone. Bilby warns about pre-releases, abandoned
projects and out-of-date AUR packages, but an app is only as trustworthy as its author.

[How Bilby compares with Komi Store, Obtainium, UniGetUI and others](docs/COMPARISON.md),
and what it leaves out on purpose.

## Try it

```sh
# The store, with the offline demo catalog
cargo run -p bilby -- --demo

# See what another device would be offered
cargo run -p bilby -- --demo --as android-tv --ui tv
cargo run -p bilby -- --demo --as windows-arm64
cargo run -p bilby -- --demo --as android-arm64 --ui mobile --size 412x892

# The same engine without a window
cargo run -p bilby-cli -- device
cargo run -p bilby-cli -- --demo --as linux-x64-arch search lumen
cargo run -p bilby-cli -- --demo --as macos-arm64 show github:kestrel-sh/kestrel
cargo run -p bilby-cli -- --demo install github:tidewave/tidewave
cargo run -p bilby-cli -- --demo updates
cargo run -p bilby-cli -- classify ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz

# Against the real sources: drop --demo.
cargo run -p bilby-cli -- search ripgrep

# Run this first on a real connection. It checks that the sources answer as expected
# and that every featured app has a file for this machine.
cargo run -p bilby-cli -- doctor
```

On desktop, `Ctrl+K` or `/` starts a search and `Esc` or `Alt+Left` goes back. Make the
window narrower than 700 px and it becomes the mobile interface. TV mode uses the arrow
keys, Enter and Escape, which is what a remote sends.

Bilby reads `GITHUB_TOKEN` (or a token saved in Settings) to raise GitHub's limit from
60 to 5,000 requests an hour, and `HTTPS_PROXY` / `ALL_PROXY` if you are behind a proxy
(untested behind a real one).

To build on Linux you need a C toolchain and `fontconfig`. To run it you need
`libxkbcommon` (and `libxkbcommon-x11` on X11).

## Tests

```sh
cargo test --workspace
```

The tests need no internet. Network tests start their own local server, and interface
tests drive the real window without a display. On Linux, seven more run the finished
program under `xvfb-run` and are skipped if it is not installed. CI runs everything on
Linux, Windows and macOS. More in [Testing](docs/DESIGN.md#8-testing).

## Working on it

```sh
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

The app can be driven by a script, which is how the screenshots were made:

```sh
cargo run -p bilby -- --demo --as android-tv --ui tv --size 1920x1080 \
    --script "down right ok" --screenshot tv.png --report tv.txt
```

`--script` replays keys (`up down left right ok back tab`), `click:X,Y`, `text:WORD`,
`install:N` and `wait`, one step every quarter second. `--report` writes what ended up
on screen as `key=value` lines. Without a display, run it under `xvfb-run` with
`SLINT_BACKEND=winit-software`.

`--as <platform>` makes the app behave as another device, which is the quickest way to
see what a phone, a TV or a Windows-on-Arm laptop would be offered. `bilby-cli
--list-platforms` lists the choices.

## Layout

```
crates/core   bilby-core  platform detection, asset classification, compatibility,
                          sources, search ranking, install, library state
crates/cli    bilby-cli   command-line front end
crates/app    bilby       Slint app: ui/desktop.slint, ui/mobile.slint, ui/tv.slint
docs/DESIGN.md            the design, its trade-offs, and what is still open
docs/COMPARISON.md        Bilby next to other stores, feature by feature
```

## Where things are stored

| | Linux | Windows | macOS |
|---|---|---|---|
| Settings and library | `~/.config/bilby/state.json` | `%APPDATA%\bilby` | `~/Library/Application Support/bilby` |
| Downloads | `~/Downloads/bilby` | same | same |
| AppImages | `~/Applications` | | |
| Cache | `~/.cache/bilby` | `%LOCALAPPDATA%\bilby` | `~/Library/Caches/bilby` |

Only the Linux column has been tried.

## License

`GPL-3.0-only` for now. It was chosen as a starting point because the interface toolkit,
Slint, is available under GPLv3 among other options. Read
[the licensing note](docs/DESIGN.md#10-ui-toolkit) before changing it.
