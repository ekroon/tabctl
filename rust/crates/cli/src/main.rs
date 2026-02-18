fn main() {
    if let Err(message) = tabctl_cli::run(std::env::args()) {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
