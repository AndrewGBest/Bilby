//! What the program is started with: the command line, and the stand-ins tests pass
//! in place of the real system.

use crate::{sound, Shell};
use bilby_core::install::{self, Dirs};
use bilby_core::platform::PRESETS;
use bilby_core::state::Settings;
use bilby_core::{Catalog, Error, Platform};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) const USAGE: &str = "\
Bilby - an app store that only shows what runs on your device

OPTIONS:
    --ui <desktop|mobile|tv>   Force an interface (default: detected; desktop and mobile
                               also switch with the window width)
    --as <platform>            Preview the store as another machine (bilby-cli --list-platforms)
    --demo                     Built-in fictional catalog, no network
    --open <id or link>        Start on an app's page (github:owner/repo, aur:package, URL)
    --search <words>           Start with a search
    --page <name>              Start on discover, search, library, settings or system
    --theme <system|light|dark>  Override the saved appearance for this run
    --lang <code>              Override the language for this run (en, es, fr, de, ...)
    --size <WxH>               Initial window size
    --state <file>             Use this file for settings and library instead of the default
    --screenshot <file.png>    Render one frame to a PNG and exit
    --script <steps>           With --screenshot: replay input first. Steps are separated by
                               spaces: up down left right ok back tab wait click:X,Y
                               text:WORD install:N
    --settle <ms>              With --screenshot: wait after the script (default 600)
    --report <file>            With --screenshot: also write the interface's state as text
    -V, --version              Print the version

ENVIRONMENT:
    GITHUB_TOKEN               Raises GitHub's API limit from 60 to 5,000 requests per hour
                               (a token saved in Settings does the same)
";

#[derive(Default)]
pub struct Options {
    pub demo: bool,
    pub platform: Option<Platform>,
    pub shell: Option<Shell>,
    pub open: Option<String>,
    pub search: Option<String>,
    pub page: Option<i32>,
    pub theme: Option<String>,
    /// The language to run in for this once, as a code such as `fr`.
    pub language: Option<String>,
    pub size: Option<(u32, u32)>,
    pub screenshot: Option<PathBuf>,
    /// Input to replay before the screenshot: `down right ok back click:640,300 ...`.
    pub script: Vec<String>,
    /// Milliseconds to wait after the script before the screenshot is taken.
    pub settle_ms: u64,
    /// With a screenshot: also write what the interface is showing, as `key=value` lines.
    pub report: Option<PathBuf>,
    /// Overrides where downloads, cache and settings go (Android passes its app directories).
    pub dirs: Option<Dirs>,
    pub state_file: Option<PathBuf>,
    /// Run catalog work on the calling thread instead of workers. For tests.
    pub inline: bool,
    /// How downloaded files are handed to the system. Defaults to the desktop's own
    /// handler; tests record the call instead of launching installers.
    pub opener: Option<Opener>,
    /// How an installed app is started ([`install::launch`] unless a test says otherwise).
    pub launcher: Option<Opener>,
    /// How another program is started with arguments (Longboard on an address,
    /// Flatpak to run or remove an app): the program and what to pass it.
    pub runner: Option<Runner>,
    /// How the person signs in to Steam on its own page: given a folder for the
    /// browser's data, returns the line the browser handed back, or nothing when the
    /// window was closed. Defaults to the store's sign-in window; tests pass a stand-in.
    pub signin: Option<SignIn>,
    /// Where the sounds of moving about go, instead of the speakers. For tests.
    pub heard: Option<Heard>,
    /// How an address is opened in the person's browser. For tests.
    pub browser: Option<Browser>,
    /// How the person is asked for a folder. For tests.
    pub picker: Option<Picker>,
    /// What the system has of FUSE (looked up when not given). For tests.
    pub fuse: Option<bilby_core::needs::Fuse>,
    /// `rpm-ostree`, on a system that is one read-only image. For tests; looked up
    /// otherwise.
    pub layering: Option<PathBuf>,
    /// How the device is restarted. Tests record it instead.
    pub restart: Option<Restart>,
    /// Shows one of the desktop's notifications: its title and what it says. A real
    /// start uses the system's own; a demo or a test shows none unless given one.
    pub notifier: Option<Notifier>,
    /// The machine whose system details and updates are shown. Defaults to this
    /// computer; the demo and previews of other devices get a made-up one.
    pub host: Option<Arc<dyn bilby_core::os::Host>>,
    /// Whether a game controller is attached (looked up when not given). For tests.
    pub controller: Option<bool>,
    /// Steam's library folders, instead of the usual places. For tests.
    pub steam_libraries: Option<Vec<PathBuf>>,
    /// The file the store itself runs from, when that is one it can put in the app menu
    /// (an AppImage). `run` fills this in; tests pass a stand-in.
    pub self_program: Option<PathBuf>,
    pub catalog_factory: Option<CatalogFactory>,
    /// The app id the store looks at for its own updates. Defaults to its GitHub
    /// repository; the demo catalog has no such project, so tests name a stand-in.
    pub store_app: Option<String>,
}

pub type Opener = Arc<dyn Fn(&std::path::Path) -> bilby_core::Result<()> + Send + Sync>;

