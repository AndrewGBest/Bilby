# untitledstore: design

Statements here are marked by how well they are supported:

- **[verified]** checked in this repository (a test, a build, a screenshot) or read in the
  primary source linked next to it.
- **[sourced]** stated by a linked source, but not independently tested. Most of these
  were read as search-result extracts rather than full pages, so follow the link before
  relying on one.
- **[assumption]** the author's belief from memory. Not checked. Treat as a to-do.

## 1. What it is

A storefront for software that is distributed as files attached to releases (GitHub
Releases first) and through community repositories (the AUR first), with one rule:

> If it cannot be installed on this machine, it is not shown.

One codebase, in Rust, with three interfaces: desktop (pointer and keyboard), mobile
(touch) and TV (remote / D-pad).

### Entirely client-side

There is no untitledstore server, account, telemetry or hosted catalog, and the design
must not grow one. The app talks only to the sources themselves (`api.github.com`,
`github.com` downloads, `aur.archlinux.org`), and everything it remembers lives in one
file on the device (`state.json`: settings, library, recent searches). Consequences that
shape the rest of this document:

- The Discover catalog is compiled into the binary and updates with the app.
- GitHub's per-device request allowance is the scarce resource, so the app caches,
  bounds how much each search checks, and shows the remaining allowance in Settings.
- Update checks, library and preferences are local; nothing syncs between devices.
- **[verified]** by test: the GitHub token is sent to the API host and to no other
  (`the_token_goes_to_the_api_and_nowhere_else`), and the state file is private to the
  user (mode 600 on Unix).

### What was borrowed from existing stores

| Idea | Seen in | Here |
|---|---|---|
| Library with update checks against the source | Obtainium, F-Droid | Library page, badge, start-up check |
| Paste a repository link to add an app | Obtainium | Search field accepts links and `owner/repo` |
| One prominent install button, details below | Flathub, GNOME Software, Play Store | Best option first, the rest listed under it |
| Say what a package can and cannot be trusted for | F-Droid's anti-features | First-run notice, checksum line, AUR warning |
| Show the steps before doing them | Linux package managers | "What will happen?" on each option |
| Focus-driven rows with a hero area | Android TV launcher, Steam Big Picture | TV interface |
| Spotlight and category chips | Most stores | Discover page |

