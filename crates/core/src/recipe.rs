//! Recipes: how to get an app that is on no store, only on its maker's website.
//!
//! A recipe is a small file that says where the maker publishes the app, how to read
//! which version is current, and where the files for each kind of device are. Anyone
//! can write one and share it: they are kept in a public repository
//! (<https://github.com/AndrewGBest/bilby-recipes>), which publishes them all as one
//! list that Bilby fetches. The format is in `docs/RECIPES.md`.
//!
//! A recipe is data and never a program. It can do three things, each of which reads
//! one address: look in a page for a pattern, see where an address redirects to, and
//! pick a value out of a JSON answer. Every address it reads or hands over for
//! download must be https and at one of the hosts the recipe names, so that a recipe
//! which says it is for one maker cannot fetch from somewhere else. [What the maker's
//! own server then redirects a download to is followed: the maker decides that, as it
//! decides what is in the file.]
//!
//! What this file says when a recipe is wrong or no longer works is for whoever
//! writes recipes, at a terminal, in English. `scripts/i18n.py` leaves this file out.

use crate::http::Http;
use serde::{Deserialize, Serialize};

/// The newest form of recipe this version of Bilby reads. A recipe that says it is
/// newer is left out, so the list can move on without breaking older Bilbys.
pub const FORMAT: u32 = 1;

/// Where the shared recipes are published as one list.
pub const SHARED: &str = "https://raw.githubusercontent.com/AndrewGBest/bilby-recipes/published/index.json";

fn first_format() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Recipe {
    #[serde(default = "first_format")]
    pub format: u32,
    /// What the app is called in ids (`recipe:<id>`), and what its file is named.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// Who makes the app.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    pub homepage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The only hosts this recipe may read from or download from. A host stands for
    /// the names under it too: `example.com` covers `cdn.example.com`.
    pub hosts: Vec<String>,
    pub version: Where,
    pub files: Vec<File>,
}

/// One file the maker publishes. Which device it suits is worked out from its name,
/// exactly as for a release on a code host.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct File {
    /// What to call the file, where its address does not end in a telling name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub url: Where,
    /// The file's SHA-256, where the maker publishes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<Where>,
}

/// A value: written out, or looked up.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Where {
    Fixed(String),
    Found(Lookup),
}

/// One look at one address. Exactly one of `page`, `redirect` and `json` is given.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    /// A page to read; `find` picks the value out of its text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// An address that redirects; the value is where it leads, or what `find` picks
    /// out of that address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect: Option<String>,
    /// An address that answers with JSON; `at` names the value in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    /// A regular expression whose first group is the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub find: Option<String>,
    /// Which match to take when the pattern is found more than once: `first` (as when
    /// nothing is said), `last`, or `newest`, the one that is the highest version.
    /// For a page that lists every release, such as a folder of downloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
    /// The way to the value: names and list positions joined by dots, `files.0.url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// What running a recipe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub version: String,
    pub files: Vec<FoundFile>,
    /// Files the recipe lists that could not be found this time, each with the reason.
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundFile {
    pub name: String,
    pub url: String,
    pub sha256: Option<String>,
}

/// The host of an https address, in lower case. `None` for anything else: another
/// scheme, a name and password before the host, a port, or characters that do not
/// belong in an address.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let plain = url.len() <= 2000 && url.chars().all(|c| c.is_ascii_graphic());
    let named = host.contains('.')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !host.is_empty();
    (plain && named).then(|| host.to_ascii_lowercase())
}

