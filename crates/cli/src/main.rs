//! `bilby-cli`: the store without a window. Useful on servers, in scripts, and for checking
//! what the compatibility engine thinks of a release.

use bilby_core::catalog::UpdateStatus;
use bilby_core::install::{self, Dirs};
use bilby_core::model::{human_count, human_size, AppId, SourceKind};
use bilby_core::platform::PRESETS;
use bilby_core::state::{now_unix, InstalledApp, State};
use bilby_core::{Catalog, Platform};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
bilby-cli - an app store that only shows what runs on your device, without the window

USAGE:
    bilby-cli [OPTIONS] <COMMAND>

COMMANDS:
    device                 Show what was detected about this machine
    discover               List featured apps that work here
    search <words>         Search all sources; only installable apps are listed
    show <id>              Files this machine can install from an app, and why others are hidden
    install <id> [n]       Download and install option n (default 1) of an app
    library                Apps installed through the store
    updates                Check the library for newer releases
    export [file]          Write the library and favourites to a file (no settings, no token)
    import <file>          Add an exported library and favourites to this device
    classify <file name>   Explain how a release file name is interpreted
    doctor                 Check that the sources answer and that every featured app
                           really has something for this machine
    recipe check <file>... Follow recipes against the real websites and say what was found
    recipe share <file>    Check a recipe and print the address that sends it to the shared list
    recipe index <folder> [file]
                           Put a folder of recipes together as the one list Bilby fetches

    <id> is github:owner/repo or aur:package; a repository URL works too

    A recipe says how to get an app from its maker's website (docs/RECIPES.md). One
    in the recipes folder of Bilby's settings folder is used on this device.

OPTIONS:
    --as <platform>        Pretend to be another machine (see --list-platforms)
    --demo                 Use the built-in fictional catalog instead of the network
    --list-platforms       Print the platform names accepted by --as
    -V, --version          Print the version

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
            "-V" | "--version" => {
                println!("bilby-cli {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => i += 1,
        }
    }
    let pretending = platform.is_some();
    let mut platform = platform.unwrap_or_else(Platform::detect);
    let dirs = Dirs::for_current_user();
    let state_path = dirs.config.join(if demo { "demo-state.json" } else { "state.json" });
    let saved = State::load(&state_path);
    // Windows programs, set up to run here through Proton: on the real device only.
    if !pretending && !demo {
        platform.offer_windows(!saved.settings.hide_windows_apps);
    }
    let token = std::env::var("GITHUB_TOKEN").ok().or(Some(saved.settings.github_token.clone()));
    // Prices are those of the country in the settings, else the one the system is set to.
    let country = bilby_core::region::country(&saved.settings.country)
        .or_else(bilby_core::region::from_env)
        .unwrap_or_else(|| "US".to_string());
    let catalog =
        if demo { Catalog::demo(platform) } else { Catalog::live_in(platform, token, dirs.cache.clone(), &country) };
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
        Some("export") if args.len() <= 2 => export(&state_path, args.get(1)),
        Some("import") if args.len() == 2 => import(&state_path, &args[1]),
        Some("library") => {
            library(&catalog, &state_path, false);
            Ok(())
        }
        Some("updates") => {
            library(&catalog, &state_path, true);
            Ok(())
        }
        Some("recipe") if args.len() >= 3 && args[1] == "check" => recipe_check(&catalog, &args[2..]),
        Some("recipe") if args.len() == 3 && args[1] == "share" => recipe_share(&catalog, &args[2]),
        Some("recipe") if (3..=4).contains(&args.len()) && args[1] == "index" => recipe_index(&args[2], args.get(3)),
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
        details
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| format!("there is no option {n}; see `bilby-cli show {id}`"))
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
        // Tests of this program must not open a file manager on whoever runs them.
        &|path: &std::path::Path| match std::env::var_os("BILBY_NO_OPEN") {
            Some(_) => Ok(()),
            None => install::open_with_system(path),
        },
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
    if !matches!(
        option.plan.action,
        install::Action::ShowCommand
            | install::Action::RunCommand
            | install::Action::RunInTerminal
            | install::Action::OpenStore
    ) {
        let mut state = State::load(state_path);
        let mut entry =
            InstalledApp::new(&details.summary, &release.version, &option.label(), &option.asset.name, now_unix());
        entry.path = outcome.file.as_ref().map(|p| p.display().to_string());
        entry.store_managed = outcome.store_managed;
        if let (true, Some(file)) = (outcome.store_managed, outcome.file.as_ref()) {
            if install::add_menu_entry(dirs, &entry.id, &entry.name, file).is_ok() {
                println!("Added to your app menu.");
            }
        }
        state.record_install(entry);
        state.save(state_path).map_err(|e| format!("installed, but the library could not be saved: {e}"))?;
    }
    Ok(())
}

fn library(catalog: &Catalog, state_path: &Path, check: bool) {
    let state = State::load(state_path);
    if state.library.is_empty() {
        println!("Nothing here yet. Apps you install with `bilby-cli install` are listed here.");
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
            Some(UpdateStatus::Available { version, .. }) if *version == app.skipped_version => {
                format!("up to date (skipping {version})")
            }
            Some(UpdateStatus::Available { version, .. }) => format!("update available: {version}"),
            Some(UpdateStatus::UpToDate) => "up to date".to_string(),
            Some(UpdateStatus::Unknown(why)) => format!("could not check: {why}"),
            None => String::new(),
        };
        println!("{:<34} {:<14} {:<28} {}", app.id, app.version, app.label, status);
    }
}