These are **[assumption]**: attributions from the author's familiarity with those apps,
not from a review of their current versions.

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
│     ├── platform   Platform::detect() / Platform::preset(name)               │
│     ├── state      settings, library, recent searches (one local JSON file)  │
│     └── version    is this release newer than the installed one?             │
│  http      trait Http ── NetHttp (rustls), CachedHttp, DemoHttp              │
└──────────────────────────────────────────────────────────────────────────────┘
```

Rules that keep it honest:

- **The UI holds no logic.** The three interfaces render one `Store` global and call its
  callbacks (`search`, `open`, `install`, `check-updates`, ...). Anything a UI shows, the
  CLI can print.
- **One controller, two execution modes.** `App::job` runs catalog work on a worker
  thread in the app and inline under test, so the UI tests exercise the real handlers.
- **`Platform` is a value, not ambient state.** Every decision takes a `&Platform`, so
  "what would an Android TV be offered?" is a function call (`--as android-tv`), and the
  whole engine is testable on one machine. **[verified]** 13 presets, all exercised in tests.
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
3. **Form factor**: an Android TV build is hidden on phones. Debug-signed APKs are kept
   (some projects publish nothing else) but ranked below release builds.
4. **Package format**: `.deb` needs dpkg, `.rpm` needs rpm, pacman packages and the AUR
   need Arch, AppImage needs glibc.
5. **libc**: a glibc build is rejected on musl; a musl build is fine anywhere.
6. **CPU**: native, or runnable through emulation (shown as "Compatible" with the reason),
   or rejected. A name that states no CPU gets the common-case assumption for its format
   and is ranked below one that does.
7. **Rank**: native over emulated; the distro's own format first (deb on Debian, rpm on
   Fedora, pacman on Arch), then AppImage, Flatpak, AUR build, archive, bare binary; TV
   builds first on TVs.

The top-ranked file becomes the Install button. **[verified]**: a made-up release of 17
files resolves to the expected single file on all 13 presets
(`compat::tests::each_platform_gets_its_natural_package`), and the same holds through
the whole catalog path, from API-shaped JSON to install option, on 7 of them
(`catalog::tests::details_offer_the_right_file_per_machine`).

Known limits of name-based detection: it is a heuristic. An APK's real ABIs and whether
it declares a TV launcher are inside the file, not its name; reading the APK manifest
after download would be exact.

The classifier is checked against a corpus of 169 file names in the styles of Rust
target triples, GoReleaser, Electron and Tauri bundlers, Android ABI splits and distro
packages (`crates/core/tests/corpus.rs`). Writing it found 14 misclassifications, since
fixed, including `arm-64` being read as 32-bit Arm. The corpus was written from memory
of those conventions **[assumption]**, not fetched from live releases, so it is a
regression suite, not proof about any project's current files. Known misses it records:
archives that name no system (`obsidian-1.6.7.tar.gz`, `MoonlightPortable-x64.zip`) are
hidden rather than guessed. Running the classifier over a crawl of real releases and
reviewing the misses remains the most valuable next step.

A property test over every corpus name, every preset and 20,000 generated names checks
that nothing is ever offered for another operating system or CPU, and that every hidden
file has a reason. **[verified]**

## 5. Sources

`trait Source { search, summary, releases, available_on }`. Adding GitLab, Codeberg /
Forgejo, F-Droid, Flathub or winget means one new file.

### GitHub

REST endpoints: `/search/repositories`, `/repos/{owner}/{repo}`,
`/repos/{owner}/{repo}/releases`. Not yet run against the live API. The client is tested
over real HTTP against a local server answering in the documented shapes
(`crates/core/tests/http_sources.rs`); the fixtures were written from the API
documentation **[assumption]**, not captured from GitHub.

**Rate limits shape the whole design.** An unauthenticated client gets 60 requests per
hour per IP address, and search has a separate, stricter limit. Conditional requests
(ETag / 304) are free only when the request is authenticated. **[sourced]**
([GitHub docs: rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api),
[best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api))

"Only show what works here" needs each candidate's release assets, which is one request
per repository. Consequences:

- **Discover costs zero requests.** It is driven by the bundled index (below).
- **The allowance is visible.** GitHub reports what is left with every response; Settings
  shows it ("42 of 60 GitHub requests left; refills in 23 minutes"), and a rate-limit
  error says how to raise the limit. **[verified]** by test.
- **Search checks a bounded number of hits** (8 without a token, 20 with), in parallel,
  and reports how many matched but had nothing for this device. Typing costs nothing:
  suggestions come from the bundled index. An AUR search is a single request, because
  its results already carry everything needed. **[verified]** by test.
- **The start-up update check can be switched off** in Settings (one request per
  library app).
- **Responses are cached on disk** for 30 minutes, and a stale copy is served when
  offline or rate limited. **[verified]** by test.
- **A personal token** lifts the limit to 5,000 per hour **[sourced]** (same page). It can
  be pasted into Settings (stored in the local state file) or given as `GITHUB_TOKEN`.
  The OAuth device flow would suit TVs, which have no keyboard, but needs a registered
  OAuth app; not done. The token is only ever attached to API-host requests.
  **[verified]** by test.
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

### App pictures

GitHub projects have no icon in the API. The store shows the owner's avatar, which for
organisations is normally the project's logo, from `https://github.com/<owner>.png`.
That is an ordinary web request to GitHub, not an API call. It is expected not to count
against the API allowance and to redirect to the avatar host **[assumption]**: the URL
form is widely used but was not exercised here, since GitHub is unreachable from the
build environment. Pictures are fetched in the background after each page of results,
decoded by what the bytes are rather than what the URL says (avatars are often JPEGs
behind a `.png` address), and cached under the owner's name; "this owner has none" is
cached too. Until a picture arrives, or if there is none, the tile shows the app's
initials. **[verified]** by tests with a stand-in server; never against GitHub.
The demo catalog has no pictures. AUR packages have none.

### The index

`crates/core/data/index.json` lists featured apps with the platforms they ship for and,
optionally, the file names of their latest release. With file names, Discover is
filtered by the same engine as installs, offline, and can say what would be installed
("Debian package · x86_64") before any request. The bundled index was written by hand
from memory **[assumption]**: its platform lists are unverified and it has no file
names.

