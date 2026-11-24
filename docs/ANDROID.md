# Bilby on Android

One package serves phones, tablets and TVs with a 64-bit ARM processor. It is the
same Rust program as on a desktop, running inside a single Android activity, with the
phone layout on a phone and the TV layout on a TV.

## What works

| | State |
|---|---|
| Starting, searching, opening an app's page | Run for real in an Android 14 emulator |
| F-Droid and Google Play as sources | Shown. Google Play is handed over to; Bilby does not search it |
| Opening a link or another store's page | Through Android's own "view" request (`crates/app/src/android.rs`) |
| Installing a downloaded `.apk` | Not yet: the file is downloaded, but handing it to Android's installer is still to do |
| A real phone or an Android TV | Not tried |

## Build it

You need the Android SDK and NDK, and `cargo-apk`:

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk --locked

# With the SDK's command-line tools:
sdkmanager "ndk;30.0.16248370" "platforms;android-34" "build-tools;34.0.0"

export ANDROID_HOME="$HOME/Android/Sdk"
export ANDROID_NDK_ROOT="$ANDROID_HOME/ndk/30.0.16248370"
```

The NDK version matters: the drawing library underneath (Skia) comes ready-built
against that one and no older one.

Then, from `crates/app`:

```sh
cargo apk build --lib            # a debug package, signed with the debug key
cargo apk run --lib              # build, install on the connected device and start it
```

For an emulator on a PC, add `--target x86_64-linux-android` (and the Rust target of
the same name).

The package's settings (its name, permissions, the launcher entries for a phone and
for a TV's home screen) are in `crates/app/Cargo.toml` under
`[package.metadata.android]`. Its icon and TV banner are in `packaging/android/res`.

## How it differs from the desktop program

- The entry point is `android_main` in `crates/app/src/lib.rs`, not `main.rs`. It
  names the folders Bilby may use (the app's own storage) and builds the same
  `Options` the desktop program does. **It is only compiled for Android**, so a
  change to `Options` or `Dirs` that compiles on a desktop can still break it; CI
  builds the Android library on every push to catch that.
- There is no command-line tool, no sign-in window and no system tray.
- The core is built with its real network client; `crates/core` on its own must also
  keep compiling for Android without it (`--no-default-features`).

## Releases

The `Release` workflow builds the package and signs it with the project's key, which
is kept in the repository's secrets (`ANDROID_KEYSTORE_BASE64`,
`ANDROID_KEYSTORE_PASSWORD`). Without the key the release goes ahead with no Android
package. A package signed with a different key cannot update an installed one, so the
key must not be replaced.
