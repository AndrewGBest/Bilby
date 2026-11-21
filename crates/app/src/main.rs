// On Windows, don't open a console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // The sign-in window is this same program on Windows and macOS, started another way.
    #[cfg(any(windows, target_os = "macos"))]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().is_some_and(|a| a == "--signin") {
            bilby_signin::main_with(&args[1..]);
        }
    }
    let options = match bilby::Options::from_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(2);
        }
    };
    match bilby::run(options) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