In keeping with the client-side rule, the index ships inside the binary. A build-time
script that regenerates it from the APIs (and records file names) is the intended way to
keep it fresh; a hosted, separately updated catalog is deliberately not part of the
design.

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

Downloads can be cancelled; a cancelled or failed download leaves no file behind.
**[verified]** by test, over real HTTP.

### Library and updates

Every install except an AUR build is recorded in the library with its version and what
was installed. "Check for updates" (also run quietly at start-up) asks each source for
the newest release that has a file for this machine and compares versions
(`version.rs`: numeric components, pre-releases sort below their release, never offers a
downgrade). One request per app, cached for 30 minutes. **[verified]** by tests at the
core, CLI and UI level.

What the library is not: the store does not ask the system what is installed. It knows
what it installed. A package handed to the system installer is recorded even if the user
cancels that installer, and removing an app from the library only deletes the file for
AppImages the store placed itself; for everything else it says to use the system's
software manager. Reading the system's package database, and Android's package list,
would close that gap.

App pages list every recent release (up to six) that has a file for this machine, so an
older version can be chosen when an update breaks something; the page says it is not the
newest and the library shows the pending update straight away. **[verified]** by test.

Not designed yet: desktop menu entries for AppImages, unpacking archives onto `PATH`.

## 7. The three interfaces

All three are Slint components in one binary and are chosen at run time
(`ui/main.slint`): TV if the device is a TV or `--ui tv`; otherwise by window width, so a
phone, a split-screen tablet and a narrow desktop window all get the mobile layout.
**[verified]**: see `docs/screenshots/`, produced by the real binary under a headless
X server with scripted key presses and clicks (`--screenshot`, `--script`).

- **Desktop**: sidebar (Discover, Search, Library, Settings), always-visible search, a
  spotlight and category chips, a card grid that reflows with the window, details with
  every compatible file and a "show the N files that won't work" disclosure. Fully usable
  from the keyboard: Ctrl+K or `/` to search, Escape or Alt+Left to go back, Tab through
  every control with a visible focus ring, Enter or Space to activate. Browse pages keep
  their scroll position while an app page is open.
- **Mobile**: one column, four-tab bottom navigation, full-screen details. Android's Back
  button leaves the app page. Respects safe-area insets (status bar, gesture bar), which
  Slint exposes on the window **[verified]** in Slint's element definitions; **not run
  on a phone**.
- **TV**: no pointer. Exactly one focused element, drawn with a thick white outline; rows
  scroll to keep it in view; a hero area describes the focused app in large type; every
  size is a fraction of the screen height so 1080p and 4K look the same. The library is
  the first row, with updates marked. Up from the top row focuses search; OK opens,
  installs, cancels a download or retries a failed lookup; Back returns. There is no
  Settings page on TV yet (a token cannot sensibly be typed with a remote).

Shared by all three: light and dark themes (following the system by default; the TV
interface is always dark), a first-run notice that says what the store is and that
nothing is reviewed, toasts for feedback, empty and error states with a way forward, and
accessible roles, labels and default actions on every control, which is also how the
tests find and operate them.

Slint's Android backend maps D-pad up/down/left/right to arrow keys, D-pad centre to
Return, and Back to a Back key **[verified]** by reading
`i-slint-backend-android-activity-1.19.0/androidwindowadapter.rs`. The TV interface
handles exactly those keys, so the same code serves a remote and a keyboard. It has not
been run on a television.

## 8. Testing

146 tests, none needing the internet. **[verified]**: all pass on Linux here. CI runs
the suite on Linux, Windows and macOS on every push. It passed on all three for the
version-picker commit (run 13). The run after that failed on Windows only, because a
test clicked a button that was below the window there; the test helper was fixed. Check
the Actions tab for the state of the latest commit rather than trusting this paragraph.

