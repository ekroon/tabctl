mod cli;

fn main() {
    // Check if first arg is "host"
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "host" {
        // "tabctl host" → run native messaging host
        tabctl_host::run();
    } else {
        // Everything else is CLI
        if let Err(message) = cli::run(std::env::args()) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}
