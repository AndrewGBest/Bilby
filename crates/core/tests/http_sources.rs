//! The network layer and both sources, exercised over real HTTP against a server on
//! localhost that answers in the shapes the GitHub REST API and the aurweb RPC document.
//!
//! The fixtures in `fixtures/` were written from the published API documentation, not
//! captured from the live services: they prove the client handles the documented shape
//! (including nulls, missing optional fields and fields it does not know), not that the
//! documentation matches production.

use bilby_core::catalog::{Catalog, UpdateStatus};
use bilby_core::compat::Level;
use bilby_core::http::{CachedHttp, Http, NetHttp};
use bilby_core::index::Index;
use bilby_core::model::{AppId, SourceKind};
use bilby_core::source::{aur::Aur, github::GitHub, Source};
use bilby_core::{Error, Platform};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    /// Send the body in pieces with pauses, so a download can be cancelled part-way.
    slow: bool,
}

impl Reply {
    fn json(body: &str) -> Reply {
        Reply {
            status: 200,
            headers: vec![("Content-Type", "application/json".into())],
            body: body.into(),
            slow: false,
        }
    }
    fn status(status: u16) -> Reply {
        Reply { status, headers: vec![], body: b"{\"message\":\"nope\"}".to_vec(), slow: false }
    }
    fn with(mut self, name: &'static str, value: &str) -> Reply {
        self.headers.push((name, value.to_string()));
        self
    }
}

/// What the server saw: `(path, lower-cased headers)` per request.
type Seen = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

struct Server {
    base: String,
    seen: Seen,
}

impl Server {
    /// Serve `routes` (path with query -> reply) until the test ends. Unknown paths get 404.
    fn start(routes: Vec<(&str, Reply)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: HashMap<String, Reply> = routes.into_iter().map(|(p, r)| (p.to_string(), r)).collect();
        let seen: Seen = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = routes.clone();
                let log = log.clone();
                std::thread::spawn(move || {
                    let _ = handle(stream, &routes, &log);
                });
            }
        });
        Server { base, seen }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn requests(&self) -> Vec<(String, HashMap<String, String>)> {
        self.seen.lock().unwrap().clone()
    }
}

fn handle(mut stream: TcpStream, routes: &HashMap<String, Reply>, log: &Seen) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    // The body of a POST, read so the client is not cut off mid-send, and kept for the test.
    let length = headers.get("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let mut body = vec![0u8; length];
    std::io::Read::read_exact(&mut reader, &mut body)?;
    headers.insert(":method".to_string(), request_line.split_whitespace().next().unwrap_or("").to_string());
    headers.insert(":body".to_string(), String::from_utf8_lossy(&body).to_string());
    log.lock().unwrap().push((path.clone(), headers));
    let reply = routes.get(&path).cloned().unwrap_or_else(|| Reply::status(404));
    let mut head =
        format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if reply.slow {
        for chunk in reply.body.chunks(reply.body.len().div_ceil(8).max(1)) {
            stream.write_all(chunk)?;
            stream.flush()?;
            std::thread::sleep(Duration::from_millis(40));
        }
    } else {
        stream.write_all(&reply.body)?;
    }
    stream.flush()
}

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(path).unwrap()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bilby-http-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A client whose "GitHub API" is the local server, ignoring any proxy in the environment.
fn client(server: &Server, token: Option<&str>) -> NetHttp {
    NetHttp::with_options(token.map(String::from), &server.url("/api"), false)
}

// ---- transport --------------------------------------------------------------------

#[test]
fn get_returns_the_body_and_identifies_itself() {
    let server = Server::start(vec![("/api/thing", Reply::json("{\"ok\":true}"))]);
    let http = client(&server, None);
    assert_eq!(http.get(&server.url("/api/thing")).unwrap(), "{\"ok\":true}");
    let (_, headers) = &server.requests()[0];
    assert!(headers["user-agent"].starts_with("bilby/"));
    assert_eq!(headers["accept"], "application/vnd.github+json");
    assert!(headers.contains_key("x-github-api-version"));
    assert!(!headers.contains_key("authorization"), "no token, no Authorization header");
}

#[test]
fn the_token_goes_to_the_api_and_nowhere_else() {
    let server = Server::start(vec![("/api/x", Reply::json("{}")), ("/elsewhere/x", Reply::json("{}"))]);
    let http = client(&server, Some("secret-token"));
    http.get(&server.url("/api/x")).unwrap();
    http.get(&server.url("/elsewhere/x")).unwrap();
    let dest = temp("token").join("file.bin");
    http.download(&server.url("/elsewhere/x"), &dest, &mut |_, _| true).unwrap();

    let requests = server.requests();
    assert_eq!(requests[0].1["authorization"], "Bearer secret-token");
    for (path, headers) in &requests[1..] {
        assert!(!headers.contains_key("authorization"), "token leaked to {path}");
        assert!(!headers.contains_key("x-github-api-version"), "{path}");
    }
    // A blank token is the same as none.
    let blank = client(&server, Some("   "));
    blank.get(&server.url("/api/x")).unwrap();
    assert!(!server.requests().last().unwrap().1.contains_key("authorization"));
}

