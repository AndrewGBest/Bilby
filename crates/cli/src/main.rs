//! `ustore`: the store without a window. Useful on servers, in scripts, and for checking
//! what the compatibility engine thinks of a release.

use std::path::Path;
use std::process::ExitCode;
use ustore_core::catalog::UpdateStatus;
use ustore_core::install::{self, Dirs};
use ustore_core::model::{human_count, human_size, AppId, SourceKind};
use ustore_core::platform::PRESETS;
use ustore_core::state::{now_unix, InstalledApp, State};
use ustore_core::{Catalog, Platform};

const USAGE: &str = "\
ustore - app store for GitHub Releases and the AUR, filtered to your machine

USAGE:
    ustore [OPTIONS] <COMMAND>

COMMANDS:
    device                 Show what was detected about this machine
    discover               List featured apps that work here
    search <words>         Search all sources; only installable apps are listed
    show <id>              Files this machine can install from an app, and why others are hidden
    install <id> [n]       Download and install option n (default 1) of an app
    library                Apps installed through the store
    updates                Check the library for newer releases
    classify <file name>   Explain how a release file name is interpreted
    doctor                 Check that the sources answer and that every featured app
                           really has something for this machine

    <id> is github:owner/repo or aur:package; a repository URL works too

OPTIONS:
    --as <platform>        Pretend to be another machine (see --list-platforms)
    --demo                 Use the built-in fictional catalog instead of the network
    --list-platforms       Print the platform names accepted by --as

ENVIRONMENT:
    GITHUB_TOKEN           Raises GitHub's API limit from 60 to 5,000 requests per hour
