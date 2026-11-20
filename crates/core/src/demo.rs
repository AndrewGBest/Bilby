//! A small, entirely fictional catalog served through the same [`Http`] interface and in
//! the same JSON shapes as the real GitHub and AUR APIs. It lets the app, the tests and
//! screenshots run with no network, while exercising the real parsing and filtering
//! code. None of these projects exist.

use crate::error::{Error, Result};
use crate::http::{hex, Http};
use crate::index::{Index, IndexEntry};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

struct DemoApp {
    owner: &'static str,
    repo: &'static str,
    name: &'static str,
    summary: &'static str,
    stars: u64,
    license: &'static str,
    category: &'static str,
    tv: bool,
    os: &'static [&'static str],
    /// Newest first: (tag, date, files).
    releases: &'static [(&'static str, &'static str, &'static [&'static str])],
}

const APPS: &[DemoApp] = &[
    DemoApp {
        owner: "tidewave",
        repo: "tidewave",
        name: "Tidewave",
        summary: "Music player for local libraries and self-hosted servers.",
        stars: 31_400,
        license: "GPL-3.0",
        category: "Media",
        tv: true,
        os: &["windows", "linux", "macos", "android", "ios"],
        releases: &[(
            "v3.4.0",
            "2026-11-02",
            &[
                "Tidewave-3.4.0-x64.msi",
                "Tidewave-3.4.0-arm64.msi",
                "Tidewave-3.4.0-x64-portable.zip",
                "Tidewave-3.4.0-universal.dmg",
                "Tidewave-3.4.0-x86_64.AppImage",
                "Tidewave-3.4.0-aarch64.AppImage",
                "tidewave_3.4.0_amd64.deb",
                "tidewave_3.4.0_arm64.deb",
                "tidewave-3.4.0-1.x86_64.rpm",
                "tidewave-3.4.0-linux-x86_64.tar.gz",
                "tidewave-3.4.0-arm64-v8a.apk",
                "tidewave-3.4.0-armeabi-v7a.apk",
                "tidewave-3.4.0-x86_64.apk",
                "tidewave-3.4.0-androidtv-arm64-v8a.apk",
                "tidewave-3.4.0-androidtv-armeabi-v7a.apk",
                "Tidewave-3.4.0.ipa",
                "SHA256SUMS",
                "tidewave-3.4.0-src.tar.gz",
            ],
        )],
    },
    DemoApp {
        owner: "lumen-labs",
        repo: "lumen",
        name: "Lumen",
        summary: "Fast, keyboard-driven text editor with built-in language servers.",
        stars: 48_200,
        license: "MIT",
        category: "Developer tools",
        tv: false,
        os: &["windows", "linux", "macos"],
        releases: &[(
            "v0.19.2",
            "2026-11-10",
            &[
                "Lumen-0.19.2-x64-Setup.exe",
                "Lumen-0.19.2-arm64-Setup.exe",
                "Lumen-0.19.2-win-x64.zip",
                "Lumen-0.19.2-arm64.dmg",
                "Lumen-0.19.2-x64.dmg",
                "Lumen-0.19.2-x86_64.AppImage",
                "Lumen-0.19.2-x86_64.AppImage.zsync",
                "lumen_0.19.2_amd64.deb",
                "lumen_0.19.2_arm64.deb",
                "lumen-0.19.2-1.x86_64.rpm",
                "lumen-0.19.2-1.aarch64.rpm",
                "lumen-0.19.2-1-x86_64.pkg.tar.zst",
                "lumen-0.19.2-x86_64-unknown-linux-musl.tar.gz",
                "checksums.txt",
            ],
        )],
    },
    DemoApp {
        owner: "kestrel-sh",
        repo: "kestrel",
        name: "Kestrel",
        summary: "A friendly prompt for any shell, written in Rust.",
        stars: 22_900,
        license: "Apache-2.0",
        category: "Developer tools",
        tv: false,
        os: &["windows", "linux", "macos"],
        releases: &[
            (
                "v1.21.0",
                "2026-10-28",
                &[
                    "kestrel-x86_64-unknown-linux-gnu.tar.gz",
                    "kestrel-x86_64-unknown-linux-musl.tar.gz",
                    "kestrel-aarch64-unknown-linux-gnu.tar.gz",
                    "kestrel-arm-unknown-linux-gnueabihf.tar.gz",
                    "kestrel-x86_64-pc-windows-msvc.zip",
                    "kestrel-aarch64-pc-windows-msvc.zip",
                    "kestrel-i686-pc-windows-msvc.zip",
                    "kestrel-x86_64-unknown-freebsd.tar.gz",
                    "kestrel-x86_64-unknown-linux-gnu.tar.gz.sha256",
                ],
            ),
            (
                "v1.20.0",
                "2026-09-14",
                &[
                    "kestrel-x86_64-unknown-linux-gnu.tar.gz",
                    "kestrel-aarch64-unknown-linux-gnu.tar.gz",
                    "kestrel-x86_64-apple-darwin.tar.gz",
                    "kestrel-aarch64-apple-darwin.tar.gz",
                    "kestrel-x86_64-pc-windows-msvc.zip",
                ],
            ),
        ],
    },
    DemoApp {
        owner: "glacier-tools",
        repo: "glacier",
        name: "Glacier",
        summary: "Encrypted, incremental backups to any storage.",
        stars: 17_300,
        license: "BSD-2-Clause",
        category: "Developer tools",
        tv: false,
        os: &["windows", "linux", "macos"],
        releases: &[(
            "v2.8.1",
            "2026-10-03",
            &[
                "glacier_2.8.1_linux_amd64.tar.gz",
                "glacier_2.8.1_linux_arm64.tar.gz",
                "glacier_2.8.1_linux_armv7.tar.gz",
                "glacier_2.8.1_linux_ppc64le.tar.gz",
                "glacier_2.8.1_linux_riscv64.tar.gz",
                "glacier_2.8.1_darwin_amd64.tar.gz",
                "glacier_2.8.1_darwin_arm64.tar.gz",
                "glacier_2.8.1_windows_amd64.zip",
                "glacier_2.8.1_windows_arm64.zip",
                "glacier_2.8.1_freebsd_amd64.tar.gz",
                "glacier_2.8.1_amd64.deb",
                "glacier_2.8.1_arm64.deb",
                "glacier-2.8.1.x86_64.rpm",
                "glacier_2.8.1_checksums.txt",
            ],
        )],
    },
    DemoApp {
        owner: "relay-app",
        repo: "relay",
        name: "Relay",
        summary: "Send files between nearby devices without the cloud.",
        stars: 54_800,
        license: "MIT",
        category: "Utilities",
        tv: false,
        os: &["windows", "linux", "macos", "android"],
        releases: &[(
            "v1.16.0",
            "2026-11-12",
            &[
                "Relay-1.16.0-windows-x86-64.exe",
                "Relay-1.16.0-windows-x86-64.zip",
                "Relay-1.16.0-macos-universal.dmg",
                "Relay-1.16.0-linux-x86-64.AppImage",
                "Relay-1.16.0-linux-x86-64.deb",
                "Relay-1.16.0-linux-arm-64.deb",
                "Relay-1.16.0-linux-x86-64.tar.gz",
                "Relay-1.16.0-android-arm64v8.apk",
                "Relay-1.16.0-android-armv7.apk",
                "Relay-1.16.0-android-x64.apk",
            ],
        )],
    },
    DemoApp {
        owner: "driftnotes",
        repo: "drift",
        name: "Drift",
        summary: "Markdown notes that sync through any folder you choose.",
        stars: 9_640,
        license: "AGPL-3.0",
        category: "Utilities",
        tv: false,
        os: &["windows", "linux", "macos", "android", "ios"],
        releases: &[(
            "v2.2.0",
            "2026-08-30",
            &[
                "Drift-Setup-2.2.0.exe",
                "Drift-2.2.0-universal.dmg",
                "Drift-2.2.0.AppImage",
                "Drift-2.2.0-arm64.AppImage",
                "drift_2.2.0_amd64.deb",
                "Drift-2.2.0.flatpak",
                "drift-2.2.0-universal.apk",
                "Drift-2.2.0.ipa",
                "latest.yml",
                "latest-mac.yml",
            ],
        )],
    },
    DemoApp {
        owner: "openfern",
        repo: "fern-launcher",
        name: "Fern Launcher",
        summary: "A minimal, distraction-free home screen.",
        stars: 4_120,
        license: "GPL-3.0",
        category: "Utilities",
        tv: false,
        os: &["android"],
        releases: &[("v0.9.3", "2026-10-19", &["fern-launcher-0.9.3.apk", "fern-launcher-0.9.3.apk.asc"])],
    },
    DemoApp {
        owner: "marquee-tv",
        repo: "marquee",
        name: "Marquee",
        summary: "Media center for the big screen: your library, ten feet away.",
        stars: 12_700,
        license: "GPL-2.0",
        category: "Media",
        tv: true,
        os: &["windows", "linux", "android"],
        releases: &[(
            "v21.1",
            "2026-11-05",
            &[
                "marquee-21.1-androidtv-arm64-v8a.apk",
                "marquee-21.1-androidtv-armeabi-v7a.apk",
                "marquee-21.1-x86_64.AppImage",
                "marquee-21.1-x64-setup.exe",
                "marquee-21.1.flatpak",
            ],
        )],
    },
    DemoApp {
        owner: "couchplay",
        repo: "couchplay",
        name: "Couchplay",
        summary: "Stream games from your PC to any screen, controller first.",
        stars: 15_900,
        license: "GPL-3.0",
        category: "Media",
        tv: true,
        os: &["windows", "linux", "macos", "android"],
        releases: &[(
            "v6.1.0",
            "2026-09-21",
            &[
                "Couchplay-6.1.0-x64.exe",
                "Couchplay-6.1.0-arm64.exe",
                "Couchplay-6.1.0.dmg",
                "Couchplay-6.1.0-x86_64.AppImage",
                "couchplay-6.1.0-android-universal.apk",
                "couchplay-6.1.0-android-tv-universal.apk",
                "Couchplay-6.1.0.apks",
            ],
        )],
    },
    DemoApp {
        owner: "skyline-tv",
        repo: "atlas",
        name: "Atlas",
        summary: "Live TV and programme guide for your own playlists.",
        stars: 3_380,
        license: "MIT",
        category: "Media",
        tv: true,
        os: &["android"],
        releases: &[(
            "v4.0.2",
            "2026-07-11",
            &["atlas-4.0.2-leanback-arm64-v8a.apk", "atlas-4.0.2-leanback-armeabi-v7a.apk"],
        )],
    },
    DemoApp {
        owner: "winspect",
        repo: "winspect",
        name: "Winspect",
        summary: "Tiling, snapping and a system monitor for Windows.",
        stars: 7_050,
        license: "MIT",
        category: "Utilities",
        tv: false,
        os: &["windows"],
        releases: &[(
            "v0.77",
            "2026-10-30",
            &["Winspect-0.77-x64.msix", "Winspect-0.77-arm64.msix", "WinspectSetup-0.77-x64.exe"],
        )],
    },
    DemoApp {
        owner: "barley",
        repo: "barley",
        name: "Barley",
        summary: "Menu bar system monitor for macOS.",
        stars: 5_210,
        license: "MIT",
        category: "Utilities",
        tv: false,
        os: &["macos"],
        releases: &[("v2.11.4", "2026-11-01", &["Barley-2.11.4.dmg", "Barley.app.zip"])],
    },
];

