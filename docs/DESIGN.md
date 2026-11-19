# untitledstore: design

Statements here are marked by how well they are supported:

- **[verified]** checked in this repository (a test, a build, a screenshot) or read in the
  primary source linked next to it.
- **[sourced]** stated by a linked source that was read, but not independently tested.
- **[assumption]** the author's belief from memory. Not checked. Treat as a to-do.

## 1. What it is

A storefront for software that is distributed as files attached to releases (GitHub
Releases first) and through community repositories (the AUR first), with one rule:

> If it cannot be installed on this machine, it is not shown.

One codebase, in Rust, with three interfaces: desktop (pointer and keyboard), mobile
(touch) and TV (remote / D-pad).

### Prior art, and whether this approach is the right one

Two existing projects already cover much of this idea, and are worth trying before
investing in a new one:

- **GitHub Store** (later renamed Komi Store): an open-source app store for GitHub,
  Codeberg, Forgejo and Gitea releases, built with Kotlin and Compose Multiplatform for
  Android and desktop. It lists a repository only if its latest release has installable
  assets. One page of its GitHub organisation says it was archived in July 2026; that was
  not confirmed elsewhere. **[sourced]**
  ([F-Droid listing](https://f-droid.org/en/packages/zed.rainxch.githubstore),
  [OpenHub-Store org](https://github.com/OpenHub-Store))
- **Obtainium**: installs and updates Android apps straight from their release pages.
  Android only. **[assumption]** on scope; it is referenced by the sources above.

What this design adds: Rust throughout, a TV interface, package-manager awareness on
Linux (deb vs rpm vs pacman vs AUR vs musl), and an explanation for every hidden file.

If the goal is "an app store that filters by my device" and Rust is a preference rather
than a requirement, forking or contributing to an existing project is the cheaper path.
If Rust, a TV interface or the AUR are requirements, a new codebase is justified, and
the compatibility engine below is the part worth getting right first: it is what the
other stores do least thoroughly, and it has no UI dependencies.

## 2. Architecture

```
            ┌────────────────────── crates/app (Slint) ───────────────────────┐
            │  desktop.slint        mobile.slint          tv.slint            │
            │        └──────────── global Store (types.slint) ─────────┘      │
            │              lib.rs: worker threads <-> Store                   │
            └───────────────────────────────┬──────────────────────────────────┘
   crates/cli (ustore) ─────────────────────┤
                                            ▼
┌──────────────────────────────── crates/core ─────────────────────────────────┐
│  catalog   browse / search / details, filtered to this machine               │
│     │                                                                        │
│     ├── index      curated list for Discover (no API calls)                  │
│     ├── source     trait Source ── github.rs, aur.rs  (add: gitlab, ...)     │
│     ├── asset      file name ──► format, OS, CPU, libc, TV                   │
│     ├── compat     (asset, platform) ──► Native / Compatible / Incompatible  │
│     ├── install    (asset, platform) ──► plan; execute plan                  │
│     └── platform   Platform::detect() / Platform::preset(name)               │
│  http      trait Http ── NetHttp (rustls), CachedHttp, DemoHttp              │
└──────────────────────────────────────────────────────────────────────────────┘
```

Rules that keep it honest:

- **The UI holds no logic.** The three interfaces render one `Store` global and call four
  callbacks (`search`, `open`, `close-details`, `install`). Anything a UI shows, the CLI
  can print.
- **`Platform` is a value, not ambient state.** Every decision takes a `&Platform`, so
  "what would an Android TV be offered?" is a function call (`--as android-tv`), and the
  whole engine is testable on one machine. **[verified]** 13 presets, all exercised in
  tests.
- **The network is behind one trait.** `DemoHttp` serves a fictional catalog in the real
  GitHub and AUR JSON shapes, so demo mode runs the production parsing and filtering
  code.

## 3. Knowing the machine (`platform.rs`)

| Fact | How it is detected | Confidence |
|---|---|---|
| OS | compile target | **[verified]** on Linux |
| CPU | compile target; on Windows the `PROCESSOR_ARCHITEW6432` / `PROCESSOR_ARCHITECTURE` variables; on Android the device ABI list | Linux **[verified]**; Windows **[assumption]** and known to be wrong for an x64 build emulated on Arm, which needs `IsWow64Process2` |
| Other CPUs it can run | Windows on Arm: x64 + x86; 64-bit Windows: x86; Apple silicon: x64 (Rosetta 2); Android: the rest of the ABI list; Linux: x86 only if the 32-bit loader exists | **[assumption]** for the emulation facts |
| Linux distro family | `ID` / `ID_LIKE` in `/etc/os-release` | **[verified]** by tests for Ubuntu, Arch derivatives, openSUSE, RHEL clones, Alpine |
| libc | musl loader present and no glibc loader | **[verified]** on glibc only |
| Installable formats | `dpkg`, `rpm`, `pacman`, `flatpak`, `snap`, AUR helper on `PATH` | **[verified]** on Ubuntu |
| Form factor | Android: `ro.build.characteristics` contains `tv`; desktop: gamescope session; otherwise window width | Android part **[assumption]**; `UiModeManager` is the authoritative API and needs JNI |

## 4. Knowing the file (`asset.rs`) and deciding (`compat.rs`)

GitHub releases carry no platform metadata, so the file name is the only signal. The
name is lower-cased and every run of punctuation becomes `_`, so whole tokens can be
matched (`_arm64_`, `_x86_64_`) without `darwin` matching `win`.

Decision order, each step able to reject with a reason shown to the user:

1. **Not an app**: checksums, signatures, sources, debug symbols, update manifests.
   Dropped silently.
2. **Operating system**: from the format (`.apk`, `.msi`, `.dmg`, `.deb`...) or name
   tokens. Unknown OS is rejected rather than guessed.
3. **Form factor**: an Android TV build is hidden on phones.
4. **Package format**: `.deb` needs dpkg, `.rpm` needs rpm, pacman packages and the AUR
   need Arch, AppImage needs glibc.
5. **libc**: a glibc build is rejected on musl; a musl build is fine anywhere.
6. **CPU**: native, or runnable through emulation (shown as "Compatible" with the reason),
   or rejected. A name that states no CPU gets the common-case assumption for its format
   and is ranked below one that does.
7. **Rank**: native over emulated; the distro's own format first (deb on Debian, rpm on
   Fedora, pacman on Arch), then AppImage, Flatpak, AUR build, archive, bare binary; TV
   builds first on TVs.

The top-ranked file becomes the Install button. **[verified]**: one fictional release of
18 files resolves to the expected single file on all 13 presets
(`compat::tests::each_platform_gets_its_natural_package`,
`catalog::tests::details_offer_the_right_file_per_machine`).

Known limits of name-based detection: it is a heuristic. An APK's real ABIs and whether
it declares a TV launcher are inside the file, not its name; reading the APK manifest
after download would be exact. The classifier's vocabulary comes from common naming
conventions (Rust target triples, Go/GoReleaser, Electron, Android ABI names)
**[assumption]**: it was tested on names written for the tests, not on a crawl of real
releases. Running it over a few thousand real releases and reviewing the misses is the
single most valuable next step.

## 5. Sources

`trait Source { search, summary, releases, available_on }`. Adding GitLab, Codeberg /
Forgejo, F-Droid, Flathub or winget means one new file.

### GitHub

REST endpoints: `/search/repositories`, `/repos/{owner}/{repo}`,
`/repos/{owner}/{repo}/releases`. Not yet run against the live API (see README).

**Rate limits shape the whole design.** An unauthenticated client gets 60 requests per
hour per IP address, and search has a separate, stricter limit. Conditional requests
(ETag / 304) are free only when the request is authenticated. **[sourced]**
([GitHub docs: rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api),
[best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api))

"Only show what works here" needs each candidate's release assets, which is one request
per repository. Consequences:

- **Discover costs zero requests.** It is driven by a curated index (below).
- **Search checks a bounded number of hits** (8 without a token, 20 with), in parallel,
  and reports how many matched but had nothing for this device.
- **Responses are cached on disk** for 30 minutes, and a stale copy is served when
  offline or rate limited. **[verified]** by test.
- **A personal token** lifts the limit to 5,000 per hour **[sourced]** (same page). Today
  it is read from `GITHUB_TOKEN`; a sign-in flow (OAuth device flow suits TVs, which have
  no keyboard) belongs on the roadmap. The token is only ever attached to
  `api.github.com` requests. **[verified]** by reading `http.rs`; not covered by a test.
- Next step: with a token, one GraphQL query can fetch the releases of many repositories
  at once. **[assumption]**, not implemented.

Release assets uploaded since June 2025 carry a SHA-256 digest in the API, which the
installer verifies before anything is opened. **[sourced]**
([GitHub changelog](https://github.blog/changelog/2025-06-03-releases-now-expose-digests-for-release-assets/));
the verification itself is **[verified]** by test, including that a mismatching file is
deleted and never opened.

### AUR

The aurweb RPC, version 5: `/rpc/v5/search/{term}?by=name-desc` and
`/rpc/v5/info?arg[]={name}`. **[sourced]**
([ArchWiki: Aurweb RPC interface](https://wiki.archlinux.org/title/Aurweb_RPC_interface))

An AUR package is a build recipe, not a binary. It is modelled as a release with one
asset of format "AUR package", which the compatibility engine allows only on Arch-family
systems. **[verified]** by test. The store does not build it for you: it shows the
command (`paru -S name`, or `git clone` + `makepkg -si` when no helper is installed) and
tells you to read the PKGBUILD, because a recipe is arbitrary code from an unvetted
author. The RPC does not say which CPUs a package supports **[assumption]**, so
that check is left to `makepkg`.

### The index

`crates/core/data/index.json` lists featured apps with the platforms they ship for and,
optionally, the file names of their latest release. With file names, Discover is
filtered by the same engine as installs, offline. The bundled index was written by hand
from memory **[assumption]**: its platform lists are unverified and it has no file
names. The intended design is a scheduled CI job that regenerates it from the APIs (with
a token) and publishes it as a static file, so clients need no API quota to browse.

## 6. Installing (`install.rs`)

`plan()` is pure and shown to the user before anything is downloaded; `execute()` runs
it. The store never runs anything as root itself.

| Format | What happens | State |
|---|---|---|
| AppImage | download, verify, `chmod +x`, move to `~/Applications` | **[verified]** by test and through the UI |
| deb, rpm, pacman package, Flatpak bundle | download, verify, hand to the system's installer (`xdg-open`); terminal equivalent is displayed | hand-off **[verified]** with a recorder; not run on a real desktop |
| exe, msi, msix | download, verify, open | written, **not run on Windows** |
| dmg, pkg | download, verify, open | written, **not run on macOS** |
| archive, bare binary | download, verify, reveal in the downloads folder | **[verified]** |
| AUR | show the command | **[verified]** |
| APK | download, verify, then Android's `PackageInstaller` via JNI | download and verify work; **the hand-off is not implemented**, the UI says so |
| ipa | download, hand to a sideloading app | not implemented beyond download |

Not designed yet: tracking what is installed, updates, uninstall, desktop menu entries
for AppImages, unpacking archives onto `PATH`.

## 7. The three interfaces

All three are Slint components in one binary and are chosen at run time
(`ui/main.slint`): TV if the device is a TV or `--ui tv`; otherwise by window width, so a
phone, a split-screen tablet and a narrow desktop window all get the mobile layout.
**[verified]**: see `docs/screenshots/`, produced by the real binary under a headless
X server with scripted key presses and clicks (`--screenshot`, `--script`).

- **Desktop**: sidebar, always-visible search, card grid that reflows with the window,
  details with every compatible file and a "show the N files that won't work" disclosure.
- **Mobile**: one column, bottom navigation, full-screen details. Respects safe-area
  insets (status bar, gesture bar), which Slint exposes on the window **[verified]** in
  Slint's element definitions; **not run on a phone**.
- **TV**: no pointer. Exactly one focused element, drawn with a thick white outline; rows
  scroll to keep it in view; a hero area describes the focused app in large type; every
  size is a fraction of the screen height so 1080p and 4K look the same. Up from the top
  row focuses search; OK opens; Back returns.

Slint's Android backend maps D-pad up/down/left/right to arrow keys, D-pad centre to
Return, and Back to a Back key **[verified]** by reading
`i-slint-backend-android-activity-1.19.0/androidwindowadapter.rs`. The TV interface
handles exactly those keys, so the same code serves a remote and a keyboard. It has not
been run on a television.

## 8. Platform support

| Platform | Where it stands | What is missing |
|---|---|---|
| Linux | Builds, runs, tested | Packaging (AppImage / Flatpak / AUR package of the store itself) |
| Windows | Core type-checks for `x86_64-pc-windows-msvc` | Build and run the app; precise CPU detection; installer |
| macOS | Core type-checks for `aarch64-apple-darwin` | Build and run; app bundle, signing, notarisation |
| Android phone / tablet | Core type-checks for `aarch64-linux-android`; `android_main` written | NDK build, APK packaging, `REQUEST_INSTALL_PACKAGES` + `PackageInstaller` JNI, TLS roots check |
| Android TV | Same as Android | Also a leanback launcher entry and banner |
| iOS | Core type-checks for `aarch64-apple-ios` | See below |

"Type-checks" means `cargo check -p ustore-core --no-default-features --target ...`
passed here: the core's own code compiles for that target. The network stack and the UI
were not cross-compiled (no cross toolchains in the build environment).

### iOS

Slint can target iOS (a tech preview since 1.12, mid-2025). **[sourced]**
([Slint 1.12 release notes](https://slint.dev/blog/slint-1.12-released))
The obstacle is the platform, not the toolkit:

- An app on iOS cannot install another app. Outside the EU (and, per one report, Japan)
  there is no supported route; sideloading relies on tools such as AltStore that
  re-sign apps with the user's developer certificate from a computer. **[sourced]**
  ([Engadget](https://engadget.com/2315074/how-to-sideload-apps-ios-even-in-united-states/))
- In the EU, alternative marketplaces are allowed on iOS 17.4+, but their apps must be
  notarised by Apple; an arbitrary `.ipa` from a GitHub release does not qualify.
  **[sourced]** ([Apple support](https://support.apple.com/en-mide/118110),
  [TechCrunch](https://techcrunch.com/2024/04/01/the-altstore-an-alternative-app-store-coming-to-eu-will-offer-patreon-backed-apps))

So an iOS build of this store can realistically be a **catalog and companion**: browse,
see which projects ship an `.ipa`, and pass the file to a sideloading tool the user
already has. It is unlikely to be accepted into the App Store. **[assumption]**
A cheaper way to serve iPhone owners is the desktop app offering "send to my phone".

tvOS is not addressed; whether Slint runs there was not checked.

## 9. UI toolkit

Chosen: **Slint 1.19**.

| | Slint | Dioxus | egui |
|---|---|---|---|
| Rendering | Its own, compiled from `.slint` markup | A webview by default; the native renderer is labelled experimental **[sourced]** ([Dioxus README](https://docs.rs/crate/dioxus/latest)) | Immediate mode |
| Android | Yes, since 1.3 **[sourced]** ([Phoronix](https://phoronix.com/news/Slint-1.3-Rust-UI-Toolkit)) | Yes, via webview **[sourced]** | Yes **[assumption]** |
| iOS | Tech preview **[sourced]** | Yes, via webview **[sourced]** | **[assumption]** partial |
| Remote / D-pad | Key events arrive as arrows; focus is ours to manage **[verified]** | Depends on webview focus behaviour, not researched | Not researched |

Why Slint: one renderer on every target (no dependence on the system webview, which on
TVs is old or absent **[assumption]**), explicit control of focus for the TV interface,
and a software renderer that makes headless screenshots trivial.

Costs: custom widgets had to be written (done), no built-in Markdown view used (release
notes are shown as plain text), and **licensing**. Slint is offered under GPLv3, a
commercial licence, or a royalty-free licence that covers desktop, mobile and web
applications, requires attribution, and excludes embedded systems. **[verified]** by
reading `LICENSES/LicenseRef-Slint-Royalty-free-2.0.md` in the crate. Whether a TV
counts as desktop/mobile or as embedded under that text is not obvious, so this
repository declares `GPL-3.0-only`, which is unambiguous. Choosing a permissive licence
for the store would mean either the royalty-free terms (and settling the TV question
with Slint) or a different toolkit; `crates/core` has no Slint dependency and could be
licensed separately.

## 10. Safety

- Files are verified against the publisher's SHA-256 when GitHub provides one, before
  they are opened. This proves the download matches the release, not that the release is
  trustworthy.
- A release is whatever its author uploaded. The store shows the source, author, stars
  or votes and licence, and does not vet anything. A real launch needs a position on
  this: at minimum a first-install warning, ideally signature checks where projects
  publish them.
- Asset names are sanitised before touching the file system **[verified]** by test; repo
  names are validated before being placed in a URL **[verified]** by test.
- No elevation: privileged installs go through the system's own installer.

## 11. Next steps, in order of value

1. Run against the live GitHub and AUR APIs and fix whatever the real responses break.
2. Run the classifier over a large sample of real release file names; fix the misses.
3. Windows and macOS builds in CI.
4. Android: NDK build, APK, `PackageInstaller` hand-off; then Android TV launcher entry.
5. Generated index published as a static file.
6. Installed-app tracking and updates.
7. GitHub sign-in (device flow), light theme, app icons and screenshots from repositories.
8. More sources: Codeberg / Forgejo, GitLab, F-Droid, Flathub.
