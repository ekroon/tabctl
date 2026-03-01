use super::*;

pub(super) fn render_local_command(
    matches: &ArgMatches,
    _action: &str,
    data: Value,
) -> Result<(), String> {
    if matches.get_flag("json") {
        if !matches.get_flag("no-pretty") {
            println!(
                "{}",
                serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(&data).map_err(|e| e.to_string())?
            );
        }
        return Ok(());
    }

    if !matches.get_flag("no-pretty") {
        println!(
            "{}",
            serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "{}",
            serde_json::to_string(&data).map_err(|e| e.to_string())?
        );
    }
    Ok(())
}

pub(super) fn render_ping_human(response: &ResponseEnvelope) -> Result<(), String> {
    if !response.ok {
        if let Some(error) = &response.error {
            eprintln!("error: {}", error.message);
            if let Some(hint) = &error.hint {
                eprintln!("hint: {hint}");
            }
        } else {
            eprintln!("error: ping failed");
        }
        return Err("request failed".to_string());
    }

    let data = response
        .data
        .as_ref()
        .and_then(Value::as_object)
        .ok_or("missing ping data")?;

    let ext_sha = data
        .get("gitSha")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let host_sha = data
        .get("hostGitSha")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let in_sync = data
        .get("versionsInSync")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let host_version = data
        .get("hostBaseVersion")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let ext_version = data
        .get("version")
        .and_then(Value::as_str)
        .map(strip_dev_suffix)
        .unwrap_or("unknown");
    let ext_dirty = data.get("dirty").and_then(Value::as_bool).unwrap_or(false);
    let host_dirty = data
        .get("hostDirty")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let dirty_marker = |d: bool| if d { "+dirty" } else { "" };

    if in_sync {
        println!(
            "\u{2705} tabctl {} (ext {}{}, host {}{}, in sync)",
            host_version,
            ext_sha,
            dirty_marker(ext_dirty),
            host_sha,
            dirty_marker(host_dirty),
        );
    } else {
        println!(
            "\u{26a0}\u{fe0f} tabctl \u{2014} out of sync (ext {}, host {})",
            ext_version, host_version,
        );
    }
    Ok(())
}

fn strip_dev_suffix(version: &str) -> &str {
    if let Some(idx) = version.find("-dev.") {
        &version[..idx]
    } else {
        version
    }
}

pub(super) fn render_response(
    response: &ResponseEnvelope,
    json_mode: bool,
    pretty: bool,
) -> Result<(), String> {
    if json_mode {
        let payload = compact_response_payload(response);
        if pretty {
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|e| e.to_string())?
            );
        }
        if !response.ok {
            return Err("request failed".to_string());
        }
        return Ok(());
    }

    if response.ok {
        if let Some(data) = &response.data {
            if pretty {
                println!(
                    "{}",
                    serde_json::to_string_pretty(data).map_err(|e| e.to_string())?
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string(data).map_err(|e| e.to_string())?
                );
            }
        } else {
            println!("ok");
        }
        return Ok(());
    }

    if let Some(error) = &response.error {
        eprintln!("error: {}", error.message);
        if let Some(hint) = &error.hint {
            eprintln!("hint: {hint}");
        }
    } else {
        eprintln!("error: request failed");
    }
    Err("request failed".to_string())
}

pub(super) fn compact_response_payload(response: &ResponseEnvelope) -> Value {
    if response.ok {
        return response
            .data
            .clone()
            .unwrap_or_else(|| json!({ "ok": true }));
    }
    if let Some(error) = &response.error {
        return json!({
            "ok": false,
            "error": error
        });
    }
    json!({
        "ok": false,
        "error": {
            "message": "request failed"
        }
    })
}
