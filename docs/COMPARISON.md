# Bilby next to other stores

What other stores for release files and packages offer, and where Bilby stands on each
point. Written 2026-11-20.

**How this was researched.** Only GitHub was reachable from where this was written, so
everything about other products comes from their repositories: READMEs, release notes
and metadata files. Project websites, F-Droid pages and wikis could not be opened. Each
claim names where it was read; nothing here was checked by running the other apps.
Release notes were read through a summarising tool, so wording is paraphrased.

Sources:

- **Komi Store** (formerly GitHub Store), the closest competitor:
  [README](https://github.com/komi-store/komi-store),
  [store listing text](https://raw.githubusercontent.com/komi-store/komi-store/main/fastlane/metadata/android/en-US/full_description.txt),
  [README at 1.7.0](https://raw.githubusercontent.com/komi-store/komi-store/1.7.0/README.md),
  [release notes 1.7.0 to 1.9.4](https://github.com/komi-store/komi-store/releases)
- **Obtainium**: [README](https://raw.githubusercontent.com/ImranR98/Obtainium/main/README.md),
  [interface strings](https://raw.githubusercontent.com/ImranR98/Obtainium/main/assets/translations/en.json)
- **UniGetUI**: [README](https://github.com/marticliment/UniGetUI)
- **Gear Lever**: [README](https://raw.githubusercontent.com/mijorus/gearlever/master/README.md);
  **AppImageLauncher**: [README](https://github.com/TheAssassin/AppImageLauncher);
  **AppImageUpdate**: [README](https://github.com/AppImageCommunity/AppImageUpdate)
- **GNOME Software**: [README](https://raw.githubusercontent.com/GNOME/gnome-software/main/README.md);
  **KDE Discover**: [README](https://raw.githubusercontent.com/KDE/discover/master/README.md)
- **Zapstore**: [README](https://raw.githubusercontent.com/zapstore/zapstore/master/README.md);
  **AM**: [README](https://raw.githubusercontent.com/ivan-hc/AM/main/README.md)

## Where Bilby is ahead

| | Bilby | Others, as far as their pages say |
|---|---|---|
| Release files **and** distribution packages in one store | GitHub, Codeberg, AUR, pacman repositories (incl. CachyOS), apt repositories | None of the stores read combines release files with the AUR and system repositories |
| TV interface | Yes, remote-only | None mentions one |
| No server of its own | Yes: talks only to the sources | Komi has relied on its own service for sign-in, discovery and caching since 1.8.0, with live GitHub as fallback |
| Says why a file is hidden | Yes, per file | Not described elsewhere |
| Linux package format, libc and emulation in the match | Yes | Komi describes CPU and OS matching |
| Command-line twin | `bilby-cli` | UniGetUI, Gear Lever, AM have one; Komi and Obtainium do not describe one |

## Feature by feature

Status: **Yes** = in Bilby; **New** = added in the change that added this document;
**Not yet** = a gap worth closing; **By design** = left out on purpose.

### Sources

| Feature | Who has it | Bilby |
|---|---|---|
| GitHub Releases | Komi, Obtainium | Yes |
| Codeberg | Komi, Obtainium | **New** (search, open by link, device matching) |
| Self-hosted Forgejo/Gitea, added by URL | Komi | Not yet. The Codeberg code already speaks the Forgejo API; what is missing is letting the user add a host |
| GitLab, F-Droid repositories, SourceHut, others | Obtainium | GitLab and F-Droid: yes. SourceHut: not yet |
| Private repositories with per-host tokens | Komi, Obtainium | Partly: one GitHub token |
| System package managers | UniGetUI, GNOME Software, Discover | Yes for pacman, apt, Flatpak (Flathub), Snap, winget and Homebrew casks; dnf and zypper not yet |
| AUR | none read | Yes |

### Discovery and search

| Feature | Who has it | Bilby |
|---|---|---|
| Featured apps by category | GNOME Software, Discover; Komi (topics) | Yes, from a list shipped with the app |
| Trending / hot / most popular feeds, daily explore feed, top charts | Komi | **By design**, for now. Komi serves these from its own backend. Doing it client-side costs one GitHub request per candidate to check for usable files, which the anonymous limit (60 an hour) cannot carry. Worth revisiting for users with a token |
| Only apps with something installable are listed | Komi | Yes, and matched to the device, not just "has an installer" |
| Search with suggestions and history | Komi (history) | Yes |
| Filters by language and sort order | Komi | Not yet |
| Paste a link to open a project | Komi (clipboard detection) | Yes (pasted into search; the clipboard is not read unasked) |
| Hide projects you have seen or do not want | Komi | Not yet |

### App page

| Feature | Who has it | Bilby |
|---|---|---|
| What the project says about itself | Komi (fully rendered README) | **New**: the start of the README as plain text. Headings, pictures and tables are not rendered |
| Release notes, stars, licence | Komi | Yes |
| Pick any older release | Komi, UniGetUI | Yes (the releases that have a file for the device) |
| Files listed with platform and size | Komi | Yes, plus the reason for each hidden one |
| Screenshots | GNOME Software, Discover | Yes: Flathub's for Flathub apps, and up to four found in a GitHub project's README |
| Download counts, installs, verified developer | Komi (downloads), Flathub, Play, App Store | Yes, wherever the source publishes them; each is labelled with what it counts |
| Rating breakdown by stars | Play, App Store, Flathub | Yes |
| Updating the operating system | GNOME Software, Discover | Yes on pacman, apt and dnf systems, and for Flatpak. No distribution release upgrades, no updates applied during a restart |
| Ratings | GNOME Software, Discover | Yes, from the same service they use (ODRS), for Flathub apps and distribution packages. One file a day; the service never learns which apps are viewed |
| Written reviews | GNOME Software, Discover | Yes, read-only, for the apps that have them. Opening such an app's page tells the ratings service which app it was; no user id is sent. Writing reviews is not there |
| Developer profile, issues and pull requests, security advisories | Komi | **By design**: the project page is one click away and does it better |
| Translating READMEs and notes | Komi | Not yet |

### Installing

| Feature | Who has it | Bilby |
|---|---|---|
| Picks the file for the device | Komi, Obtainium | Yes |
| Checksum checked against what the host publishes | Komi | Yes |
| Signature, certificate or attestation checks | Komi, Obtainium, AppImageUpdate, Zapstore | Not yet |
| Shows what will happen before doing it | not described elsewhere | Yes |
| AppImage in the app menu | Gear Lever, AppImageLauncher, AM | **New**; the replaced version's file is deleted on update |
| Silent install on Android (Shizuku, root) | Komi; Obtainium and Zapstore in the background | Not yet: Bilby's Android package does not hand files to Android's installer yet |
| Pause and resume downloads | Komi | Not yet (downloads can be cancelled) |
| Download mirrors | Komi | **By design** for now: a mirror is a third party serving the files |
| Proxy | Komi (settings screen) | From the `HTTPS_PROXY` / `ALL_PROXY` environment variables, which the HTTP library reads; untested behind a real proxy, and there is no settings screen |
| Bulk install | UniGetUI | Not yet |

### Updates

| Feature | Who has it | Bilby |
|---|---|---|
| Check for updates, update one or all | most | Yes |
| Skip a version / ignore updates | Komi, Obtainium, UniGetUI | **New**: skip the waiting version. Ignoring an app for good is not there yet |
| Pre-release channel | Komi, Obtainium | Yes, as one setting (Komi also has it per app) |
| Checks in the background with notifications | Komi, Obtainium, UniGetUI, Gear Lever, GNOME Software | Yes: Bilby can wait in the system tray, check on its own and say what is waiting |
| Updates installed without asking | Komi, Obtainium, UniGetUI, Zapstore | Yes, when switched on in Settings; what needs the person is left for them |
| The store updates itself | not described | Finds its own update; applying it is still a manual install |

### Library

| Feature | Who has it | Bilby |
|---|---|---|
| Favourites kept on the device | Komi | **New** |
| Starred repositories from an account | Komi | **By design**: needs sign-in |
| Export and import | Komi, Obtainium, UniGetUI | **New**, from `bilby-cli`; not yet a button in the app, and not Obtainium's format |
| Adopt apps that are already installed | Komi | Partly: packages installed by pacman or apt are recognised |
| Recently viewed, categories | Komi, Obtainium | Categories: yes. Recently viewed: not yet |

### Everything else

| Feature | Who has it | Bilby |
|---|---|---|
| Sign-in | Komi (optional) | **By design**: no Bilby account. Signing in to GitHub, Steam or GOG is optional and stays between the device and that service |
| Light and dark themes | Komi (plus AMOLED, dynamic colour, skins) | Yes (light, dark, follow the system) |
| Translated interface | Komi (14 languages), Obtainium | Yes: twelve languages, not yet checked by native speakers |
| Keyboard, remote and screen-reader use | not documented elsewhere | Keyboard and remote: yes, tested. Screen readers: every control has a tested label, but no real screen reader has been tried |
| Android, Windows, macOS, Linux apps you can download today | Komi | Yes, for all four. Linux is the system it has been used on most; see the README for what has been tried where |

## What to do next, in order

1. **Installing APKs on Android**, through Android's own installer.
2. **Self-hosted Forgejo/Gitea hosts** and a second token.
3. **Reading Obtainium's export format** when importing a library.
4. **Pause and resume** for downloads.
5. **A native speaker's review** of each translation.