/// Projects that exist in the demo "GitHub" but are not featured: an abandoned one and
/// one that has only ever published pre-releases.
const UNLISTED: &[DemoApp] = &[
    DemoApp {
        owner: "oldtimer",
        repo: "relic",
        name: "Relic",
        summary: "A calculator from another era.",
        stars: 812,
        license: "MIT",
        category: "Utilities",
        tv: false,
        os: &["linux", "windows"],
        releases: &[("v1.4.2", "2021-03-09", &["relic-1.4.2-x86_64.AppImage", "relic-1.4.2-setup.exe"])],
    },
    DemoApp {
        owner: "edge-labs",
        repo: "nightjar",
        name: "Nightjar",
        summary: "Experimental window compositor.",
        stars: 2_310,
        license: "MIT",
        category: "Utilities",
        tv: false,
        os: &["linux"],
        releases: &[("v0.9.0-beta.3", "2026-11-15", &["nightjar-0.9.0-beta.3-x86_64.AppImage"])],
    },
];

fn all_apps() -> impl Iterator<Item = &'static DemoApp> {
    APPS.iter().chain(UNLISTED.iter())
}

/// A fictional project hosted on Codeberg: found by search, not featured.
const CODEBERG: &[DemoApp] = &[DemoApp {
    owner: "fernleaf",
    repo: "fernleaf",
    name: "Fernleaf",
    summary: "Plain-text notes that sync through any folder.",
    stars: 1_840,
    license: "GPL-3.0-or-later",
    category: "Utilities",
    tv: false,
    os: &["windows", "linux", "macos", "android"],
    releases: &[(
        "v1.6.0",
        "2026-10-19",
        &[
            "fernleaf-1.6.0-x86_64.AppImage",
            "fernleaf_1.6.0_amd64.deb",
            "fernleaf-1.6.0-aarch64.AppImage",
            "fernleaf-1.6.0-windows-x64-setup.exe",
            "fernleaf-1.6.0-macos-universal.dmg",
            "fernleaf-1.6.0-arm64-v8a.apk",
            "fernleaf-1.6.0-armeabi-v7a.apk",
        ],
    )],
}];

