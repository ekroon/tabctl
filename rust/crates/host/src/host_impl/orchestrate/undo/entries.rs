use serde_json::Value;

#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub(super) tab_id: Option<i64>,
    pub(super) url: Option<String>,
    pub(super) pinned: Option<bool>,
    pub(super) active: bool,
    pub(super) window_id: i64,
    pub(super) index: i64,
    pub(super) group_id: i64,
    pub(super) group_title: Option<String>,
    pub(super) group_color: Option<String>,
    pub(super) group_collapsed: Option<bool>,
    pub(super) create_if_missing: bool,
}

pub(super) fn normalize(undo: &Value) -> Result<Vec<Entry>, String> {
    let action = undo["action"].as_str().ok_or("Undo action missing")?;
    let values: Vec<&Value> = match action {
        "move-tab" => vec![undo],
        "restore" | "close" | "archive" | "merge-window" | "group-ungroup" | "group-assign"
        | "group-gather" | "move-group" => undo["tabs"]
            .as_array()
            .ok_or("Undo tabs missing")?
            .iter()
            .collect(),
        _ => return Err(format!("Unknown undo action: {action}")),
    };
    let mut entries = Vec::new();
    for value in values {
        let from = value.get("from").unwrap_or(value);
        entries.push(Entry {
            tab_id: value["tabId"].as_i64(),
            url: value["url"].as_str().map(str::to_owned),
            pinned: value["pinned"].as_bool(),
            active: value["active"].as_bool().unwrap_or(false),
            window_id: from["windowId"]
                .as_i64()
                .ok_or("Undo source window missing")?,
            index: from["index"].as_i64().unwrap_or(-1),
            group_id: from["groupId"].as_i64().unwrap_or(-1),
            group_title: from["groupTitle"].as_str().map(str::to_owned),
            group_color: from["groupColor"].as_str().map(str::to_owned),
            group_collapsed: from["groupCollapsed"].as_bool(),
            create_if_missing: value["createIfMissing"]
                .as_bool()
                .unwrap_or(action == "close"),
        });
    }
    entries.sort_by_key(|entry| (entry.window_id, entry.index));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_close_and_flat_move_records_preserve_restoration_metadata() {
        let close = normalize(&serde_json::json!({"action":"close","tabs":[{
            "url":"https://a.example","active":true,"pinned":false,
            "from":{"windowId":4,"index":2,"groupId":7,"groupTitle":"Work","groupColor":"blue"}
        }]}))
        .unwrap();
        assert!(close[0].create_if_missing);
        assert_eq!(close[0].index, 2);
        assert!(close[0].active);
        let moved = normalize(&serde_json::json!({"action":"move-group","tabs":[{
            "tabId":10,"windowId":4,"index":1,"groupId":7
        }]}))
        .unwrap();
        assert!(!moved[0].create_if_missing);
        assert_eq!(moved[0].tab_id, Some(10));
    }
}
