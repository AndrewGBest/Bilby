# Bilby: design

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

There is no Bilby server, account, telemetry or hosted catalog, and the design
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
| "Update" and "Update all" that just do it | Play Store, F-Droid | One-click updates from the Library, queued one at a time |
| Choosing an older version | F-Droid, Obtainium | Version picker on the app page |
| Suggestions while typing | Most stores | Instant matches from the bundled catalog, no network |
| The publisher's picture next to the app | GitHub itself | Owner avatars, cached on the device |
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
   crates/cli (bilby-cli) ─────────────────────┤
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
  whole engine is testable on one machine. **[verified]** 14 presets, all exercised in tests.
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
files resolves to the expected single file on all 14 presets
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

`trait Source { search, summary, releases, available_on }`. Three exist: GitHub Releases,
the AUR, and the system's own repositories. Adding GitLab, Codeberg / Forgejo, F-Droid,
Flathub or winget means one new file.

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

### System repositories (pacman, apt)

"Repositories" could be done two ways. One is a client for each distribution's web
service (archlinux.org's package search, a mirror's database for CachyOS, packages.
debian.org, ...): one integration per distribution, a network request per search, and
no way to know which of them this machine can actually install from. The other, chosen
here, is to read what the system's own package manager already has on disk:

- **pacman**: `/var/lib/pacman/sync/<repo>.db` for every repository in `pacman.conf`.
  That covers Arch's `core`, `extra` and `multilib`, CachyOS's `cachyos`, `cachyos-v3`,
  `cachyos-core-v3`, `cachyos-extra-v3` and the v4/znver4 variants
  ([CachyOS wiki](https://wiki.cachyos.org/features/optimized_repos/)), Chaotic-AUR,
  EndeavourOS, Manjaro, or a private repository, with no code specific to any of them.
- **apt**: `/var/lib/apt/lists/*_Packages` for every configured archive, suite and
  component, including PPAs and vendor repositories.

It costs no requests, works offline, and lists exactly what `pacman -S` / `apt install`
can install, because it is the same data. The trade-offs: the lists are as fresh as the
last `pacman -Sy` / `apt update` (the store does not refresh them, which needs root),
and a repository this machine does not have configured is not browsable, on the grounds
that its packages could not be installed anyway.

Details:

- A package in several repositories is listed from the first one in `pacman.conf`
  order, as pacman itself resolves a bare name; with apt the newest version wins.
- CachyOS's optimised packages declare architectures such as `x86_64_v3`. They are
  treated as x86_64: a machine has such a repository configured only if its CPU
  qualifies **[sourced]** (same wiki page).
- Databases may be gzip, zstd or lz4 compressed or plain; the format is taken from the
  file's first bytes. xz is not supported yet and such a database is skipped.
- Results are ranked by how well the name matches (there is no popularity to go by),
  with libraries, `-dev`, `-doc` and similar listed after applications.
- What is installed comes from `/var/lib/pacman/local` and `/var/lib/dpkg/status`, so a
  package shows "Installed" with the version you have, and "Update available" when the
  repository has a newer one.
- Installing is left to the package manager: the store shows `sudo pacman -S name` or
  `sudo apt install name`. It does not run them, in keeping with never elevating
  itself, and does not add these to its own library, since the package manager already
  tracks and updates them. On Arch-based systems the steps add that an installed
  package should be updated with a full `pacman -Syu`, because partial upgrades are
  unsupported there **[assumption]** (from memory of the Arch wiki).
- The demo catalog uses fictional repositories and never reads the real machine's.

Status: the apt reader was run against the real Ubuntu 24.04 lists on the build machine
(lz4-compressed, about 70,000 packages; `ripgrep 14.1.0-1` from `noble/universe`, `curl`
recognised as installed). **[verified]** The pacman reader is tested against databases
the tests build in the layout `repo-add` produces (`name-version/desc` entries with
`%NAME%`-style fields, per [alpm-repo-desc](https://man.archlinux.org/man/extra/alpm-repo-db/alpm-repo-desc.5.en))
but that layout was written from memory **[assumption]** and no real Arch or CachyOS
database was available to read. Parsing a large distribution takes a few seconds in a
debug build; it happens once, in the background at start-up, and again only when the
files change.

Not done: dnf (Fedora) and zypper (openSUSE) metadata, Flatpak remotes (Flathub),
winget and Homebrew. Each is another `Backend` in `source/system.rs` or another
`Source`; nothing else in the app needs to change.

### The store's own updates

The store treats its own repository (`STORE_REPOSITORY` in `catalog.rs`) as one more
app. At start-up, unless start-up checks are switched off, it asks for the newest
release that has a file for this device and compares it with its own version. If there
is one, Settings shows it with a "View update" button that opens the store's own app
page, where the usual rules apply: only files for this device, checksum verified. One
request, cached like the rest. There is no separate updater and no update server.
**[verified]** by test with a stand-in project; never against the real repository,
which has no releases yet.

`.github/workflows/release.yml` builds archives for Linux x86_64, Windows x86_64 and
Apple-silicon macOS when a `v*` tag is pushed, named so the classifier resolves them
(**[verified]** by `the_stores_own_release_files_resolve_on_every_desktop`). The
workflow itself has never run.

What this does not do yet: replace the running program. The release files are archives,
so today "installing" the update downloads it and shows the folder; swapping the binary
is manual. A copy installed from a package manager should be updated through that
instead. Closing that gap means publishing an AppImage, a `.deb` and a Windows
installer, or having the store replace its own executable and restart.

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

"Update" on the Library page updates in place: it looks up the newest release for this
device and installs its best option without a detour through the app page, showing
progress on the row; it can be cancelled, and a failure is reported and leaves the old
entry untouched. "Update all" queues every waiting update and works through them one
at a time; a failure is reported and the rest carry on, and cancelling stops the queue. **[verified]** by tests headless, offline
and with real threads.

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

### Search order

Results from all sources go into one list, best match first (`rank.rs`). Most of the
score is how close the app's name is to what was typed; a quarter is how many people use
it (stars or votes, on a scale that suits each source); the rest is small nudges: on the
hand-picked list, already installed, flagged out of date, and where its own source
placed it. A source that publishes no numbers (a distribution's repository) gets a
middle value for that part, so missing data never sinks a result. [assumption: the
weights are a first guess and have not been tuned against real searches.]

### What is shown, and what is one press away

The screens are meant to work for someone who does not know what a `.deb` is, and to
look like the stores that person already uses. Hiding secondary options until they are
asked for is the usual advice for this
([Nielsen Norman Group on progressive disclosure](https://www.nngroup.com/articles/progressive-disclosure/)).

An app's page follows the order Apple and Google use. [sourced where linked; the rest is
from memory of both stores, not checked against them while writing]

| Part of the page | App Store / Google Play | Bilby |
|---|---|---|
| Top | Icon, name, developer, one button | The same. The button says Install, Update, or Open once an AppImage is installed |
| Key facts row | Rating, size, age rating, chart position | Stars or votes, download size, last updated, licence |
| Reassurance | "Works on this iPhone"; Play's data-safety section | "Made for this device", and that the download is checked when it is |
| Short description | Apple's subtitle (up to 30 characters); Play's short description (80) ([Apple](https://developer.apple.com/app-store/product-page/), [Google](https://support.google.com/googleplay/android-developer/answer/13393723)) | The project's one-line description |
| Screenshots | Up to ten ([Apple](https://developer.apple.com/app-store/product-page/)) | Up to four, found in the project's README (`screenshots.rs`). Badges, logos, icons, small images, GIFs and SVGs are left out, and pictures are only fetched from GitHub's own hosts so that no third party learns who looked. Packages from repositories have none yet |
| Description | First lines, then "more" | The start of the README, folded to a few lines with "Read more" |
| What's New | Shown on the page and in Updates ([Apple](https://developer.apple.com/app-store/product-page/)) | Release notes, folded; shown above the description once the app is installed |
| Ratings and reviews | Yes | Star ratings where a ratings service knows the app (Flathub, distribution packages). Written reviews for the same apps, read-only |
| Information | A plain table at the bottom: provider, size, compatibility, ... | The same: from, made by, version, released, licence, works on, kind of file, size |
| Everything technical | Not shown at all | Under *More options and details*: every file with its name, command and steps, older versions, and the hidden files with the reason for each |

Lists (tiles, rows) show the icon, name, one line and popularity, plus whether the app
is installed or has an update. They say nothing about packaging. The device is named in
everyday words ("This Linux computer"); the exact system and processor are in Settings
and in each app's Information table.

Lists have an Install or Update button on each tile and row, as both stores do. It
opens the app's page and starts the file that suits the device, so the install can be
watched and stopped. *Other versions and files* sits directly under the main button:
the version picker, every way to install here, and the files for other devices, which
can be downloaded anyway (checked, saved, never run).

Known differences that are still gaps: reviews can be read but not written, no "top
charts", and Open works for AppImages, Flatpaks and web apps but not for packages
handed to the system installer.

### Flathub, logos and ratings

**Flathub** (`source/flathub.rs`) is a source wherever Flatpak is set up. Its API gives
what release pages lack: an icon, screenshots, a description and an id that ratings
are keyed by. Install runs `flatpak install --noninteractive --assumeyes --or-update
flathub <id>`, Open runs `flatpak run <id>`, and removing the app from the library runs
`flatpak uninstall`. Flatpaks installed by something else are recognised from
Flatpak's own folders and can be opened. A `.flatpak` file on a release page is
installed with `flatpak install --user`. [The API shapes and command options were read
from Flathub's and Flatpak's source and documentation; neither the live service nor a
real `flatpak` has been run against. Tests use a stand-in server and a stand-in
program.]

**Logos.** An app's icon is, in order: the icon its source gives (Flathub); for a
GitHub project, an early image in its README that calls itself a logo or icon or is
drawn small; else `icon.png` or `logo.png` at the usual places in the repository; else
the owner's picture; else initials on a colour. README and repository logos are found
when the app's page is opened, kept on disk under the app's name, and used in lists
from then on. PNG and JPEG only: many projects' logos are SVG, which the app cannot
draw yet, so those fall back to the owner's picture.

**Ratings** (`ratings.rs`, `appstream.rs`) come from ODRS, the service behind the
ratings in GNOME Software and KDE Discover. It publishes one file with the star counts
of every app. Bilby fetches that file at most once a day and keeps it on disk. Because
everyone gets the same file, the service learns that this device uses ratings and
nothing about which apps are looked at. Apps are matched by AppStream id: directly for
Flathub; for pacman and apt packages, and GitHub projects, through the catalog the
distribution already keeps on disk (`/usr/share/swcatalog` and its older locations),
which costs no requests. An app with fewer than five ratings shows none. Ratings can
be turned off in Settings, which also stops the download. In search order a rating
counts as popularity, pulled towards the middle when few people gave one.
[The file's shape was read from ODRS's source; the live service was unreachable. The
catalog reader has only read files written for its tests.]

**Written reviews** [Decision, Andrew, 2026-11-20] come from the same service and are
shown on the page of any app it knows; an app without them has no Reviews section at
all. They are asked for when such a page is opened, up to eight, best first.

What this costs in privacy, plainly: unlike the ratings file, a request for reviews
names the app, so the service learns which app's page was opened from which address,
and its [privacy policy](https://github.com/GNOME/odrs-web/blob/master/odrs/templates/privacy.html)
says addresses are logged for five years. Bilby sends nothing else that identifies the
device: no user id (GNOME Software and Discover send a hash of the user name and
machine id), a system name of "unknown", and no app version. The Settings toggle
covers ratings and reviews together and says this in its description; turning it off
stops both.

Reviews are read-only. Writing or voting on one needs the persistent id above, which a
store with no accounts would have to invent for the user; that is left for a decision
of its own. [Whether the live service answers a request without a user id was read
from its source code, where it is allowed, and not tried. If it refuses, the app
simply shows no reviews.]

The number of ratings is shown beside the score and counts towards search order, as
stars do for GitHub projects: it is the nearest thing these apps have to a measure of
how widely they are used.

### TV

The TV interface follows Android TV's published guidance, with tvOS's where they agree:

| Rule | Source | Bilby |
|---|---|---|
| Keep content inside the overscan-safe area: 48dp at the sides, 27dp top and bottom | [Android TV layouts](https://developer.android.com/design/ui/tv/guides/styles/layouts) | Margins are exactly these; backgrounds run to the edge |
| Everything reachable with the D-pad; Back goes to the previous place | [TV app quality](https://developer.android.com/docs/quality-guidelines/tv-app-quality), [navigation](https://developer.android.com/design/ui/tv/guides/foundations/navigation-on-tv) | Arrows, OK and Back do everything; Back closes the file list, then the page, then leaves it to the system |
| Exactly one focused element, shown by scale, border or glow; scale 1.025 to 1.1 | [Focus system](https://developer.android.com/design/ui/tv/guides/styles/focus-system) | A white border and about 1.05x growth |
| Text readable from 3 m; body text 12sp and up in the TV type scale | [Design for TV](https://developer.android.com/design/ui/tv/guides/foundations/design-for-tv), [type scale](https://github.com/androidx/androidx/blob/androidx-main/tv/tv-material/src/main/java/androidx/tv/material3/tokens/TypeScaleTokens.kt) | Nothing below 11dp; body 12 to 14dp |
| Four cards fill a row at 196dp each with 20dp between | [Cards](https://developer.android.com/design/ui/tv/guides/components/cards) | Four tiles a row, 20dp apart |
| tvOS: safe area 60pt top/bottom, 80pt sides; text 23pt minimum at 1080p | [Apple layout](https://developer.apple.com/design/human-interface-guidelines/layout), [typography](https://developer.apple.com/design/human-interface-guidelines/typography) | Sides are inside Apple's too (96px vs 80pt); smallest text is 22px at 1080p, a point under Apple's minimum |

**Websites as apps (Linux TVs only).** A TV cannot use a website without help, and the
sister project [Longboard](https://github.com/AndrewGBest/Longboard) is a browser driven
by a remote. So on Linux with the TV interface, a *Websites* source appears
(`source/web.rs`): a short hand-written list of free sites, plus any site whose address
is typed into search. Adding one writes an entry to the TV's app list that starts
Longboard with `--url=<address>`; the app page's button then becomes Open, and removing
it from the library deletes the entry. Nothing is downloaded. Longboard itself is
featured under *Made for TV* and installed like any other app; without it, a website's
page says so and adding fails with the same advice.
[verified in tests with a stand-in for Longboard; never run against the real one.
Longboard keeps its own list of web apps with a Home tile and optional separate
sign-ins, and has no command to add to it, so a site added here opens in Longboard but
is not on Longboard's Home. A `--add-app=` option in Longboard would close that gap.]

An app's page on TV has three buttons in a row (the main one, Favourite, Other files),
the facts row, a picture and the description. Still missing on TV: choosing an older
version, a settings screen, and "update all".

## 8. Testing

168 tests, none needing the internet. **[verified]**: all pass on Linux here. CI runs
the suite on Linux, Windows and macOS on every push. It passed on all three for the
version-picker commit (run 13). The run after that failed on Windows only, because a
test clicked a button that was below the window there; the test helper was fixed. Check
the Actions tab for the state of the latest commit rather than trusting this paragraph.

| Suite | Count | What it exercises |
|---|---|---|
| `bilby-core` unit tests | 80 | detection, classification, compatibility, versions, state file, install plans and execution, index, suggestions, icons, catalog |
| `core/tests/corpus.rs` | 4 | 169 real-world-style names; cross-OS/CPU safety property; 20,000 generated names |
| `core/tests/http_sources.rs` | 16 | the real HTTP client and both sources against a local server: headers, token scoping, rate limits, allowance, progress, cancel, cache, documented JSON shapes, garbage responses |
| `cli/tests/cli.rs` | 8 | the `bilby-cli` binary end to end, including install, library, updates and `doctor` |
| `app/tests/ui.rs` | 39 | the real window on Slint's headless backend: clicks by accessible label, key presses, all three interfaces |
| `app/tests/offline.rs` | 7 | the same window with a transport that goes offline or rate limited on demand, then recovers |
| `app/tests/pictures.rs` | 1 | app pictures: fetched, decoded, shown, cached on disk, harmless when missing |
| `app/tests/binary.rs` | 7 | the real application (threads, event loop, software renderer) under `xvfb-run`, Linux only |
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

"Type-checks" means `cargo check -p bilby-core --no-default-features --target ...`
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
   `bilby-cli doctor` does this in one command: it searches, loads an app, checks every
   featured entry against this machine and prints GitHub's remaining allowance. It also
   exposes wrong entries in the hand-written featured list.
2. Run the classifier over a large sample of real release file names; fix the misses.
3. Run the app on real Windows and macOS desktops; package it for each.
4. Android: NDK build, APK, `PackageInstaller` hand-off; then Android TV launcher entry.
5. Regenerate the bundled index at build time, with file names.
6. Ask the system what is installed (dpkg/rpm/pacman databases, Android's package list)
   instead of relying on the store's own record.
7. Screenshots and a project's own icon from its repository; Markdown release notes.
8. More repositories: dnf and zypper metadata, Flatpak remotes (Flathub), winget,
   Homebrew, F-Droid; more release hosts: self-hosted Forgejo / Gitea, GitLab
   (Codeberg is in, untested against the live site). Try the pacman reader on a real
   Arch or CachyOS system first.
9. Translations; a Settings page for TV.

[COMPARISON.md](COMPARISON.md) sets Bilby against Komi Store, Obtainium, UniGetUI and
others feature by feature and orders the remaining gaps.
