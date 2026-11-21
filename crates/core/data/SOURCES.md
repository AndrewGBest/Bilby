# Where the bundled lists come from

| File | What it is | From | Licence |
|---|---|---|---|
| `index.json` | The starter apps on the home page | Written for Bilby | GPL-3.0-only, with Bilby |
| `related.json` | Apps that are different ways to use one service | Written for Bilby; every id checked to exist | GPL-3.0-only, with Bilby |
| `web.json` | About ninety well-known websites, with names and what each is for | Written for Bilby; every address checked to answer | GPL-3.0-only, with Bilby |
| `web-top.txt` | Hosts among the 1,000 most visited by Chrome's users | The Chrome UX Report, through [zakird/crux-top-lists](https://github.com/zakird/crux-top-lists); tidied (see the file's first line) | Chrome UX Report data, by Google. Stated as CC BY-SA 4.0 by the Tranco project, which uses it; Google's own licence page was not readable when this was written |
| `adult-hosts.txt.gz` | Addresses of websites for adults | The "porn" extension of [StevenBlack/hosts](https://github.com/StevenBlack/hosts) (MIT), which merges lists by Sinfonietta, BigDargon and Clefspeare13 (MIT) and by Tiuxo and brijrajparmar27 (CC BY 4.0). Leading `www.` removed, sorted, compressed | MIT and CC BY 4.0, as above |

All fetched 2026-11-21. `web-top.txt` and `adult-hosts.txt.gz` go stale and want
refreshing now and then; nothing in Bilby fetches them at run time.
