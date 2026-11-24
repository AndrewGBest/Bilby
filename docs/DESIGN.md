# Bilby: design

How Bilby is built, and why. For what it does, start with the [README](../README.md);
for building and running it, see [DEVELOPMENT.md](DEVELOPMENT.md).

## 1. What it is

An app store that brings every place software comes from into one window: package
managers, other stores, and developers' own release pages. It keeps one rule:

> If it cannot be installed on this device, it is not shown.

One codebase, in Rust, with three interfaces: desktop (pointer and keyboard), phone
(touch) and TV (remote).

### Entirely on the device

Bilby has no server, no account, no telemetry and no hosted catalog, and the design
keeps it that way. The app talks only to the sources themselves, and everything it
remembers lives in one file on the device (`state.json`: settings, library, recent
searches). That one decision shapes the rest:

- The Discover catalog is compiled into the program and is updated with it.
- GitHub's request allowance is treated as a budget: responses are cached, each search
  checks a bounded number of projects, and Settings shows what is left.
- Update checks, the library and preferences are local. Nothing is synchronised, so
  there is nothing to leak.
- Tests hold it to this. A GitHub token is sent to GitHub's API and to no other host
  (`the_token_goes_to_the_api_and_nowhere_else`), and the state file is readable only
  by its owner.

### Built on what works elsewhere

Bilby takes the patterns people already know from the stores they use:

| Idea | Familiar from | In Bilby |
|---|---|---|
| A library that checks its sources for updates | Obtainium, F-Droid | Library page, badge, check at start-up |
| "Update" and "Update all" that simply do it | Play Store, F-Droid | One-press updates from the Library, queued one at a time |
| Choosing an older version | F-Droid, Obtainium | Version picker on every app's page |
| Suggestions while typing | Most stores | Instant matches from the bundled catalog, with no network |
| Paste a project's link to add it | Obtainium | The search box accepts links and `owner/repo` |
| One prominent button, details below | Flathub, GNOME Software, Play Store | The best file first, the rest one press away |
| Say what a package can be trusted for | F-Droid's anti-features | First-run notice, checksum line, AUR notice |
| Show the steps before taking them | Linux package managers | "What will happen?" on every option |
| Rows you move through, with the focused app described large | Android TV, Steam Big Picture | The TV interface |

### Where Bilby stands among other stores

Komi Store and Obtainium showed that release pages can be a store. Bilby builds on
that idea and takes it further in four directions:

- **Every source at once.** Release pages, Flathub, the Snap Store, the distribution's
  own repositories, the AUR, winget, Homebrew, Steam, the Mac App Store, the Microsoft
  Store, F-Droid and
  websites, merged into one result per app.
- **A real TV interface**, driven entirely by a remote.
- **A compatibility engine that knows Linux**: deb or rpm or pacman, glibc or musl,
  which processors a device can emulate, and a stated reason for every file it hides.
- **Rust throughout**, with the engine free of any interface code, so it also drives
  a command-line tool and compiles for Android and iOS.

[COMPARISON.md](COMPARISON.md) sets Bilby beside the others feature by feature.

## 2. Architecture

```
            ┌────────────────────── crates/app (Slint) ───────────────────────┐
            │  desktop.slint        mobile.slint          tv.slint            │
            │        └──────────── global Store (types.slint) ─────────┘      │
            │        App (src/*.rs): worker threads <-> Store                 │
            └───────────────────────────────┬──────────────────────────────────┘
   crates/cli (bilby-cli) ──────────────────┤        crates/signin (sign-in window)
                                            ▼
┌──────────────────────────────── crates/core ─────────────────────────────────┐
│  catalog   browse / search / details, filtered to this device                │
│     │                                                                        │
│     ├── index, essentials   what the home page shows (no requests)           │
│     ├── source     trait Source: one file per source                         │
│     ├── merge, related      one app from several sources; versions of a      │
│     │                       service                                          │
│     ├── rank       search order                                              │
│     ├── asset      file name ──► format, system, processor, libc, TV         │
│     ├── compat     (file, device) ──► Native / Compatible / Incompatible     │
│     ├── install, needs      plan an install; carry it out; what it requires  │
│     ├── platform   Platform::detect() / Platform::preset(name)               │
│     ├── state      settings, library, recent searches (one local file)       │
│     ├── os         the operating system's own updates                        │
│     └── version    is this release newer than the installed one?             │
│  http      trait Http ── NetHttp (rustls), CachedHttp, DemoHttp              │
└──────────────────────────────────────────────────────────────────────────────┘
```

Four rules keep it sound:

- **The interface holds no logic.** The three interfaces draw one `Store` and call its
  callbacks (`search`, `open`, `install`, `check-updates`, ...). Anything an interface
  shows, the command-line tool can print.
- **One controller, two ways to run it.** `App::job` runs catalog work on a worker
  thread in the app and inline under test, so the interface tests drive the real
  handlers.
- **The device is a value.** Every decision takes a `&Platform`, so "what would an
  Android TV be offered?" is a function call (`--as android-tv`) and the whole engine
  is tested on one machine, across 14 device presets.
- **The network is behind one trait.** `DemoHttp` serves a fictional catalog in the
  sources' real formats, so the demo runs the same parsing and filtering code as the
  real store.

## 3. Knowing the device (`platform.rs`)

