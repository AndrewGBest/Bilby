# What Bilby asks of Longboard

[Longboard](https://github.com/AndrewGBest/Longboard) is a web browser made for a TV
remote. Bilby uses it on a Linux TV for two things, and this page is the agreement
between the two programs. Released copies of Bilby depend on it, so neither side
should change it without the other.

## Opening a website

A web app in Bilby's library is a website. On a Linux TV, Bilby starts Longboard on
its address; if Longboard is already running, the second copy passes the address to
the first, which comes to the front.

Bilby recognises Longboard however it was installed (an AppImage, an unpacked folder,
its Flatpak, or on the `PATH`) and installs it first when it is missing
(`install.rs`, `ensure_longboard` in the app).

## Signing in to Steam

Bilby can mark the games a person already owns on Steam. A remote cannot drive an
ordinary browser window, so on a TV the sign-in happens in Longboard, which hands the
list back in a file. The rules are in `crates/core/src/steam_account.rs`.

Bilby starts Longboard like this:

```
longboard --hand-back=steam-library --hand-back-to=/absolute/path/steam.json --url=https://store.steampowered.com/login/?redir=&redir_ssl=1
```

Longboard then:

1. Shows Steam's own sign-in page. The person signs in there. Neither program reads
   or types the password.
2. Asks the person whether to share the list of what they own with the program that
   opened it. The answer starts on "Do not share".
3. On a yes, writes one line to the file named by `--hand-back-to`, readable by this
   user only, whole or not at all (a `.part` file, then a rename):

   ```json
   {"steamid":"76561198000000001","owned":[440,570]}
   ```

   Nothing else from Steam goes into it: no cookie, no key, no other field.
4. On a no, an unanswered question or a failure, writes `{"declined":true}` so that
   Bilby stops waiting.

`steam-library` is the only thing that can be asked for. A program cannot name an
address, a page or a cookie for Longboard to hand back.

Bilby waits for the file (`await_hand_back`), keeps the list of owned apps in its
state file, and deletes the hand-back file.

## What has been tried

Both halves are tested against stand-ins: Bilby's with an invented Longboard
(`crates/app/tests/ui.rs`), Longboard's with an invented Steam. Nobody has yet
completed a real Steam sign-in through the pair.
