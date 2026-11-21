<p align="center">
  <img src="crates/app/ui/icons/logo.svg" width="104" alt="Bilby logo: a long-eared bilby face">
</p>

<h1 align="center">Bilby</h1>

<p align="center">
  <b>An app store that only shows what runs on your device.</b><br>
  Every kind of app: free and paid, open source and not, games included.<br>
  For Windows, macOS and Linux, on a desktop or a TV. Android is on the way.<br>
  No Bilby account. No server of ours. No tracking.
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

Apps are scattered. One is in your system's package manager, another on a store, a
third is a page of 18 download files where you have to work out which one is yours.

**Bilby puts them in one place and works out what fits.** It looks at your system
(Windows, macOS, or which kind of Linux), your processor and the kind of device you
are on, shows only what runs there, and gives each app one **Install** button.

The catalog is as big as the sources themselves. Bilby does not keep its own list of
approved apps; it searches each source's whole catalog live.

It is not limited to one kind of app or one kind of source. It searches the places
your device can install from: package managers (winget, Homebrew, Flathub, the Snap
Store, your Linux distribution's repositories), other stores (Steam, the Mac App
Store) and developers' own release pages (GitHub, GitLab, Codeberg). The
[full list](#where-apps-come-from) is below.

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

- Search whole catalogs at once, not a hand-made list: everything in winget on
  Windows, Homebrew and the Mac App Store on a Mac, Flathub, the Snap Store and your
  distribution's repositories on Linux (tens of thousands of packages on a typical
  system), plus Steam and every project with releases on GitHub and GitLab.
- Or browse the home page. It opens with the apps and sites nearly everyone looks for
  (a browser, a video player, the big names), each in the version for your device
  and chosen for the kind of device it is: things to watch and play on a TV, a
  browser and the usual first installs on a computer. It is a list of about 140,
  chosen by people, which you can switch off in Settings. Below it are
  web apps by purpose and, on Linux, what is popular and trending on Flathub.
- One app is one result, even when several sources have it. Its page shows which
  source the button uses and lets you switch.
- Different ways to use the same service are gathered on one page: for YouTube,
  Spotify, Discord and ten or so others, the website as a web app, the service's own
  app and the best-known ones by others, so you can pick what suits your device.
- Narrow a search to one source, or switch off in Settings the sources you never
  want. The app with exactly the name you typed comes first; after that, how many
  people use an app counts for more than how its name is spelt. On a TV a search
  fills the page: top results, then rows for apps, games, web apps and packages.
- Adult content is part of the catalog and handled with care. In a search it has a
  section of its own, each item covered until you choose it, and choosing one asks
  you to confirm you are an adult. It appears on the home page only if you ask for
  that. Those who want no cover at all can switch it off.
- **Family mode** removes adult content altogether, and gambling and piracy sites
  with it: not listed, not covered, not openable. A four-digit **PIN** can guard that, the other family settings, adult
  content, and, if you want, installing or removing anything.

**Install it**

- One button installs it. For a downloaded file, Bilby fetches the right one for your
  device and hands it to your system's installer: an `.exe` or `.msi` on Windows, a
  `.dmg` or `.pkg` on macOS, a package, AppImage or Flatpak on Linux. For a package
  from winget, the Snap Store or your distribution, it runs the install itself, and
  your system asks for your password when one is needed.
- The one exception is the AUR, where building asks questions as it goes: Bilby opens
  a terminal on the build, and shows the command only if it cannot find a terminal.
- Downloads are checked against the checksum the source publishes, when there is one.
- Bilby checks what an app needs before you download it. An AppImage on a Linux system
  without FUSE 2 says so on its page, and Bilby installs the missing piece from your
  distribution along with the app. An app that closes the moment it starts is reported with
  the reason.
- Every other file and every older version is one press away, never hidden for good.
- For things only another store can install (Steam, the Mac App Store), Bilby opens
  that store's app on the right page and lets it take over. Buying, downloading and
  updating happen there, not in Bilby. If Steam itself is missing, Bilby offers to
  install it.