/// Write the library and favourites to a file (or standard output) to move or share.
fn export(state_path: &Path, to: Option<&String>) -> Result<(), String> {
    let text = State::load(state_path).backup_json().map_err(|e| e.to_string())?;
    match to {
        Some(file) => {
            std::fs::write(file, text).map_err(|e| format!("couldn't write {file}: {e}"))?;
            println!("Saved to {file}");
        }
        None => println!("{text}"),
    }
    Ok(())
}

/// Merge an exported file into this device's library and favourites.
fn import(state_path: &Path, file: &str) -> Result<(), String> {
    let mut state = State::load(state_path);
    let text = std::fs::read_to_string(file).map_err(|e| format!("couldn't read {file}: {e}"))?;
    let (apps, favourites) = state.restore(&text).map_err(|e| e.to_string())?;
    state.save(state_path).map_err(|e| e.to_string())?;
    println!(
        "Added {apps} to the library and {favourites} to favourites. Run `bilby-cli updates` to see what is newer."
    );
    Ok(())
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
        report(quota.remaining > 0, quota.describe(bilby_core::state::now_unix()));
    }
    if problems == 0 {
        println!("\nAll checks passed.");
        Ok(())
    } else {
        Err(format!("{problems} check{} failed", if problems == 1 { "" } else { "s" }))
    }
}

fn classify(catalog: &Catalog, name: &str) {
    match bilby_core::asset::classify(name) {
        None => println!("{name}: not an installable file (checksum, signature, source, ...)"),
        Some(info) => {
            let v = bilby_core::compat::evaluate(&info, &catalog.platform);
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

/// Follow each recipe against the real website, and say what it found, asking for
/// each file to see that it is there. Nothing is downloaded or installed. Fails when any recipe, or any file of one, is not found.
fn recipe_check(catalog: &Catalog, files: &[String]) -> Result<(), String> {
    // Asked afresh every time: an answer remembered from before a recipe was mended
    // would say it still fails.
    let http = bilby_core::http::NetHttp::new(None);
    let mut failed = 0;
    for file in files {
        let recipe = match bilby_core::recipe::read_file(Path::new(file)) {
            Ok(recipe) => recipe,
            Err(why) => {
                println!("✗ {file}: {why}");
                failed += 1;
                continue;
            }
        };
        match recipe.run(&http) {
            Err(e) => {
                println!("✗ {} ({file}): {e}", recipe.name);
                failed += 1;
            }
            Ok(found) => {
                // Said after everything was looked at, under one mark for the recipe.
                let mut said = Vec::new();
                let mut wrong = found.missing.len();
                for one in &found.files {
                    let what = match bilby_core::asset::classify(&one.name) {
                        Some(info) => format!("{}, {:?}, {:?}", info.format.label(), info.os, info.arch),
                        None => "not a file Bilby installs".to_string(),
                    };
                    let here = bilby_core::asset::classify(&one.name)
                        .map(|info| format!("{:?}", bilby_core::compat::evaluate(&info, &catalog.platform).level))
                        .unwrap_or_else(|| "-".to_string());
                    said.push(format!("    {}  ({what}; on this device: {here})", one.name));
                    said.push(format!("      {}", one.url));
                    if let Some(sum) = &one.sha256 {
                        said.push(format!("      sha256 {sum}"));
                    }
                    wrong += usize::from(bilby_core::asset::classify(&one.name).is_none());
                    // Is the file really there? The address is asked for and the
                    // answer dropped unread, so nothing is downloaded.
                    if let Err(e) = bilby_core::http::Http::lands_at(&http, &one.url) {
                        said.push(format!("      ✗ the file does not answer: {e}"));
                        wrong += 1;
                    }
                }
                for missing in &found.missing {
                    said.push(format!("    not found: {missing}"));
                }
                println!("{} {} {}", if wrong == 0 { "✓" } else { "✗" }, recipe.name, found.version);
                println!("{}", said.join("\n"));
                failed += usize::from(wrong > 0);
            }
        }
    }
    match failed {
        0 => Ok(()),
        n => Err(format!("{n} of the recipes need mending")),
    }
}

/// Write every recipe in a folder as the one list that is published. A folder with a
/// broken recipe in it makes no list, so that a mistake is noticed where it was made.
fn recipe_index(folder: &str, to: Option<&String>) -> Result<(), String> {
    let (recipes, bad) = bilby_core::recipe::read_folder(Path::new(folder));
    if let Some((path, why)) = bad.first() {
        return Err(format!("{}: {why}", path.display()));
    }
    if recipes.is_empty() {
        return Err(format!("there are no recipes in {folder}"));
    }
    let text = bilby_core::recipe::as_shelf(&recipes);
    match to {
        Some(file) => {
            std::fs::write(file, text).map_err(|e| format!("cannot write {file}: {e}"))?;
            println!("Wrote {} recipes to {file}", recipes.len());
        }
        None => print!("{text}"),
    }
    Ok(())
}

/// Check a recipe, then print the address of the page on GitHub that proposes it for
/// the shared list, with the file already filled in. Nothing is sent from here: the
/// person opens the address and sends it under their own GitHub account.
fn recipe_share(catalog: &Catalog, file: &str) -> Result<(), String> {
    recipe_check(catalog, std::slice::from_ref(&file.to_string()))?;
    let recipe = bilby_core::recipe::read_file(Path::new(file))?;
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    println!();
    println!("It works. To share it, open this address and press \"Propose new file\":");
    println!(
        "https://github.com/AndrewGBest/bilby-recipes/new/main/recipes?filename={}.json&value={}",
        recipe.id,
        bilby_core::http::url_encode(&text)
    );
    Ok(())
}
