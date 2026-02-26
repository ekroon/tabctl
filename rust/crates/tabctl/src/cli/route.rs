use super::*;

pub(super) fn route_command(matches: &ArgMatches) -> Result<RoutedCommand, String> {
    let json = matches.get_flag("json");
    let pretty = !matches.get_flag("no-pretty");
    let profile = matches
        .get_one::<String>("profile")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let progress = matches.get_flag("progress");

    let (command, sub) = matches
        .subcommand()
        .ok_or_else(|| "No command provided. Use --help for usage.".to_string())?;

    let action = match command {
        "dedupe" => "analyze".to_string(),
        "groups" | "group" => "group-list".to_string(),
        "list" if sub.get_flag("groups") => "group-list".to_string(),
        name => name.to_string(),
    };

    let mut params = collect_scope_params(sub);
    match command {
        "analyze" | "dedupe" => {
            if command == "dedupe" {
                params.insert("dedupe".to_string(), Value::Bool(true));
                copy_opt_bool(sub, "include-stale", &mut params, "includeStale");
                copy_opt_bool(sub, "confirm", &mut params, "confirmed");
            }
            copy_opt_u64(sub, "stale-days", &mut params, "staleDays");
            copy_opt_bool(sub, "window-title", &mut params, "windowTitle");
            if progress {
                params.insert("progress".to_string(), Value::Bool(true));
            }
        }
        "open" => {
            copy_many_strings(sub, "url", &mut params, "urls");
            copy_opt_string(sub, "color", &mut params, "color");
            copy_opt_i64(sub, "before-tab", &mut params, "beforeTabId");
            copy_opt_i64(sub, "after-tab", &mut params, "afterTabId");
            copy_opt_string(sub, "after-group", &mut params, "afterGroup");
            copy_opt_bool(sub, "new-window", &mut params, "newWindow");
            copy_opt_string(sub, "window-group", &mut params, "windowGroup");
            copy_opt_i64(sub, "window-tab", &mut params, "windowTabId");
            copy_opt_string(sub, "window-url", &mut params, "windowUrl");
            copy_opt_bool(sub, "new-group", &mut params, "newGroup");
            copy_opt_bool(sub, "allow-duplicates", &mut params, "allowDuplicates");
        }
        "group-update" => {
            copy_opt_string(sub, "title", &mut params, "title");
            copy_opt_string(sub, "color", &mut params, "color");
            copy_opt_bool(sub, "collapsed", &mut params, "collapsed");
            copy_opt_bool(sub, "expanded", &mut params, "expanded");
        }
        "group-assign" => {
            copy_opt_bool(sub, "create", &mut params, "create");
            copy_opt_string(sub, "color", &mut params, "color");
            copy_opt_bool(sub, "collapsed", &mut params, "collapsed");
            copy_opt_bool(sub, "expanded", &mut params, "expanded");
        }
        "merge-window" => {
            copy_opt_i64(sub, "from", &mut params, "fromWindowId");
            copy_opt_i64(sub, "to", &mut params, "toWindowId");
            copy_opt_bool(sub, "close-source", &mut params, "closeSource");
            copy_opt_bool(sub, "confirm", &mut params, "confirmed");
        }
        "close" => {
            copy_opt_string(sub, "apply", &mut params, "analysisId");
            copy_opt_bool(sub, "confirm", &mut params, "confirmed");
            copy_opt_bool(sub, "dry-run", &mut params, "dryRun");
        }
        "report" => {
            copy_opt_string(sub, "format", &mut params, "format");
            copy_opt_string(sub, "out", &mut params, "out");
        }
        "undo" => {
            if let Some(txid) = sub.get_one::<String>("txid") {
                params.insert("txid".to_string(), Value::String(txid.to_string()));
            } else if let Some(txid) = sub.get_one::<String>("txid-flag") {
                params.insert("txid".to_string(), Value::String(txid.to_string()));
            }
            copy_opt_bool(sub, "latest", &mut params, "latest");
        }
        "history" => copy_opt_u64(sub, "limit", &mut params, "limit"),
        "inspect" => {
            copy_many_strings(sub, "signal", &mut params, "signals");
            copy_opt_string(sub, "selector-attr", &mut params, "selectorAttr");
            copy_opt_u64(sub, "signal-concurrency", &mut params, "signalConcurrency");
            copy_opt_u64(sub, "signal-timeout-ms", &mut params, "signalTimeoutMs");
            copy_opt_string(sub, "wait-for", &mut params, "waitFor");
            copy_opt_u64(sub, "wait-timeout-ms", &mut params, "waitTimeoutMs");
            if let Some(path) = sub.get_one::<String>("signal-config") {
                let content = fs::read_to_string(path)
                    .map_err(|e| format!("Failed to read signal config {path}: {e}"))?;
                let config: Value = serde_json::from_str(&content)
                    .map_err(|e| format!("Invalid JSON in signal config {path}: {e}"))?;
                params.insert("signalConfig".to_string(), config);
            }
            if let Some(values) = sub.get_many::<String>("selector") {
                let default_attr = sub
                    .get_one::<String>("selector-attr")
                    .map(|s| s.as_str())
                    .unwrap_or("text");
                let specs = parse_selector_args(values, default_attr)?;
                if !specs.is_empty() {
                    params.insert("selectorSpecs".to_string(), Value::Array(specs));
                }
            }
            if progress {
                params.insert("progress".to_string(), Value::Bool(true));
            }
        }
        "screenshot" => {
            copy_opt_string(sub, "mode", &mut params, "mode");
            copy_opt_string(sub, "format", &mut params, "format");
            copy_opt_u64(sub, "quality", &mut params, "quality");
            copy_opt_u64(sub, "tile-max-dim", &mut params, "tileMaxDim");
            copy_opt_u64(sub, "max-bytes", &mut params, "maxBytes");
            copy_opt_string(sub, "wait-for", &mut params, "waitFor");
            copy_opt_u64(sub, "wait-timeout-ms", &mut params, "waitTimeoutMs");
            copy_opt_string(sub, "out", &mut params, "out");
            if progress {
                params.insert("progress".to_string(), Value::Bool(true));
            }
        }
        "move-tab" | "move-group" => {
            copy_opt_i64(sub, "before-tab", &mut params, "beforeTabId");
            copy_opt_i64(sub, "after-tab", &mut params, "afterTabId");
            copy_opt_string(sub, "before-group", &mut params, "beforeGroupTitle");
            copy_opt_string(sub, "after-group", &mut params, "afterGroupTitle");
            copy_opt_bool(sub, "new-window", &mut params, "newWindow");
        }
        _ => {}
    }

    Ok(RoutedCommand {
        action,
        params: Value::Object(params),
        json,
        pretty,
        progress,
        profile,
    })
}