/// Is this id one a recipe may have? Lower-case letters, digits and hyphens.
pub fn valid_id(id: &str) -> bool {
    (1..=60).contains(&id.len())
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A version is put into addresses and file names, so only what is safe there counts.
fn valid_version(version: &str) -> bool {
    (1..=60).contains(&version.len())
        && version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

fn with_version(text: &str, version: Option<&str>) -> String {
    match version {
        Some(version) => text.replace("{version}", version),
        None => text.to_string(),
    }
}

/// An address found in a page, made whole against the page's own.
fn whole(base: &str, link: &str) -> String {
    // A page's links are written for HTML, where an ampersand is spelled out.
    let link = link.trim().replace("&amp;", "&");
    if link.starts_with("https://") || link.starts_with("http://") {
        return link;
    }
    if let Some(rest) = link.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let origin_end = base.find("://").map(|at| at + 3).unwrap_or(0);
    let path_start = base[origin_end..].find(['/', '?', '#']).map(|at| origin_end + at).unwrap_or(base.len());
    if link.starts_with('/') {
        return format!("{}{link}", &base[..path_start]);
    }
    // Beside the page: everything up to the last slash of its path.
    let path = base[path_start..].split(['?', '#']).next().unwrap_or("");
    let folder = path.rfind('/').map(|at| &path[..=at]).unwrap_or("/");
    format!("{}{folder}{link}", &base[..path_start])
}

impl Recipe {
    /// May this recipe read or download from that address?
    pub fn allows(&self, url: &str) -> bool {
        host_of(url).is_some_and(|host| {
            self.hosts.iter().any(|allowed| {
                let allowed = allowed.to_ascii_lowercase();
                host == allowed || host.ends_with(&format!(".{allowed}"))
            })
        })
    }

    /// What is wrong with the recipe as written, without fetching anything. `None`
    /// when it can be used.
    pub fn problem(&self) -> Option<String> {
        if self.format == 0 || self.format > FORMAT {
            return Some(format!("it is written in form {}, and this Bilby reads up to form {FORMAT}", self.format));
        }
        if !valid_id(&self.id) {
            return Some("its id must be lower-case letters, digits and hyphens, sixty at most".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 80 {
            return Some("it needs a name of at most eighty characters".into());
        }
        if self.summary.len() > 200 {
            return Some("its summary is longer than two hundred characters".into());
        }
        if self.hosts.is_empty() || self.hosts.len() > 8 {
            return Some("it must name between one and eight hosts".into());
        }
        for host in &self.hosts {
            // A host with no dot would stand for every site under an ending like "com".
            if host_of(&format!("https://{host}/")).is_none() || host.starts_with('.') || host.ends_with('.') {
                return Some(format!("\"{host}\" is not a host name such as example.com"));
            }
        }
        if !self.allows(&self.homepage) {
            return Some("its homepage must be an https address at one of its hosts".into());
        }
        if let Some(icon) = &self.icon {
            if !self.allows(icon) {
                return Some("its icon must be an https address at one of its hosts".into());
            }
        }
        if let Some(problem) = self.value_problem("version", &self.version, false) {
            return Some(problem);
        }
        if let Where::Fixed(version) = &self.version {
            if !valid_version(version) {
                return Some("its version may hold only letters, digits and . - _ +".into());
            }
        }
        if self.files.is_empty() || self.files.len() > 40 {
            return Some("it must list between one and forty files".into());
        }
        for (at, file) in self.files.iter().enumerate() {
            let what = format!("file {}", at + 1);
            if let Some(problem) = self.value_problem(&format!("the address of {what}"), &file.url, true) {
                return Some(problem);
            }
            if let Some(sum) = &file.sha256 {
                if let Some(problem) = self.value_problem(&format!("the checksum of {what}"), sum, false) {
                    return Some(problem);
                }
            }
            if file
                .name
                .as_deref()
                .is_some_and(|name| name.is_empty() || name.len() > 150 || name.contains(['/', '\\']))
            {
                return Some(format!("the name of {what} must be a plain file name"));
            }
        }
        None
    }

    fn value_problem(&self, what: &str, value: &Where, is_address: bool) -> Option<String> {
        let lookup = match value {
            Where::Fixed(text) if is_address && !self.allows(&with_version(text, Some("1"))) => {
                return Some(format!("{what} must be an https address at one of its hosts"));
            }
            Where::Fixed(text) if text.is_empty() => return Some(format!("{what} is empty")),
            Where::Fixed(_) => return None,
            Where::Found(lookup) => lookup,
        };
        let given = [&lookup.page, &lookup.redirect, &lookup.json];
        let Some(address) =
            given.iter().copied().flatten().next().filter(|_| given.iter().copied().flatten().count() == 1)
        else {
            return Some(format!("{what} must give exactly one of \"page\", \"redirect\" and \"json\""));
        };
        if !self.allows(&with_version(address, Some("1"))) {
            return Some(format!("{what} reads {address}, which is not an https address at one of its hosts"));
        }
        if !matches!(lookup.pick.as_deref(), None | Some("first" | "last" | "newest")) {
            return Some(format!("the \"pick\" of {what} must be \"first\", \"last\" or \"newest\""));
        }
        if lookup.pick.is_some() && lookup.find.is_none() {
            return Some(format!("{what} has a \"pick\" and no \"find\" to pick among"));
        }
        if lookup.json.is_some() {
            if lookup.at.as_deref().is_none_or(str::is_empty) || lookup.find.is_some() {
                return Some(format!("{what} reads JSON, so it needs \"at\" and no \"find\""));
            }
            return None;
        }
        if lookup.at.is_some() {
            return Some(format!("{what} has an \"at\", which is only for JSON"));
        }
        match &lookup.find {
            None if lookup.page.is_some() => Some(format!("{what} reads a page, so it needs a \"find\"")),
            None => None,
            // As it will be used: with a version where the pattern asks for one.
            Some(pattern) => match pattern_of(&with_version(pattern, Some("1"))) {
                Ok(_) => None,
                Err(why) => Some(format!("the \"find\" of {what} {why}")),
            },
        }
    }

    /// Follow the recipe: read the current version and the address of every file.
    /// Fails when the version cannot be read or no file at all can be found, and says
    /// which and why in words for whoever mends the recipe: they are printed at a
    /// terminal and are not translated. The store says something plainer.
    pub fn run(&self, http: &dyn Http) -> std::result::Result<Found, String> {
        if let Some(problem) = self.problem() {
            return Err(format!("the recipe cannot be used: {problem}"));
        }
        let version = self.look(http, &self.version, None, false).map_err(|why| format!("version: {why}"))?;
        if !valid_version(&version) {
            return Err(format!("version: what it found, \"{}\", is not a version", short(&version)));
        }
        let mut files = Vec::new();
        let mut missing = Vec::new();
        for (at, file) in self.files.iter().enumerate() {
            match self.file(http, file, &version) {
                Ok(found) => files.push(found),
                Err(why) => missing.push(format!("file {}: {why}", at + 1)),
            }
        }
        if files.is_empty() {
            return Err(format!("files: {}", missing.join("; ")));
        }
        Ok(Found { version, files, missing })
    }

    fn file(&self, http: &dyn Http, file: &File, version: &str) -> std::result::Result<FoundFile, String> {
        let url = self.look(http, &file.url, Some(version), true)?;
        if !self.allows(&url) {
            return Err(format!("it led to {}, which is not one of the recipe's hosts", short(&url)));
        }
        let from_url = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
        let name = file.name.as_deref().map(|name| with_version(name, Some(version))).unwrap_or(from_url);
        if name.is_empty() {
            return Err("its address does not end in a file name, and the recipe gives none".into());
        }
        let sha256 = match &file.sha256 {
            None => None,
            Some(sum) => {
                let hex = self.look(http, sum, Some(version), false)?.to_ascii_lowercase();
                if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("what it found for a checksum, \"{}\", is not a SHA-256", short(&hex)));
                }
                Some(hex)
            }
        };
        Ok(FoundFile { name, url, sha256 })
    }

    /// One value. `is_address`: what is found is a link, to be made whole against the
    /// address it was found at.
    fn look(
        &self,
        http: &dyn Http,
        value: &Where,
        version: Option<&str>,
        is_address: bool,
    ) -> std::result::Result<String, String> {
        let lookup = match value {
            Where::Fixed(text) => return Ok(with_version(text, version)),
            Where::Found(lookup) => lookup,
        };
        let address =
            [&lookup.page, &lookup.redirect, &lookup.json].into_iter().flatten().next().ok_or("nothing to read")?;
        let address = with_version(address, version);
        if !self.allows(&address) {
            return Err(format!("{} is not one of the recipe's hosts", short(&address)));
        }
        let picked = |text: &str| -> std::result::Result<String, String> {
            let pattern = lookup.find.as_deref().ok_or("no pattern")?;
            let pattern = pattern_of(&with_version(pattern, version.map(regex::escape).as_deref()))?;
            let mut matches =
                pattern.captures_iter(text).filter_map(|found| found.get(1)).map(|group| group.as_str().trim());
            let found = match lookup.pick.as_deref() {
                Some("last") => matches.last(),
                // What cannot be read as a version loses to what can.
                Some("newest") => matches.reduce(|best, one| match crate::version::compare(one, best) {
                    Some(std::cmp::Ordering::Greater) => one,
                    None if crate::version::compare(best, best).is_none() => one,
                    _ => best,
                }),
                _ => matches.next(),
            };
            found
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("nothing at {} matches the pattern", short(&address)))
        };
        let value = if lookup.json.is_some() {
            let text = http.get(&address).map_err(|e| e.to_string())?;
            let json: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| format!("{} did not answer with JSON", short(&address)))?;
            let path = lookup.at.as_deref().unwrap_or("");
            let mut here = &json;
            for step in path.split('.') {
                here = match step.parse::<usize>() {
                    Ok(position) if here.is_array() => here.get(position),
                    _ => here.get(step),
                }
                .ok_or_else(|| format!("the answer of {} has no \"{path}\"", short(&address)))?;
            }
            match here {
                serde_json::Value::String(text) => text.trim().to_string(),
                serde_json::Value::Number(number) => number.to_string(),
                _ => return Err(format!("\"{path}\" in the answer of {} is not a text or a number", short(&address))),
            }
        } else if lookup.redirect.is_some() {
            let landed = http.lands_at(&address).map_err(|e| e.to_string())?;
            match lookup.find {
                Some(_) => picked(&landed)?,
                None => landed,
            }
        } else {
            picked(&http.get(&address).map_err(|e| e.to_string())?)?
        };
        if value.is_empty() || value.len() > 2000 {
            return Err(format!("what was found at {} is empty or far too long", short(&address)));
        }
        Ok(if is_address { whole(&address, &value) } else { value })
    }
}

