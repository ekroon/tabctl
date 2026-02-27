use super::*;

pub(super) fn render_local_command(
    matches: &ArgMatches,
    action: &str,
    data: Value,
) -> Result<(), String> {
    let payload = json!({
        "ok": true,
        "action": action,
        "data": data
    });
    if matches.get_flag("json") {
        if !matches.get_flag("no-pretty") {
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

pub(super) fn render_response(
    response: &ResponseEnvelope,
    json_mode: bool,
    pretty: bool,
    full: bool,
) -> Result<(), String> {
    if json_mode {
        let payload = if full {
            serde_json::to_value(response).map_err(|e| e.to_string())?
        } else {
            compact_response_payload(response)
        };
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
            "error": {
                "message": error.message,
                "hint": error.hint
            }
        });
    }
    json!({
        "ok": false,
        "error": {
            "message": "request failed"
        }
    })
}
