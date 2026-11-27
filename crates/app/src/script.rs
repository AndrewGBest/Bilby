//! Driving the app without a person: replaying a script, then a screenshot and a
//! report of what is on screen. How the screenshots are made and how the real program
//! is tested.

use crate::{AppWindow, Shell, Store};
use slint::{ComponentHandle, Model, SharedString};
use std::path::PathBuf;

/// Replays scripted input through the window exactly as real key presses and clicks
/// would arrive, one step every 250 ms, then saves a frame and quits. This is how the
/// interfaces are exercised headlessly (CI, documentation screenshots).
pub(crate) fn screenshot_after_script(
    ui: &AppWindow,
    script: Vec<String>,
    settle_ms: u64,
    path: PathBuf,
    report: Option<PathBuf>,
) {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    let settle = std::time::Duration::from_millis(if settle_ms == 0 { 600 } else { settle_ms });
    let weak = ui.as_weak();
    let mut steps = script.into_iter().filter(|s| !s.is_empty());
    let timer = std::rc::Rc::new(slint::Timer::default());
    let timer_handle = timer.clone();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        let Some(ui) = weak.upgrade() else { return };
        let window = ui.window();
        let key = |text: SharedString| {
            window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            window.dispatch_event(WindowEvent::KeyReleased { text });
        };
        match steps.next() {
            Some(step) => match step.as_str() {
                "up" => key(Key::UpArrow.into()),
                "down" => key(Key::DownArrow.into()),
                "left" => key(Key::LeftArrow.into()),
                "right" => key(Key::RightArrow.into()),
                "ok" => key(Key::Return.into()),
                "back" => key(Key::Escape.into()),
                "tab" => key(Key::Tab.into()),
                "wait" => {}
                other => {
                    if let Some((x, y)) = other.strip_prefix("click:").and_then(|p| p.split_once(',')) {
                        let position = slint::LogicalPosition::new(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                        window.dispatch_event(WindowEvent::PointerMoved { position });
                        window
                            .dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
                        window.dispatch_event(WindowEvent::PointerReleased {
                            position,
                            button: PointerEventButton::Left,
                        });
                    } else if let Some(id) = other.strip_prefix("update:") {
                        ui.global::<Store>().invoke_update_app(id.into());
                    } else if let Some(index) = other.strip_prefix("install:") {
                        // The same callback the Install button calls; independent of layout.
                        ui.global::<Store>().invoke_install(index.parse().unwrap_or(0));
                    } else if let Some(text) = other.strip_prefix("text:") {
                        for c in text.chars() {
                            key(c.to_string().into());
                        }
                    } else {
                        eprintln!("unknown script step \"{other}\"");
                    }
                }
            },
            None => {
                timer_handle.stop();
                let weak = ui.as_weak();
                let path = path.clone();
                let report = report.clone();
                slint::Timer::single_shot(settle, move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let result = ui.window().take_snapshot().map_err(|e| e.to_string()).and_then(|pixels| {
                        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                        let mut encoder =
                            png::Encoder::new(std::io::BufWriter::new(file), pixels.width(), pixels.height());
                        encoder.set_color(png::ColorType::Rgba);
                        encoder.set_depth(png::BitDepth::Eight);
                        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                        writer.write_image_data(pixels.as_bytes()).map_err(|e| e.to_string())
                    });
                    if let Err(e) = result {
                        eprintln!("screenshot failed: {e}");
                    }
                    if let Some(report) = &report {
                        if let Err(e) = std::fs::write(report, describe(&ui)) {
                            eprintln!("report failed: {e}");
                        }
                    }
                    let _ = slint::quit_event_loop();
                });
            }
        }
    });
}

/// What the interface is showing, as `key=value` lines. Lets a script check the real,
/// threaded application from outside.
pub(crate) fn describe(ui: &AppWindow) -> String {
    let store = ui.global::<Store>();
    let shell = match ui.get_effective() {
        Shell::Desktop => "desktop",
        Shell::Mobile => "mobile",
        Shell::Tv => "tv",
    };
    let first_option = store.get_options().row_data(0).map(|o| o.label.to_string()).unwrap_or_default();
    [
        format!("shell={shell}"),
        format!("page={}", store.get_page()),
        format!("details_open={}", store.get_details_open()),
        format!("details_loading={}", store.get_details_loading()),
        format!("details_error={}", store.get_details_error()),
        format!("app={}", store.get_app().id),
        format!("options={}", store.get_options().row_count()),
        format!("first_option={first_option}"),
        format!("hidden={}", store.get_hidden().row_count()),
        format!("installed_version={}", store.get_installed_version()),
        format!("install_state={}", store.get_install_state()),
        format!("install_message={}", store.get_install_message()),
        format!("results={}", store.get_results().row_count()),
        format!("searching={}", store.get_searching()),
        format!("library={}", store.get_library().row_count()),
        format!("updates={}", store.get_updates_count()),
        format!("updates_waiting={}", store.get_updates_waiting()),
        format!("checking_updates={}", store.get_checking_updates()),
        format!("updating={}", store.get_updating_id()),
        format!("store_update={}", store.get_store_update()),
        format!("system={}", store.get_system_name()),
        format!("system_state={}", store.get_system_state()),
        format!("system_headline={}", store.get_system_headline()),
        format!("system_updates={}", store.get_system_updates().row_count()),
        format!("toast={}", store.get_toast()),
    ]
    .join("\n")
        + "\n"
}