- Optionally, sign in to Steam to have the games you already own marked Owned. On
  every device you choose how: scan a QR code with the Steam app on your phone, or
  sign in on Steam's own page in a small browser window (Longboard on a TV). Bilby
  never sees your password and keeps only the list of what you own.

**Keep everything up to date**

- The Library lists what you installed through Bilby and tells you when something
  newer is out.
- Update one app or all of them, or skip a version you do not want.
- On Linux, Bilby can update the system itself too, through pacman, apt, dnf and Flatpak.

**Use it from the sofa**

- The TV layout is driven with a remote: arrows, OK and Back. Home opens on a wide
  search bar with the apps below it; Library, Settings and the device's own page are
  tabs along the top. An app's page is a column to move down through: its
  buttons, a row of pictures (OK shows one large), the description (OK opens it, with
  any links to follow), a row of reviews, and its details. Today that means a
  computer connected to a TV; Android TV comes with the Android package.
- On a Linux TV, websites can be added like apps and open in
  [Longboard](https://github.com/AndrewGBest/Longboard), a browser made for remotes.

## Where apps come from

Each source only appears on devices it applies to. A Windows user never sees the AUR.

| Source | Shown on | What you get |
|---|---|---|
| **GitHub Releases** | Windows, macOS, Linux | Any project, open source or not, that publishes files with its releases |
| **GitLab** | Windows, macOS, Linux | The same, for projects on GitLab.com |
| **winget** | Windows | The Windows Package Manager's catalog |
| **Homebrew** | macOS | Thousands of Mac apps. Bilby downloads each from its publisher and checks it; Homebrew itself is not needed |
| **Flathub** | Linux with Flatpak | Its whole catalog, with ratings, reviews and install counts |
| **Your distribution** | Arch (pacman), Debian and Ubuntu (apt), Fedora (dnf) and openSUSE (zypper) families | Every package your system is already set up to install |
| **The AUR** | Arch-based systems | Community build recipes |
| **Snap Store** | Linux with snapd | Its whole catalog |
| **F-Droid** | Android (when the Android app arrives) | Its catalog of free Android apps |
| **Steam** | Windows, macOS, Linux | Games and apps, free and paid, with their prices. Bilby hands over to the Steam app, which sells, installs and runs them |
| **Mac App Store** | macOS | Its catalog, free and paid, with prices and ratings. Bilby hands over to the App Store app |
| **Codeberg** | Windows, macOS, Linux | Projects by link and from the home page. Codeberg is turning away searches at the moment |
| **Your own sources** | Everywhere | A list of apps on your own server or storage, added by its address ([how to publish one](docs/OWN-SOURCES.md)) |
| **Web apps** | Windows, macOS, Linux | Websites added like apps: about ninety well-known ones recommended, a thousand more found by name, and any address you type. They open in your own browser, or in Longboard on a Linux TV |

## Your privacy

Bilby has no server and no account of its own, so there is nobody for it to report to.
Your library and settings are one file on your device. Bilby talks directly to the
sources, and what leaves your device is what those requests carry:

| When you | Bilby contacts | It learns |
|---|---|---|
| Search or open an app | Every source switched on for your device (a search), or that app's source (opening it) | What you searched for or opened, and your network address, as with any website |
| Open Bilby, or check for updates | The sources of the apps in your library, and GitHub for Bilby's own releases | Which apps were checked for updates. Can be switched off for start-up |
| See a web app's tile | That website, and wherever it keeps its icon | That someone fetched its front page and icon, once; the icon is then kept on your device |
| Have ratings switched on | The ratings service GNOME and KDE use | That *someone* downloaded the full list of ratings, once a day |
| Open an app that has reviews | The same service | Which app's reviews were asked for. No user id is sent |
| Sign in to Steam | Steam | That this device signed in, as with any browser or app. Optional. With a QR code, Steam gives Bilby a key to the account, which Bilby uses once to read the list and then discards; in a browser, no key reaches Bilby at all. Either way only the list of what you own is kept, on this device, and Sign out removes it |
| Sign in to GitHub | GitHub | Which account is asking. Optional; it only raises GitHub's hourly limit, and Bilby asks for no access to the account |

Ratings and reviews can be switched off in Settings, which stops both requests.

## How ready is it

Bilby is early. It works on Linux, where it has been tried for real; much of the rest
is written and tested but not yet tried on a real machine. Plainly:

| | State |
|---|---|
| **Linux** | Works. Tried on Debian 13: real searches on GitHub, GitLab, Flathub, apt and Steam, and a real install from Flathub. Search and package details also run for real on Fedora and openSUSE |
| **Installing distribution packages** | The install commands were run for real on Fedora and openSUSE. Doing it through the system's password prompt has not been tried on a real desktop, nor has any of it on Arch |
| **Snap Store, AUR** | Written and tested against stand-ins. Not tried end to end |
| **Steam, Mac App Store** | Searching Steam works for real. Handing over to the Steam or App Store app has not been tried on a machine that has them |
| **Signing in to Steam** | Nobody has completed a sign-in yet, by QR code or browser. On a TV, the browser way also waits on a Longboard release |
| **Windows, macOS** | Downloads are published and the tests pass on both. Nobody has checked the window by eye yet, so expect rough edges |
| **winget** (Windows) | Written without a Windows machine to try it on. English-language Windows only |
| **Homebrew** (macOS) | Searching and choosing the right download work against the real catalog. Installing from it has not been tried on a Mac |
| **Android** (phone, tablet, TV) | Not installable yet. The core compiles for it and the phone and TV layouts exist |
| **iOS** | Could only ever browse, not install ([why](docs/DESIGN.md#ios)) |
| **System updates** | Finds updates correctly on Debian. Installing them has not been run on a real system. Not available on openSUSE yet |
| **Arch** | Written from documentation and tested against stand-ins, not on a real system |
| **TV** | Works with arrow keys, OK and Back. Used on a Linux TV with a physical remote by the project's owner, whose reports shaped the layout; not tried on other TVs or remotes |

Bilby does not review apps itself. Some sources do (Flathub, Steam, the Mac App Store,
your distribution); a release page on GitHub is checked by nobody. Bilby says which
source an app comes from and warns about test versions, abandoned projects and
out-of-date packages, but an app is only as trustworthy as whoever published it.

## Good to know

- **GitHub's limit.** Without signing in, GitHub allows 60 requests an hour for
  everyone on your network. Bilby tells you when it runs out. Signing in to GitHub in
  Settings (you type a short code on GitHub's site) raises it to 5,000. Flathub and your distribution's packages have no such limit.
- **Where things go.** Downloaded files are saved in `Downloads/bilby` in your home
  folder unless you choose another folder in Settings, where you can also have Bilby
  ask each time. AppImages are then moved to `Applications` in your home folder;
  packages, Flatpaks and store apps go where your system puts them.
- **Reduce motion.** Settings can turn animations off.
- **Sounds.** A quiet tick or pop as you move about and press things. Settings can
  turn them off.
- **From a terminal.** `bilby-cli` can search, show, install and check for updates, and
  can export your library to move it to another device. It does not do everything the
  app does: system updates and signing in are in the app only.

## Learn more

| | |
|---|---|
| [How Bilby compares](docs/COMPARISON.md) | Next to Komi Store, Obtainium, GNOME Software and others, and what it leaves out on purpose |
| [Design notes](docs/DESIGN.md) | How it decides what runs on your device, and every trade-off made along the way |
| [Building and testing](docs/DEVELOPMENT.md) | Run it from source, try it as another device, and work on it |

## Licence

Bilby is free software under the [GNU General Public License, version 3](LICENSE)
(GPL-3.0-only). You may use, study, share and change it; if you share a changed
version, it must be under the same licence. It comes with no warranty.

That licence covers Bilby itself. The apps it lists are not part of Bilby and have
whatever licences their publishers chose, including paid and closed-source ones.

Why this licence: the interface toolkit, Slint, is used under GPLv3
([details](docs/DESIGN.md#10-ui-toolkit)).
