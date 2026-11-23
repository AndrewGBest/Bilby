//! Writing a review, and where it can go.
//!
//! One place takes a review straight from the store: the Open Desktop Ratings
//! Service, which GNOME Software and KDE Discover read too. It asks for no account,
//! only a made-up id that stays the same for one person, so the same person cannot
//! review an app twice.
//!
//! The big stores take reviews only on their own pages, from someone signed in there,
//! and offer no way to send one from outside. For those the store opens the page with
//! the box to write in, having copied what the person wrote so it can be pasted.
//!
//! The service's rules (which fields, how long) were read from its source
//! (<https://github.com/GNOME/odrs-web>, `odrs/views_api.py`), and its refusal of a
//! request with fields missing was seen from the live service. No review was sent to
//! it while writing this: a made-up review would sit on a real app's page.

use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::{valid_key, AppId, SourceKind};
use crate::platform::{Os, Platform};

pub const ODRS_SUBMIT: &str = "https://odrs.gnome.org/1.0/reviews/api/submit";

/// The id of the place that takes reviews from here.
pub const OPEN_SERVICE: &str = "odrs";

/// The service's own limits.
pub const TITLE_LIMIT: usize = 70;
pub const TEXT_LIMIT: usize = 3000;

/// Somewhere a review of one app can be left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// [`OPEN_SERVICE`], or the store's short name (`steam`).
    pub id: String,
    /// What to call it.
    pub name: String,
    /// Sent from here; otherwise the store's own page is opened.
    pub from_here: bool,
    /// From here: the id the service knows the app by. Otherwise the page to open.
    pub target: String,
    /// What happens, and who the store takes reviews from.
    pub note: String,
}

/// The open service, for an app it can know.
pub fn open_place(appstream_id: &str) -> Place {
    Place {
        id: OPEN_SERVICE.to_string(),
        name: "Linux app stores".to_string(),
        from_here: true,
        target: appstream_id.to_string(),
        note: "Sent from here. It shows in Bilby, GNOME Software and KDE Discover.".to_string(),
    }
}

/// The page where a store takes a review of the app, when it has one that can be
/// opened on this device. `page` is the app's own page at the store, for the stores
/// whose pages go by a name instead of the id.
pub fn store_place(id: &AppId, page: &str, platform: &Platform) -> Option<Place> {
    if !valid_key(id.source, &id.key) {
        return None;
    }
    let (target, note) = match id.source {
        SourceKind::Steam => (
            format!("https://store.steampowered.com/app/{}/#review_create", id.key),
            "Opens Steam's page with your words copied, ready to paste. Steam takes reviews from people who own the game.",
        ),
        SourceKind::Gog => {
            let slug =
                page.strip_prefix("https://www.gog.com/en/game/").filter(|slug| crate::source::gog::slug_ok(slug))?;
            (
                format!("https://www.gog.com/en/game/{slug}#reviews"),
                "Opens GOG's page with your words copied, ready to paste. GOG takes reviews from people who own the game.",
            )
        }
        // Only the Store app on Windows has a box to write in.
        SourceKind::MicrosoftStore if platform.os == Os::Windows => (
            format!("ms-windows-store://review/?ProductId={}", id.key),
            "Opens the Microsoft Store with your words copied, ready to paste. It takes reviews from people who have the app.",
        ),
        // Likewise Apple's, on its own devices.
        SourceKind::MacAppStore if platform.os == Os::MacOs => (
            format!("https://apps.apple.com/app/id{}?action=write-review", id.key),
            "Opens the App Store with your words copied, ready to paste. It takes reviews from people who have the app.",
        ),
        SourceKind::GooglePlay => (
            format!("https://play.google.com/store/apps/details?id={}", id.key),
            "Opens Google Play's page with your words copied, ready to paste. It takes reviews from people who have installed the app.",
        ),
        _ => return None,
    };
    Some(Place {
        id: id.source.prefix().to_string(),
        name: id.source.label().to_string(),
        from_here: false,
        target,
        note: note.to_string(),
    })
}

/// May this address be opened to leave a review? Only the pages [`store_place`] makes.
pub fn link_ok(url: &str) -> bool {
    let number = |rest: &str| (1..=12).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_digit());
    url.strip_prefix("https://store.steampowered.com/app/")
        .and_then(|rest| rest.strip_suffix("/#review_create"))
        .is_some_and(number)
        || url
            .strip_prefix("https://www.gog.com/en/game/")
            .and_then(|rest| rest.strip_suffix("#reviews"))
            .is_some_and(crate::source::gog::slug_ok)
        || url
            .strip_prefix("ms-windows-store://review/?ProductId=")
            .is_some_and(|rest| valid_key(SourceKind::MicrosoftStore, rest))
        || url
            .strip_prefix("https://apps.apple.com/app/id")
            .and_then(|rest| rest.strip_suffix("?action=write-review"))
            .is_some_and(number)
        || url
            .strip_prefix("https://play.google.com/store/apps/details?id=")
            .is_some_and(|rest| valid_key(SourceKind::GooglePlay, rest))
}

