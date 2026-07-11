#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = sanser_desktop_lib::run() {
        eprintln!("Sanser failed to start: {error}");
        std::process::exit(1);
    }
}
