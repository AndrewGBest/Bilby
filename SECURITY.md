# Security

Bilby downloads programs and hands them to the system to install, so mistakes here
matter. Reports are welcome and taken seriously.

## Reporting a problem

**Please do not open a public issue.** Use GitHub's private reporting instead:

1. Go to the repository's [Security tab](https://github.com/AndrewGBest/Bilby/security).
2. Choose **Report a vulnerability**.
3. Say what you found, how to reproduce it, and which version and system you used.

Only the maintainer sees the report. You can expect a first answer within a week.
Bilby has one maintainer and no company behind it, so a fix may take longer than
that; you will be told what is planned and when, and credited in the release notes
unless you would rather not be.

## Which versions get fixes

Only the newest release. Bilby updates itself, and every release replaces the one
before it.

## What counts

Things we most want to hear about:

- Bilby installing or running something other than what its page showed: a download
  that is not checked when a checksum was published, a file fetched over plain http,
  a way for one source to pass itself off as another.
- A source's answer (a name, a description, a link, a file name) making Bilby run a
  command, write outside its own folders, or open an address that is not https.
- Anything that sends information off the device beyond what the
  [README's privacy table](README.md#your-privacy) lists.
- A way round Family mode or the PIN without knowing the PIN.
- A sign-in (GitHub, Steam, GOG) keeping or exposing more than the README says.
- The release files themselves: a way to tamper with a build or its update.

Things that are not Bilby's to fix:

- **An app in a source is malicious.** Bilby lists what sources publish and does not
  review apps. Report it to the source (Flathub, GitHub, the AUR, ...). If Bilby
  hid a warning the source gave, that *is* ours.
- A problem that needs someone who already controls the device or the person's
  account on it.

## How Bilby is built to be safe

The [design notes](docs/DESIGN.md#11-safety) describe the rules the code follows:
nothing runs as root inside Bilby, downloads are checked against published
checksums, only https addresses are opened, and file names from the network never
become paths without being checked.

Every release file comes with a `SHA256SUMS` file and a signed record of where it was
built, which you can check with GitHub's tool:

```sh
gh attestation verify bilby-v0.1.41-x86_64.AppImage --repo AndrewGBest/Bilby
```