| Fact | How it is known |
|---|---|
| System | The build target |
| Processor | The build target; on Windows the `PROCESSOR_ARCHITEW6432` and `PROCESSOR_ARCHITECTURE` variables; on Android the device's ABI list |
| Other processors it can run | Windows on Arm runs x64 and x86; 64-bit Windows runs x86; Apple silicon runs x64 through Rosetta 2; Android follows its ABI list; Linux runs x86 where the 32-bit loader is present |
| Linux family | `ID` and `ID_LIKE` in `/etc/os-release`, tested for Ubuntu, Arch and its derivatives, openSUSE, RHEL clones and Alpine |
| C library | The musl loader present without the glibc one |
| Installable formats | `dpkg`, `rpm`, `pacman`, `flatpak`, `snap` and an AUR helper, found on the `PATH` |
| Kind of device | See [Knowing it is a TV](#knowing-it-is-a-tv); otherwise the window's width |

Detection runs for real on Debian, Fedora, openSUSE and Arch. `bilby-cli device`
prints what was found.

## 4. Knowing the file (`asset.rs`) and deciding (`compat.rs`)

A release page says nothing about which device a file is for, so Bilby reads the file
name. The name is lower-cased and every run of punctuation becomes `_`, so whole
tokens are matched (`_arm64_`, `_x86_64_`) and `darwin` is never mistaken for `win`.

The decision runs in a fixed order, and every rejection carries a reason that is
shown to the person:

1. **Not an app.** Checksums, signatures, sources, debug symbols and update manifests
   are set aside.
2. **System.** From the format (`.apk`, `.msi`, `.dmg`, `.deb`...) or the name. A file
   that names no system is held back, never guessed at.
3. **Kind of device.** An Android TV build is for TVs. Debug-signed APKs are kept,
   because some projects publish nothing else, and ranked below release builds.
4. **Package format.** `.deb` needs dpkg, `.rpm` needs rpm, pacman packages and the
   AUR need an Arch system, an AppImage needs glibc.
5. **C library.** A glibc build is not offered on musl; a musl build runs anywhere.
6. **Processor.** Native, or runnable through emulation (shown as "Compatible", with
   the reason), or not offered.
7. **Rank.** Native before emulated; the distribution's own format first (deb on
   Debian, rpm on Fedora, pacman on Arch), then AppImage, Flatpak, AUR build, archive,
   bare program; TV builds first on a TV. A read-only system such as Fedora Kinoite is
   offered the Flatpak first. A setting lets the person choose which kind comes first.

The top-ranked file becomes the Install button. A made-up release of 17 files
resolves to the one expected file on all 14 presets
(`compat::tests::each_platform_gets_its_natural_package`), and the same holds through
the whole path from a source's answer to the install button
(`catalog::tests::details_offer_the_right_file_per_machine`).

The classifier is held to a corpus of 169 file names in the styles of Rust target
triples, GoReleaser, Electron and Tauri bundlers, Android ABI splits and distribution
packages (`crates/core/tests/corpus.rs`). On top of it, a property test runs every
corpus name and 20,000 generated names against every preset and checks two
guarantees: nothing is ever offered for another system or processor, and every hidden
file has a reason.

The engine is deliberately cautious: where a name does not say enough, the file is
listed under "hidden" with the reason, where it can still be downloaded by choice.
Reading an APK's own manifest after download is the natural next refinement.

## 5. Sources

`trait Source { search, summary, releases, available_on }`. Each source is one file
and appears only on the systems it applies to:

| Source | Windows | macOS | Linux | Android | How it installs |
|---|:-:|:-:|:-:|:-:|---|
| GitHub Releases | ✓ | ✓ | ✓ | ✓ | Bilby fetches the file, checks it and installs it |
| GitLab, Codeberg | ✓ | ✓ | ✓ | ✓ | The same |
| Your own sources | ✓ | ✓ | ✓ | ✓ | The same, from a list on your own server |
| Flathub | | | with Flatpak | | Bilby runs Flatpak |
| The distribution's repositories | | | pacman, apt, dnf, zypper | | Bilby runs the package manager behind the system's password prompt |
| AUR | | | Arch family | | Bilby opens a terminal on the build |
| Snap Store | | | with snapd | | Bilby runs `snap install` |
| winget | ✓ | | | | Bilby runs `winget install` |
| Homebrew casks | | ✓ | | | Bilby fetches the publisher's own download and checks it against the cask's SHA-256; Homebrew itself is not needed |
| F-Droid | | | | ✓ | Bilby fetches the APK |
| Steam | ✓ | ✓ | ✓ | | Bilby hands over to the Steam app |
| Mac App Store | | ✓ | | | Bilby hands over to the App Store app |
| Microsoft Store | ✓ | | | | Bilby hands over to the Store app |
| GOG | ✓ | ✓ | ✓ | | Bilby opens the game's page on gog.com |
| Google Play | | | | ✓ | Bilby hands over to Google Play |
| Web apps | ✓ | ✓ | ✓ | | Adds the site among the device's apps |

**One press, no commands.** A package from a distribution's repository, a snap or a
winget package is installed when the button is pressed. What needs administrator
rights runs through the desktop's own password prompt (`pkexec`), so Bilby never
sees the password. An AUR build asks questions as it goes, so Bilby opens a terminal
on it.

**Every source is asked at the same time.** A search takes as long as its slowest
source, not as long as all of them added together, and the results are merged in a
fixed order so the list comes out the same every time.

**Switching sources off.** Settings lists every source this device has, each with a
switch. Results can also be narrowed to one source with the chips under the search
box.

**One app, several sources** (`merge.rs`). The same app in a repository, on Flathub
and on its release page is one result, marked with how many sources carry it. Its
page says which source the button uses and offers the others. Catalogs are trusted to
have one app per name; a project on a code host joins them when it shares its owner's
name with the app, is well known, or is a stated mirror. The rule is conservative:
when in doubt, two entries stay separate. A website is never merged with a program:
Plex's site and Plex's desktop app are two things to choose between, and both are
listed.

**Versions of a service** (`related.rs`, `data/related.json`). VacuumTube, FreeTube
and NewPipe are different ways to use YouTube, and which one suits depends on the
device. The relationship is curated in one file: a family per service, its members in
order of preference, each with the systems and devices it runs on and a label that
tells it apart ("Plex website", "Plex for desktop", "Plex HTPC"). Searching for the
service lists its members first, each as itself; a member's page lists the others
under "Other ways to use" the service. That list is kept apart from "other places to
get" the same app, so the two choices are never confused. Every entry was checked to
exist at its source.

