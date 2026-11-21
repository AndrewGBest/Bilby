# Publishing apps from your own server

Bilby can take apps from a list you host yourself: a home server, a NAS, an
object-storage bucket, a file in a repository. No permission is needed from anyone.
People add the list's address under **Settings → Your own sources**, and its apps are
searched, shown on the home page and installed like any others.

## The list

One JSON file, at any `https://` address (or `http://` for a server on your own
network):

```json
{
  "name": "Workshop apps",
  "apps": [
    {
      "id": "gauge",
      "name": "Gauge",
      "summary": "Reads the workshop's sensors.",
      "author": "The workshop",
      "homepage": "https://example.org/gauge",
      "license": "MIT",
      "icon": "icons/gauge.png",
      "description": "A longer description, shown on the app's page.",
      "releases": [
        {
          "version": "2.1.0",
          "date": "2026-11-01",
          "notes": "What changed in this version.",
          "files": [
            { "url": "gauge-2.1.0-x86_64.AppImage", "sha256": "…64 hex digits…", "size": 104857600 },
            { "url": "gauge-2.1.0-setup-x64.exe" },
            { "url": "gauge-2.1.0-universal.dmg" },
            { "url": "https://dl.example.org/get?id=7", "name": "gauge_2.1.0_amd64.deb" }
          ]
        }
      ]
    }
  ]
}
```

| Field | Needed | What it is |
|---|---|---|
| `name` (top) | no | What the source is called in Bilby. Its server's name is used if left out |
| `id` | yes | Letters, digits, `-`, `_`, `.`. Keep it the same for ever: it is how an installed copy is matched to its updates |
| `name` | yes | What people see |
| `summary`, `author`, `homepage`, `license`, `description` | no | Shown on the app's tile and page |
| `icon` | no | A PNG or JPEG. Used only from an `https://` address |
| `releases` | yes | Newest first. Each needs a `version` and at least one file |
| `files[].url` | yes | A full address, a path from the server's root (`/files/x`), or a name beside the list (`x.AppImage`) |
| `files[].name` | no | The file's name, when the address does not end in it |
| `files[].sha256` | no | The file's SHA-256. When given, Bilby checks the download against it and refuses a file that differs |
| `files[].size` | no | In bytes, for showing the size before downloading |

## How Bilby picks the file

Exactly as it does for a release on GitHub: from the file's name. Name files the way
projects usually do and the right one is offered on each device:

- `…x86_64.AppImage`, `…aarch64.AppImage`, `…amd64.deb`, `…x86_64.rpm`, `….flatpak` for Linux
- `…setup.exe`, `…x64.msi` for Windows
- `….dmg`, `….pkg` for macOS
- `….apk` for Android

A file Bilby cannot place is still listed under "Other versions and files".

## What to know

- **Trust.** Bilby does not review a list or what is on it. Whoever adds your list is
  trusting you, and the Settings page tells them so. Give a `sha256` for every file,
  and serve the list over `https://`.
- **Updates.** Bilby reads the list again when it looks for updates. Put a new release
  first in `releases` and people who installed the app are offered it.
- **Nothing is sent to you** beyond the ordinary request for the list and the files.
- Removing a source in Settings takes its apps out of searches. Apps already installed
  from it stay installed.
