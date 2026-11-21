# Prompt for the Longboard session: handing a Steam library back to Bilby

Paste everything below the line into the Longboard session.

---

Bilby (my app store, `AndrewGBest/Bilby`) lets people sign in to Steam so it can mark
the games they already own. On a TV it does that through Longboard, because a remote
can drive Longboard and cannot drive a plain browser window. Bilby's half is built and
released. Longboard's half exists only as a proposal on the branch `steam-hand-back`
(commit `a2835ea`), written from the Bilby side. I want it in Longboard properly.

**I am deciding now that Longboard may do this.** Record it in `docs/DECISIONS.md` as
my decision, dated today, replacing the "proposed, not yet decided" entry on the
branch. Keep the table of choices from that entry, and mark anything you change.

## What Longboard has to do

Bilby starts Longboard like this, and this contract must not change, because released
copies of Bilby already use it:

```
longboard --hand-back=steam-library --hand-back-to=/absolute/path/steam.json --url=https://store.steampowered.com/login/?redir=&redir_ssl=1
```

1. Open Steam's own sign-in page. The person signs in there (typing, or Steam's QR
   code). Longboard never reads or types the password (Hard rule 10).
2. Once they are past the sign-in page, find out who is signed in **without putting
   any script into Steam's pages**: fetch `https://store.steampowered.com/` from the
   main process with the page's session (`session.fetch`, which sends its cookies)
   and read the account number from `data-userinfo` on `#application_config`.
3. Ask the person, in Longboard's usual question panel: share the list of games and
   tools they own on Steam with the program that opened Longboard? The highlight
   starts on "Do not share". Back is a no.
4. Only on a yes, fetch `https://store.steampowered.com/dynamicstore/userdata/` the
   same way and write one line to the file, for this user only (mode 0600), whole or
   not at all (write a `.part` file, then rename):

   ```json
   {"steamid":"76561198000000001","owned":[440,570]}
   ```

   `owned` is `rgOwnedApps` from Steam's answer and nothing else from it: no cookie,
   no token, no other field.
5. A no, a question left unanswered, or a failure to fetch writes
   `{"declined":true}` instead, so Bilby stops waiting.
6. The same request arriving while Longboard is already running (the second copy
   passing its arguments on) must work too.

What can be asked for is a named list inside Longboard with this one entry. A program
must not be able to name an address, a page, a selector or a cookie.

## Start from the branch, do not redo it

Read `steam-hand-back` first: `src/main/handback.ts` (the pure rules, unit-tested in
`test/handback.test.ts`), the wiring in `src/main/browser.ts`, `src/main/index.ts`,
`src/shared/chrome.ts` and `src/chrome/index.ts`, and two end-to-end scenarios in
`test/e2e/run.mjs` that run the whole exchange against an invented store using
`--hand-back-site=http://127.0.0.1:<port>/...` (honoured only with `--automation`,
only on this machine's own address).

Main has moved since the branch was cut (it was cut at 0.3.13), so rebase or
re-apply it, resolve conflicts in `browser.ts`, and review it as you would anyone
else's change. Change whatever does not fit how Longboard is built.

## What was verified on the branch, and what was not

- Verified: the unit tests; both end-to-end scenarios (a yes and a no) against the
  invented store; `npm run check`; that Longboard started with these options shows
  Steam's real sign-in page; that the account-number reader finds nobody on Steam's
  real front page when signed out; that `session.fetch` sends the session's cookies
  (tried against a local server).
- **Not verified by anyone: any of it signed in to the real Steam.** That Steam's
  front page says who is signed in the way the code expects, and that the list comes
  back, was read in the Playnite project's Steam plugin, not seen. I will sign in
  myself on the TV to test; you must not (rule 10).
- Not tested: the request arriving at an already-running Longboard.
- One full `npm run e2e` on the branch had four failures in "a profile of your own"
  while another build was loading the machine; that scenario passed when run alone.
  Run the whole suite again on a quiet machine.

## Things I want you to check or decide, and tell me

- Whether this counts as a background request under rule 12. The branch says no:
  both requests happen because the person asked to sign in, and the second only
  after a yes. Confirm or disagree, and record it.
- The words of the question. Longboard cannot know which program started it, so the
  branch says "the program that opened Longboard". Improve it if you can do so
  honestly.
- These Steam addresses are the ones Steam's own web pages use, not ones it
  publishes for other programs. If they stop answering, nothing must be shared and
  the person must be told. Make sure that is what happens.

## When it is done

Add a test for the already-running case, update `docs/STATUS.md` with what is
verified and what is not, and publish a Longboard release. Tell me the version, so I
can test signing in to Steam from Bilby on the TV: Bilby → Settings → "Or sign in to
Steam in a browser (Longboard)".

## One more thing, separate from the above

I installed Longboard on my TV through Bilby and it did not start. It starts fine
installed the same way on a Debian 13 test machine, so it is something about the TV.
The next Bilby release reports why an app closed straight away (it recognises a
missing FUSE, a sandbox that cannot start, and a missing library). When I send you
that message, treat it as a bug report against the AppImage.