**Search order** (`rank.rs`). How many people use an app is the largest share of its
score (0.45), just ahead of how close its name is to what was typed (0.40). An app
that is widely used and is about the thing outranks a little-used one that merely has
the word in its name; the app whose name is exactly what was typed gets a fixed bonus
and stays first. A source that publishes no figures gets a neutral value, so missing
data never sinks a result.

**The apps everyone expects** (`essentials.rs`, `data/essentials.json`). The sources'
figures say what is popular on each source. They do not say that someone setting up a
computer wants a browser, a video player and the names everybody knows, whichever
source carries them. That is editorial judgement, so it is a curated list: about 140
apps and sites, each with the id it goes by at every source, every one checked to
exist. Each also says which devices it suits and where it leads: a TV opens with
things to watch and play, a computer with a browser and what people install first, a
phone with what people use on phones. In a search they get a fixed lift, enough to
put the real Firefox above its look-alikes and not enough to beat an exact name. One
switch in Settings turns the list off, leaving only what the sources say.

### The home page

The home page costs no requests. It is composed from the bundled lists, worked out
once and remembered:

- **Essentials** first, each in the version this device can use.
- **A row per kind** (web browsers, music and video, chat, games, work, handy tools,
  developer tools), in the order that suits the device. Websites sit in the same rows
  as apps of their kind. Nothing appears twice: what leads the page is not repeated in
  the row beside it.
- **Web apps**, one row with every listed website.
- **The sources' own lists**, where a source keeps them: popular, trending, and new
  and updated on Flathub. These leave out anything a row above already shows.

On a TV, *Made for TV* comes first. On a desktop and a phone, "All" shows the start
of each section with "Show all", and a chip per section shows the whole of it.

### Web apps (`source/web.rs`, `data/SOURCES.md`)