pub(super) fn collect_scope_params(sub: &ArgMatches) -> Map<String, Value> {
    let mut params = Map::new();
    copy_many_i64(sub, "tab", &mut params, "tabIds");
    copy_opt_string(sub, "group", &mut params, "groupTitle");
    copy_opt_i64(sub, "group-id", &mut params, "groupId");
    copy_opt_bool(sub, "ungrouped", &mut params, "ungrouped");
    copy_opt_string(sub, "window", &mut params, "windowId");
    if let Ok(Some(value)) = sub.try_get_one::<bool>("all") {
        if *value {
            params.insert("all".to_string(), Value::Bool(true));
        }
    }
    copy_opt_u64(sub, "limit", &mut params, "limit");
    copy_opt_u64(sub, "offset", &mut params, "offset");
    if let Ok(Some(no_page)) = sub.try_get_one::<bool>("no-page") {
        if *no_page {
            params.insert("page".to_string(), Value::Bool(false));
        }
    }
    params
}

pub(super) fn copy_opt_string(
    sub: &ArgMatches,
    src: &str,
    out: &mut Map<String, Value>,
    key: &str,
) {
    if let Ok(Some(value)) = sub.try_get_one::<String>(src) {
        out.insert(key.to_string(), Value::String(value.to_string()));
    }
}

pub(super) fn copy_opt_i64(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Ok(Some(value)) = sub.try_get_one::<i64>(src) {
        out.insert(key.to_string(), Value::from(*value));
    }
}

pub(super) fn copy_opt_u64(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Ok(Some(value)) = sub.try_get_one::<u64>(src) {
        out.insert(key.to_string(), Value::from(*value));
    }
}

pub(super) fn copy_opt_bool(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Ok(Some(value)) = sub.try_get_one::<bool>(src) {
        if *value {
            out.insert(key.to_string(), Value::Bool(true));
        }
    }
}

pub(super) fn copy_many_strings(
    sub: &ArgMatches,
    src: &str,
    out: &mut Map<String, Value>,
    key: &str,
) {
    if let Ok(Some(values)) = sub.try_get_many::<String>(src) {
        let entries: Vec<Value> = values.map(|v| Value::String(v.to_string())).collect();
        if !entries.is_empty() {
            out.insert(key.to_string(), Value::Array(entries));
        }
    }
}

pub(super) fn copy_many_i64(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Ok(Some(values)) = sub.try_get_many::<String>(src) {
        let mut ids = Vec::new();
        for value in values {
            if let Ok(id) = value.parse::<i64>() {
                ids.push(Value::from(id));
            }
        }
        if !ids.is_empty() {
            out.insert(key.to_string(), Value::Array(ids));
        }
    }
}

/// Parse `--selector` arguments into selectorSpecs array.
/// Accepts two formats:
///   name=css-selector   (shorthand)
///   {"name":"x","selector":".cls","attr":"href",...}  (JSON object)
pub(super) fn parse_selector_args<'a>(
    values: impl Iterator<Item = &'a String>,
    default_attr: &str,
) -> Result<Vec<Value>, String> {
    let mut specs = Vec::new();
    for raw in values {
        let trimmed = raw.trim();
        if trimmed.starts_with('{') {
            let obj: Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("Invalid JSON in --selector '{trimmed}': {e}"))?;
            specs.push(obj);
        } else if let Some(eq_pos) = trimmed.find('=') {
            let name = &trimmed[..eq_pos];
            let selector = &trimmed[eq_pos + 1..];
            if selector.is_empty() {
                return Err(format!("Empty CSS selector in --selector '{trimmed}'"));
            }
            let mut m = Map::new();
            m.insert("name".to_string(), Value::String(name.to_string()));
            m.insert("selector".to_string(), Value::String(selector.to_string()));
            m.insert("attr".to_string(), Value::String(default_attr.to_string()));
            specs.push(Value::Object(m));
        } else {
            return Err(format!(
                "Invalid --selector format '{trimmed}'. Expected name=css or JSON object."
            ));
        }
    }
    Ok(specs)
}