| Suite | Count | What it exercises |
|---|---|---|
| `ustore-core` unit tests | 66 | detection, classification, compatibility, versions, state file, install plans and execution, index, suggestions, icons, catalog |
| `core/tests/corpus.rs` | 4 | 169 real-world-style names; cross-OS/CPU safety property; 20,000 generated names |
| `core/tests/http_sources.rs` | 16 | the real HTTP client and both sources against a local server: headers, token scoping, rate limits, allowance, progress, cancel, cache, documented JSON shapes, garbage responses |
| `cli/tests/cli.rs` | 8 | the `ustore` binary end to end, including install, library, updates and `doctor` |
| `app/tests/ui.rs` | 34 | the real window on Slint's headless backend: clicks by accessible label, key presses, all three interfaces |
| `app/tests/offline.rs` | 5 | the same window with a transport that goes offline or rate limited on demand, then recovers |
| `app/tests/pictures.rs` | 1 | app pictures: fetched, decoded, shown, cached on disk, harmless when missing |
| `app/tests/binary.rs` | 6 | the real application (threads, event loop, software renderer) under `xvfb-run`, Linux only |
| `app/tests/contrast.rs` | 1 | WCAG contrast of every text/background pairing in both themes |
| app unit tests | 5 | option parsing, release-note rendering, formatting |

Things the tests found while being written, all fixed:

- Escape did nothing until a control had been focused; Android's Back was not handled
  on mobile; focus was lost when an app page was opened from the keyboard.
- The spotlight banner only reacted on its button.
- At 320 px wide the app page and library ran off the right edge
  (`everything_fits_on_the_narrowest_phone` now checks every control on every page).
- The light theme's chips and status colours were as low as 3.7:1 contrast; all text
  pairings now reach 4.5:1.
- 14 file-name misclassifications (section 4).
- A test that clicked fixed coordinates broke when the layout changed; scripted runs
  now trigger actions by name instead.

Found by reading the code rather than by a test, then pinned with tests: updates between
release candidates (`rc.1` to `rc.2`) and Arch package releases (`-1` to `-2`) were never
offered; the Windows hand-off went through `cmd.exe`, which interprets characters such
as `&` in file names chosen by release authors (now Explorer; **not run on Windows**);
and the page promised a checksum check for digest formats it could not check.

Also checked automatically: every button, tab and switch has an accessible name; a
state file that cannot be written or read produces a message instead of a crash.

Gaps: no test talks to the live services. The Windows and macOS runs use the headless
backend, so they prove the code builds and behaves there, not that the window looks
right or that installers launch. Nothing runs on Android or iOS. There are no visual
regression tests, only the screenshots in `docs/screenshots/`.

## 9. Platform support

| Platform | Where it stands | What is missing |
|---|---|---|
| Linux | Builds, runs, tested | Packaging (AppImage / Flatpak / AUR package of the store itself) |
| Windows | Builds; test suite passes in CI (headless) | Run it on a real desktop; precise CPU detection; try the installer hand-off; packaging |
| macOS | Builds; test suite passes in CI (headless) | Run it on a real desktop; app bundle, signing, notarisation |
| Android phone / tablet | Core type-checks for `aarch64-linux-android`; `android_main` written | NDK build, APK packaging, `REQUEST_INSTALL_PACKAGES` + `PackageInstaller` JNI, TLS roots check |
| Android TV | Same as Android | Also a leanback launcher entry and banner |
| iOS | Core type-checks for `aarch64-apple-ios` | See below |

"Type-checks" means `cargo check -p ustore-core --no-default-features --target ...`
passes (in CI on every push): the core's own code compiles for that target. The network
stack and the UI have not been built for Android or iOS.

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

## 10. UI toolkit

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

## 11. Safety

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

## 12. Next steps, in order of value

1. Run against the live GitHub and AUR APIs and fix whatever the real responses break.
   `ustore doctor` does this in one command: it searches, loads an app, checks every
   featured entry against this machine and prints GitHub's remaining allowance. It also
   exposes wrong entries in the hand-written featured list.
2. Run the classifier over a large sample of real release file names; fix the misses.
3. Run the app on real Windows and macOS desktops; package it for each.
4. Android: NDK build, APK, `PackageInstaller` hand-off; then Android TV launcher entry.
5. Regenerate the bundled index at build time, with file names.
6. Ask the system what is installed (dpkg/rpm/pacman databases, Android's package list)
   instead of relying on the store's own record.
7. Screenshots and a project's own icon from its repository; Markdown release notes.
8. More sources: Codeberg / Forgejo, GitLab, F-Droid, Flathub.
9. Translations; a Settings page for TV.
