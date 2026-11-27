//! Apps fetched from their makers' websites, each by following a recipe.
//!
//! The recipes come from three places. The shared list is fetched from the public
//! repository where people send theirs (see [`crate::recipe`]). The list bundled with
//! this version of Bilby stands in when that cannot be fetched. And the person's own
//! folder holds recipes they are writing or keep to themselves, which win over a
//! shared one of the same id so that a mended recipe can be tried before it is sent.

use super::Source;
use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::{AppId, AppSummary, Asset, Release, SourceKind};
use crate::platform::Platform;
use crate::recipe::{self, Recipe};
use std::sync::Arc;

pub struct Recipes {
    http: Arc<dyn Http>,
    /// Where the shared list is fetched from. `None` to use only what is bundled.
    shared: Option<String>,
    own: Vec<Recipe>,
}

impl Recipes {
    pub fn new(http: Arc<dyn Http>) -> Recipes {
        Recipes { http, shared: Some(recipe::SHARED.to_string()), own: Vec::new() }
    }

    /// Fetch the shared list from another address, or not at all.
    pub fn shared_at(mut self, address: Option<String>) -> Recipes {
        self.shared = address;
        self
    }

    /// Add the person's own recipes.
    pub fn with_own(mut self, own: Vec<Recipe>) -> Recipes {
        self.own = own;
        self
    }

    /// Every recipe in use: the person's own, then the shared ones they do not replace.
    pub fn all(&self) -> Vec<Recipe> {
        // A list that cannot be fetched, or that is not a list, must not empty the
        // store: what came with Bilby is used instead.
        let shared = self
            .shared
            .as_deref()
            .and_then(|address| self.http.get(address).ok())
            .and_then(|text| recipe::shelf(&text))
            .filter(|list| !list.is_empty())
            .unwrap_or_else(recipe::bundled);
        let mut all = self.own.clone();
        all.extend(shared.into_iter().filter(|one| !self.own.iter().any(|mine| mine.id == one.id)));
        all
    }

    fn find(&self, key: &str) -> Result<Recipe> {
        self.all()
            .into_iter()
            .find(|recipe| recipe.id == key)
            .ok_or_else(|| Error::NotFound(format!("There is no recipe for “{key}” any more.")))
    }

    fn summary_of(&self, recipe: &Recipe) -> AppSummary {
        let mut summary = AppSummary::new(AppId::new(SourceKind::Recipe, recipe.id.clone()), recipe.name.trim());
        summary.summary = recipe.summary.trim().to_string();
        summary.author = recipe.author.trim().to_string();
        summary.homepage = recipe.homepage.clone();
        summary.license = recipe.license.clone();
        summary.category = recipe.category.clone();
        summary.icon_url = recipe.icon.clone();
        // The maker's site, for the app's page and its tile: it is where the files
        // come from, which is what the person is trusting.
        summary.origin = recipe::host_of(&recipe.homepage).map(|host| host.trim_start_matches("www.").to_string());
        summary
    }
}

impl Source for Recipes {
    fn kind(&self) -> SourceKind {
        SourceKind::Recipe
    }

    fn available_on(&self, _platform: &Platform) -> bool {
        true
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<AppSummary>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut hits: Vec<AppSummary> = self
            .all()
            .iter()
            .filter(|recipe| {
                let text = format!("{} {} {} {}", recipe.name, recipe.id, recipe.summary, recipe.author).to_lowercase();
                !words.is_empty() && words.iter().all(|word| text.contains(word.as_str()))
            })
            .map(|recipe| self.summary_of(recipe))
            .collect();
        hits.truncate(limit.max(1));
        Ok(hits)
    }

    fn summary(&self, key: &str) -> Result<AppSummary> {
        Ok(self.summary_of(&self.find(key)?))
    }

    fn releases(&self, key: &str) -> Result<Vec<Release>> {
        let recipe = self.find(key)?;
        // Why it failed is for whoever mends the recipe (`bilby-cli recipe check`
        // prints it); the person is told what it means for them.
        let found = recipe.run(self.http.as_ref()).map_err(|_| {
            Error::Parse(format!(
                "{} could not be found on its maker's website, which may have changed. Its recipe needs mending.",
                recipe.name
            ))
        })?;
        Ok(vec![Release {
            version: found.version,
            // A website seldom says when, and nothing here depends on it.
            published: String::new(),
            prerelease: false,
            downloads: None,
            notes: String::new(),
            assets: found
                .files
                .into_iter()
                .map(|file| Asset {
                    name: file.name,
                    size: 0,
                    url: file.url,
                    digest: file.sha256.map(|hex| format!("sha256:{hex}")),
                    info: None,
                })
                .collect(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::tests::{tide_web, TIDE};

    const SHARED: &str = "https://lists.example/index.json";

    #[test]
    fn apps_with_a_recipe_are_found_and_their_files_come_from_the_makers_site() {
        let mut web = tide_web();
        let list: &'static str = Box::leak(format!(r#"{{ "format": 1, "recipes": [ {TIDE} ] }}"#).into_boxed_str());
        web.pages.insert(SHARED, list);
        let recipes = Recipes::new(Arc::new(web)).shared_at(Some(SHARED.into()));
        let hits = recipes.search("sea charts", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.to_string(), "recipe:tide");
        assert_eq!((hits[0].name.as_str(), hits[0].origin.as_deref()), ("Tide", Some("tide.example")));
        assert!(recipes.search("harbour", 10).unwrap().is_empty());

        let releases = recipes.releases("tide").unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "3.2.1");
        assert_eq!(releases[0].assets.len(), 4);
        assert_eq!(
            releases[0].assets[0].digest.as_deref(),
            Some("sha256:abcdefabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123")
        );
        assert!(recipes.releases("gone").is_err());
    }

    #[test]
    fn with_no_list_to_fetch_the_bundled_one_is_used_and_the_persons_own_recipe_wins() {
        // Nothing answers: the list that came with Bilby stands in.
        let offline = Recipes::new(Arc::new(tide_web())).shared_at(Some(SHARED.into()));
        let bundled = crate::recipe::bundled();
        assert_eq!(offline.all().len(), bundled.len());

        // The person's own recipe replaces the shared one of the same id.
        let mut mine = bundled[0].clone();
        mine.name = "My mended one".into();
        let both = Recipes::new(Arc::new(tide_web())).shared_at(None).with_own(vec![mine.clone()]);
        let all = both.all();
        assert_eq!(all.len(), bundled.len());
        assert_eq!(all.iter().find(|one| one.id == mine.id).map(|one| one.name.as_str()), Some("My mended one"));
    }
}
