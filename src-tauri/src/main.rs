#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `raketo print …` and friends run as a command instead of opening the app.
    if args.first().is_some_and(|a| raketo_client::is_command(a)) {
        #[cfg(windows)]
        attach_console();
        std::process::exit(raketo_client::run(&args));
    }
    raketo_lib::run()
}

/// The app is a GUI program on Windows; borrow the terminal it was started from
/// so command output shows up there.
#[cfg(windows)]
fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
