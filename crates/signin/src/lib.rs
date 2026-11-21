//! A small browser window for signing in to another store on its own page.
//!
//! Steam will tell a signed-in browser which games the account owns, and nobody
//! else without a key that has to be kept secret. So the store shows Steam's own
//! sign-in page in a window, and once the person has signed in there it asks Steam,
//! as that browser, for the list. The password is typed into Steam's page and never
//! passes through this program; what comes back is the account's number and the ids
//! of what it owns, printed as one line for the store to read.
//!
//! The window is deliberately not a browser: it goes to Steam's addresses and
//! nowhere else, opens no other windows, and shows which site it is on in its title.

use std::path::Path;

/// Where the window starts. Signed in already, Steam sends it on to the store's
/// front page, which is where the list is asked for.
pub const STEAM_START: &str = "https://store.steampowered.com/login/?redir=&redir_ssl=1";

/// The one page whose word is taken for who is signed in.
pub const STEAM_STORE_HOST: &str = "store.steampowered.com";

/// Run in Steam's pages. On the store's own pages, once Steam says someone is signed
/// in, it asks Steam for what the account owns and hands the answer to the window.
pub const STEAM_SCRIPT: &str = r#"
(function () {
  if (window.top !== window || location.protocol !== 'https:' || location.hostname !== 'store.steampowered.com') return;
  var sent = false;
  function look() {
    if (sent) return;
    var config = document.getElementById('application_config');
    if (!config) return;
    var user;
    try { user = JSON.parse(config.getAttribute('data-userinfo') || '{}'); } catch (e) { return; }
    if (!user || !user.logged_in || !user.steamid) return;
    sent = true;
    fetch('/dynamicstore/userdata/?t=' + Date.now(), { credentials: 'include', cache: 'no-store' })
      .then(function (answer) { return answer.json(); })
      .then(function (data) {
        window.ipc.postMessage(JSON.stringify({ steamid: String(user.steamid), owned: data.rgOwnedApps || [] }));
      })
      .catch(function () { sent = false; });
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', look);
  else look();
  window.addEventListener('load', look);
})();
"#;

/// The host of an https address, lower-cased. Anything else has none.
pub fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // A name before an @ is a trick to make one address look like another.
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = authority.split(':').next()?.to_ascii_lowercase();
    (!host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')).then_some(host)
}

/// May the sign-in window go to this address? Steam's own sites, and the
/// "are you a robot" checks its sign-in page can show.
pub fn steam_may_visit(url: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    let Some(host) = https_host(url) else { return false };
    let within = |site: &str| host == site || host.ends_with(&format!(".{site}"));
    let path = url.strip_prefix("https://").and_then(|rest| rest.find('/').map(|at| &rest[at..])).unwrap_or("/");
    within("steampowered.com")
        || within("steamcommunity.com")
        || within("hcaptcha.com")
        || host == "www.recaptcha.net"
        || (host == "www.google.com" && path.starts_with("/recaptcha/"))
}

/// What the page handed over, checked: an account number and the ids it owns, as one
/// line of JSON. Nothing else a page says is passed on.
pub fn steam_result(page: &str, message: &str) -> Option<String> {
    if https_host(page).as_deref() != Some(STEAM_STORE_HOST) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(message).ok()?;
    let id = value.get("steamid")?.as_str()?;
    if id.len() < 5 || id.len() > 20 || !id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let owned: Vec<u64> =
        value.get("owned")?.as_array()?.iter().filter_map(|v| v.as_u64()).filter(|id| *id <= u32::MAX as u64).collect();
    Some(serde_json::json!({ "steamid": id, "owned": owned }).to_string())
}

enum Happened {
    Done(String),
    At(String),
}

/// Show Steam's sign-in page and wait. Prints the result and ends the program: with 0
/// when the list was handed over, 3 when the person closed the window, 2 when the
/// window could not be made.
pub fn run_steam(data_dir: &Path) -> ! {
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebViewBuilder};

    let event_loop = EventLoopBuilder::<Happened>::with_user_event().build();
    let window = match WindowBuilder::new()
        .with_title("Sign in to Steam")
        .with_inner_size(LogicalSize::new(520.0, 760.0))
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(why) => {
            eprintln!("could not open a window: {why}");
            std::process::exit(2);
        }
    };

    let mut context = WebContext::new(Some(data_dir.to_path_buf()));
    let done = event_loop.create_proxy();
    let moved = event_loop.create_proxy();
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(STEAM_START)
        .with_initialization_script(STEAM_SCRIPT)
        .with_devtools(false)
        .with_navigation_handler(|url| steam_may_visit(&url))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_on_page_load_handler(move |event, url| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = moved.send_event(Happened::At(url));
            }
        })
        .with_ipc_handler(move |request| {
            if let Some(result) = steam_result(&request.uri().to_string(), request.body()) {
                let _ = done.send_event(Happened::Done(result));
            }
        });

    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "ios", target_os = "android"))]
    let built = builder.build(&window);
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios", target_os = "android")))]
    let built = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        match window.default_vbox() {
            Some(area) => builder.build_gtk(area),
            None => {
                eprintln!("could not open a window: nowhere to put the page");
                std::process::exit(2);
            }
        }
    };
    let _view = match built {
        Ok(view) => view,
        Err(why) => {
            eprintln!("could not show a web page: {why}");
            std::process::exit(2);
        }
    };

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Happened::Done(result)) => {
                println!("{result}");
                std::process::exit(0);
            }
            // There is no address bar, so the title says whose page this is.
            Event::UserEvent(Happened::At(url)) => {
                let host = https_host(&url).unwrap_or_default();
                window.set_title(&if host.is_empty() {
                    "Sign in to Steam".to_string()
                } else {
                    format!("Sign in to Steam  ·  {host}")
                });
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => std::process::exit(3),
            _ => {}
        }
    })
}

