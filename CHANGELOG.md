# Changelog

What changed in each release, for people who use Bilby. The newest is first.
Versions follow [semantic versioning](https://semver.org); while Bilby is at 0.x,
any release may change how something works.

## Unreleased

- The words on an app's tile stay sharp with the pointer over it. The tile grew
  slightly when hovered, which smeared its text.

## 0.1.43 (2026-11-27)

- **A search for an app also lists others of its kind.** Looking for Chrome shows
  Chromium, Ungoogled Chromium, Brave, Vivaldi, Edge and Opera straight after it.
  Thirty kinds of app are covered, from browsers and video players to password
  managers, game launchers and code editors. Names of things Bilby does not have work
  too: "photoshop" leads to GIMP and Krita, "excel" to LibreOffice, "airdrop" to
  LocalSend. It goes with "Recommend the apps everyone knows" in Settings.
- Nothing is looked for or shown while you type in the search box, only when you
  press Enter. The suggestions that appeared with each letter are gone.

## 0.1.42 (2026-11-25)

- On a TV, the Updates section at the end of Settings can be seen. The list stopped
  short of it before, leaving it below the screen.

## 0.1.41 (2026-11-25)

- **Apps that Bilby did not install can be updated from Bilby.** Under "Also installed
  here", an app with an update waiting says so and has an Update button, and so does
  its page. Flatpak, apt and dnf update that one app. pacman updates everything
  together, so there Bilby asks whether to update the whole system.
- Apps the system's package manager installed can be opened from their page.
- After Bilby's logo changes, the app menu and a TV launcher show the new one. They
  kept the old picture before, because it was written over the same file.
- Bilby's own page in the store shows its logo.
- The release files come with a `SHA256SUMS` file and a signed record of where they
  were built.
- `bilby --version` and `bilby-cli --version`.
- For contributors: a contributing guide, a security policy, issue forms, and checks
  for the Android build, dependency licences and security advisories.

## 0.1.40 (2026-11-24)

- **The Library lists apps that were installed some other way**, under "Also
  installed here", and can remove them.
- **winget on Linux.** Windows programs from winget's catalog can be found and
  installed on Linux, where they run through Proton.
- **Web apps choose their browser.** A web app opens in your default browser when it
  can show a site in a window of its own; otherwise in another installed browser
  that can, and if there is none Bilby offers to install one. On a TV they still
  open in Longboard.
- A web app is also put on the desktop as an icon.
- An app's page says which source its download comes from, with a link to its page
  there.

## 0.1.39 (2026-11-23)

- **Write a review** from an app's page, on a desktop and on a TV. Where an app can
  be reviewed in more than one place you choose which, or all of them.
- **Windows apps on Linux**, run through Proton.
- **The first Android package**, for phones, tablets and TVs.
- **GOG** as a source, with sign-in to mark the games you own.
- More stores are shown on the systems they apply to.
- A new logo.

## 0.1.34 to 0.1.38 (2026-11-23)

- Automatic updates, and Bilby can wait in the system tray.
- Categories, with a page of them on the TV.
- A queue for downloads.
- Uninstalling apps that Bilby did not install.
- A switch to show apps that do not run on this device.
- A web app can open on a particular page of its site.
- More web apps for TVs.

## 0.1.28 to 0.1.33 (2026-11-22)

- **Twelve languages.**
- System updates show their progress; systems that are one whole image (Fedora
  Atomic) are updated their own way.
- An on-screen keyboard for the TV, and the system's password can be typed on it.
- A "Made for TV" section, and a service's own TV app comes first on a TV.
- Web apps open in a window of their own on a desktop.
- Many fixes to the desktop layout, icons and categories.

## 0.1.10 to 0.1.27 (2026-11-21)

- **Adult content** has a covered section and an age question; **Family mode**
  removes it, with gambling and piracy sites, and a PIN can guard the settings.
- **Web apps** on every system, with their own icons.
- **Your own sources:** a list of apps on your own server.
- Bilby says what an app needs before it is downloaded. An AppImage on a system
  without FUSE 2 is handled for you.
- Bilby updates itself and offers to restart.
- A recommended list of the apps nearly everyone looks for, chosen for the kind of
  device.
- Choose where downloads go, and which kind of app is offered first.
- Different ways to use one service are gathered on one page.
- The TV layout gained a Settings tab, a search bar on Home, and app pages that
  scroll.
- Sounds, which can be switched off.

## 0.1.0 to 0.1.9 (2026-11-20)

The first releases, as files that run as they are on Linux, Windows and macOS.

- Sources: GitHub, GitLab, Codeberg, the AUR, Flathub, the Snap Store, winget,
  Homebrew, F-Droid, and the package repositories of Arch, Debian, Ubuntu, Fedora and
  openSUSE.
- Steam and the Mac App Store, handed over to; sign in to Steam to mark what you own.
- One result for an app that several sources have.
- Desktop, phone and TV layouts.
- A library with update checks, favourites, skipping a version, export and import.
- Ratings and reviews.
- Updating the operating system from the Library.
- Signing in to GitHub with a code.