/// A pattern as the recipe wrote it, ready to use, or what is wrong with it.
fn pattern_of(pattern: &str) -> std::result::Result<regex::Regex, String> {
    if pattern.len() > 400 {
        return Err("is longer than four hundred characters".into());
    }
    // The size limit keeps a pattern from growing into something huge when built.
    let built = regex::RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .build()
        .map_err(|_| "is not a regular expression Bilby can read".to_string())?;
    if built.captures_len() < 2 {
        return Err("needs a group in brackets around the part that is wanted".into());
    }
    Ok(built)
}

/// The start of a text, for saying what was found without printing a whole page.
fn short(text: &str) -> String {
    let cut: String = text.chars().take(120).collect();
    if cut.len() < text.len() {
        format!("{cut}…")
    } else {
        cut
    }
}

/// A list of recipes as it is published or bundled.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Shelf {
    #[serde(default = "first_format")]
    pub format: u32,
    #[serde(default)]
    pub recipes: Vec<serde_json::Value>,
}

/// The usable recipes in a published list. One that is broken, or written in a newer
/// form than this Bilby reads, is left out and the rest still count. `None` when the
/// text is not such a list at all.
pub fn shelf(text: &str) -> Option<Vec<Recipe>> {
    let shelf: Shelf = serde_json::from_str(text).ok()?;
    let mut seen = std::collections::HashSet::new();
    Some(
        shelf
            .recipes
            .into_iter()
            .filter_map(|one| serde_json::from_value::<Recipe>(one).ok())
            .filter(|recipe| recipe.problem().is_none() && seen.insert(recipe.id.clone()))
            .collect(),
    )
}

