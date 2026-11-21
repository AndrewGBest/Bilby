<p align="center">
  <img src="crates/app/ui/icons/logo.svg" width="104" alt="Bilby logo: a long-eared bilby face">
</p>

<h1 align="center">Bilby</h1>

<p align="center">
  <b>An app store that only shows what runs on your device.</b><br>
  For Windows, macOS and Linux, on a desktop or a TV. Android is on the way.<br>
  No account. No server of ours. No tracking.
</p>

<p align="center">
  <a href="#download">Download</a> ·
  <a href="#what-you-can-do">What it does</a> ·
  <a href="#where-apps-come-from">Sources</a> ·
  <a href="#how-ready-is-it">Status</a> ·
  <a href="docs/DEVELOPMENT.md">Build it</a>
</p>

![Bilby's Discover page on a desktop](docs/screenshots/desktop-discover.png)

## The idea

Outside the big app stores, getting an app usually means a page of download files and
working out which one is yours.

**Bilby works it out for you.** It looks at your system (Windows, macOS, or which kind
of Linux), your processor and the kind of device you are on, and turns a release with
18 files into one **Install** button. The other 17 stay out of the way unless you ask
for them.

It gathers apps from wherever they are published: project release pages on GitHub and
Codeberg everywhere, plus Flathub and your distribution's own repositories on Linux.

| On a desktop | On a phone | On a TV |
|:---:|:---:|:---:|
| <img src="docs/screenshots/desktop-details.png" alt="An app's page on a desktop"> | <img src="docs/screenshots/mobile-details.png" width="220" alt="An app's page on a phone"> | <img src="docs/screenshots/tv-home.png" alt="The home screen on a TV"> |
| Mouse and keyboard | Touch (the layout exists; no Android package yet) | Remote only |

<sub>The screenshots show the built-in demo catalog. The apps in it are made up.</sub>

## Download

Get the newest version from the [releases page](https://github.com/AndrewGBest/Bilby/releases/latest).
One file per system, and each runs as it is:

| Your system | File to download | To start it |
|---|---|---|
| **Linux** (x86_64) | `bilby-…-x86_64.AppImage` | Make it executable (`chmod +x`, or tick "executable" in the file's Properties), then open it |
| **Windows** (x64) | `bilby-…-x86_64-windows.exe` | Open it. Windows will warn about an unknown publisher: choose *More info*, then *Run anyway* |
| **macOS** (Apple silicon) | `bilby-…-aarch64-macos.dmg` | Open it and drag Bilby to Applications. The first time, right-click the app and choose *Open* |

On Linux, Bilby adds itself to your app menu the first time it runs.

> [!NOTE]
> The files are not signed by a registered developer yet, which is what the Windows and
> macOS warnings are about.

There is no Android package yet, and no Flatpak
([why not](docs/DESIGN.md#appearing-among-the-devices-apps)).

## What you can do

**Find an app**

- Browse a short starter list of well-known apps that ships with Bilby. On Linux, also
  what is popular and trending on Flathub.
- One app is one result, even when several sources have it. Its page shows which
  source the button uses and lets you switch.
- Different apps for the same service (YouTube has several) are gathered on one
  page, so you can pick the version that suits your device.
- Search every source your device has at once, or narrow to one. Sources you never
  want can be switched off in Settings. The best match comes first: the app
  with the name you typed, then the ones most people use.
- See what you are getting: pictures and a description, and, where the source provides
  them, download counts, star ratings, written reviews and whether the developer is
  verified. Ratings and reviews exist mainly for Linux apps.

**Install it**

- One button installs it. Bilby never hands you a command to type. It fetches the
  right file for your device and hands it to your system's installer: an `.exe` or `.msi` on Windows, a `.dmg` or `.pkg` on macOS, a package,
  AppImage or Flatpak on Linux.
- Downloads are checked against the checksum the source publishes, when there is one.
- Every other file and every older version is one press away, never hidden for good.
- For things only another store can install (Steam, the Mac App Store), Bilby opens
  that store on the right page and lets it take over. Paying and signing in happen
  there, not in Bilby.
- Optionally, sign in to Steam to have the games you already own marked Owned. You
  sign in on Steam's own page, in a small window (in Longboard on a TV). Bilby is
  given the list of what you own and nothing else: no password, and nothing that
  could act as your account.

**Keep everything up to date**

- The Library lists what you installed and tells you when something newer is out.
- Update one app or all of them, or skip a version you do not want.
- On Linux, Bilby can update the system itself too, through pacman, apt, dnf and Flatpak.

**Use it from the sofa**

- The TV layout is driven with a remote: arrows, OK and Back. Today that means a
  computer connected to a TV; Android TV comes with the Android package.
- On a Linux TV, websites can be added like apps and open in
  [Longboard](https://github.com/AndrewGBest/Longboard), a browser made for remotes.

## Where apps come from

Each source only appears on devices it applies to. A Windows user never sees the AUR.

| Source | Shown on | What you get |
|---|---|---|
| **GitHub Releases** | Windows, macOS, Linux | Any project that publishes files with its releases |
| **GitLab** | Windows, macOS, Linux | The same, for projects on GitLab.com |
| **winget** | Windows | The Windows Package Manager's catalog |
| **Homebrew** | macOS | Thousands of Mac apps. Bilby downloads each from its publisher and checks it; Homebrew itself is not needed |
| **Flathub** | Linux with Flatpak | Its whole catalog, with ratings, reviews and install counts |
| **Your distribution** | Arch (pacman), Debian and Ubuntu (apt), Fedora (dnf) and openSUSE (zypper) families | Every package your system is already set up to install |
| **The AUR** | Arch-based systems | Community build recipes |
| **Snap Store** | Linux with snapd | Its whole catalog |
| **F-Droid** | Android (when the Android app arrives) | Its catalog of free Android apps |
| **Steam** | Windows, macOS, Linux | Games and apps with their prices. Bilby hands over to the Steam app, which installs and runs them |
| **Mac App Store** | macOS | Its catalog with prices and ratings. Bilby hands over to the App Store app |
| **Codeberg** | Windows, macOS, Linux | Projects by link and from the home page. Codeberg is turning away searches at the moment |
| **Websites** | Linux TVs | Sites that open like apps in Longboard |

## Your privacy

Bilby has no server and no account, so there is nobody for it to report to. Your library
and settings are one file on your device. What does leave your device:

| When you | Bilby contacts | It learns |
|---|---|---|
| Search or open an app | That app's source (GitHub, Flathub, ...) | What you searched for or opened, as with any website |
| Have ratings switched on | The ratings service GNOME and KDE use | That *someone* downloaded the full list of ratings, once a day |
| Open an app that has reviews | The same service | Which app's reviews were asked for. No user id is sent |
| Sign in to Steam | Steam, from the sign-in window | What any browser signing in tells Steam. Optional. Bilby keeps the list of what you own on this device; Sign out removes it and the window's data |
| Sign in to GitHub | GitHub | Which account is asking. Optional; it only raises GitHub's hourly limit, and Bilby asks for no access to the account |

Ratings and reviews can be switched off in Settings, which stops both requests.

## How ready is it

Bilby works and is being used, but it is early. Plainly:

| | State |
|---|---|
| **Linux** | Works. Tried on Debian 13: real searches on every source and a real install from Flathub |
| **Windows, macOS** | Downloads are published and the tests pass on both. Nobody has checked the window by eye yet, so expect rough edges |
| **winget** (Windows) | Written without a Windows machine to try it on. English-language Windows only |
| **Homebrew** (macOS) | Searching and choosing the right download work against the real catalog. Installing from it has not been tried on a Mac |
| **Android** (phone, tablet, TV) | Not installable yet. The core compiles for it and the phone and TV layouts exist |
| **iOS** | Could only ever browse, not install ([why](docs/DESIGN.md#ios)) |
| **System updates** | Finds updates correctly on Debian. Installing them has not been run on a real system |
| **Arch, Fedora** | Written from documentation and tested against stand-ins, not on real systems |
| **TV** | Works with arrow keys, OK and Back. Not yet tried with a physical remote |

Nobody reviews the apps in the catalog. Bilby warns about test versions, abandoned
projects and out-of-date packages, but an app is only as trustworthy as its author.

## Good to know

- **GitHub's limit.** Without signing in, GitHub allows 60 requests an hour for
  everyone on your network. Bilby tells you when it runs out. Signing in to GitHub in
  Settings (you type a short code on GitHub's site) raises it to 5,000. Flathub and your distribution's packages have no such limit.
- **Reduce motion.** Settings can turn animations off.
- **From a terminal.** `bilby-cli` does everything the app does, and can export your
  library to move it to another device.

## Learn more

| | |
|---|---|
| [How Bilby compares](docs/COMPARISON.md) | Next to Komi Store, Obtainium, GNOME Software and others, and what it leaves out on purpose |
| [Design notes](docs/DESIGN.md) | How it decides what runs on your device, and every trade-off made along the way |
| [Building and testing](docs/DEVELOPMENT.md) | Run it from source, try it as another device, and work on it |

## Licence

GPL-3.0-only for now. Read [the licensing note](docs/DESIGN.md#10-ui-toolkit) before
relying on that: it was chosen as a starting point and may change.