fn forgejo_repo_json(app: &DemoApp) -> Value {
    json!({
        "name": app.repo,
        "full_name": format!("{}/{}", app.owner, app.repo),
        "owner": { "login": app.owner },
        "description": app.summary,
        "html_url": format!("https://codeberg.org/{}/{}", app.owner, app.repo),
        "stars_count": app.stars,
        "fork": false,
        "archived": false,
    })
}

fn forgejo_releases_json(app: &DemoApp) -> Value {
    Value::Array(
        app.releases
            .iter()
            .map(|(tag, date, files)| {
                json!({
                    "tag_name": tag,
                    "draft": false,
                    "prerelease": false,
                    "published_at": format!("{date}T12:00:00Z"),
                    "body": format!("## What's new in {} {tag}\n\n- Faster sync\n- Fixes for reported crashes\n\n(Fictional release notes: this is demo data.)", app.name),
                    "assets": files.iter().map(|f| json!({
                        "name": f,
                        "size": fake_size(f),
                        "browser_download_url": format!("https://codeberg.org/{}/{}/releases/download/{tag}/{f}", app.owner, app.repo),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

struct DemoAurPackage {
    name: &'static str,
    version: &'static str,
    summary: &'static str,
    votes: u64,
    maintainer: &'static str,
    out_of_date: bool,
}

const AUR: &[DemoAurPackage] = &[
    DemoAurPackage {
        name: "lumen-bin",
        version: "0.19.2-1",
        summary: "Fast, keyboard-driven text editor (prebuilt binary)",
        votes: 412,
        maintainer: "ostrich",
        out_of_date: false,
    },
    DemoAurPackage {
        name: "lumen-git",
        version: "0.19.2.r41.g3f2c9ab-1",
        summary: "Fast, keyboard-driven text editor (development version)",
        votes: 96,
        maintainer: "ostrich",
        out_of_date: false,
    },
    DemoAurPackage {
        name: "kestrel-git",
        version: "1.21.0.r7.g88a1b0e-1",
        summary: "A friendly prompt for any shell (development version)",
        votes: 58,
        maintainer: "magpie",
        out_of_date: false,
    },
    DemoAurPackage {
        name: "tidewave",
        version: "3.4.0-1",
        summary: "Music player for local libraries and self-hosted servers",
        votes: 233,
        maintainer: "heron",
        out_of_date: false,
    },
    DemoAurPackage {
        name: "quillfeed",
        version: "1.8.0-2",
        summary: "Terminal feed reader with offline sync",
        votes: 187,
        maintainer: "wren",
        out_of_date: true,
    },
];

/// Contents of every demo download.
const PAYLOAD: &[u8] = b"bilby demo payload\n";

fn fake_size(name: &str) -> u64 {
    // Stable, plausible-looking sizes derived from the name.
    let h = name.bytes().fold(7u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let base = if name.ends_with(".tar.gz") || name.ends_with(".zip") { 6 } else { 38 };
    (base + h % 60) * 1024 * 1024 + h % 900_000
}

fn repo_json(app: &DemoApp) -> Value {
    json!({
        "name": app.repo,
        "full_name": format!("{}/{}", app.owner, app.repo),
        "owner": { "login": app.owner },
        "description": app.summary,
        "html_url": format!("https://github.com/{}/{}", app.owner, app.repo),
        "stargazers_count": app.stars,
        "license": { "spdx_id": app.license },
    })
}

fn releases_json(app: &DemoApp) -> Value {
    let digest = format!("sha256:{}", hex(&Sha256::digest(PAYLOAD)));
    Value::Array(
        app.releases
            .iter()
            .map(|(tag, date, files)| {
                json!({
                    "tag_name": tag,
                    "draft": false,
                    "prerelease": tag.contains("beta") || tag.contains("-rc"),
                    "published_at": format!("{date}T12:00:00Z"),
                    "body": format!("## What's new in {} {tag}\n\n- Faster startup\n- Fixes for reported crashes\n- Updated translations\n\n(Fictional release notes: this is demo data.)", app.name),
                    "assets": files.iter().map(|f| json!({
                        "name": f,
                        "size": fake_size(f),
                        "browser_download_url": format!("https://github.com/{}/{}/releases/download/{tag}/{f}", app.owner, app.repo),
                        "digest": digest,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn aur_json(p: &DemoAurPackage) -> Value {
    json!({
        "Name": p.name, "PackageBase": p.name, "Version": p.version, "Description": p.summary,
        "URL": "https://example.invalid/", "NumVotes": p.votes, "Popularity": 1.0, "Maintainer": p.maintainer,
        "OutOfDate": if p.out_of_date { json!(1_792_000_000i64) } else { Value::Null }, "LastModified": 1_793_000_000i64, "URLPath": format!("/cgit/aur.git/snapshot/{}.tar.gz", p.name),
        "License": ["MIT"],
    })
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && s.is_char_boundary(i + 1) && s.is_char_boundary(i + 3) {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn matches(query: &str, haystacks: &[&str]) -> bool {
    let q = query.to_lowercase();
    q.split_whitespace().all(|word| haystacks.iter().any(|h| h.to_lowercase().contains(word)))
}

/// A PNG that looks like an app window, so the demo has something to show where a real
/// project would have screenshots. `variant` changes the layout, `seed` the colour.
fn mock_screenshot(seed: u32, variant: usize) -> Vec<u8> {
    const W: usize = 800;
    const H: usize = 500;
    let hue = [(0x5b, 0x8d, 0xef), (0xe0, 0x6c, 0x9f), (0x3f, 0xb5, 0xa3), (0xe2, 0xa0, 0x3f), (0x9b, 0x7b, 0xe8)]
        [seed as usize % 5];
    let mut pixels = vec![0u8; W * H * 3];
    let mut fill = |x0: usize, y0: usize, x1: usize, y1: usize, c: (u8, u8, u8)| {
        for y in y0..y1.min(H) {
            for x in x0..x1.min(W) {
                let i = (y * W + x) * 3;
                pixels[i..i + 3].copy_from_slice(&[c.0, c.1, c.2]);
            }
        }
    };
    fill(0, 0, W, H, (0x1b, 0x1e, 0x26));
    fill(0, 0, W, 44, (0x25, 0x29, 0x33));
    fill(16, 14, 136, 30, hue);
    match variant % 3 {
        1 => {
            // A sidebar and a list.
            fill(0, 44, 190, H, (0x21, 0x24, 0x2d));
            for row in 0..6 {
                fill(18, 70 + row * 40, 150, 86 + row * 40, if row == 1 { hue } else { (0x3a, 0x3f, 0x4c) });
            }
            for row in 0..7 {
                fill(220, 74 + row * 56, 760, 114 + row * 56, (0x2a, 0x2e, 0x39));
                fill(236, 88 + row * 56, 420 + (row * 53) % 200, 100 + row * 56, (0x58, 0x5f, 0x70));
            }
        }
        2 => {
            // A grid of cards.
            for row in 0..2 {
                for col in 0..3 {
                    let (x, y) = (30 + col * 256, 74 + row * 206);
                    fill(x, y, x + 228, y + 180, (0x2a, 0x2e, 0x39));
                    fill(x, y, x + 228, y + 96, if (row + col) % 2 == 0 { hue } else { (0x3a, 0x3f, 0x4c) });
                    fill(x + 14, y + 116, x + 150, y + 130, (0x58, 0x5f, 0x70));
                }
            }
        }
        _ => {
            // A document.
            fill(140, 74, 660, H, (0x23, 0x27, 0x30));
            fill(180, 110, 480, 136, hue);
            for line in 0..11 {
                fill(180, 168 + line * 28, 620 - (line * 67) % 180, 180 + line * 28, (0x58, 0x5f, 0x70));
            }
        }
    }
    encode_png(W, H, &pixels)
}

/// A square picture that looks like an app icon: a coloured tile with a pale mark on it.
fn mock_icon(seed: u32) -> Vec<u8> {
    const S: usize = 128;
    let hue = [(0x5b, 0x8d, 0xef), (0xe0, 0x6c, 0x9f), (0x3f, 0xb5, 0xa3), (0xe2, 0xa0, 0x3f), (0x9b, 0x7b, 0xe8)]
        [seed as usize % 5];
    let mut pixels = vec![0u8; S * S * 3];
    for y in 0..S {
        for x in 0..S {
            let (dx, dy) = (x as i32 - 64, y as i32 - 64);
            let d = dx * dx + dy * dy;
            // A pale ring on the tile; which part is cut away depends on the seed.
            let ring = (26 * 26..=44 * 44).contains(&d)
                && !((seed.is_multiple_of(3) && dx > 0 && dy < 0) || (seed % 3 == 1 && dy > 12));
            let c = if ring { (0xf4, 0xf6, 0xfb) } else { hue };
            pixels[(y * S + x) * 3..(y * S + x) * 3 + 3].copy_from_slice(&[c.0, c.1, c.2]);
        }
    }
    encode_png(S, S, &pixels)
}

/// PNG: signature, IHDR, one IDAT of zlib-compressed rows (each led by filter 0), IEND.
fn encode_png(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 3 + 1) * height);
    for row in pixels.chunks(width * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &raw).expect("writing to memory");
    let data = encoder.finish().expect("writing to memory");
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        png.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(body);
        png.extend_from_slice(kind);
        png.extend_from_slice(body);
        png.extend_from_slice(&crc.sum().to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend_from_slice(&(width as u32).to_be_bytes());
    header.extend_from_slice(&(height as u32).to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &data);
    chunk(b"IEND", &[]);
    png
}

/// Fictional apps on Flathub.
struct DemoFlatpak {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    developer: &'static str,
    license: &'static str,
    version: &'static str,
    date_unix: u64,
    size: u64,
    installs: u64,
    /// Star counts, one to five, as the ratings service would report them.
    stars: [u32; 5],
}

const FLATPAKS: &[DemoFlatpak] = &[
    DemoFlatpak {
        id: "io.quillpad.Quillpad",
        name: "Quillpad",
        summary: "Notes and to-do lists that stay on your computer.",
        developer: "Quillpad contributors",
        license: "GPL-3.0-or-later",
        version: "4.2.0",
        date_unix: 1_793_750_400,
        size: 38_400_000,
        installs: 41_200,
        stars: [12, 9, 30, 160, 610],
    },
    DemoFlatpak {
        id: "org.sundial.Sundial",
        name: "Sundial",
        summary: "A calm calendar for your day and your week.",
        developer: "Sundial project",
        license: "MIT",
        version: "2.9.1",
        date_unix: 1_792_886_400,
        size: 21_900_000,
        installs: 8_300,
        stars: [4, 6, 18, 70, 122],
    },
    DemoFlatpak {
        id: "org.reelstudio.Reel",
        name: "Reel",
        summary: "Cut, join and title your home videos.",
        developer: "Reel Studio",
        license: "GPL-3.0-or-later",
        version: "1.14.0",
        date_unix: 1_794_268_800,
        size: 142_000_000,
        installs: 96_500,
        stars: [40, 35, 90, 300, 520],
    },
];

fn flatpak_appstream(app: &DemoFlatpak) -> Value {
    let media = format!("https://dl.flathub.org/media/{}", app.id);
    json!({
        "type": "desktop-application",
        "id": app.id,
        "name": app.name,
        "summary": app.summary,
        "description": format!("<p>{} {} is free software, made by volunteers.</p><p>What it does:</p><ul><li>Starts quickly</li><li>Works without an account</li><li>Keeps your data on your computer</li></ul><p>(Fictional app: this is demo data.)</p>", app.name, app.summary.trim_end_matches('.')),
        "developer_name": app.developer,
        "icon": format!("{media}/icon.png"),
        "project_license": app.license,
        "releases": [{ "version": app.version, "timestamp": app.date_unix.to_string(), "description": "<p>Changes:</p><ul><li>Faster to open</li><li>Fixes for reported problems</li></ul>" }],
        "screenshots": [
            { "sizes": [{ "width": "224", "src": format!("{media}/docs/screenshot-1-small.png") }, { "width": "752", "src": format!("{media}/docs/screenshot-1.png") }], "caption": "The main window" },
            { "sizes": [{ "width": "752", "src": format!("{media}/docs/screenshot-2.png") }] },
        ],
        "urls": { "homepage": format!("https://example.invalid/{}", app.name.to_lowercase()) },
    })
}

/// Delay per tenth of a demo download, so progress is visible. Tests set it to 0.
pub static DOWNLOAD_STEP_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(60);

pub struct DemoHttp;

impl Http for DemoHttp {
    fn get(&self, url: &str) -> Result<String> {
        let not_found = || Error::NotFound(url.to_string());
        if let Some(rest) = url.strip_prefix("https://api.github.com/search/repositories?q=") {
            let q = url_decode(rest.split('&').next().unwrap_or(""));
            let q = q.split(" in:").next().unwrap_or("").to_string();
            let items: Vec<Value> =
                all_apps().filter(|a| matches(&q, &[a.name, a.repo, a.summary])).map(repo_json).collect();
            return Ok(json!({ "total_count": items.len(), "items": items }).to_string());
        }
        if let Some(rest) = url.strip_prefix("https://api.github.com/repos/") {
            let path = rest.split('?').next().unwrap_or("");
            let mut parts = path.split('/');
            let (owner, repo, tail) = (parts.next(), parts.next(), parts.next());
            let app = all_apps().find(|a| Some(a.owner) == owner && Some(a.repo) == repo).ok_or_else(not_found)?;
            return match tail {
                None => Ok(repo_json(app).to_string()),
                Some("releases") => Ok(releases_json(app).to_string()),
                Some("readme") => Ok(json!({
                    "name": "README.md",
                    "download_url": format!("https://raw.githubusercontent.com/{}/{}/main/README.md", app.owner, app.repo),
                })
                .to_string()),
                _ => Err(not_found()),
            };
        }
        if let Some(rest) = url.strip_prefix("https://raw.githubusercontent.com/") {
            let mut parts = rest.split('/');
            let (owner, repo) = (parts.next(), parts.next());
            let app = all_apps().find(|a| Some(a.owner) == owner && Some(a.repo) == repo).ok_or_else(not_found)?;
            return Ok(format!(
                "<p align=\"center\"><img src=\"logo.png\" width=\"120\"></p>\n\n# {name}\n\n[![build](https://example.invalid/badge.svg)](https://example.invalid)\n\n{summary}\n\n![Main window](docs/screenshot-1.png)\n![Another view](docs/screenshot-2.png)\n![A third](docs/screenshot-3.png)\n\n## Features\n\n- Starts in under a second\n- Works offline\n- Keyboard shortcuts for **everything**\n\n## Getting started\n\nInstall {name}, open it, and follow the short tour. See the [handbook](https://example.invalid/docs) for more.\n\n(Fictional project: this is demo data.)\n",
                name = app.name,
                summary = app.summary,
            ));
        }
        if let Some(id) = url.strip_prefix("https://flathub.org/api/v2/appstream/") {
            return FLATPAKS
                .iter()
                .find(|a| a.id == id)
                .map(|a| flatpak_appstream(a).to_string())
                .ok_or_else(not_found);
        }
        if let Some(id) = url.strip_prefix("https://flathub.org/api/v2/summary/") {
            let app = FLATPAKS.iter().find(|a| a.id == id).ok_or_else(not_found)?;
            return Ok(
                json!({ "arches": ["x86_64", "aarch64"], "download_size": app.size, "installed_size": app.size * 3 })
                    .to_string(),
            );
        }
        if let Some(rest) = url.strip_prefix("https://codeberg.org/api/v1/repos/search?q=") {
            let q = url_decode(rest.split('&').next().unwrap_or(""));
            let data: Vec<Value> =
                CODEBERG.iter().filter(|a| matches(&q, &[a.name, a.repo, a.summary])).map(forgejo_repo_json).collect();
            return Ok(json!({ "ok": true, "data": data }).to_string());
        }
        if let Some(rest) = url.strip_prefix("https://codeberg.org/api/v1/repos/") {
            let path = rest.split('?').next().unwrap_or("");
            let mut parts = path.split('/');
            let (owner, repo, tail) = (parts.next(), parts.next(), parts.next());
            let app = CODEBERG.iter().find(|a| Some(a.owner) == owner && Some(a.repo) == repo).ok_or_else(not_found)?;
            return match tail {
                None => Ok(forgejo_repo_json(app).to_string()),
                Some("releases") => Ok(forgejo_releases_json(app).to_string()),
                _ => Err(not_found()),
            };
        }
        if let Some(rest) = url.strip_prefix("https://aur.archlinux.org/rpc/v5/search/") {
            let q = url_decode(rest.split('?').next().unwrap_or(""));
            let results: Vec<Value> = AUR.iter().filter(|p| matches(&q, &[p.name, p.summary])).map(aur_json).collect();
            return Ok(
                json!({ "resultcount": results.len(), "results": results, "type": "search", "version": 5 }).to_string()
            );
        }
        if let Some(name) = url.strip_prefix("https://aur.archlinux.org/rpc/v5/info?arg[]=") {
            let name = url_decode(name);
            let results: Vec<Value> = AUR.iter().filter(|p| p.name == name).map(aur_json).collect();
            return Ok(json!({ "resultcount": results.len(), "results": results, "type": "multiinfo", "version": 5 })
                .to_string());
        }
        Err(not_found())
    }

    fn post_json(&self, url: &str, body: &str) -> Result<String> {
        if url.starts_with("https://flathub.org/api/v2/search") {
            let query = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v["query"].as_str().map(str::to_string))
                .unwrap_or_default();
            let hits: Vec<Value> = FLATPAKS
                .iter()
                .filter(|a| matches(&query, &[a.name, a.summary, a.id]))
                .map(|a| {
                    json!({
                        "app_id": a.id, "id": a.id.replace('.', "_"), "name": a.name, "summary": a.summary,
                        "icon": format!("https://dl.flathub.org/media/{}/icon.png", a.id),
                        "developer_name": a.developer, "project_license": a.license,
                        "installs_last_month": a.installs, "arches": ["x86_64", "aarch64"],
                    })
                })
                .collect();
            return Ok(json!({ "hits": hits, "query": query, "totalHits": hits.len() }).to_string());
        }
        if url == crate::ratings::ODRS_FETCH {
            let asked = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| v["app_id"].as_str().map(str::to_string))
                .unwrap_or_default();
            let Some(app) = FLATPAKS.iter().find(|a| a.id == asked) else { return Ok("[]".to_string()) };
            let review = |rating: u32, who: &str, days_before: u64, title: &str, text: String| {
                json!({
                    "app_id": app.id, "rating": rating, "summary": title, "description": text, "user_display": who,
                    "date_created": app.date_unix - days_before * 86_400, "version": app.version, "distro": "Demo Linux",
                    "karma_up": 4, "karma_down": 0, "score": rating, "locale": "en",
                })
            };
            return Ok(json!([
                review(100, "Marta", 3, "Exactly what I wanted", format!("{} opens fast and stays out of the way. I moved everything over in an evening.", app.name)),
                review(80, "J. Okafor", 21, "Good, with one wish", "Does what it says. I would like bigger text in the lists, but it has been reliable for months.".to_string()),
                review(40, "sam", 60, "Lost my window layout", "After the last update it forgot how I had things arranged. Everything else still works. (Fictional review: this is demo data.)".to_string()),
            ])
            .to_string());
        }
        Err(Error::NotFound(url.to_string()))
    }

    fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
        // Star ratings, in the shape the ratings service publishes them.
        if url == crate::ratings::ODRS_RATINGS {
            let all: serde_json::Map<String, Value> = FLATPAKS
                .iter()
                .map(|a| {
                    let [one, two, three, four, five] = a.stars;
                    let total = one + two + three + four + five;
                    (a.id.to_string(), json!({ "star0": 0, "star1": one, "star2": two, "star3": three, "star4": four, "star5": five, "total": total }))
                })
                .collect();
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let bytes = Value::Object(all).to_string().into_bytes();
            progress(bytes.len() as u64, Some(bytes.len() as u64));
            std::fs::write(dest, &bytes)?;
            return Ok(hex(&Sha256::digest(&bytes)));
        }
        // Icons: Flathub's, and the logo a fictional project's README shows.
        if url.ends_with("/icon.png") || url.ends_with("/logo.png") {
            let seed = url.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let bytes = mock_icon(seed);
            progress(bytes.len() as u64, Some(bytes.len() as u64));
            std::fs::write(dest, &bytes)?;
            return Ok(hex(&Sha256::digest(&bytes)));
        }
        // The fictional owners have no pictures.
        if url.ends_with(".png?size=128") {
            return Err(Error::NotFound(url.to_string()));
        }
        // Their READMEs do show the app: a drawn stand-in for a screenshot.
        if let Some(at) = url.find("/docs/screenshot-") {
            let variant = url[at..].bytes().filter(u8::is_ascii_digit).map(|d| (d - b'0') as usize).next().unwrap_or(1);
            let seed = url.bytes().take(at).fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let bytes = mock_screenshot(seed, variant);
            progress(bytes.len() as u64, Some(bytes.len() as u64));
            std::fs::write(dest, &bytes)?;
            return Ok(hex(&Sha256::digest(&bytes)));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let total = PAYLOAD.len() as u64;
        for step in 1..=10u64 {
            std::thread::sleep(std::time::Duration::from_millis(
                DOWNLOAD_STEP_MS.load(std::sync::atomic::Ordering::Relaxed),
            ));
            if !progress(total * step / 10, Some(total)) {
                return Err(Error::Cancelled);
            }
        }
        std::fs::write(dest, PAYLOAD)?;
        Ok(hex(&Sha256::digest(PAYLOAD)))
    }
}

/// Fictional repositories for the system's package manager: a CachyOS-style layout for
/// pacman and an Ubuntu-style one for apt. Each catalog only uses the one its platform has.
pub fn system_repos() -> Vec<crate::source::system::Backend> {
    use crate::model::SourceKind;
    use crate::source::system::{Backend, SystemPackage};
    let package = |name: &str, version: &str, summary: &str, repo: &str, arch: &str| SystemPackage {
        name: name.to_string(),
        version: version.to_string(),
        summary: summary.to_string(),
        repo: repo.to_string(),
        arch: arch.to_string(),
        homepage: format!("https://example.invalid/{name}"),
        license: "GPL-3.0-or-later".to_string(),
        size: 18 * 1024 * 1024,
        built: "2026-11-08".to_string(),
    };
    vec![
        Backend::Fixed {
            kind: SourceKind::Pacman,
            packages: vec![
                package(
                    "swiftkernel",
                    "6.17.4-2",
                    "Linux kernel tuned for desktop responsiveness.",
                    "cachyos-v3",
                    "x86_64_v3",
                ),
                package("pinecone", "0.61.0-1.1", "Fast fuzzy finder for the terminal.", "cachyos-v3", "x86_64_v3"),
                package("harbor", "5.3.1-1", "Run and manage containers without a daemon.", "extra", "x86_64"),
                package("mistral-fonts", "4.1-1", "A family of clear interface typefaces.", "extra", "any"),
                package("coreshell", "5.3-1", "The standard command interpreter.", "core", "x86_64"),
            ],
            // One current, one with a newer version waiting in the repository.
            installed: vec![("coreshell".into(), "5.3-1".into()), ("harbor".into(), "5.2.0-1".into())],
        },
        Backend::Fixed {
            kind: SourceKind::Apt,
            packages: vec![
                package(
                    "harbor",
                    "5.0.3+ds1-2ubuntu1",
                    "Run and manage containers without a daemon.",
                    "noble-updates/universe",
                    "amd64",
                ),
                package("pinecone", "0.44.1-1", "Fast fuzzy finder for the terminal.", "noble/universe", "amd64"),
                package("fonts-mistral", "4.1-1", "A family of clear interface typefaces.", "noble/main", "all"),
            ],
            installed: vec![("harbor".into(), "5.0.3+ds1-2ubuntu1".into())],
        },
    ]
}

/// Discover-page index for the demo catalog.
pub fn index() -> Index {
    let mut apps: Vec<IndexEntry> = APPS
        .iter()
        .map(|a| IndexEntry {
            id: format!("github:{}/{}", a.owner, a.repo),
            name: a.name.to_string(),
            author: a.owner.to_string(),
            summary: a.summary.to_string(),
            category: a.category.to_string(),
            os: a.os.iter().map(|s| s.to_string()).collect(),
            tv: a.tv,
            // Files of the recent releases, as an index generator would record them.
            files: a.releases.iter().flat_map(|(_, _, files)| files.iter()).map(|f| f.to_string()).collect(),
        })
        .collect();
    apps.extend(AUR.iter().map(|p| IndexEntry {
        id: format!("aur:{}", p.name),
        name: p.name.to_string(),
        author: p.maintainer.to_string(),
        summary: p.summary.to_string(),
        category: "Arch User Repository".to_string(),
        os: vec!["arch".to_string()],
        tv: false,
        files: vec![],
    }));
    Index { apps }
}