/// The recipes that came with this version of Bilby: the shared list as it stood when
/// it was built, for a first start with no internet.
pub fn bundled() -> Vec<Recipe> {
    shelf(include_str!("../data/recipes.json")).unwrap_or_default()
}

/// One recipe from its file. The file's name must be the recipe's id.
pub fn read_file(path: &std::path::Path) -> std::result::Result<Recipe, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("it could not be read ({e})"))?;
    let recipe: Recipe = serde_json::from_str(&text).map_err(|e| format!("it is not a recipe: {e}"))?;
    if let Some(problem) = recipe.problem() {
        return Err(problem);
    }
    if path.file_stem().and_then(|stem| stem.to_str()) != Some(recipe.id.as_str()) {
        return Err(format!("it should be named {}.json, after its id", recipe.id));
    }
    Ok(recipe)
}

/// The recipes in a folder, by id, with the files that are not usable recipes and why.
pub fn read_folder(folder: &std::path::Path) -> (Vec<Recipe>, Vec<(std::path::PathBuf, String)>) {
    let mut paths: Vec<_> = std::fs::read_dir(folder)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    paths.retain(|path: &std::path::PathBuf| path.extension().is_some_and(|ext| ext == "json") && path.is_file());
    paths.sort();
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    for path in paths {
        match read_file(&path) {
            Ok(recipe) => good.push(recipe),
            Err(why) => bad.push((path, why)),
        }
    }
    (good, bad)
}

/// Recipes as the one list that is published.
pub fn as_shelf(recipes: &[Recipe]) -> String {
    let shelf = Shelf {
        format: FORMAT,
        recipes: recipes.iter().filter_map(|recipe| serde_json::to_value(recipe).ok()).collect(),
    };
    serde_json::to_string_pretty(&shelf).unwrap_or_default() + "\n"
}

#[cfg(test)]
pub(crate) mod tests;
