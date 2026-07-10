fn main() {
    if let Err(error) = sanser_desktop_lib::run() {
        eprintln!("Sanser failed to start: {error}");
        std::process::exit(1);
    }
}