/// What someone wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    /// 1 to 5.
    pub stars: u8,
    pub title: String,
    pub text: String,
    /// The name shown next to it.
    pub name: String,
}

impl Draft {
    /// The words, as they are copied for a store's own box.
    pub fn words(&self) -> String {
        match (self.title.trim(), self.text.trim()) {
            ("", text) => text.to_string(),
            (title, "") => title.to_string(),
            (title, text) => format!("{title}\n\n{text}"),
        }
    }

    /// Why this cannot be sent anywhere yet, in words for the person writing it.
    pub fn problem(&self) -> Option<&'static str> {
        if !(1..=5).contains(&self.stars) {
            return Some("Choose how many stars first.");
        }
        if self.text.trim().chars().count() < 15 {
            return Some("Write a little more: a sentence or two about what you think of it.");
        }
        None
    }

    /// Why the open service would refuse it.
    pub fn problem_from_here(&self) -> Option<&'static str> {
        let (title, text) = (self.title.trim(), self.text.trim());
        if title.chars().count() < 3 {
            return Some("Give your review a short title.");
        }
        if title.chars().count() > TITLE_LIMIT {
            return Some("The title is too long: keep it to 70 characters.");
        }
        if text.chars().count() > TEXT_LIMIT {
            return Some("The review is too long: keep it to 3,000 characters.");
        }
        if [title, text].iter().any(|part| part.contains('<') || part.contains('>')) {
            return Some("A review cannot contain angle brackets.");
        }
        if self.name.trim().is_empty() {
            return Some("Say what name to show next to your review.");
        }
        None
    }
}

/// Who is writing, as far as the service is told.
pub struct Writer<'a> {
    /// Forty letters and digits made up once on this device. It tells the service
    /// that two reviews are from the same person and nothing about who that is.
    pub id: &'a str,
    /// `en`, `pt_BR`.
    pub locale: &'a str,
    /// The system's name, as other stores send it (`Fedora Linux`).
    pub system: &'a str,
    /// The version of the app the review is about, when known.
    pub version: &'a str,
}

/// A new id for [`Writer::id`]: forty hexadecimal digits from the system's randomness.
pub fn new_writer_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    // Each `RandomState` is keyed from the system's random source.
    let mut id = String::new();
    while id.len() < 40 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(id.len() as u64);
        id.push_str(&format!("{:016x}", hasher.finish()));
    }
    id.truncate(40);
    id
}