#[test]
fn errors_are_told_apart() {
    let server = Server::start(vec![
        ("/api/limited", Reply::status(403).with("x-ratelimit-remaining", "0")),
        ("/api/secondary", Reply::status(429).with("retry-after", "30")),
        ("/api/forbidden", Reply::status(403).with("x-ratelimit-remaining", "41")),
        ("/api/broken", Reply::status(500)),
        ("/other/limited", Reply::status(403).with("x-ratelimit-remaining", "0")),
    ]);
    let http = client(&server, None);
    let err = |path: &str| http.get(&server.url(path)).unwrap_err();

    match err("/api/limited") {
        Error::RateLimited { source, hint } => {
            assert_eq!(source, "GitHub");
            assert!(hint.contains("Sign in"), "without a token the hint says how to raise the limit: {hint}");
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
    assert!(matches!(err("/api/secondary"), Error::RateLimited { .. }));
    assert!(
        matches!(err("/api/forbidden"), Error::Status { code: 403, .. }),
        "a 403 with quota left is not a rate limit"
    );
    assert!(matches!(err("/api/broken"), Error::Status { code: 500, .. }));
    assert!(matches!(err("/api/missing"), Error::NotFound(_)));
    assert!(matches!(err("/other/limited"), Error::Status { code: 403, .. }), "only the API host is read as GitHub");

    // With a token the advice changes: adding one would not help.
    match client(&server, Some("t")).get(&server.url("/api/limited")).unwrap_err() {
        Error::RateLimited { hint, .. } => assert!(!hint.contains("Add a GitHub token"), "{hint}"),
        other => panic!("{other:?}"),
    }

    // Nothing listening at all: a sentence a person can act on, no library internals.
    let dead = NetHttp::with_options(None, "http://127.0.0.1:1", false);
    match dead.get("http://127.0.0.1:1/x?secret=1").unwrap_err() {
        Error::Network(message) => {
            assert_eq!(message, "couldn't connect to 127.0.0.1:1. Check your internet connection.");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_remaining_allowance_is_tracked() {
    let limits = |remaining: &str| {
        Reply::json("{}")
            .with("x-ratelimit-limit", "60")
            .with("x-ratelimit-remaining", remaining)
            .with("x-ratelimit-reset", "1700001380")
    };
    let server = Server::start(vec![
        ("/api/a", limits("59")),
        ("/api/b", limits("58")),
        (
            "/api/out",
            Reply::status(403)
                .with("x-ratelimit-limit", "60")
                .with("x-ratelimit-remaining", "0")
                .with("x-ratelimit-reset", "1700001380"),
        ),
        ("/other", limits("1")),
    ]);
    let http = client(&server, None);
    assert_eq!(http.quota(), None, "unknown until GitHub has answered once");
    http.get(&server.url("/api/a")).unwrap();
    http.get(&server.url("/api/b")).unwrap();
    http.get(&server.url("/other")).unwrap();
    let quota = http.quota().unwrap();
    assert_eq!((quota.remaining, quota.limit), (58, 60), "other hosts do not change it");
    assert_eq!(quota.describe(1_700_000_000), "58 of 60 GitHub requests left; refills in 23 minutes");
    assert_eq!(quota.describe(1_800_000_000), "58 of 60 GitHub requests left; refills now");

    let _ = http.get(&server.url("/api/out"));
    assert_eq!(http.quota().unwrap().remaining, 0, "an error response updates it too");

    // The cache passes it through.
    let cached = CachedHttp::new(http, temp("quota"), Duration::from_secs(60));
    assert_eq!(cached.quota().unwrap().remaining, 0);
}

#[test]
fn downloads_report_progress_and_hash_what_they_wrote() {
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let reply = Reply { status: 200, headers: vec![], body: body.clone(), slow: false };
    let server = Server::start(vec![("/files/app.bin", reply)]);
    let http = client(&server, None);
    let dest = temp("download").join("deep").join("app.bin");

    let mut calls = Vec::new();
    let hash = http
        .download(&server.url("/files/app.bin"), &dest, &mut |done, total| {
            calls.push((done, total));
            true
        })
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    use sha2::{Digest, Sha256};
    assert_eq!(hash, bilby_core::http::hex(&Sha256::digest(&body)));
    assert!(calls.windows(2).all(|w| w[0].0 < w[1].0), "progress only moves forward");
    assert_eq!(calls.last().unwrap(), &(200_000, Some(200_000)));
    assert!(!dest.with_extension("part").exists(), "no leftovers");
}

#[test]
fn a_cancelled_download_leaves_nothing_behind() {
    let reply = Reply { status: 200, headers: vec![], body: vec![7u8; 400_000], slow: true };
    let server = Server::start(vec![("/files/big.bin", reply)]);
    let http = client(&server, None);
    let dest = temp("cancel").join("big.bin");
    let mut calls = 0;
    let err = http
        .download(&server.url("/files/big.bin"), &dest, &mut |_, _| {
            calls += 1;
            calls < 2
        })
        .unwrap_err();
    assert!(matches!(err, Error::Cancelled));
    assert!(!dest.exists(), "a cancelled download must not look finished");
    assert!(!dest.with_extension("part").exists());
}

#[test]
fn a_failed_download_does_not_create_the_file() {
    let server = Server::start(vec![]);
    let http = client(&server, None);
    let dest = temp("missing").join("x.bin");
    assert!(matches!(http.download(&server.url("/nope"), &dest, &mut |_, _| true), Err(Error::NotFound(_))));
    assert!(!dest.exists());
}

#[test]
fn the_cache_absorbs_a_rate_limit() {
    // First the API answers, then it starts refusing: the cached answer keeps the app usable.
    let good = Server::start(vec![("/api/data", Reply::json("{\"n\":1}"))]);
    let dir = temp("cache");
    let fresh = CachedHttp::new(client(&good, None), dir.clone(), Duration::from_secs(600));
    assert_eq!(fresh.get(&good.url("/api/data")).unwrap(), "{\"n\":1}");
    assert_eq!(fresh.get(&good.url("/api/data")).unwrap(), "{\"n\":1}");
    assert_eq!(good.requests().len(), 1, "the second read never reached the network");

    // Same URL, expired cache, server now rate limiting. The entry is keyed by URL, so
    // point a zero-TTL cache at a client whose requests fail.
    let dead = NetHttp::with_options(None, "http://127.0.0.1:1", false);
    let expired = CachedHttp::new(dead, dir, Duration::ZERO);
    assert_eq!(expired.get(&good.url("/api/data")).unwrap(), "{\"n\":1}", "stale beats nothing");
}

// ---- GitHub -----------------------------------------------------------------------

fn github(server: &Server) -> GitHub {
    GitHub::with_api(Arc::new(client(server, None)), &server.url("/api"))
}

#[test]
fn github_search_parses_the_documented_shape() {
    let query =
        "/api/search/repositories?q=tetris%20in%3Aname%2Cdescription%20fork%3Afalse%20archived%3Afalse&per_page=5";
    let server = Server::start(vec![(query, Reply::json(&fixture("github_search.json")))]);
    let hits = github(&server).search("tetris", 5).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, AppId::new(SourceKind::GitHub, "dtrupenn/Tetris"));
    assert_eq!(hits[0].name, "Tetris");
    assert_eq!(hits[0].author, "dtrupenn");
    assert_eq!(hits[0].popularity, Some(1));
    assert_eq!(hits[0].license.as_deref(), Some("MIT"));
    assert_eq!(hits[0].homepage, "https://github.com/dtrupenn/Tetris");
    // Null description and null licence are fine.
    assert_eq!(hits[1].summary, "");
    assert_eq!(hits[1].license, None);
}

#[test]
fn github_releases_skip_drafts_and_keep_digests() {
    let server = Server::start(vec![(
        "/api/repos/octocat/Hello-World/releases?per_page=6",
        Reply::json(&fixture("github_releases.json")),
    )]);
    let releases = github(&server).releases("octocat/Hello-World").unwrap();
    let tags: Vec<&str> = releases.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(tags, ["v2.0.0-rc.1", "v1.0.0"], "the draft is gone");
    assert!(releases[0].prerelease);
    assert_eq!(releases[0].notes, "", "a null body is empty notes");
    assert_eq!(releases[0].assets[0].digest, None);
    let stable = &releases[1];
    assert_eq!(stable.published, "2013-02-27");
    assert_eq!(stable.assets.len(), 3);
    assert_eq!(
        stable.assets[0].digest.as_deref(),
        Some("sha256:2151b604e3429bff440b9fbc03eb3617bc2603cda96c95b9bb05277f9ddba255")
    );
    assert_eq!(stable.assets[2].size, 0, "a missing size is tolerated");
    // How often each release's files were fetched, added up; a file without a count adds none.
    assert_eq!((releases[0].downloads, stable.downloads), (Some(3), Some(42)));
}

#[test]
fn an_archived_github_project_says_so() {
    let repo = |archived: bool| {
        format!(
            r#"{{"name":"old","full_name":"a/old","owner":{{"login":"a"}},"description":null,"html_url":"https://github.com/a/old","stargazers_count":9,"license":null,"archived":{archived}}}"#
        )
    };
    let server = Server::start(vec![
        ("/api/repos/a/old", Reply::json(&repo(true))),
        ("/api/repos/a/live", Reply::json(&repo(false))),
    ]);
    assert!(github(&server).summary("a/old").unwrap().warning.unwrap().contains("no longer maintained"));
    assert_eq!(github(&server).summary("a/live").unwrap().warning, None);
}

#[test]
fn github_details_end_to_end_over_http() {
    let repo = r#"{"name":"Hello-World","full_name":"octocat/Hello-World","owner":{"login":"octocat"},
        "description":"My first repository","html_url":"https://github.com/octocat/Hello-World","stargazers_count":80,"license":null}"#;
    let server = Server::start(vec![
        ("/api/repos/octocat/Hello-World", Reply::json(repo)),
        ("/api/repos/octocat/Hello-World/releases?per_page=6", Reply::json(&fixture("github_releases.json"))),
    ]);
    let catalog = |preset: &str| {
        let http: Arc<dyn Http> = Arc::new(client(&server, None));
        let source: Arc<dyn Source> = Arc::new(GitHub::with_api(http.clone(), &server.url("/api")));
        Catalog::with_sources(Platform::preset(preset).unwrap(), http, vec![source], Index::default())
    };
    let id = AppId::new(SourceKind::GitHub, "octocat/Hello-World");

    // Debian: the stable .deb wins over the newer release candidate's AppImage.
    let debian = catalog("linux-x64-debian").details(&id).unwrap();
    let release = debian.release.unwrap();
    assert_eq!(release.version, "v1.0.0");
    assert_eq!(release.options[0].asset.name, "hello_1.0.0_amd64.deb");
    assert_eq!(release.options[0].verdict.level, Level::Native);
    assert!(release.hidden.iter().any(|h| h.name == "hello-1.0.0-arm64.dmg" && h.reason == "Built for macOS"));
    assert!(release.hidden.iter().any(|h| h.name == "example.zip"), "an archive that names no system is hidden");

    // Fedora has no use for the .deb; only the pre-release has something, so it is offered.
    let fedora = catalog("linux-x64-fedora").details(&id).unwrap();
    let release = fedora.release.unwrap();
    assert_eq!(release.version, "v2.0.0-rc.1");
    assert!(release.prerelease);

    // Apple silicon gets the dmg; Windows gets an explanation.
    let mac = catalog("macos-arm64").details(&id).unwrap();
    assert_eq!(mac.release.unwrap().options[0].asset.name, "hello-1.0.0-arm64.dmg");
    let windows = catalog("windows-x64").details(&id).unwrap();
    assert!(windows.unavailable_reason.unwrap().contains("Windows"));

    assert_eq!(
        catalog("linux-x64-debian").check_update(&id, "v0.9.0"),
        UpdateStatus::Available { version: "v1.0.0".into(), label: "Debian package · x86_64".into() }
    );
}

#[test]
fn a_rate_limited_search_is_reported_not_swallowed() {
    let server = Server::start(vec![(
        "/api/search/repositories?q=anything%20in%3Aname%2Cdescription%20fork%3Afalse%20archived%3Afalse&per_page=8",
        Reply::status(403).with("x-ratelimit-remaining", "0"),
    )]);
    let http: Arc<dyn Http> = Arc::new(client(&server, None));
    let source: Arc<dyn Source> = Arc::new(GitHub::with_api(http.clone(), &server.url("/api")));
    let catalog =
        Catalog::with_sources(Platform::preset("linux-x64-debian").unwrap(), http, vec![source], Index::default());
    let outcome = catalog.search("anything");
    assert!(outcome.apps.is_empty());
    assert_eq!(outcome.errors.len(), 1);
    assert_eq!(
        outcome.errors[0],
        "GitHub's request limit for this device is used up for now. Sign in to GitHub in Settings to raise the limit, or try again later."
    );
}

// ---- AUR --------------------------------------------------------------------------

fn aur(server: &Server) -> Aur {
    Aur::with_rpc(Arc::new(client(server, Some("github-token"))), &server.url("/rpc/v5"))
}

#[test]
fn aur_search_and_info_parse_the_documented_shape() {
    let server = Server::start(vec![
        ("/rpc/v5/search/paru?by=name-desc", Reply::json(&fixture("aur_search.json"))),
        ("/rpc/v5/info?arg[]=paru", Reply::json(&fixture("aur_info.json"))),
        ("/rpc/v5/info?arg[]=ghost", Reply::json(r#"{"resultcount":0,"results":[],"type":"multiinfo","version":5}"#)),
    ]);
    let source = aur(&server);

    let hits = source.search("paru", 10).unwrap();
    assert_eq!(hits.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["paru", "paru-bin"], "most votes first");
    assert_eq!(hits[0].popularity, Some(1045));
    assert_eq!(hits[0].version.as_deref(), Some("2.0.4-1"));
    assert_eq!(hits[1].author, "orphaned", "a null maintainer");
    assert!(hits[1].warning.as_deref().unwrap().contains("out of date"));
    assert_eq!(hits[0].warning, None);
    // Recent interest: the AUR's own figure, on a scale of nought to one.
    let (paru, paru_bin) = (hits[0].momentum.unwrap(), hits[1].momentum.unwrap());
    assert!(paru > paru_bin && paru < 1.0 && paru_bin > 0.25, "{paru} {paru_bin}");
    assert_eq!((hits[0].downloads, hits[0].verified), (None, false), "the AUR counts no downloads");

    let summary = source.summary("paru").unwrap();
    assert_eq!(summary.license.as_deref(), Some("GPL-3.0-or-later"));
    let releases = source.releases("paru").unwrap();
    assert_eq!(releases[0].version, "2.0.4-1");
    assert_eq!(releases[0].published, "2024-07-09");
    assert_eq!(releases[0].assets[0].url, "https://aur.archlinux.org/cgit/aur.git/snapshot/paru.tar.gz");

    assert!(matches!(source.summary("ghost"), Err(Error::NotFound(_))));
    // The GitHub token is never sent to the AUR.
    assert!(server.requests().iter().all(|(_, headers)| !headers.contains_key("authorization")));
}

#[test]
fn an_aur_search_costs_one_request() {
    let server = Server::start(vec![("/rpc/v5/search/paru?by=name-desc", Reply::json(&fixture("aur_search.json")))]);
    let http: Arc<dyn Http> = Arc::new(client(&server, None));
    let source: Arc<dyn Source> = Arc::new(Aur::with_rpc(http.clone(), &server.url("/rpc/v5")));
    let on = |preset: &str| {
        Catalog::with_sources(Platform::preset(preset).unwrap(), http.clone(), vec![source.clone()], Index::default())
    };

    let outcome = on("linux-x64-arch").search("paru");
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    let found: Vec<_> =
        outcome.apps.iter().map(|a| (a.name.as_str(), a.install_label.as_deref(), a.version.as_deref())).collect();
    assert_eq!(
        found,
        [("paru", Some("AUR package"), Some("2.0.4-1")), ("paru-bin", Some("AUR package"), Some("2.0.4-1"))]
    );
    assert_eq!(server.requests().len(), 1, "no per-package lookups: the search result is enough");

    // Not an Arch system: the AUR is not asked at all.
    assert!(on("linux-x64-debian").search("paru").apps.is_empty());
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn aur_errors_and_short_queries() {
    let server = Server::start(vec![("/rpc/v5/search/zz?by=name-desc", Reply::json(&fixture("aur_error.json")))]);
    let source = aur(&server);
    assert!(source.search("z", 10).unwrap().is_empty(), "one letter never reaches the server");
    assert_eq!(server.requests().len(), 0);
    match source.search("zz", 10) {
        Err(Error::Parse(message)) => assert_eq!(message, "Query arg too small."),
        other => panic!("{other:?}"),
    }
    // Names that could change the URL are rejected before any request.
    assert!(source.summary("a/b").is_err());
    assert!(source.summary("x?arg[]=y").is_err());
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn garbage_from_the_server_is_an_error_not_a_crash() {
    let server = Server::start(vec![
        ("/api/repos/a/b/releases?per_page=6", Reply::json("<html>502 Bad Gateway</html>")),
        ("/api/repos/a/c/releases?per_page=6", Reply::json("{\"message\":\"Not an array\"}")),
        ("/rpc/v5/info?arg[]=x", Reply::json("")),
    ]);
    assert!(matches!(github(&server).releases("a/b"), Err(Error::Parse(_))));
    assert!(matches!(github(&server).releases("a/c"), Err(Error::Parse(_))));
    assert!(matches!(aur(&server).summary("x"), Err(Error::Parse(_))));
}

// ---- Codeberg (Forgejo API) ----

const FORGEJO_SEARCH: &str = r#"{"ok":true,"data":[
  {"id":1,"name":"notes","full_name":"fern/notes","owner":{"id":7,"login":"fern","avatar_url":"x"},
   "description":"Plain notes","html_url":"https://codeberg.org/fern/notes","stars_count":321,"fork":false,"archived":false,"unknown_field":[1,2]},
  {"id":2,"name":"notes-fork","full_name":"other/notes-fork","owner":{"login":"other"},
   "description":"","html_url":"https://codeberg.org/other/notes-fork","stars_count":2,"fork":true,"archived":false},
  {"id":3,"name":"old-notes","full_name":"fern/old-notes","owner":{"login":"fern"},
   "html_url":"https://codeberg.org/fern/old-notes","archived":true}]}"#;

const FORGEJO_RELEASES: &str = r#"[
  {"id":9,"tag_name":"v2.0.0-draft","draft":true,"prerelease":false,"published_at":null,"body":null,"assets":[]},
  {"id":8,"tag_name":"v1.6.0","name":"1.6","draft":false,"prerelease":false,"published_at":"2026-10-19T08:00:00Z",
   "body":"Faster sync","assets":[
     {"id":1,"name":"notes_1.6.0_amd64.deb","size":1048576,"download_count":4,"browser_download_url":"https://codeberg.org/fern/notes/releases/download/v1.6.0/notes_1.6.0_amd64.deb"},
     {"id":2,"name":"notes-1.6.0-arm64-v8a.apk","size":2097152,"browser_download_url":"https://codeberg.org/fern/notes/releases/download/v1.6.0/notes-1.6.0-arm64-v8a.apk"}]}]"#;

#[test]
fn codeberg_search_and_releases_parse_the_documented_shape() {
    use bilby_core::source::forgejo::Forgejo;
    let server = Server::start(vec![
        ("/forge/v1/repos/search?q=notes&limit=5&sort=stars&order=desc", Reply::json(FORGEJO_SEARCH)),
        (
            "/forge/v1/repos/fern/notes",
            Reply::json(
                r#"{"name":"notes","full_name":"fern/notes","owner":{"login":"fern"},"description":"Plain notes","html_url":"https://codeberg.org/fern/notes","stars_count":321}"#,
            ),
        ),
        ("/forge/v1/repos/fern/notes/releases?limit=6", Reply::json(FORGEJO_RELEASES)),
    ]);
    let source = Forgejo::with_api(Arc::new(client(&server, Some("github-token"))), &server.url("/forge/v1"));

    let hits = source.search("notes", 5).unwrap();
    assert_eq!(hits.len(), 1, "forks and archived repositories are left out");
    assert_eq!(hits[0].id, AppId::new(SourceKind::Codeberg, "fern/notes"));
    assert_eq!(hits[0].popularity, Some(321));
    assert_eq!(source.summary("fern/notes").unwrap().author, "fern");

    let releases = source.releases("fern/notes").unwrap();
    assert_eq!(releases.len(), 1, "drafts are skipped");
    assert_eq!((releases[0].version.as_str(), releases[0].published.as_str()), ("v1.6.0", "2026-10-19"));
    assert_eq!(releases[0].assets.len(), 2);
    assert_eq!(releases[0].assets[0].digest, None, "Forgejo publishes no checksum with a file");

    assert!(matches!(source.releases("../x"), Err(Error::NotFound(_))), "keys are checked before they reach a URL");
    for (path, headers) in server.requests() {
        assert!(!headers.contains_key("authorization"), "the GitHub token must not go to Codeberg ({path})");
    }
}

// ---- Flathub ----

#[test]
fn flathub_search_and_details_parse_the_shapes_its_source_code_describes() {
    use bilby_core::source::flathub::Flathub;
    let search = r#"{"hits":[
      {"app_id":"org.example.Notes","id":"org_example_Notes","name":"Notes","summary":"Write things down",
       "icon":"https://dl.flathub.org/media/org/example/Notes/icon.png","developer_name":"Example","project_license":"MIT",
       "installs_last_month":1234,"arches":["x86_64"],"unknown":true,"verification_verified":true,"trending":3.5},
      {"app_id":"../../etc","name":"bad"}],
      "query":"notes","totalHits":2}"#;
    let appstream = r#"{"type":"desktop-application","id":"org.example.Notes","name":"Notes","summary":"Write things down",
      "description":"<p>Plain notes.</p><ul><li>Fast</li></ul>","developer_name":"Example",
      "icon":"https://evil.example/icon.png","project_license":"MIT",
      "releases":[{"version":"2.1","timestamp":1793750400,"description":"<p>Fixes</p>"},{"version":"2.0","timestamp":"1790000000"}],
      "screenshots":[{"sizes":[{"width":"224","src":"https://dl.flathub.org/media/a-small.png"},{"width":752,"src":"https://dl.flathub.org/media/a.png"}]},
                     {"sizes":[{"width":"1504","src":"https://dl.flathub.org/media/b.webp"}]},
                     {"sizes":[{"width":"752","src":"https://elsewhere.example/c.png"}]}],
      "urls":{"homepage":"https://example.org/notes","vcs_browser":"https://github.com/example/notes"},
      "metadata":{"flathub::verification::verified":"true","flathub::verification::method":"website"}}"#;
    let server = Server::start(vec![
        ("/fh/search?locale=en", Reply::json(search)),
        ("/fh/appstream/org.example.Notes", Reply::json(appstream)),
        (
            "/fh/stats/org.example.Notes",
            Reply::json(r#"{"installs_total":98765,"installs_last_month":1234,"installs_per_day":{}}"#),
        ),
        (
            "/fh/summary/org.example.Notes",
            Reply::json(r#"{"arches":["x86_64"],"download_size":4096,"installed_size":9000}"#),
        ),
    ]);
    let roots = temp("flathub-roots");
    std::fs::create_dir_all(roots.join("app").join("org.example.Notes")).unwrap();
    let source = Flathub::with_api(Arc::new(client(&server, Some("github-token"))), &server.url("/fh"), vec![roots]);

    let hits = source.search("notes", 5).unwrap();
    assert_eq!(hits.len(), 1, "an id that could not be a real app is dropped");
    assert_eq!(hits[0].id, AppId::new(SourceKind::Flathub, "org.example.Notes"));
    assert_eq!(hits[0].popularity, Some(1234));
    assert!(hits[0].verified, "Flathub has confirmed the developer");
    assert_eq!(hits[0].momentum, Some(0.75), "being installed more than before");
    assert_eq!(hits[0].icon_url.as_deref(), Some("https://dl.flathub.org/media/org/example/Notes/icon.png"));
    assert_eq!(hits[0].installed_version.as_deref(), Some("installed"), "found in Flatpak's own folder");
    let sent = &server.requests()[0].1;
    assert_eq!(sent[":method"], "POST");
    assert!(sent[":body"].contains(r#""query":"notes""#), "{}", sent[":body"]);
    assert!(!sent.contains_key("authorization"), "the GitHub token must not go to Flathub");

    let summary = source.summary("org.example.Notes").unwrap();
    assert_eq!(summary.homepage, "https://example.org/notes");
    assert_eq!((summary.popularity, summary.downloads, summary.verified), (Some(1234), Some(98765), true));
    assert_eq!(summary.icon_url, None, "an icon served from somewhere else is not fetched");

    let releases = source.releases("org.example.Notes").unwrap();
    assert_eq!((releases[0].version.as_str(), releases[0].published.as_str()), ("2.1", "2026-11-04"));
    assert_eq!(releases[0].notes, "Fixes");
    assert_eq!(releases[0].assets[0].size, 4096);
    assert_eq!(source.about("org.example.Notes").unwrap().as_deref(), Some("Plain notes.\n\n•  Fast"));
    assert_eq!(
        source.pictures("org.example.Notes"),
        ["https://dl.flathub.org/media/a.png"],
        "the size nearest a page; no WebP (cannot be drawn), nothing from another host"
    );
    assert!(matches!(source.summary("not-an-id"), Err(Error::NotFound(_))));
}

// ---- GitLab, Homebrew, the Snap Store and F-Droid ----
//
// Unlike the fixtures above, these were captured from the live services and trimmed.

#[test]
fn gitlab_projects_and_their_release_links_are_read() {
    use bilby_core::source::gitlab::GitLab;
    let server = Server::start(vec![
        (
            "/gl/projects?search=inkscape&order_by=star_count&sort=desc&visibility=public&per_page=5",
            Reply::json(&fixture("gitlab_projects.json")),
        ),
        ("/gl/projects/fdroid%2Ffdroidclient/releases?per_page=6", Reply::json(&fixture("gitlab_releases.json"))),
    ]);
    let source = GitLab::with_api(Arc::new(client(&server, Some("github-token"))), &server.url("/gl"));
    let hits = source.search("inkscape", 5).unwrap();
    assert_eq!(hits[0].id, AppId::new(SourceKind::GitLab, "inkscape/inkscape"));
    assert_eq!((hits[0].name.as_str(), hits[0].popularity), ("inkscape", Some(4029)));
    assert!(hits[0].icon_url.as_deref().unwrap().starts_with("https://gitlab.com/"));
    assert!(source.search("ab", 5).unwrap().is_empty(), "GitLab refuses searches under three letters");

    let releases = source.releases("fdroid/fdroidclient").unwrap();
    assert_eq!(releases[0].version, "1.23.2");
    let names: Vec<&str> = releases[0].assets.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"org.fdroid.fdroid_1023052.apk"), "a link's file name is what gets classified: {names:?}");
    assert!(matches!(source.releases("../x"), Err(Error::NotFound(_))));
    for (path, headers) in server.requests() {
        assert!(!headers.contains_key("authorization"), "the GitHub token must not go to GitLab ({path})");
    }
}

#[test]
fn a_homebrew_cask_becomes_a_checked_download_for_this_mac() {
    use bilby_core::source::homebrew::Homebrew;
    let server = Server::start(vec![
        ("/brew/analytics/cask-install/30d.json", Reply::json(&fixture("brew_analytics.json"))),
        ("/brew/cask/firefox.json", Reply::json(&fixture("brew_cask.json"))),
    ]);
    let mac = Platform::preset("macos-arm64").unwrap();
    let source = Homebrew::with_api(Arc::new(client(&server, None)), &server.url("/brew"), &mac);
    assert!(source.available_on(&mac) && !source.available_on(&Platform::preset("windows-x64").unwrap()));

    // Names are matched against the list of installs; details come from the cask itself.
    let hits = source.search("firefox", 1).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, AppId::new(SourceKind::Homebrew, "firefox"));
    assert_eq!((hits[0].name.as_str(), hits[0].summary.as_str()), ("Mozilla Firefox", "Web browser"));
    assert!(hits[0].popularity.unwrap() > 1000, "installs in the last thirty days");

    let release = &source.releases("firefox").unwrap()[0];
    let file = &release.assets[0];
    assert!(file.name.ends_with(".dmg") && file.url.starts_with("https://"), "{}", file.name);
    assert!(file.digest.as_deref().unwrap().starts_with("sha256:"), "the cask's checksum is checked after download");
    assert_eq!(file.info.as_ref().unwrap().format, bilby_core::asset::Format::Dmg);
    assert!(matches!(source.summary("../x"), Err(Error::NotFound(_))));
}

#[test]
fn the_snap_store_is_searched_and_installs_are_left_to_snapd() {
    use bilby_core::source::snap::SnapStore;
    let fields = "title,summary,description,publisher,version,media,download,license";
    let find = format!("/snap/find?q=vlc&fields={fields}");
    let info = format!("/snap/info/vlc?fields={fields}");
    let server = Server::start(vec![
        (find.as_str(), Reply::json(&fixture("snap_find.json"))),
        (info.as_str(), Reply::json(&fixture("snap_info.json"))),
    ]);
    let source = SnapStore::with_api(Arc::new(client(&server, None)), &server.url("/snap"));
    assert!(!source.available_on(&Platform::preset("linux-x64-debian").unwrap()), "only where snapd is set up");

    let hits = source.search("vlc", 5).unwrap();
    assert_eq!(hits[0].id, AppId::new(SourceKind::Snap, "vlc"));
    assert_eq!((hits[0].name.as_str(), hits[0].author.as_str()), ("VLC", "VideoLAN"));
    let release = &source.releases("vlc").unwrap()[0];
    assert!(!release.version.is_empty() && release.assets[0].size > 0);
    assert_eq!(release.assets[0].info.as_ref().unwrap().format, bilby_core::asset::Format::SnapStore);
    assert!(source.about("vlc").unwrap().unwrap().starts_with("VLC is"));
}

#[test]
fn f_droid_apps_are_found_and_each_version_is_one_apk() {
    use bilby_core::source::fdroid::FDroid;
    let server = Server::start(vec![
        ("/fs/search_apps?q=newpipe", Reply::json(&fixture("fdroid_search.json"))),
        ("/fd/api/v1/packages/org.schabi.newpipe", Reply::json(&fixture("fdroid_package.json"))),
    ]);
    let source = FDroid::with_api(Arc::new(client(&server, None)), &server.url("/fs"), &server.url("/fd"));
    assert!(source.available_on(&Platform::preset("android-arm64").unwrap()));
    assert!(!source.available_on(&Platform::preset("linux-x64-debian").unwrap()));

    let hits = source.search("newpipe", 3).unwrap();
    assert_eq!(hits[0].id, AppId::new(SourceKind::FDroid, "org.schabi.newpipe"));
    assert_eq!(hits[0].name, "NewPipe");
    let releases = source.releases("org.schabi.newpipe").unwrap();
    assert_eq!(releases[0].version, "0.29.3", "the version F-Droid suggests comes first");
    assert!(releases[0].assets[0].url.ends_with("/repo/org.schabi.newpipe_1017.apk"));
    assert_eq!(releases.len(), 2, "the same version listed twice is offered once");
}

// ---- other stores, handed over to ----

#[test]
fn steam_apps_are_found_and_handed_to_steam() {
    use bilby_core::source::steam::Steam;
    let server = Server::start(vec![
        ("/st/storesearch/?term=portal&l=english&cc=US", Reply::json(&fixture("steam_search.json"))),
        ("/st/appdetails?appids=620&l=english&cc=US", Reply::json(&fixture("steam_app.json"))),
    ]);
    let http = || Arc::new(client(&server, Some("github-token")));
    let linux = Platform::preset("linux-x64-debian").unwrap();
    let mac = Platform::preset("macos-arm64").unwrap();
    let with_steam = Steam::with_api(http(), &server.url("/st"), &linux, vec![], true);

    // On Linux, Windows-only games are offered too (Steam runs them through Proton).
    let hits = with_steam.search("portal", 10).unwrap();
    assert_eq!(hits.iter().map(|h| h.name.as_str()).collect::<Vec<_>>()[..3], ["Portal 2", "Portal", "Portal Knights"]);
    assert_eq!(hits[0].id, AppId::new(SourceKind::Steam, "620"));
    assert_eq!(hits[0].price.as_deref(), Some("$9.99"));
    // On a Mac, only what Steam has for the Mac.
    let on_mac = Steam::with_api(http(), &server.url("/st"), &mac, vec![], true).search("portal", 10).unwrap();
    assert!(
        on_mac.iter().all(|h| h.name.starts_with("Bridge Constructor")),
        "{:?}",
        on_mac.iter().map(|h| &h.name).collect::<Vec<_>>()
    );

    let app = with_steam.summary("620").unwrap();
    assert_eq!((app.name.as_str(), app.author.as_str(), app.price.as_deref()), ("Portal 2", "Valve", Some("$9.99")));
    assert!(app.popularity.unwrap() > 100_000 && !app.summary.contains("&quot;"), "{}", app.summary);
    let release = &with_steam.releases("620").unwrap()[0];
    let links: Vec<&str> = release.assets.iter().map(|a| a.url.as_str()).collect();
    assert_eq!(links, ["steam://install/620", "steam://store/620"], "install first, its store page second");
    assert_eq!(app.warning, None, "Portal 2 has a Linux build and Steam is here: nothing to warn about");
    assert_eq!(with_steam.pictures("620").len(), 2);

    // Without Steam on the machine there is nothing to hand over to: the web page says how to get it.
    let without = Steam::with_api(http(), &server.url("/st"), &linux, vec![], false);
    let release = &without.releases("620").unwrap()[0];
    assert_eq!(release.assets[0].url, "https://store.steampowered.com/app/620");
    assert!(without.summary("620").unwrap().warning.unwrap().contains("Steam is not installed"));

    assert!(!with_steam.available_on(&Platform::preset("android-arm64").unwrap()));
    assert!(matches!(with_steam.summary("620x"), Err(Error::NotFound(_))));
    for (path, headers) in server.requests() {
        assert!(!headers.contains_key("authorization"), "the GitHub token must not go to Steam ({path})");
    }
}

#[test]
fn mac_app_store_apps_are_found_and_handed_to_the_app_store() {
    use bilby_core::source::appstore::MacAppStore;
    let fixture_text = fixture("appstore_search.json");
    let server = Server::start(vec![
        ("/as/search?term=pixelmator&entity=macSoftware&country=us&limit=5", Reply::json(&fixture_text)),
        ("/as/lookup?id=6746662575&entity=macSoftware&country=us", Reply::json(&fixture_text)),
    ]);
    let source = MacAppStore::with_api(Arc::new(client(&server, None)), &server.url("/as"));
    assert!(source.available_on(&Platform::preset("macos-arm64").unwrap()));
    assert!(!source.available_on(&Platform::preset("windows-x64").unwrap()));

    let hits = source.search("pixelmator", 5).unwrap();
    assert_eq!(hits[0].id, AppId::new(SourceKind::MacAppStore, "6746662575"));
    assert_eq!(hits[0].price.as_deref(), Some("Free"));
    assert_eq!(hits[1].price.as_deref(), Some("$49.99"));
    let rating = hits[0].rating.unwrap();
    assert!(rating.stars > 4.0 && rating.count > 100);
    assert!(hits[0].icon_url.as_deref().unwrap().contains(".mzstatic.com/"));

    let release = &source.releases("6746662575").unwrap()[0];
    assert_eq!(release.assets[0].url, "macappstore://apps.apple.com/app/id6746662575");
    assert!(release.assets[0].size > 0 && release.published.len() == 10);
}
