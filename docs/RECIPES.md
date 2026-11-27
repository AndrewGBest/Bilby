# Recipes: apps from their makers' websites

Some apps are on no store and in no repository. Their maker puts a download on a
website and that is all. A recipe tells Bilby how to get such an app: where the
maker publishes it, how to read which version is current, and where the file for each
kind of device is. With a recipe the app is searched for, installed and updated like
any other.

Recipes are shared. They live in a public repository,
[AndrewGBest/bilby-recipes](https://github.com/AndrewGBest/bilby-recipes), one small
file per app, and anyone can send one. That repository checks every recipe against
the real website each day and publishes them all as one list. Bilby fetches the list,
and carries a copy of it for a first start with no internet.

## What a recipe can and cannot do

A recipe is data, never a program. It has three ways to look something up, and each
reads one address:

| Way | What it does |
|---|---|
| `page` + `find` | Reads a page and takes what a pattern picks out of its text |
| `redirect` | Sees where an address leads ("latest" links), and takes that address, or what `find` picks out of it |
| `json` + `at` | Reads a JSON answer and takes the value at a place in it |

It cannot run a command, read a file on the device, or send anything. It cannot fetch
from anywhere but the hosts it names: every address it reads, and every file it hands
over for download, must be `https` and at one of its `hosts`. A recipe that says it is
for `example.com` cannot download from somewhere else, and a page on `example.com`
that starts linking somewhere else loses that file instead of leading Bilby there.
What the maker's own server redirects a download to is followed, since the maker
decides that as it decides what is in the file.

Bilby does not know who made a file on a website the way it knows who signed a store's
package. Where the maker publishes a checksum the recipe can read it, and Bilby then
refuses a download that does not match. [Checking the publisher's signature on a
Windows installer or a Mac app is not done yet.]

## The file

A recipe is a JSON file named after its id, `tide.json`:

```json
{
  "format": 1,
  "id": "tide",
  "name": "Tide",
  "summary": "Charts of the sea, for sailors.",
  "author": "Tide Works",
  "homepage": "https://tide.example/",
  "license": "GPL-3.0-only",
  "icon": "https://tide.example/icon-256.png",
  "hosts": ["tide.example"],
  "version": { "page": "https://tide.example/download", "find": "Version ([0-9.]+)" },
  "files": [
    { "url": "https://cdn.tide.example/{version}/tide-{version}-x86_64.AppImage",
      "sha256": { "page": "https://tide.example/{version}/SHA256SUMS",
                  "find": "([0-9a-f]{64})  tide-{version}-x86_64.AppImage" } },
    { "url": { "page": "https://tide.example/download", "find": "href=\"([^\"]+\\.dmg)\"" } },
    { "name": "tide_{version}_amd64.deb",
      "url": { "redirect": "https://tide.example/latest/deb" } },
    { "url": { "json": "https://tide.example/api/latest", "at": "files.0.url" } }
  ]
}
```

| Field | |
|---|---|
| `format` | `1`. A Bilby that meets a recipe in a newer form leaves it out |
| `id` | Lower-case letters, digits and hyphens. The app is `recipe:<id>` |
| `name`, `summary`, `author` | As the store shows them. The summary is one plain sentence |
| `homepage` | The maker's page for the app, at one of the hosts |
| `license`, `icon` | Optional. The icon is at one of the hosts |
| `hosts` | One to eight host names. A host covers the names under it: `tide.example` covers `cdn.tide.example` |
| `version` | The current version: written out, or looked up |
| `files` | One entry per file the maker publishes, up to forty |

A file has a `url`, written out or looked up, and may have a `name` and a `sha256`.
Which device a file suits is worked out from its name, as for any release file:
`tide-3.2.1-x86_64.AppImage`, `tide_3.2.1_arm64.deb`, `Tide-3.2.1-arm64-v8a.apk`. Give
a `name` where the address does not end in a telling one (`…/latest/deb`,
`Discord.deb`). `bilby-cli classify <file name>` says how a name is read.

`{version}` in an address, a name or a pattern stands for the version that was found.

Looking something up:

- `"page"` with `"find"`: a regular expression, and the value is its first group in
  brackets. A link found this way is made whole against the page it was found on.
- `"redirect"`: the value is the address it leads to. With `"find"`, the value is
  what the pattern picks out of that address, which is how a version is read from
  `…/releases/2026.5/app-2026.5.deb`.
- `"json"` with `"at"`: names and list positions joined by dots, `files.0.url`.

A version may hold letters, digits and `. - _ +`. One recipe has one version, so an
app whose phone and computer versions are numbered apart gets a recipe for each
(`mullvad-vpn` and `mullvad-vpn-android`).

## Writing one and trying it

Put the file in the `recipes` folder of Bilby's settings folder
(`~/.config/bilby/recipes/` on Linux). Bilby uses a recipe there on this device, and
it wins over a shared one of the same id, so a mended recipe can be tried first.

```sh
bilby-cli recipe check tide.json     # follow it against the real website
bilby-cli show recipe:tide           # what this device would be offered
bilby-cli install recipe:tide
```

`check` downloads nothing. It prints the version and each file it found, what kind of
file Bilby takes each to be, and what it could not find and why.

## Sharing it

```sh
bilby-cli recipe share tide.json
```

checks the recipe and prints the address of a page on GitHub with the file filled in.
Open it, press "Propose new file", and it becomes a pull request under your own
GitHub account. Bilby sends nothing itself. The repository's own notes say what a
recipe needs to be accepted; the short of it is that the hosts must be the maker's
own, and the app must be one its maker gives away or sells there.

## When a website changes

A recipe breaks when its website is rearranged. Then the app's page says that it
could not be found on its maker's website, and the app stays installed and keeps
working. The daily check in the shared repository notices too. Mending is the same as
writing: change the file, `check` it, share it. A recipe mended in the shared list
reaches every Bilby the next time it fetches the list, with no new release of Bilby.
