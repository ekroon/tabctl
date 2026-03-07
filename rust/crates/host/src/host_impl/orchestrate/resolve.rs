use serde_json::Value;

use super::OrchStep;

/// Shared group resolution from a snapshot.
///
/// Provides `resolve_group` which finds a group by ID or title and returns
/// matching window/group/tab info needed by group-update, group-ungroup, etc.
#[derive(Debug, Clone)]
pub(crate) struct GroupMatch {
    pub(crate) window_id: i64,
    pub(crate) window_incognito: bool,
    pub(crate) group_id: i64,
    pub(crate) title: Option<String>,
    pub(crate) color: Option<String>,
    pub(crate) collapsed: Option<bool>,
    pub(crate) tabs: Vec<MatchedTab>,
}

#[derive(Debug, Clone)]
pub(crate) struct MatchedTab {
    pub(crate) tab_id: i64,
    pub(crate) window_id: i64,
    pub(crate) index: Option<i64>,
    pub(crate) group_id: i64,
    pub(crate) group_title: Option<String>,
    pub(crate) group_color: Option<String>,
    pub(crate) group_collapsed: Option<bool>,
}

/// Resolve a group from a snapshot by ID or title, optionally scoped to a window.
///
/// Returns `Ok(GroupMatch)` on unique match, `Err(OrchStep::Error)` on not-found
/// or ambiguity.
pub(crate) fn resolve_group(
    snapshot: &Value,
    group_id_param: Option<i64>,
    group_title_param: Option<&str>,
    window_id_param: Option<i64>,
) -> Result<GroupMatch, OrchStep> {
    let windows = snapshot
        .get("windows")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
        .clone();

    let mut matches: Vec<GroupMatch> = Vec::new();

    for win in &windows {
        let win_id = win.get("windowId").and_then(Value::as_i64).unwrap_or(0);
        let win_incognito = win
            .get("incognito")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let groups = win.get("groups").and_then(Value::as_array);
        let tabs = win.get("tabs").and_then(Value::as_array);

        let Some(groups) = groups else { continue };

        for group in groups {
            let gid = group.get("groupId").and_then(Value::as_i64).unwrap_or(-1);
            let gtitle = group.get("title").and_then(Value::as_str);
            let gcolor = group.get("color").and_then(Value::as_str);
            let gcollapsed = group.get("collapsed").and_then(Value::as_bool);

            let id_match = group_id_param.is_some_and(|id| id == gid);
            let title_match = group_title_param
                .is_some_and(|t| gtitle.is_some_and(|gt| gt.eq_ignore_ascii_case(t)));

            if !id_match && !title_match {
                continue;
            }

            if let Some(filter_win) = window_id_param {
                if win_id != filter_win {
                    continue;
                }
            }

            let matched_tabs = tabs
                .map(|t| {
                    t.iter()
                        .filter(|tab| tab.get("groupId").and_then(Value::as_i64) == Some(gid))
                        .map(|tab| MatchedTab {
                            tab_id: tab.get("tabId").and_then(Value::as_i64).unwrap_or(0),
                            window_id: win_id,
                            index: tab.get("index").and_then(Value::as_i64),
                            group_id: gid,
                            group_title: gtitle.map(String::from),
                            group_color: gcolor.map(String::from),
                            group_collapsed: gcollapsed,
                        })
                        .collect()
                })
                .unwrap_or_default();

            matches.push(GroupMatch {
                window_id: win_id,
                window_incognito: win_incognito,
                group_id: gid,
                title: gtitle.map(String::from),
                color: gcolor.map(String::from),
                collapsed: gcollapsed,
                tabs: matched_tabs,
            });
        }
    }

    match matches.len() {
        0 => Err(OrchStep::Error {
            message: "Group not found".to_string(),
            hint: Some("Use tabctl group-list to see existing groups.".to_string()),
        }),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            if group_id_param.is_some() {
                Err(OrchStep::Error {
                    message: "Group id is ambiguous. Provide a windowId.".to_string(),
                    hint: Some("Use --window to disambiguate group ids.".to_string()),
                })
            } else {
                let n = matches.len();
                let title = group_title_param.unwrap_or("?");
                Err(OrchStep::Error {
                    message: format!(
                        "Ambiguous group title: found {n} groups named \"{title}\". \
                         Use group-gather to merge duplicates, --group-id to target by ID, \
                         or --window to narrow scope."
                    ),
                    hint: Some(
                        "Use group-gather to merge duplicates, --group-id to target by ID, \
                         or --window to narrow scope."
                            .to_string(),
                    ),
                })
            }
        }
    }
}