A website is added like an app on every system. On a Linux TV it opens in
[Longboard](https://github.com/AndrewGBest/Longboard), a browser driven by a remote;
anywhere else in the person's own browser. Nothing is downloaded, and removing it
from the library removes its entry.

Three bundled lists stand behind it:

- **About ninety well-known sites**, each written up by hand with a name and a
  purpose. These are the ones the home page recommends.
- **The thousand most visited sites**, so that a site is found by typing part of its
  name. They are found by searching and are deliberately never recommended: a list of
  what is visited is not a list of what is good.
- **Some sixty thousand addresses of sites for adults**, so that one is recognised
  whether or not it is on the other lists, including an address typed by hand.

**Longboard is fetched when it is needed.** Bilby recognises Longboard however it was
installed (an AppImage, an unpacked folder, its Flatpak, or on the `PATH`). Where a
website or a Steam sign-in on a TV needs it and it is not there, Bilby installs it
first and carries on. The person is never sent to fetch something themselves.

A web app's picture is the site's own (`icons::fetch_site`): the icon its front page
names, preferring the large one made for phones' home screens. It is asked of the
site and of nobody else, with no icon service in between, and kept on disk.

### GitHub

REST endpoints: `/search/repositories`, `/repos/{owner}/{repo}` and
`/repos/{owner}/{repo}/releases`.

**The request allowance shapes the design.** Without signing in, GitHub allows 60
requests an hour per address, with a stricter limit on searches
([GitHub docs](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)).
"Only show what works here" needs each candidate's release files, which is one request
per project. So:

- **Discover costs nothing.** It is driven by the bundled lists.
- **The allowance is visible.** GitHub reports what is left with every answer;
  Settings shows it ("42 of 60 GitHub requests left; refills in 23 minutes").
- **A search checks a bounded number of projects** (8 without signing in, 20 with), in
  parallel, and reports how many matched but had nothing for this device. Typing
  costs nothing: suggestions come from the bundled lists.
- **Answers are cached on disk** for 30 minutes, and a kept copy is served when
  offline or when the allowance runs out.
- **Signing in lifts the limit to 5,000 an hour.** See
  [Signing in to GitHub](#signing-in-to-github-github_loginrs).

Release files uploaded since June 2025 carry a SHA-256 digest
([GitHub changelog](https://github.blog/changelog/2025-06-03-releases-now-expose-digests-for-release-assets/)).
Bilby verifies it before anything is opened; a file that does not match is deleted
and never run.

### AUR

The aurweb RPC, version 5
([ArchWiki](https://wiki.archlinux.org/title/Aurweb_RPC_interface)). An AUR search is
a single request, because its results carry everything needed.

An AUR package is a build recipe, not a program. It is modelled as a release with one
file of the kind "AUR package", which the engine offers only on Arch-family systems.
Bilby opens a terminal on the build and says plainly that a recipe is code from its
author, worth reading first. Out-of-date and unmaintained packages are marked.

### System repositories (`source/system.rs`)

Bilby reads what the system's own package manager already has on disk, rather than
asking a web service per distribution:

- **pacman**: `/var/lib/pacman/sync/<repo>.db` for every repository in `pacman.conf`.
  That covers Arch's own repositories, CachyOS's optimised ones, Chaotic-AUR,
  EndeavourOS, Manjaro and private repositories, with no code specific to any of them.
- **apt**: `/var/lib/apt/lists/*_Packages` for every configured archive, including
  PPAs and vendor repositories.
- **dnf**: asked from its own cache (`dnf -C repoquery`): 67,000 packages in under a
  second.
- **zypper**: openSUSE's repository lists, read as a stream.

This costs no requests, works offline, and lists exactly what the package manager can
install, because it is the same data. A repository the device does not use is not
shown, on the grounds that its packages could not be installed anyway.

- A package in several repositories is listed from the first in `pacman.conf` order,
  as pacman itself resolves it; with apt the newest version wins.
- Compressed lists (gzip, zstd, lz4) are recognised by their first bytes.
- Applications are ranked ahead of libraries, `-dev` and `-doc` packages.
- What is installed comes from the package manager's own records, so a package shows
  "Installed" with its version, and "Update available" when there is a newer one.
- These packages stay in the package manager's care: Bilby installs them and leaves
  tracking and updating to the system, where it belongs.

### Flathub (`source/flathub.rs`)

Flathub is a source wherever Flatpak is set up. It supplies what release pages lack:
an icon, screenshots, a description and the id that ratings are keyed by. Install
runs `flatpak install`, Open runs `flatpak run`, and Remove runs `flatpak uninstall`.
Flatpaks installed by anything else are recognised from Flatpak's own folders and can
be opened from Bilby.

A `.flatpak` bundle on a release page is installed for the person's own account.
Bilby remembers which Flatpak it became, so its page offers Open and Remove like any
other; the bundle itself never lands in the downloads folder and is discarded once
Flatpak has taken the app out of it.

### Other stores, handed over to

Some things can only be installed by the store that sells them. For those Bilby is
the front door: it finds the app, shows its price, pictures and ratings, and opens
that store's own app on it (`steam://install/620`,
`macappstore://apps.apple.com/app/id...`,
`ms-windows-store://pdp/?ProductId=...`). Signing in, paying, downloading and
updating stay with that store.

For Steam, Bilby reads the files Steam keeps beside its library to know what is
installed, and offers Open. On Linux a Windows-only game is offered with a note that
Steam runs it through Proton. On a device without Steam, a game's page offers
"Install Steam" from whichever source this device has.

Only links Bilby built itself from a checked number are ever opened, and they are
checked again at the moment of opening.

### Signing in to Steam, to know what is owned (`steam_account.rs`, `crates/signin`)

Bilby marks owned games as Owned in place of their price. Steam tells a signed-in
browser what an account owns, so the person signs in on Steam's own page and that
browser hands Bilby one line: the account's number and the ids it owns. That line is
all Bilby keeps. Three ways are offered:

- **A QR code** (`steam_qr.rs`), on any device: Bilby draws Steam's sign-in code, the
  person scans it with the Steam app on their phone and approves there. On a TV the
  code fills the screen. The short-lived key Steam returns is used once, to ask what
  the account owns, and dropped; it is never stored, cached or written into a
  message, and tests hold it to that.
- **A small window of Bilby's own** (`bilby-signin`), on a computer or phone layout,
  built on the system's web engine. It goes to Steam's addresses and nowhere else,
  opens no other windows, and shows which site it is on in its title. On Linux it is
  a separate program, so Bilby itself starts on systems without WebKitGTK.
- **Longboard**, on a TV, which a remote can drive. Longboard asks the person whether
  to share the list once they are signed in.

### Adult content, Family mode and the PIN (`parental.rs`)

Things meant for adults are part of the catalog, kept apart rather than hidden:

- In a search they have a section of their own, last, and each is *covered*: the tile
  says "Adult content" and nothing of what it is.
- Choosing one asks "Are you 18 or over?", with No highlighted. A yes is remembered.
- On the home page they appear only if the person switched that on, and covered.
- **Family mode** takes them out altogether: not listed, not covered, not openable.
  It removes gambling and piracy sites too. Outside Family mode those are shown like
  any other site when searched for, and are never recommended.
- A four-digit **PIN**, kept as a salted hash, guards these settings and, if chosen,
  installing and removing. Making things stricter never needs it: anyone may switch
  Family mode on. It is a household lock, built to stop a child with the remote.

What counts as adult is the sources' judgement, not Bilby's: Steam's own descriptors
for adult-only content, Flathub's content rating where it works out to 18, and the
bundled list of adult sites. Each game's own page checks again, so a search that
could not ask Steam still ends at a page that does.

### What an AppImage needs (`needs.rs`)

Most AppImages load FUSE 2 to start, and many current systems ship only FUSE 3. Bilby
finds this out before the app is opened and deals with it, in this order:

1. The system's own package, where its package manager can install one.
2. On a read-only system, `rpm-ostree install --idempotent fuse fuse-libs`, followed
   by "Restart now / Restart later", since it works from the next restart.
3. Otherwise, unpacking the AppImage into a folder and starting the program inside,
   which needs no FUSE and no password.

Either way there is nothing for the person to go and do. Only packages the system
itself vouches for are used. The notice appears only where an AppImage is what is
installed or about to be: an app that is here as its Flatpak says nothing about FUSE.

### App pictures (`icons.rs`)

An app's icon is, in order: the icon its source gives; for Flathub, the address made
from the app's id, so a list shows it before the app's page was ever opened; for a
GitHub project, the logo its README leads with or one at the usual places in the
repository; else the owner's picture; else initials on a colour.

Pictures are fetched in the background a few at a time, recognised by what the bytes
are rather than what the address says, and cached on disk along with "this one has
none". When one arrives, only the tiles it belongs to are updated. A project's logo
is read as an ordinary file from the project, which leaves the API allowance alone.

### Ratings and reviews (`ratings.rs`, `appstream.rs`)

Ratings come from ODRS, the service behind the ratings in GNOME Software and KDE
Discover. It publishes one file with every app's star counts, which Bilby fetches at
most once a day. Because everyone gets the same file, the service learns that this
device uses ratings and nothing about which apps are looked at. Apps are matched by
AppStream id: directly for Flathub, and for distribution packages and GitHub projects
through the catalog the distribution already keeps on disk. An app with fewer than
five ratings shows none.

Written reviews come from the same service, for apps it knows, up to eight, best
first. A request for reviews names the app, so Bilby sends as little as it can with
it: no user id, a system name of "unknown" and no app version, where other stores
send a hash of the user and machine. The Settings switch covers ratings and reviews
together, says this in its description, and stops both.

### Signs of how much an app is used

A store that mixes sources has no single number. The rule is: use everything a source
gives away for free, say plainly what each figure counts, and never show a blank or a
zero as if it were a verdict.

| Source | Shown | Counts towards search order | Left out on purpose |
| --- | --- | --- | --- |
| GitHub, Codeberg | Stars; downloads of the listed releases' files; "archived" notice | Both, and the notice | Forks, watchers, open issues: they measure developer activity, not use |
| Flathub | Installs this month and in total; verified developer | Installs, verification, a rising trend | The favourites count, which is tiny beside installs |
| AUR | Votes; out-of-date and unmaintained notices | Votes, the AUR's time-weighted popularity, the notices | |
| Distribution packages | Their rating, where the ratings service knows them | The rating; otherwise a neutral value | |
| Ratings service | Average, number of ratings, share at each star, reviews | The average, trusted in proportion to the number of ratings | |

- **One figure on a tile.** The rating with its count, else downloads, else the
  source's own count. An app with none shows its author, not an empty badge.
- **Every figure on the app's page**, each captioned with what it counts: "194k
  downloads, of recent versions", "13k stars, on GitHub".
- **The rating as a breakdown**: the average, the number of ratings and a bar per
  star, following Baymard Institute's findings that people rely on the number and
  spread of ratings more than on the average
  ([distribution summary](https://baymard.com/blog/user-ratings-distribution-summary)).
- **A count of zero is not shown.** Zero stars usually means "new", not "bad".
- **Downloads before stars**, because a star costs nothing to give: a 2024 study found
  millions of suspected fake stars on GitHub
  ([He et al.](https://arxiv.org/abs/2412.13459)). A star count with almost no
  downloads lowers an app's score a little for the same reason.

Counting how many people use Bilby itself would need telemetry, and is ruled out.

### The store's own updates

Bilby treats its own project (`STORE_REPOSITORY` in `catalog.rs`) as one more app.
At start-up it asks for the newest release with a file for this device and compares
it with its own version. Updating places the new copy, points the app-menu entry at
it and offers "Restart Bilby now / Later"; an older copy that is started later hands
over to the newer one. There is no separate updater and no update server: the same
rules apply as for any app, including the checksum.

`.github/workflows/release.yml` builds every release: an AppImage for Linux, a
program for Windows and a disk image for Apple-silicon Macs, each with the
command-line tool alongside, named so that Bilby's own classifier picks the right one
(`the_stores_own_release_files_resolve_on_every_desktop`).

## 6. Installing (`install/`)

`plan()` is pure and can be shown before anything is downloaded; `execute()` carries
it out. Bilby never runs as administrator itself.

| Kind | What happens |
|---|---|
| AppImage | Download, verify, mark executable, move to `~/Applications`, add to the app menu |
| Flatpak, from Flathub or a bundle | Flatpak installs it; Bilby opens and removes it through Flatpak |
| Distribution package, snap, winget | The package manager installs it, behind the system's password prompt where one is needed |
| deb, rpm or pacman file from a release page | Download, verify, hand to the system's installer |
| exe, msi, msix | Download, verify, open |
| dmg, pkg | Download, verify, open |
| Archive, bare program | Download, verify, show in the downloads folder |
| AUR | A terminal opens on the build |
| Web app | An entry among the device's apps |
| Steam, Mac App Store, Microsoft Store | The other store's app takes over |

Downloads can be cancelled; a cancelled or failed download leaves no file behind.
What an install needs first (FUSE for an AppImage, Longboard for a website on a TV) is
installed as part of it.

### Library and updates

Every install is recorded with its version and what was installed. "Check for
updates" (also run quietly at start-up) asks each source for the newest release with
a file for this device and compares versions (`version.rs`: numeric components,
pre-releases below their release, never a downgrade). One request per app, cached.

"Update" on the Library page updates in place, with progress on the row; it can be
cancelled, and a failure leaves the old version untouched. "Update all" queues every
waiting update and works through them one at a time.

Open works for AppImages, Flatpaks, Steam games and web apps. Remove deletes what
Bilby placed, has Flatpak uninstall what Flatpak holds, and takes down the app-menu
entry. Apps installed by anything else on the device are recognised and marked
Installed.

App pages list every recent release (up to six) with a file for this device, so an
older version can be chosen when an update breaks something; the page says it is not
the newest and the library shows the pending update straight away.

### Appearing among the device's apps

An AppImage is one file, and nothing else puts it in the app menu. So Bilby does:
every AppImage it places gets an entry with its own icon, and a new version replaces
the entry. On its first real start Bilby also writes its own entry
(`~/.local/share/applications/bilby.desktop`), pointing at the file it was started
from, and corrects it if that file moves. Settings has a switch that removes the
entry and keeps it away.

This is why Bilby ships as an AppImage and not as a Flatpak: a store has to run the
system's package manager, start other programs and write app-menu entries, which is
what a Flatpak's sandbox is designed to restrict.

## 7. The three interfaces

All three are Slint components in one program, chosen when it starts
(`ui/main.slint`): TV on a TV, otherwise by the window's width, so a phone, a
split-screen tablet and a narrow desktop window all get the phone layout. The
screenshots in `docs/screenshots/` are made by the real program, driven by a script.

- **Desktop**: a sidebar (Discover, Search, Library, Settings), an always-visible
  search box, a spotlight and category chips, and a grid of cards that reflows with
  the window. Fully usable from the keyboard: Ctrl+K or `/` to search, Escape or
  Alt+Left to go back, Tab through every control with a visible focus ring. Browse
  pages keep their place while an app's page is open.
- **Phone**: one column, four tabs along the bottom, full-screen app pages. Android's
  Back button leaves an app's page. Safe-area insets are respected.
- **TV**: no pointer. Exactly one focused element, with a thick white outline; what is
  off screen scrolls to keep it in view; every size is a fraction of the screen's
  height, so 1080p and 4K look the same. See [TV](#tv).

Shared by all three: light and dark themes (following the system; the TV is always
dark), a first-run notice that says what the store is, toasts, empty and error states
that offer a way forward, and accessible roles, labels and default actions on every
control, which is also how the tests find and operate them.

### What is shown, and what is one press away

The screens are made for someone who does not know what a `.deb` is, and look like
the stores that person already uses. Secondary options stay out of the way until
asked for
([Nielsen Norman Group on progressive disclosure](https://www.nngroup.com/articles/progressive-disclosure/)).

An app's page follows the order of the App Store and Google Play:

| Part of the page | App Store / Google Play | Bilby |
|---|---|---|
| Top | Icon, name, developer, one button | The same. The button says Install, Update or Open |
| Key facts | Rating, size, age rating, chart position | Rating, downloads, size, last updated, licence |
| Reassurance | "Works on this iPhone" | That Bilby checks the download is the genuine file; "Not for this device" where it is not |
| Screenshots | Up to ten | The source's own (Flathub), else up to four from the project's README, fetched only from the project's own host |
| Description | First lines, then "more" | The start of the description, with "Read more" |
| What's New | On the page and in Updates | Release notes, shown above the description once the app is installed |
| Ratings and reviews | Yes | Star ratings and written reviews for every app the ratings service knows |
| Information | A plain table at the bottom | The same: from, made by, version, released, licence, works on, kind of file, size |
| Everything technical | Not shown | Under *Other versions and files*: every file with its steps, older versions, and the hidden files with the reason for each |

Lists show the icon, name, one line and one figure, plus whether the app is installed
or has an update. They say nothing about packaging. The device is named in everyday
words ("This Linux computer"). Every tile has an Install or Update button that opens
the app's page and starts the right file, so the install can be watched and stopped;
a website's says Add.

### Motion

Things move for a reason, briefly, and the same way everywhere, as Android's Material
guidance and Apple's interface guidelines both ask
([Material motion](https://m3.material.io/styles/motion/overview),
[Apple: Motion](https://developer.apple.com/design/human-interface-guidelines/motion)).

| What | How it moves |
|---|---|
| Starting a download, install or update | The button fills from left to right and becomes Cancel; a bar below shows the same. When the amount is unknown, a band travels instead |
| Finishing | A tick pops in beside the result; a failure gets a plain mark |
| Changing page, opening an app | The new page fades up a few pixels (220 ms). Leaving is immediate |
| Pressing a button or a tile | It gives slightly and springs back; tiles lift under the pointer |
| Messages | Rise into place and fade |
| TV | The focused item grows and gains a border; screens fade in |

Settings has **Reduce motion**, which makes all of it instant.

**Sounds** (`app/src/sound.rs`). A tick as the highlight moves, a soft pop on a press,
a lower note on the way back. They are made in code from sine waves that die away
within a few hundredths of a second, so nothing is recorded or borrowed. They are
quiet by design, play from a thread of their own, and a device without sound simply
stays silent. One switch turns them off.

### Signing in to GitHub (`github_login.rs`)

Nothing needs a GitHub account. Signing in only lifts GitHub's allowance from 60
requests an hour to 5,000.

It uses GitHub's device flow
([GitHub docs](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow)):
Bilby shows a short code, the person enters it at `github.com/login/device` on any
device, and Bilby is handed a token. That suits a TV, where typing a token with a
remote is not realistic, and it needs no server and no secret. The token is asked for
with no permissions and is sent only to `api.github.com`. Sign out forgets it. A
personal token can be pasted instead.

### Knowing it is a TV

A 4K monitor and a 4K TV look the same to a computer, so the TV layout is chosen in
three steps, the first that applies:

1. **The command line**: `--ui tv` or `--ui desktop`.
2. **The choice in Settings** (Layout: Automatic, Desktop, TV). It is remembered and
   switches at once. From the TV layout the way back is on the "This TV" page, where
   a remote can reach it.
3. **What the device looks like.** Android says when it is a TV. Elsewhere, Bilby
   recognises the sessions people run on a TV: Plasma Bigscreen, Steam's big-screen
   session (gamescope) and Kodi.

For an ordinary desktop shown on a television, Bilby asks once when a game controller
is attached: "Are you using this on a TV?". The answer is remembered.

Choosing the TV layout is more than appearance: TV builds of apps are preferred, and
websites open in Longboard.

### The operating system's own updates (`os.rs`)

KDE Discover and GNOME Software update the system as well as apps, and so does Bilby.

**Where it lives.** The Library, already the place for "what needs updating", leads
with a card for the system: its name, one line of status and an Update button. The
card opens "This device": the status in large type, what is waiting, and everything
about the machine (system and version, kernel, desktop, processor, memory, storage).
On a TV it is the entry at the end of the top bar.

**How.** Through the system's own package manager, run the way a person would run it,
with nothing extra to depend on.

| System | Looking for updates | Installing them |
| --- | --- | --- |
| pacman (Arch, CachyOS, ...) | `checkupdates` where installed, else `pacman -Qu` | `pkexec pacman -Syu --noconfirm` |
| apt (Debian, Ubuntu, ...) | `apt-get -s upgrade` (a dry run) | `pkexec`: `apt-get update`, then `apt-get -y upgrade` |
| dnf (Fedora, ...) | `dnf check-update` | `pkexec dnf -y upgrade --refresh` |
| Flatpak | `flatpak remote-ls --updates` | `flatpak update -y` |
| Image-based (Fedora Atomic, SteamOS) | Flatpak | Flatpak; the page points to the system's own updater for the rest |
| Windows, macOS | | A button opens Windows Update or Software Update |

Rules it keeps:

- **Looking never asks for a password and never changes anything.** In particular it
  never runs `pacman -Sy`: refreshing pacman's lists without upgrading is the partial
  upgrade Arch does not support
  ([ArchWiki](https://wiki.archlinux.org/title/System_maintenance)). `checkupdates`
  looks in a private copy instead.
- **Installing asks through the desktop's own password prompt.** Bilby never sees the
  password. A closed prompt is reported as "nothing was changed".
- **The age of the lists is shown**, and "Check again" refreshes stale ones.
- **A restart is called for** when the system says so, or when a newer kernel has
  replaced the running one.
- **The demo and `--as` previews use a made-up machine** and cannot touch the real one.

### TV

The TV interface follows Android TV's published guidance, and tvOS's where they agree:

| Rule | Source | Bilby |
|---|---|---|
| Keep content inside the overscan-safe area: 48dp at the sides, 27dp top and bottom | [Android TV layouts](https://developer.android.com/design/ui/tv/guides/styles/layouts) | Margins are exactly these; backgrounds run to the edge |
| Everything reachable with the D-pad; Back goes to the previous place | [TV app quality](https://developer.android.com/docs/quality-guidelines/tv-app-quality) | Arrows, OK and Back do everything |
| Exactly one focused element, shown by scale and border | [Focus system](https://developer.android.com/design/ui/tv/guides/styles/focus-system) | A white border and about 1.05x growth |
| Text readable from 3 m | [Design for TV](https://developer.android.com/design/ui/tv/guides/foundations/design-for-tv) | Body text 12 to 14dp |
| Four cards fill a row | [Cards](https://developer.android.com/design/ui/tv/guides/components/cards) | Four tiles a row, 20dp apart |

**Getting around.** Tabs along the top (Home, Library, Settings and the device's own
page), a wide search bar under them, then rows of apps with the focused one described
in large type. Up from the apps reaches search, then the tabs; Left and Right move
along; OK or Down goes in. Back closes the list of files, then the app's page, then
goes up to the bar, then Home. The search bar stays in place on a page of results, so
the next search is one press away, and typing with it highlighted goes straight in.

**An app's page** is a column to move down through: its buttons (the main one,
Favourite, Versions, Remove), a row of pictures, the description, a row of reviews
and its details. Reviews open one at a time, large. Versions lists the files of one
release; Left and Right move to older and newer ones. There is no legend of remote
buttons: arrows, OK and Back do what they do on every TV.

**The Back key.** Remotes send a key of their own for Back (`XF86Back`), which
Slint's desktop backend does not pass on. Bilby reads the window system's events
itself and turns it into Escape, and Select into Return. In the TV layout the right
and "back" mouse buttons do the same, because air-mouse remotes send those.

**Long rows stay fast.** Only the tiles on or beside the screen are drawn, however
long a row is.

The TV layout was shaped by use: the project's owner runs it on a Linux TV with a
physical remote, and the navigation above is the result of that feedback.

### The system's password on a TV (`askpass.rs`)

Changing the system (its packages, a new release of it) needs the password of someone
allowed to. The system's own prompt is a box nobody can type in with a remote. So in
the TV layout Bilby asks itself, with its on-screen keyboard, and hands the answer to
`sudo` through the helper `sudo -A` asks with: a small program that reads from a pipe
Bilby waits at. The password goes from the screen to `sudo` and nowhere else; only
the number of characters typed is shown or held by the interface. In the desktop
layout the same stand-in runs the system's `pkexec` as before. It needs `sudo` and a
person allowed to use it.

### Languages (`i18n.rs`, `scripts/i18n.py`)

Bilby comes in twelve languages: English, Spanish, French, German, Portuguese
(Brazil), Italian, Russian, Chinese (Simplified), Japanese, Korean, Hindi and Arabic.
It starts in the system's own where that is one of them, and Settings has the list
(on a TV, a row that steps through it). The heading of that setting is always in
English as well, so it can be found by someone who cannot read the language Bilby
is in. `--lang <code>` sets it for one run.

Text is written once, in English, where it is used:

- The interface's own text is `@tr("...")` in the `.slint` files. Slint builds the
  translations into the program from `i18n/po/<language>/LC_MESSAGES/bilby.po`.
- What the code puts on screen (notices, errors, what an install did, the summaries
  in the lists Bilby carries) passes through `i18n::tr` on its way to the screen. A
  message with something filled in ("Plex is in your library") is recognised by its
  pattern ("{} is in your library"), so the code that makes messages, the core
  included, is unchanged and still speaks English to logs and tests.

`scripts/i18n.py` gathers the English into `i18n/en.json` and writes the `.po` files
from `i18n/<language>.json`, one file per language: English text to translated text,
or a list of forms for text that counts things (Russian has three, Arabic six). CI
runs it with `--check`; a test holds every language to saying everything, with the
same things filled in.

Names and descriptions of apps are the stores' own. Flathub is asked for them in the
language in use and gives them where an app's makers published them; everything else
(most projects on GitHub, reviews) stays as it was written. Nothing is sent to a
translation service.

Not done: Arabic is written right to left but the layout is not mirrored; the
translations were written without a native speaker's review; dates and large numbers
keep their English form.

## 8. Testing

More than 400 tests, none needing the internet. CI runs them on Linux, Windows and
macOS on every push, builds the Android app, and checks that the core compiles for
iOS.

| Suite | Tests | What it exercises |
|---|---|---|
| `bilby-core` unit tests | 221 | Detection, classification, compatibility, versions, state, install plans and their execution, every source's parsing, ranking, merging, the home page |
| `core/tests/corpus.rs` | 4 | 169 file names; the cross-system safety property; 20,000 generated names |
| `core/tests/http_sources.rs` | 28 | The real HTTP client and the sources against a local server: headers, token scoping, rate limits, progress, cancelling, the cache, malformed answers |
| `cli/tests/cli.rs` | 9 | The `bilby-cli` program end to end, including install, library, updates and `doctor` |
| `app/tests/ui/` | 121 | The real window on Slint's headless backend: presses by accessible label, key presses, all three interfaces |
| `app/tests/offline.rs` | 7 | The same window with a network that goes offline or runs out of allowance on demand, then recovers |
| `app/tests/pictures.rs` | 1 | App pictures: fetched, decoded, shown, cached, harmless when missing |
| `app/tests/binary.rs` | 7 | The finished program (threads, event loop, renderer) under a virtual display |
| `app/tests/contrast.rs` | 1 | WCAG contrast of every text and background pairing, in both themes |
| `app/tests/languages.rs` | 1 | Every language says everything, and the interface follows the one chosen |
| App and sign-in unit tests | 18 | Option parsing, release notes, formatting, the sign-in window's rules |

What the tests guarantee, beyond behaviour:

- Every button, tab and switch has an accessible name, and all text reaches 4.5:1
  contrast in both themes.
- Every control on every page fits a 320 px wide phone
  (`everything_fits_on_the_narrowest_phone`).
- A state file that cannot be read or written produces a message, never a crash.
- Nothing is ever offered for another system or processor.

Beyond the tests, the store has been run for real: searches, app pages and installs
against the live sources on Debian, Fedora, openSUSE and Arch, and the release
workflow on every release. The [README](../README.md#how-ready-is-it) keeps the
current list of what has been tried on which system.

## 9. Platform support

| Platform | Where it stands | Next |
|---|---|---|
| Linux | Released as an AppImage; in use on a Linux TV with a remote | A package in the AUR |
| Windows | Released as a single program; the full test suite passes there on every push | Signing by a registered developer |
| macOS | Released as a disk image for Apple silicon; the full test suite passes there on every push | Signing and notarisation |
| Android phone, tablet and TV | Released as one package for 64-bit ARM; run in an emulator ([details](ANDROID.md)) | The hand-over to Android's installer, and trying it on real devices |
| iOS | The core compiles for iOS on every push | See below |

### iOS

Slint runs on iOS ([Slint 1.12 release notes](https://slint.dev/blog/slint-1.12-released)),
so the interface carries over. What iOS decides is how apps are installed: an app
there does not install other apps, and alternative marketplaces in the EU carry only
apps Apple has notarised
([Apple support](https://support.apple.com/en-mide/118110)).

So Bilby on iOS is a **catalog and companion**: browse everything, see which projects
publish an `.ipa`, and pass the file to the sideloading tool the person already
uses. The desktop app offering "send to my phone" serves the same people from the
other side.

## 10. UI toolkit

**Slint 1.19**, chosen over Dioxus and egui for three reasons:

- **One renderer on every target.** Slint draws the interface itself, the same way on
  a desktop, a phone and a TV, with no dependence on a system web view.
- **Focus is ours to manage**, which is what a remote-driven interface needs. Slint's
  Android backend maps the D-pad to arrow keys, its centre to Return and Back to a
  Back key, so the same code serves a remote and a keyboard.
- **A software renderer**, which makes the real program testable and its screenshots
  reproducible without a graphics card.

Custom widgets were written for the store's own look, and release notes are shown as
plain text.

**Licence.** Slint is offered under GPLv3, a commercial licence, or a royalty-free
licence for desktop, mobile and web applications. Bilby is `GPL-3.0-only`: it is
unambiguous, it covers the TV as well as the desktop, and it suits a store that
should stay open. The full text is in `LICENSE`, every crate declares it, and all 681
dependencies were checked on 2026-11-21 for compatibility with it. `crates/core` has
no Slint dependency and could be licensed separately if that were ever wanted.

## 11. Safety

- **Downloads are verified** against the publisher's SHA-256 wherever the source
  provides one, before anything is opened. A file that does not match is deleted.
- **Bilby says where an app comes from.** Flathub, Steam, the Mac App Store and
  distributions review what they carry; a release page is its author's own. Each
  app's page names its source, author, popularity and licence, and marks test
  versions, abandoned projects and out-of-date packages.
- **Bilby never runs as administrator.** What needs those rights goes through the
  system's own password prompt, and Bilby never sees the password.
- **Names are not trusted.** File names are made safe before touching the file
  system, project names are validated before going into an address, and app-menu
  entries are escaped as the desktop specification requires, because all of these
  come from release authors.
- **Only links Bilby built are opened**, checked again at the moment of opening.
- **Secrets stay put.** A GitHub token goes to GitHub's API only; a Steam key is used
  once and dropped; the state file is private to its owner.

## 12. Next steps, in order of value

1. On Android: the hand-over to Android's installer, and trying the package on real
   phones and TVs.
2. Signed releases for Windows and macOS.
3. A package of Bilby in the AUR.
4. Regenerate the bundled starter list at build time, with file names, so Discover
   can say what would be installed before any request.
5. Read an APK's own manifest after download, for exact processor and TV support,
   and ask Windows directly which processor it runs on (`IsWow64Process2`), for an
   x64 build of Bilby running on Windows on Arm.
6. System updates on openSUSE, and moving to a distribution's next release.
7. Top charts, and Open for packages the system installed.
8. More sources: self-hosted Forgejo and Gitea, Nix; more of what winget lists for
   Linux (`.msi` installers and archives); and Android apps on Linux through a
   translation layer.
9. Languages: a native speaker's review of each, mirrored layout for Arabic, more
   languages.

[COMPARISON.md](COMPARISON.md) sets Bilby beside Komi Store, Obtainium, UniGetUI and
others, feature by feature.