";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut demo = false;
    let mut platform = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--demo" => {
                demo = true;
                args.remove(i);
            }
            "--list-platforms" => {
                println!("{}", PRESETS.join("\n"));
                return ExitCode::SUCCESS;
            }
            "--as" if i + 1 < args.len() => {
                let name = args.remove(i + 1);
                args.remove(i);
                match Platform::preset(&name) {
                    Some(p) => platform = Some(p),
                    None => {
                        eprintln!("unknown platform \"{name}\". Known: {}", PRESETS.join(", "));
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => i += 1,
        }
    }
    let platform = platform.unwrap_or_else(Platform::detect);
    let dirs = Dirs::for_current_user();
    let state_path = dirs.config.join(if demo { "demo-state.json" } else { "state.json" });
    let saved = State::load(&state_path);
    let token = std::env::var("GITHUB_TOKEN").ok().or(Some(saved.settings.github_token.clone()));
    let catalog = if demo { Catalog::demo(platform) } else { Catalog::live(platform, token, dirs.cache.clone()) };
    let mut catalog = catalog;
    catalog.include_prereleases = saved.settings.include_prereleases;

    let result = match args.first().map(String::as_str) {
        Some("device") => {
            device(&catalog);
            Ok(())
        }
        Some("discover") => {
            discover(&catalog);
            Ok(())
        }
        Some("search") if args.len() > 1 => {
            search(&catalog, &args[1..].join(" "));
            Ok(())
        }
        Some("show") if args.len() == 2 => show(&catalog, &args[1]),
        Some("install") if args.len() >= 2 => install_cmd(&catalog, &dirs, &state_path, &args[1], args.get(2)),
        Some("doctor") => doctor(&catalog, demo),
        Some("library") => {
            library(&catalog, &state_path, false);
            Ok(())
        }
        Some("updates") => {
            library(&catalog, &state_path, true);
            Ok(())
        }
        Some("classify") if args.len() == 2 => {
            classify(&catalog, &args[1]);
            Ok(())
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn device(catalog: &Catalog) {
    println!("{}", catalog.platform.title());
    for (k, v) in catalog.platform.facts() {
        println!("  {k:<20} {v}");
    }
    let sources: Vec<_> = catalog.active_sources().iter().map(|s| s.label()).collect();
    println!("  {:<20} {}", "Sources", sources.join(", "));
}

fn discover(catalog: &Catalog) {
    for section in catalog.featured() {
        println!("{}", section.title);
        for app in section.apps {
            println!("  {:<38} {}", app.id.to_string(), app.summary);
        }
        println!();
    }
}

fn search(catalog: &Catalog, query: &str) {
    let outcome = catalog.search(query);
    for app in &outcome.apps {
        let pop = app.popularity.map(human_count).unwrap_or_default();
        println!(
            "{:<34} {:<12} {:<28} {}",
            app.id.to_string(),
            app.version.clone().unwrap_or_default(),
            app.install_label.clone().unwrap_or_default(),
            pop
        );
        if !app.summary.is_empty() {
            println!("    {}", app.summary);
        }
    }
    if outcome.apps.is_empty() {
        println!("Nothing installable on {} matches \"{query}\".", catalog.platform.title());
    }
    if outcome.hidden > 0 {
        println!("({} more matched but have no files for this machine)", outcome.hidden);
    }
    for e in outcome.errors {
        eprintln!("warning: {e}");
    }
}

fn parse_id(id: &str) -> Result<AppId, String> {
    AppId::from_user_input(id).ok_or_else(|| format!("\"{id}\" is not an app id; use github:owner/repo or aur:package"))
}

fn show(catalog: &Catalog, id: &str) -> Result<(), String> {
    let details = catalog.details(&parse_id(id)?).map_err(|e| e.to_string())?;
    let s = &details.summary;
    println!("{} by {}  ({})", s.name, s.author, s.id.source.label());
    if !s.summary.is_empty() {
        println!("{}", s.summary);
    }
    if let Some(w) = &s.warning {
        println!("! {w}");
    }
    let Some(release) = details.release else {
        println!("\n{}", details.unavailable_reason.unwrap_or_default());
        return Ok(());
    };
    if let Some(installed) = &s.installed_version {
        println!("Installed on this system: {installed}");
    }
    if release.published.is_empty() {
        println!("\nVersion {}", release.version);
    } else {
        println!("\nRelease {} ({})", release.version, release.published);
    }
    if let Some(reason) = &details.unavailable_reason {
        println!("{reason}");
    } else {
        println!("Works on {}:", catalog.platform.title());
    }
    for (n, o) in release.options.iter().enumerate() {
        let mark = if n == 0 { "*" } else { " " };
        println!("{mark} {}. {:<46} {:<30} {}", n + 1, o.asset.name, o.label(), human_size(o.asset.size));
        if let Some(note) = &o.verdict.note {
            println!("       note: {note}");
        }
        if let Some(cmd) = &o.plan.command {
            println!("       $ {cmd}");
        }
    }
    if !release.hidden.is_empty() {
        println!("Hidden ({}):", release.hidden.len());
        for h in &release.hidden {
            println!("    {:<46} {}", h.name, h.reason);
        }
    }
    Ok(())
}

fn install_cmd(
    catalog: &Catalog,
    dirs: &Dirs,
    state_path: &Path,
    id: &str,
    choice: Option<&String>,
) -> Result<(), String> {
    let details = catalog.details(&parse_id(id)?).map_err(|e| e.to_string())?;
    let release = details.release.ok_or_else(|| details.unavailable_reason.clone().unwrap_or_default())?;
    let n: usize = match choice {
        Some(c) => c.parse().map_err(|_| format!("\"{c}\" is not an option number"))?,
        None => 1,
    };
    let option = release.options.get(n.wrapping_sub(1)).ok_or_else(|| {
        details.unavailable_reason.clone().unwrap_or_else(|| format!("there is no option {n}; see `ustore show {id}`"))
    })?;
    for step in &option.plan.steps {
        println!("- {step}");
    }
    let http = catalog.http();
    let mut last = 0;
    let outcome = install::execute(
        &option.plan,
        &option.asset,
        http.as_ref(),
        dirs,
        &install::open_with_system,
        &mut |done, total| {
            if let Some(total) = total.filter(|t| *t > 0) {
                let pct = done * 100 / total;
                if pct >= last + 10 {
                    last = pct;
                    eprintln!("  {pct}%");
                }
            }
            true
        },
    )
    .map_err(|e| e.to_string())?;
    println!("{}", outcome.message);
    if option.plan.action != install::Action::ShowCommand {
        let mut state = State::load(state_path);
        let mut entry =
            InstalledApp::new(&details.summary, &release.version, &option.label(), &option.asset.name, now_unix());
        entry.path = outcome.file.as_ref().map(|p| p.display().to_string());
        entry.store_managed = outcome.store_managed;
        state.record_install(entry);
        state.save(state_path).map_err(|e| format!("installed, but the library could not be saved: {e}"))?;
    }
    Ok(())
}

fn library(catalog: &Catalog, state_path: &Path, check: bool) {
    let state = State::load(state_path);
    if state.library.is_empty() {
        println!("Nothing here yet. Apps you install with `ustore install` are listed here.");
        return;
    }
    let statuses = if check {
        let apps: Vec<_> =
            state.library.iter().filter_map(|a| Some((AppId::parse(&a.id)?, a.version.clone()))).collect();
        catalog.check_updates(&apps)
    } else {
        vec![]
    };
    for (i, app) in state.library.iter().enumerate() {
        let status = match statuses.get(i) {
            Some(UpdateStatus::Available { version, .. }) => format!("update available: {version}"),
            Some(UpdateStatus::UpToDate) => "up to date".to_string(),
            Some(UpdateStatus::Unknown(why)) => format!("could not check: {why}"),
            None => String::new(),
        };
        println!("{:<34} {:<14} {:<28} {}", app.id, app.version, app.label, status);
    }
}

/// A self-check against whatever the catalog talks to. Each line is one thing a person
/// would otherwise discover by something not working.
fn doctor(catalog: &Catalog, demo: bool) -> Result<(), String> {
    // Probe with an app the featured list says exists for this machine.
    let first = catalog.featured().into_iter().flat_map(|s| s.apps).find(|a| a.id.source == SourceKind::GitHub);
    let (query, probe) = match &first {
        Some(app) => (app.name.clone(), app.id.clone()),
        None => ("ripgrep".to_string(), AppId::new(SourceKind::GitHub, "BurntSushi/ripgrep")),
    };
    let query = query.as_str();
    let mut problems = 0;
    let mut report = |ok: bool, line: String| {
        println!("{} {line}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            problems += 1;
        }
    };

    println!("Checking as {}{}", catalog.platform.title(), if demo { " (demo catalog)" } else { "" });
    let sources: Vec<_> = catalog.active_sources().iter().map(|s| s.label()).collect();
    report(!sources.is_empty(), format!("sources for this machine: {}", sources.join(", ")));

    let outcome = catalog.search(query);
    for error in &outcome.errors {
        report(false, error.clone());
    }
    report(
        outcome.errors.is_empty(),
        format!(
            "search \"{query}\": {} installable, {} without a file for this machine",
            outcome.apps.len(),
            outcome.hidden
        ),
    );

    match catalog.details(&probe) {
        Ok(details) => match details.release.as_ref().and_then(|r| r.options.first().map(|o| (r, o))) {
            Some((release, option)) => report(
                true,
                format!("{probe} {}: would install {} ({})", release.version, option.asset.name, option.label()),
            ),
            None => report(false, format!("{probe}: {}", details.unavailable_reason.unwrap_or_default())),
        },
        Err(e) => report(false, format!("{probe}: {e}")),
    }

    // The featured list ships inside the app and promises each entry works here.
    // Hold it to that: one request per entry.
    let featured: Vec<_> = catalog.featured().into_iter().flat_map(|s| s.apps).collect();
    let mut seen = std::collections::HashSet::new();
    let mut checked = 0;
    for app in featured.iter().filter(|a| seen.insert(a.id.clone())) {
        checked += 1;
        match catalog.details(&app.id) {
            Ok(details) if details.unavailable_reason.is_none() => {}
            Ok(details) => {
                report(false, format!("featured {}: {}", app.id, details.unavailable_reason.unwrap_or_default()))
            }
            Err(e) => report(false, format!("featured {}: {e}", app.id)),
        }
    }
    report(true, format!("checked {checked} featured apps"));

    if let Some(quota) = catalog.quota() {
        report(quota.remaining > 0, quota.describe(ustore_core::state::now_unix()));
    }
    if problems == 0 {
        println!("\nAll checks passed.");
        Ok(())
    } else {
        Err(format!("{problems} check{} failed", if problems == 1 { "" } else { "s" }))
    }
}

fn classify(catalog: &Catalog, name: &str) {
    match ustore_core::asset::classify(name) {
        None => println!("{name}: not an installable file (checksum, signature, source, ...)"),
        Some(info) => {
            let v = ustore_core::compat::evaluate(&info, &catalog.platform);
            println!("{name}");
            println!("  format   {}", info.format.label());
            println!("  system   {:?}", info.os);
            println!("  cpu      {:?}", info.arch);
            println!(
                "  on {}: {:?}{}",
                catalog.platform.title(),
                v.level,
                v.note.map(|n| format!(" ({n})")).unwrap_or_default()
            );
        }
    }
}
