use super::*;

pub(super) fn route_command(matches: &ArgMatches) -> Result<RoutedCommand, String> {
    let json = matches.get_flag("json");
    let pretty = !matches.get_flag("no-pretty");
    let profile = matches
        .get_one::<String>("profile")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let progress = matches.get_flag("progress");

    let (action, params) = match matches.subcommand() {
        Some(("ping", _sub)) => ("ping".to_string(), Value::Object(Map::new())),
        Some(("history", sub)) => {
            let mut params = Map::new();
            if let Some(limit) = sub.get_one::<u64>("limit") {
                params.insert("limit".to_string(), Value::from(*limit));
            }
            ("history".to_string(), Value::Object(params))
        }
        Some((command, _sub)) => return Err(format!("Unsupported routed command: {command}")),
        None => return Err("No command provided. Use --help for usage.".to_string()),
    };

    Ok(RoutedCommand {
        action,
        params,
        json,
        pretty,
        progress,
        profile,
    })
}