pub(crate) fn window_is_incognito(snapshot: &Value, window_id: i64) -> bool {
    snapshot
        .get("windows")
        .and_then(Value::as_array)
        .and_then(|windows| {
            windows
                .iter()
                .find(|window| window.get("windowId").and_then(Value::as_i64) == Some(window_id))
        })
        .and_then(|window| window.get("incognito").and_then(Value::as_bool))
        .unwrap_or(false)
}

/// Resolve a windowId parameter from snapshot — handles "active" / "last-focused" aliases.
pub(crate) fn resolve_window_id(snapshot: &Value, raw: &Value) -> Option<i64> {
    if let Some(n) = raw.as_i64() {
        return Some(n);
    }
    if let Some(s) = raw.as_str() {
        let normalized = s.trim().to_lowercase();
        if normalized == "active" {
            return snapshot
                .get("windows")
                .and_then(Value::as_array)
                .and_then(|wins| {
                    wins.iter().find_map(|w| {
                        if w.get("focused").and_then(Value::as_bool) == Some(true) {
                            w.get("windowId").and_then(Value::as_i64)
                        } else {
                            None
                        }
                    })
                });
        }
        if normalized == "last-focused" {
            // Find window containing the tab with highest lastAccessedAt
            let best = snapshot
                .get("windows")
                .and_then(Value::as_array)
                .and_then(|wins| {
                    wins.iter()
                        .filter_map(|w| {
                            let max_ts =
                                w.get("tabs").and_then(Value::as_array).and_then(|tabs| {
                                    tabs.iter()
                                        .filter_map(|t| {
                                            t.get("lastAccessedAt").and_then(Value::as_f64)
                                        })
                                        .reduce(f64::max)
                                })?;
                            let wid = w.get("windowId").and_then(Value::as_i64)?;
                            Some((wid, max_ts))
                        })
                        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                        .map(|(wid, _)| wid)
                });
            // Fall back to focused window if no lastAccessedAt data
            return best
                .or_else(|| resolve_window_id(snapshot, &Value::String("active".to_string())));
        }
        return s.trim().parse::<i64>().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 3, "windowId": 100, "index": 2, "groupId": -1}
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}
                ]
            }, {
                "windowId": 200,
                "focused": false,
                "tabs": [
                    {"tabId": 4, "windowId": 200, "index": 0, "groupId": 20, "groupTitle": "Dev", "groupColor": "red", "groupCollapsed": true}
                ],
                "groups": [
                    {"groupId": 20, "title": "Dev", "color": "red", "collapsed": true}
                ]
            }]
        })
    }

    #[test]
    fn resolve_by_id_unique() {
        let m = resolve_group(&snapshot(), Some(10), None, None).unwrap();
        assert_eq!(m.group_id, 10);
        assert_eq!(m.window_id, 100);
        assert_eq!(m.tabs.len(), 2);
    }

    #[test]
    fn resolve_by_title_ambiguous() {
        let r = resolve_group(&snapshot(), None, Some("Dev"), None);
        assert!(r.is_err());
        if let Err(OrchStep::Error { message, .. }) = r {
            assert!(message.contains("Ambiguous"));
        }
    }

    #[test]
    fn resolve_by_title_with_window_filter() {
        let m = resolve_group(&snapshot(), None, Some("Dev"), Some(200)).unwrap();
        assert_eq!(m.group_id, 20);
        assert_eq!(m.window_id, 200);
        assert_eq!(m.tabs.len(), 1);
    }

    #[test]
    fn resolve_not_found() {
        let r = resolve_group(&snapshot(), Some(999), None, None);
        assert!(r.is_err());
        if let Err(OrchStep::Error { message, .. }) = r {
            assert!(message.contains("not found"));
        }
    }

    #[test]
    fn resolve_window_id_aliases() {
        let snap = snapshot();
        assert_eq!(
            resolve_window_id(&snap, &Value::String("active".to_string())),
            Some(100)
        );
        // Without lastAccessedAt data, last-focused falls back to active (focused window)
        assert_eq!(
            resolve_window_id(&snap, &Value::String("last-focused".to_string())),
            Some(100)
        );
        assert_eq!(resolve_window_id(&snap, &serde_json::json!(200)), Some(200));
    }

    #[test]
    fn resolve_last_focused_uses_last_focused_at() {
        // Window 200 has the most recently focused tab despite not being focused
        let snap = serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "lastAccessedAt": 1000.0}
                ],
                "groups": []
            }, {
                "windowId": 200,
                "focused": false,
                "tabs": [
                    {"tabId": 2, "windowId": 200, "lastAccessedAt": 2000.0}
                ],
                "groups": []
            }]
        });
        assert_eq!(
            resolve_window_id(&snap, &Value::String("active".to_string())),
            Some(100),
            "active should return focused window"
        );
        assert_eq!(
            resolve_window_id(&snap, &Value::String("last-focused".to_string())),
            Some(200),
            "last-focused should return window with highest lastAccessedAt"
        );
    }
}