/// The command line of the sign-in window: `steam --data-dir <dir>`. Ends the program.
pub fn main_with(args: &[String]) -> ! {
    let dir = args.iter().position(|a| a == "--data-dir").and_then(|at| args.get(at + 1));
    match (args.first().map(String::as_str), dir) {
        (Some("steam"), Some(dir)) => run_steam(Path::new(dir)),
        _ => {
            eprintln!("usage: steam --data-dir <dir>");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_only_goes_to_steam() {
        for ok in [
            STEAM_START,
            "https://store.steampowered.com/",
            "https://login.steampowered.com/jwt/finalizelogin",
            "https://steamcommunity.com/login/settoken",
            "https://help.steampowered.com/en/wizard/HelpWithLogin",
            "https://www.google.com/recaptcha/enterprise/anchor?k=x",
            "about:blank",
        ] {
            assert!(steam_may_visit(ok), "{ok}");
        }
        for no in [
            "http://store.steampowered.com/",
            "https://steampowered.com.example.org/login",
            "https://evilsteampowered.com/",
            "https://store.steampowered.com@example.org/",
            "https://www.google.com/search?q=steam",
            "https://example.org/?https://store.steampowered.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "steam://run/440",
        ] {
            assert!(!steam_may_visit(no), "{no}");
        }
    }

    #[test]
    fn only_the_stores_own_page_is_believed_and_only_for_ids() {
        let page = "https://store.steampowered.com/";
        let said = r#"{"steamid":"76561198000000001","owned":[440,570,"x",-1,99999999999],"cookie":"secret"}"#;
        assert_eq!(steam_result(page, said).unwrap(), r#"{"owned":[440,570],"steamid":"76561198000000001"}"#);
        assert!(steam_result("https://steamcommunity.com/", said).is_none(), "another site's word is not taken");
        assert!(steam_result("https://store.steampowered.com.example.org/", said).is_none());
        assert!(steam_result(page, r#"{"steamid":"abc","owned":[]}"#).is_none());
        assert!(steam_result(page, r#"{"steamid":"76561198000000001"}"#).is_none());
        assert!(steam_result(page, "not json").is_none());
    }
}
