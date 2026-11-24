---
name: release
description: Release a new version of Bilby - changelog, version, CI, the Release workflow, and checking what was published. Only when the maintainer asks for a release.
disable-model-invocation: true
argument-hint: "[version, like 0.1.41; default: the next patch version]"
---

# Release Bilby

Version asked for: `$ARGUMENTS` (if empty, the patch version after the one in
`Cargo.toml`).

A published release and its tag cannot be changed or removed afterwards (the
repository makes them immutable). If something is wrong after publishing, the fix is a
new version. So stop and say so at the first step that fails; never retry by deleting.

1. **Start clean.** On `main`, up to date with `origin/main`, nothing uncommitted. If
   there is unreleased work on another branch, ask before going on.
2. **Changelog.** In `CHANGELOG.md`, move the lines under "Unreleased" to a new heading
   `## X.Y.Z (YYYY-MM-DD)` with today's date, and leave "Unreleased" empty above it.
   Compare with `git log <last tag>..HEAD`: add anything a person using Bilby would
   notice that is missing. This section becomes the release's notes.
3. **Version.** Change `version` under `[workspace.package]` in `Cargo.toml`, then
   `cargo update --workspace --offline` so `Cargo.lock` follows.
4. **Check.** The full check from `CLAUDE.md`, and build the Android library
   (`cargo apk build --lib` in `crates/app`), because the release builds it signed.
5. **Commit** as `Version X.Y.Z` and push to `main`.
6. **Wait for CI** on that commit: `gh run list --workflow ci.yml --commit <sha>`, then
   `gh run watch <id>`. Every job must pass.
7. **Start the release:** `gh workflow run release.yml --ref main -f version=vX.Y.Z`,
   then watch it. It checks the version against `Cargo.toml`, builds Linux, Windows,
   macOS and Android, and publishes.
8. **Look at what was published:** `gh release view vX.Y.Z`. There should be eight
   files: the AppImage, the Windows `.exe`, the macOS `.dmg`, the Android `.apk`, three
   archives with the command-line tool, and `SHA256SUMS`. The notes should be the
   changelog section. Say so plainly if the `.apk` is missing (the signing key was not
   available) or anything else differs.
9. **Report** the link, what passed, and anything that needed fixing on the way.