pub fn writer_id_ok(id: &str) -> bool {
    id.len() == 40 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Send a review to the open service. An error carries the service's own reason
/// where it gave one ("already reviewed this app").
pub fn send(http: &dyn Http, appstream_id: &str, draft: &Draft, writer: &Writer) -> Result<()> {
    if let Some(problem) = draft.problem().or_else(|| draft.problem_from_here()) {
        return Err(Error::Unsupported(problem.to_string()));
    }
    if !writer_id_ok(writer.id) || appstream_id.trim().is_empty() {
        return Err(Error::Unsupported("This review cannot be sent from here.".into()));
    }
    let or_unknown = |text: &str| if text.trim().is_empty() { "unknown".to_string() } else { text.trim().to_string() };
    let body = serde_json::json!({
        "app_id": appstream_id.trim(),
        "locale": or_unknown(writer.locale),
        "distro": or_unknown(writer.system),
        "version": or_unknown(writer.version),
        "user_hash": writer.id,
        "user_display": draft.name.trim(),
        "summary": draft.title.trim(),
        "description": draft.text.trim(),
        // The service counts in hundredths: five stars is 100.
        "rating": u32::from(draft.stars) * 20,
    });
    let answer = http.submit_json(ODRS_SUBMIT, &body.to_string())?;
    let said: serde_json::Value = serde_json::from_str(&answer)
        .map_err(|_| Error::Parse("The ratings service's answer could not be read.".into()))?;
    if said.get("success").and_then(|ok| ok.as_bool()) == Some(true) {
        return Ok(());
    }
    let why = said.get("msg").and_then(|msg| msg.as_str()).unwrap_or_default();
    Err(Error::Unsupported(match why {
        "already reviewed this app" => "You have already reviewed this app there.".to_string(),
        "account has been disabled due to abuse" => {
            "The ratings service no longer takes reviews from this device.".to_string()
        }
        "" => "The ratings service did not take the review.".to_string(),
        other => format!("The ratings service did not take the review: {other}"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    struct Service {
        answer: &'static str,
        got: Mutex<Vec<String>>,
    }

    impl Http for Service {
        fn get(&self, url: &str) -> Result<String> {
            Err(Error::NotFound(url.to_string()))
        }
        fn download(&self, url: &str, _: &Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
            Err(Error::NotFound(url.to_string()))
        }
        fn submit_json(&self, url: &str, body: &str) -> Result<String> {
            assert_eq!(url, ODRS_SUBMIT);
            self.got.lock().unwrap().push(body.to_string());
            Ok(self.answer.to_string())
        }
    }

    fn draft() -> Draft {
        Draft {
            stars: 4,
            title: " Does the job ".into(),
            text: "Starts fast and stays out of the way.".into(),
            name: "Ana".into(),
        }
    }

    fn writer(id: &str) -> Writer<'_> {
        Writer { id, locale: "en", system: "", version: "2.1" }
    }

    #[test]
    fn a_review_is_sent_with_everything_the_service_asks_for() {
        let service = Service { answer: r#"{"success": true, "msg": "added"}"#, got: Mutex::new(Vec::new()) };
        let id = new_writer_id();
        assert!(writer_id_ok(&id) && id != new_writer_id());
        send(&service, "org.example.App", &draft(), &writer(&id)).unwrap();
        let sent: serde_json::Value = serde_json::from_str(&service.got.lock().unwrap()[0]).unwrap();
        // The fields the service refuses a review without.
        for field in
            ["app_id", "locale", "summary", "description", "user_hash", "version", "distro", "rating", "user_display"]
        {
            assert!(sent.get(field).is_some_and(|v| v.as_str() != Some("")), "{field}");
        }
        assert_eq!(sent["rating"], 80);
        assert_eq!(sent["summary"], "Does the job");
        assert_eq!(sent["distro"], "unknown");
        assert_eq!(sent["user_hash"].as_str().unwrap().len(), 40);
    }

    #[test]
    fn a_refusal_is_told_in_the_services_words_and_nothing_unfinished_is_sent() {
        let service = Service {
            answer: r#"{"success": false, "msg": "already reviewed this app"}"#,
            got: Mutex::new(Vec::new()),
        };
        let id = new_writer_id();
        let said = send(&service, "org.example.App", &draft(), &writer(&id)).unwrap_err().to_string();
        assert!(said.contains("already reviewed"), "{said}");

        let quiet = Service { answer: "{}", got: Mutex::new(Vec::new()) };
        for unfinished in [
            Draft { stars: 0, ..draft() },
            Draft { text: "Good".into(), ..draft() },
            Draft { title: "".into(), ..draft() },
            Draft { title: "x".repeat(71), ..draft() },
            Draft { text: "I <b>love</b> this app so much".into(), ..draft() },
            Draft { name: " ".into(), ..draft() },
        ] {
            assert!(send(&quiet, "org.example.App", &unfinished, &writer(&id)).is_err());
        }
        assert!(send(&quiet, "org.example.App", &draft(), &writer("someone")).is_err(), "no made-up id");
        assert!(quiet.got.lock().unwrap().is_empty());
    }

    #[test]
    fn each_store_is_opened_on_the_page_with_its_review_box() {
        let linux = Platform::preset("linux-x64-arch").unwrap();
        let windows = Platform::preset("windows-x64").unwrap();
        let mac = Platform::preset("macos-arm64").unwrap();
        let place = |id: &str, page: &str, platform: &Platform| store_place(&AppId::parse(id).unwrap(), page, platform);

        let steam = place("steam:620", "", &linux).unwrap();
        assert_eq!(steam.target, "https://store.steampowered.com/app/620/#review_create");
        assert!(!steam.from_here && steam.id == "steam" && steam.note.contains("own the game"));
        let gog = place("gog:1207658924", "https://www.gog.com/en/game/the_witcher_3_wild_hunt", &linux).unwrap();
        assert_eq!(gog.target, "https://www.gog.com/en/game/the_witcher_3_wild_hunt#reviews");
        assert!(place("gog:1207658924", "https://www.gog.com/", &linux).is_none(), "no page, nowhere to send them");
        assert_eq!(
            place("play:org.mozilla.firefox", "", &linux).unwrap().target,
            "https://play.google.com/store/apps/details?id=org.mozilla.firefox"
        );
        // A store whose box only exists in its own app is offered only where that app is.
        assert!(place("msstore:9NBLGGH4NNS1", "", &linux).is_none());
        assert_eq!(
            place("msstore:9NBLGGH4NNS1", "", &windows).unwrap().target,
            "ms-windows-store://review/?ProductId=9NBLGGH4NNS1"
        );
        assert!(place("mas:497799835", "", &linux).is_none());
        assert_eq!(
            place("mas:497799835", "", &mac).unwrap().target,
            "https://apps.apple.com/app/id497799835?action=write-review"
        );
        assert!(place("flathub:org.example.App", "", &linux).is_none());

        for made in [&steam.target, &gog.target, "ms-windows-store://review/?ProductId=9NBLGGH4NNS1"] {
            assert!(link_ok(made), "{made}");
        }
        for other in [
            "https://store.steampowered.com/app/620/",
            "https://example.org/#review_create",
            "ms-windows-store://review/?ProductId=../x",
            "file:///etc/passwd",
        ] {
            assert!(!link_ok(other), "{other}");
        }
    }

    #[test]
    fn the_words_copied_for_a_store_are_the_title_and_the_text() {
        assert_eq!(draft().words(), "Does the job\n\nStarts fast and stays out of the way.");
        assert_eq!(Draft { title: String::new(), ..draft() }.words(), "Starts fast and stays out of the way.");
        // A store's own box needs no title or name from here.
        assert!(Draft { title: String::new(), name: String::new(), ..draft() }.problem().is_none());
    }
}
