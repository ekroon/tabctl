mod cli;
mod launcher;

fn main() {
    // Check if first arg is "host"
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "host" {
        // "tabctl host" → run native messaging host
        // "tabctl host --launcher" → run legacy launcher
        if args.len() > 2 && args[2] == "--launcher" {
            launcher::run();
        } else {
            tabctl_host::run();
        }
    } else {
        // Everything else is CLI
        if let Err(message) = cli::run(std::env::args()) {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}