pub type SignIn = Arc<dyn Fn(&str, &std::path::Path) -> bilby_core::Result<Option<String>> + Send + Sync>;

/// The answer to "which folder?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Folder(PathBuf),
    Cancelled,
    /// This system has no dialog to ask with.
    NoDialog,
}
pub type Picker = Arc<dyn Fn(&std::path::Path) -> Picked + Send + Sync>;

/// Ask with the system's own folder dialog. The demo and tests never reach this.
pub(crate) fn system_picker(start: &std::path::Path) -> Picked {
    let has_dialog = cfg!(any(target_os = "macos", target_os = "windows"))
        || ["kdialog", "zenity"].iter().any(|program| {
            std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(program).is_file()))
        });
    match install::pick_folder(start) {
        Some(folder) => Picked::Folder(folder),
        None if has_dialog => Picked::Cancelled,
        None => Picked::NoDialog,
    }
}

/// Ask the system to restart, as its own menu would.
pub(crate) fn restart_device() -> bilby_core::Result<()> {
    let ran = std::process::Command::new("systemctl").arg("reboot").status();
    match ran {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(Error::Unsupported("the system refused".into())),
        Err(e) => Err(Error::Unsupported(e.to_string())),
    }
}

pub type Restart = Arc<dyn Fn() -> bilby_core::Result<()> + Send + Sync>;
pub type Notifier = Arc<dyn Fn(&str, &str) + Send + Sync>;
pub type Heard = Arc<dyn Fn(sound::Sound) + Send + Sync>;
pub type Browser = Arc<dyn Fn(&str) -> bilby_core::Result<()> + Send + Sync>;
pub type Runner = Arc<dyn Fn(&std::path::Path, &[String]) -> bilby_core::Result<()> + Send + Sync>;

/// Builds the catalog for a platform and the current settings. The default talks to the
/// real sources (or the demo); tests substitute one that can go offline on demand.
pub type CatalogFactory = Arc<dyn Fn(&Platform, &Settings, &Dirs) -> Arc<Catalog> + Send + Sync>;

impl Options {
    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut o = Options::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"));
            match arg.as_str() {
                "--demo" => o.demo = true,
                "--ui" => {
                    o.shell = Some(match value("--ui")?.as_str() {
                        "desktop" => Shell::Desktop,
                        "mobile" => Shell::Mobile,
                        "tv" => Shell::Tv,
                        other => return Err(format!("unknown interface \"{other}\" (desktop, mobile, tv)")),
                    })
                }
                "--as" => {
                    let name = value("--as")?;
                    o.platform = Some(
                        Platform::preset(&name)
                            .ok_or_else(|| format!("unknown platform \"{name}\". Known: {}", PRESETS.join(", ")))?,
                    );
                }
                "--open" => o.open = Some(value("--open")?),
                "--search" => o.search = Some(value("--search")?),
                "--page" => {
                    o.page = Some(match value("--page")?.as_str() {
                        "discover" => 0,
                        "search" => 1,
                        "library" => 2,
                        "settings" => 3,
                        "system" => 4,
                        other => {
                            return Err(format!(
                                "unknown page \"{other}\" (discover, search, library, settings, system)"
                            ))
                        }
                    })
                }
                "--lang" => o.language = Some(value("--lang")?),
                "--theme" => {
                    let v = value("--theme")?;
                    if !["system", "light", "dark"].contains(&v.as_str()) {
                        return Err(format!("unknown theme \"{v}\" (system, light, dark)"));
                    }
                    o.theme = Some(v);
                }
                "--size" => {
                    let v = value("--size")?;
                    let parsed = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                    o.size = Some(parsed.ok_or_else(|| format!("--size expects WIDTHxHEIGHT, got \"{v}\""))?);
                }
                "--state" => o.state_file = Some(PathBuf::from(value("--state")?)),
                "--screenshot" => o.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--report" => o.report = Some(PathBuf::from(value("--report")?)),
                "--script" => o.script = value("--script")?.split(' ').map(str::to_string).collect(),
                "--settle" => {
                    o.settle_ms = value("--settle")?.parse().map_err(|_| "--settle expects milliseconds".to_string())?
                }
                "-h" | "--help" => return Err(USAGE.to_string()),
                other => return Err(format!("unknown option \"{other}\"\n\n{USAGE}")),
            }
        }
        Ok(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse() {
        let args = ["--demo", "--ui", "tv", "--size", "1920x1080", "--as", "android-tv", "--page", "library"];
        let o = Options::from_args(args.map(String::from).into_iter()).unwrap();
        assert!(o.demo);
        assert_eq!(o.shell, Some(Shell::Tv));
        assert_eq!(o.size, Some((1920, 1080)));
        assert_eq!(o.page, Some(2));
        assert!(Options::from_args(["--ui", "watch"].map(String::from).into_iter()).is_err());
        assert!(Options::from_args(["--theme", "pink"].map(String::from).into_iter()).is_err());
        assert!(Options::from_args(["--page", "nowhere"].map(String::from).into_iter()).is_err());
    }
}
