use super::*;

pub(super) fn run_help(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let mut cli = build_cli();
    let command_name = sub.get_one::<String>("command");

    if matches.get_flag("json") {
        return if let Some(name) = command_name {
            let cmd = cli
                .find_subcommand_mut(name)
                .ok_or_else(|| format!("Unknown command: {name}"))?;
            let mut buf = Vec::new();
            cmd.write_long_help(&mut buf).map_err(|e| e.to_string())?;
            let text = String::from_utf8(buf).map_err(|e| e.to_string())?;
            render_local_command(matches, "help", json!({ "command": name, "text": text }))
        } else {
            let data = structured_help(&cli);
            render_local_command(matches, "help", data)
        };
    }

    let help_text = if let Some(name) = command_name {
        let cmd = cli
            .find_subcommand_mut(name)
            .ok_or_else(|| format!("Unknown command: {name}"))?;
        let mut buf = Vec::new();
        cmd.write_long_help(&mut buf).map_err(|e| e.to_string())?;
        String::from_utf8(buf).map_err(|e| e.to_string())?
    } else {
        let mut buf = Vec::new();
        cli.write_long_help(&mut buf).map_err(|e| e.to_string())?;
        String::from_utf8(buf).map_err(|e| e.to_string())?
    };
    println!("{help_text}");
    Ok(())
}

fn structured_help(cli: &Command) -> serde_json::Value {
    let version = env!("CARGO_PKG_VERSION");

    let format_opt = |a: &clap::Arg| -> Option<String> {
        if let Some(long) = a.get_long() {
            let vn = a.get_value_names().map(|v| {
                v.iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            });
            Some(match vn {
                Some(ref v) if !v.is_empty() => format!("--{long} <{v}>"),
                _ => format!("--{long}"),
            })
        } else if let Some(idx) = a.get_index() {
            let id = a.get_id().as_str();
            Some(format!("<{id}> (positional {idx})"))
        } else {
            None
        }
    };

    let global_ids: std::collections::HashSet<&str> = cli
        .get_arguments()
        .filter(|a| a.get_id() != "help" && a.get_id() != "version")
        .map(|a| a.get_id().as_str())
        .collect();

    let global_opts: Vec<String> = cli
        .get_arguments()
        .filter(|a| a.get_id() != "help" && a.get_id() != "version")
        .filter_map(&format_opt)
        .collect();

    // Derive usage from clap
    let mut usage_buf = Vec::new();
    let _ = cli.clone().write_help(&mut usage_buf);
    let usage_text = String::from_utf8_lossy(&usage_buf);
    let usage = usage_text
        .lines()
        .find(|l| l.starts_with("Usage:"))
        .map(|l| l.trim_start_matches("Usage:").trim().to_string())
        .unwrap_or_else(|| "tabctl [OPTIONS] [COMMAND]".to_string());

    let commands: Vec<serde_json::Value> = cli
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(|c| {
            let name = c.get_name().to_string();
            let mut entry = serde_json::Map::new();
            entry.insert("name".into(), json!(name));

            let aliases: Vec<&str> = c.get_visible_aliases().collect();
            if !aliases.is_empty() {
                entry.insert("aliases".into(), json!(aliases));
            }

            let opts: Vec<String> = c
                .get_arguments()
                .filter(|a| a.get_id() != "help" && a.get_id() != "version")
                .filter(|a| !global_ids.contains(a.get_id().as_str()))
                .filter_map(&format_opt)
                .collect();
            if !opts.is_empty() {
                entry.insert("options".into(), json!(opts));
            }

            serde_json::Value::Object(entry)
        })
        .collect();

    json!({
        "version": version,
        "usage": usage,
        "commands": commands,
        "globalOptions": global_opts
    })
}

pub(super) fn run_version(matches: &ArgMatches, _sub: &ArgMatches) -> Result<(), String> {
    let version = env!("CARGO_PKG_VERSION");
    if matches.get_flag("json") {
        return render_local_command(
            matches,
            "version",
            json!({
                "version": version,
                "mode": std::env::var("TABCTL_VERSION_MODE").ok()
            }),
        );
    }
    println!("{version}");
    Ok(())
}

pub(super) fn load_profile_registry(
    allow_missing: bool,
) -> Result<(String, PathBuf, ProfileRegistry), String> {
    let config_dir = resolve_config_dir()?;
    let profiles_path = PathBuf::from(&config_dir).join("profiles.json");
    if !profiles_path.exists() {
        if allow_missing {
            return Ok((
                config_dir,
                profiles_path,
                ProfileRegistry {
                    default: None,
                    profiles: HashMap::new(),
                },
            ));
        }
        return Err(
            "profiles.json not found. Run `tabctl setup --browser <edge|chrome>` first."
                .to_string(),
        );
    }
    let content = fs::read_to_string(&profiles_path)
        .map_err(|e| format!("failed to read profiles.json: {e}"))?;
    let registry = serde_json::from_str::<ProfileRegistry>(&content)
        .map_err(|e| format!("failed to parse profiles.json: {e}"))?;
    Ok((config_dir, profiles_path, registry))
}

pub(super) fn save_profile_registry(
    config_dir: &str,
    profiles_path: &Path,
    registry: &ProfileRegistry,
) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    let content = serde_json::to_string_pretty(registry).map_err(|e| e.to_string())?;
    fs::write(profiles_path, content).map_err(|e| format!("failed to write profiles.json: {e}"))
}

pub(super) fn run_profile_list(matches: &ArgMatches, _sub: &ArgMatches) -> Result<(), String> {
    let (config_dir, profiles_path, registry) = load_profile_registry(true)?;
    let mut names = registry.profiles.keys().cloned().collect::<Vec<_>>();
    names.sort();
    let profiles = names
        .iter()
        .filter_map(|name| registry.profiles.get(name).map(|entry| (name, entry)))
        .map(|(name, entry)| {
            json!({
                "name": name,
                "browser": browser_name(&entry.browser),
                "extensionId": entry.extension_id,
                "dataDir": entry.data_dir,
                "hostPath": entry.host_path
            })
        })
        .collect::<Vec<_>>();
    render_local_command(
        matches,
        "profile-list",
        json!({
            "configDir": config_dir,
            "profilesPath": profiles_path.display().to_string(),
            "default": registry.default,
            "profiles": profiles
        }),
    )
}

pub(super) fn run_profile_show(matches: &ArgMatches, _sub: &ArgMatches) -> Result<(), String> {
    let (_config_dir, profiles_path, registry) = load_profile_registry(false)?;
    let selected_name = matches
        .get_one::<String>("profile")
        .cloned()
        .or_else(|| registry.default.clone())
        .ok_or_else(|| {
            "No active profile. Use `tabctl profile-list` or `tabctl setup` first.".to_string()
        })?;
    let entry = registry
        .profiles
        .get(&selected_name)
        .ok_or_else(|| format!("Profile \"{selected_name}\" not found in profiles.json"))?;
    let socket = resolve_socket_endpoint(Some(&selected_name))
        .map(|endpoint| endpoint.as_uri())
        .ok();
    render_local_command(
        matches,
        "profile-show",
        json!({
            "name": selected_name,
            "default": registry.default,
            "profilesPath": profiles_path.display().to_string(),
            "browser": browser_name(&entry.browser),
            "extensionId": entry.extension_id,
            "dataDir": entry.data_dir,
            "hostPath": entry.host_path,
            "socket": socket
        }),
    )
}

pub(super) fn run_profile_switch(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let profile_name = sub
        .get_one::<String>("name")
        .ok_or_else(|| "Missing profile name".to_string())?;
    let (config_dir, profiles_path, mut registry) = load_profile_registry(false)?;
    if !registry.profiles.contains_key(profile_name) {
        return Err(format!(
            "Profile \"{profile_name}\" not found in profiles.json"
        ));
    }
    registry.default = Some(profile_name.to_string());
    save_profile_registry(&config_dir, &profiles_path, &registry)?;
    render_local_command(
        matches,
        "profile-switch",
        json!({
            "default": profile_name,
            "profilesPath": profiles_path.display().to_string()
        }),
    )
}

pub(super) fn run_profile_remove(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let profile_name = sub
        .get_one::<String>("name")
        .ok_or_else(|| "Missing profile name".to_string())?;
    let (config_dir, profiles_path, mut registry) = load_profile_registry(false)?;
    if registry.profiles.remove(profile_name).is_none() {
        return Err(format!(
            "Profile \"{profile_name}\" not found in profiles.json"
        ));
    }
    if registry.default.as_deref() == Some(profile_name) {
        let mut remaining = registry.profiles.keys().cloned().collect::<Vec<_>>();
        remaining.sort();
        registry.default = remaining.first().cloned();
    }
    let default_profile = registry.default.clone();
    let remaining_count = registry.profiles.len();
    save_profile_registry(&config_dir, &profiles_path, &registry)?;
    render_local_command(
        matches,
        "profile-remove",
        json!({
            "removed": profile_name,
            "default": default_profile,
            "remaining": remaining_count,
            "profilesPath": profiles_path.display().to_string()
        }),
    )
}

pub(super) fn run_doctor(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let fix = sub.get_flag("fix");
    let config_dir = resolve_config_dir()?;
    let profiles_path = PathBuf::from(&config_dir).join("profiles.json");
    let registry = if profiles_path.exists() {
        let content = fs::read_to_string(&profiles_path)
            .map_err(|e| format!("failed to read profiles.json: {e}"))?;
        serde_json::from_str::<ProfileRegistry>(&content)
            .map_err(|e| format!("failed to parse profiles.json: {e}"))?
    } else {
        ProfileRegistry {
            default: None,
            profiles: HashMap::new(),
        }
    };

    let mut profile_names = registry.profiles.keys().cloned().collect::<Vec<_>>();
    profile_names.sort();

    let mut reports = Vec::new();
    let mut healthy_count = 0usize;
    let mut repaired_count = 0usize;
    let mut connectivity_unhealthy_count = 0usize;
    for profile_name in profile_names {
        let Some(entry) = registry.profiles.get(&profile_name).cloned() else {
            continue;
        };
        let evaluation = evaluate_doctor_profile(&profile_name, &entry, fix)?;
        if evaluation.healthy {
            healthy_count += 1;
        }
        if evaluation.repaired {
            repaired_count += 1;
        }
        if !evaluation.connectivity_healthy {
            connectivity_unhealthy_count += 1;
        }
        reports.push(evaluation.report);
    }

    let total = reports.len();
    let data = json!({
        "configDir": config_dir,
        "profilesPath": profiles_path.display().to_string(),
        "fixApplied": fix,
        "summary": {
            "total": total,
            "healthy": healthy_count,
            "unhealthy": total.saturating_sub(healthy_count),
            "repaired": repaired_count,
            "connectivityUnhealthy": connectivity_unhealthy_count
        },
        "profiles": reports
    });
    render_local_command(matches, "doctor", data)
}

pub(super) struct DoctorProfileEvaluation {
    pub(super) report: Value,
    pub(super) healthy: bool,
    pub(super) repaired: bool,
    pub(super) connectivity_healthy: bool,
}

pub(super) fn evaluate_doctor_profile(
    profile_name: &str,
    entry: &ProfileEntry,
    fix: bool,
) -> Result<DoctorProfileEvaluation, String> {
    let mut report = profile_health_report(profile_name, entry)?;
    let mut static_healthy = report
        .get("healthy")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut connectivity = profile_connectivity_report(profile_name, entry);
    let mut connectivity_healthy = connectivity
        .get("healthy")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    report["staticHealthy"] = Value::Bool(static_healthy);
    report["connectivity"] = connectivity;

    let mut repaired = false;
    if fix && !static_healthy {
        let repair = attempt_profile_repair(profile_name, entry);
        report["repair"] = match repair {
            Ok(details) => {
                repaired = true;
                json!({ "attempted": true, "ok": true, "details": details })
            }
            Err(error) => json!({ "attempted": true, "ok": false, "error": error }),
        };
        let repaired_report = profile_health_report(profile_name, entry)?;
        static_healthy = repaired_report
            .get("healthy")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(checks) = repaired_report.get("checks") {
            report["checks"] = checks.clone();
        }
        report["staticHealthy"] = Value::Bool(static_healthy);
        connectivity = profile_connectivity_report(profile_name, entry);
        connectivity_healthy = connectivity
            .get("healthy")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        report["connectivity"] = connectivity;
    } else if fix && !connectivity_healthy {
        report["repair"] = json!({
            "attempted": false,
            "ok": false,
            "reason": "No local profile artifact issue detected; follow manualSteps under connectivity."
        });
    }

    if fix {
        report["extensionSync"] = match attempt_profile_extension_sync(profile_name) {
            Ok(sync) => {
                if sync
                    .get("updated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    repaired = true;
                }
                sync
            }
            Err(error) => json!({
                "attempted": true,
                "ok": false,
                "error": error
            }),
        };
    }

    let healthy = static_healthy && connectivity_healthy;
    report["healthy"] = Value::Bool(healthy);
    Ok(DoctorProfileEvaluation {
        report,
        healthy,
        repaired,
        connectivity_healthy,
    })
}

pub(super) fn run_policy(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let config_dir = resolve_config_dir()?;
    let policy_path = PathBuf::from(&config_dir).join("policy.json");
    let mut created = false;

    if sub.get_flag("init") {
        fs::create_dir_all(&config_dir).map_err(|e| format!("failed to create config dir: {e}"))?;
        let payload = json!({
            "protect": {
                "pinned": true,
                "groupTitles": ["🔒"]
            }
        });
        let content = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        fs::write(&policy_path, content)
            .map_err(|e| format!("failed to write policy.json: {e}"))?;
        created = true;
    }

    let exists = policy_path.exists();
    let mut policy = Value::Null;
    if exists {
        let content = fs::read_to_string(&policy_path)
            .map_err(|e| format!("failed to read policy.json: {e}"))?;
        policy = serde_json::from_str(&content).map_err(|e| format!("invalid policy.json: {e}"))?;
    }

    let summary = policy
        .get("protect")
        .and_then(Value::as_object)
        .map(|protect| {
            json!({
                "pinned": protect.get("pinned").and_then(Value::as_bool).unwrap_or(false),
                "groupTitles": protect.get("groupTitles").cloned().unwrap_or_else(|| json!([]))
            })
        })
        .unwrap_or_else(|| json!({}));

    let data = json!({
        "policyPath": policy_path.display().to_string(),
        "exists": exists,
        "initialized": sub.get_flag("init"),
        "created": created,
        "summary": summary,
        "policy": policy
    });
    render_local_command(matches, "policy", data)
}

pub(super) fn run_skill(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let mut args = vec![
        "-y".to_string(),
        "skills".to_string(),
        "add".to_string(),
        "https://github.com/ekroon/tabctl".to_string(),
        "--skill".to_string(),
        "tabctl".to_string(),
        "--yes".to_string(),
    ];
    if sub.get_flag("global") {
        args.push("--global".to_string());
    }
    if let Some(agents) = sub.get_many::<String>("agent") {
        for agent in agents {
            args.push("-a".to_string());
            args.push(agent.to_string());
        }
    }

    let mut command = ProcessCommand::new("npx");
    for arg in &args {
        command.arg(arg);
    }
    let output = command
        .output()
        .map_err(|e| format!("failed to execute npx skills installer: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        if !stderr.is_empty() {
            return Err(format!("skill install failed: {stderr}"));
        }
        return Err(format!(
            "skill install failed with status {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }

    let data = json!({
        "global": sub.get_flag("global"),
        "agents": sub
            .get_many::<String>("agent")
            .map(|vals| vals.map(|v| v.to_string()).collect::<Vec<_>>())
            .unwrap_or_default(),
        "command": format!("npx {}", args.join(" ")),
        "stdout": stdout,
        "stderr": stderr
    });
    render_local_command(matches, "skill", data)
}

pub(super) fn browser_name(browser: &Browser) -> &'static str {
    match browser {
        Browser::Edge => "edge",
        Browser::Chrome => "chrome",
    }
}

pub(super) fn command_path_healthy(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    if Path::new(trimmed).is_absolute() {
        return Path::new(trimmed).exists();
    }
    let command_name = if cfg!(windows) { "where" } else { "which" };
    ProcessCommand::new(command_name)
        .arg(trimmed)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(super) fn profile_connectivity_report(profile_name: &str, entry: &ProfileEntry) -> Value {
    match send_request("ping", Value::Object(Map::new()), Some(profile_name), false) {
        Ok(response) => {
            if response.ok {
                json!({
                    "healthy": true,
                    "checks": {
                        "pingOk": true
                    },
                    "data": response.data
                })
            } else {
                json!({
                    "healthy": false,
                    "checks": {
                        "pingOk": false
                    },
                    "error": response.error.as_ref().map(|error| {
                        json!({
                            "message": error.message,
                            "hint": error.hint
                        })
                    }),
                    "manualSteps": connectivity_manual_steps(profile_name, entry)
                })
            }
        }
        Err(error) => json!({
            "healthy": false,
            "checks": {
                "pingOk": false
            },
            "error": {
                "message": error
            },
            "manualSteps": connectivity_manual_steps(profile_name, entry)
        }),
    }
}

pub(super) fn connectivity_manual_steps(profile_name: &str, entry: &ProfileEntry) -> Vec<String> {
    vec![
        format!("Verify connection: tabctl --profile {profile_name} ping"),
        "Ensure the browser extension is loaded and active for this profile.".to_string(),
        format!(
            "Rerun setup for this profile: tabctl setup --browser {} --name {profile_name} --extension-id {}",
            browser_name(&entry.browser),
            entry.extension_id
        ),
    ]
}

pub(super) fn profile_health_report(
    profile_name: &str,
    entry: &ProfileEntry,
) -> Result<Value, String> {
    let host_path_exists = Path::new(&entry.host_path).exists();
    let (wrapper_exports_profile, wrapper_exports_config_dir, wrapper_exports_data_dir) =
        if host_path_exists {
            let content = fs::read_to_string(&entry.host_path)
                .map_err(|e| format!("failed to read host wrapper script: {e}"))?;
            (
                content.contains("TABCTL_PROFILE"),
                content.contains("TABCTL_CONFIG_DIR"),
                content.contains("TABCTL_DATA_DIR"),
            )
        } else {
            (false, false, false)
        };
    let binary_path_exists = command_path_healthy(&entry.node_path);
    let manifest_path =
        resolve_manifest_dir(browser_name(&entry.browser))?.join(format!("{HOST_NAME}.json"));
    let manifest_exists = manifest_path.exists();

    let mut manifest_path_matches = false;
    let mut manifest_origin_matches = false;
    if manifest_exists {
        let manifest_content = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("failed to read native host manifest: {e}"))?;
        let manifest_json: Value = serde_json::from_str(&manifest_content)
            .map_err(|e| format!("invalid native host manifest: {e}"))?;
        manifest_path_matches = manifest_json
            .get("path")
            .and_then(Value::as_str)
            .map(|path| path == entry.host_path)
            .unwrap_or(false);
        let expected_origin = format!("chrome-extension://{}/", entry.extension_id);
        manifest_origin_matches = manifest_json
            .get("allowed_origins")
            .and_then(Value::as_array)
            .map(|origins| {
                origins
                    .iter()
                    .any(|origin| origin.as_str() == Some(&expected_origin))
            })
            .unwrap_or(false);
    }

    let healthy = host_path_exists
        && wrapper_exports_profile
        && wrapper_exports_config_dir
        && wrapper_exports_data_dir
        && binary_path_exists
        && manifest_exists
        && manifest_path_matches
        && manifest_origin_matches;
    Ok(json!({
        "name": profile_name,
        "browser": browser_name(&entry.browser),
        "extensionId": entry.extension_id,
        "hostPath": entry.host_path,
        "binaryPath": entry.node_path,
        "manifestPath": manifest_path.display().to_string(),
        "healthy": healthy,
        "checks": {
            "hostPathExists": host_path_exists,
            "hostWrapperExportsProfile": wrapper_exports_profile,
            "hostWrapperExportsConfigDir": wrapper_exports_config_dir,
            "hostWrapperExportsDataDir": wrapper_exports_data_dir,
            "binaryPathExists": binary_path_exists,
            "manifestExists": manifest_exists,
            "manifestPathMatchesProfile": manifest_path_matches,
            "manifestAllowedOriginMatchesProfile": manifest_origin_matches
        }
    }))
}

pub(super) fn attempt_profile_repair(
    profile_name: &str,
    entry: &ProfileEntry,
) -> Result<Value, String> {
    let data_dir = resolve_data_dir(None)?;
    let wrapper_dir = PathBuf::from(&data_dir).join("profiles").join(profile_name);
    let tabctl_binary_path = resolve_tabctl_binary_path();
    let wrapper_path = write_host_wrapper(&tabctl_binary_path, profile_name, &wrapper_dir)?;
    let manifest_path = write_native_manifest(
        browser_name(&entry.browser),
        &wrapper_path,
        &entry.extension_id,
        entry.user_data_dir.as_deref(),
    )?;
    register_profile(
        &data_dir,
        profile_name,
        browser_name(&entry.browser),
        &entry.extension_id,
        &wrapper_path,
        &manifest_path,
    )?;
    Ok(json!({
        "wrapperPath": wrapper_path.display().to_string(),
        "manifestPath": manifest_path.display().to_string()
    }))
}

pub(super) fn attempt_profile_extension_sync(profile_name: &str) -> Result<Value, String> {
    let allow_download = std::env::var("TABCTL_SETUP_FETCH_EXTENSION")
        .ok()
        .as_deref()
        != Some("0");
    let release_source = resolve_extension_release_source(None, None, None, None)?;
    let result = sync_extension_release(&release_source, allow_download)?;
    let mut payload = json!({
        "attempted": true,
        "ok": true,
        "updated": result.updated,
        "targetVersion": result.target_version,
        "installedVersion": result.installed_version,
        "activePath": result.active_path
    });
    if result.updated {
        match send_request(
            "reload",
            Value::Object(Map::new()),
            Some(profile_name),
            false,
        ) {
            Ok(reload_response) => {
                payload["reload"] = json!({
                    "attempted": true,
                    "ok": reload_response.ok
                });
            }
            Err(error) => {
                payload["reload"] = json!({
                    "attempted": true,
                    "ok": false,
                    "error": error
                });
            }
        }
    } else {
        payload["reload"] = json!({
            "attempted": false,
            "ok": true
        });
    }
    Ok(payload)
}

pub(super) fn run_upgrade(matches: &ArgMatches, _sub: &ArgMatches) -> Result<(), String> {
    let profile_filter = matches
        .get_one::<String>("profile")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let (_config_dir, _profiles_path, registry) = load_profile_registry(false)?;
    let profile_names: Vec<String> = if let Some(ref name) = profile_filter {
        if !registry.profiles.contains_key(name) {
            return Err(format!("Profile \"{name}\" not found in profiles.json"));
        }
        vec![name.clone()]
    } else {
        let mut names = registry.profiles.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    };

    if profile_names.is_empty() {
        return Err(
            "No profiles configured. Run `tabctl setup --browser <edge|chrome>` first.".to_string(),
        );
    }

    let cli_version = env!("CARGO_PKG_VERSION");
    let current_binary = resolve_tabctl_binary_path();
    let json_mode = matches.get_flag("json");

    if !json_mode {
        eprintln!("\u{2b06} Upgrading tabctl profiles...");
    }

    let mut profile_results = Vec::new();
    let mut any_updated = false;
    let mut any_failed = false;

    for profile_name in &profile_names {
        let Some(entry) = registry.profiles.get(profile_name) else {
            continue;
        };

        let mut result = json!({ "name": profile_name });
        let mut needs_reload = false;

        // Step 1: Repair wrapper if binary path changed
        let wrapper_stale = entry.node_path != current_binary;
        if wrapper_stale {
            match attempt_profile_repair(profile_name, entry) {
                Ok(details) => {
                    result["wrapperRepair"] = json!({
                        "updated": true,
                        "previousBinary": entry.node_path,
                        "currentBinary": current_binary,
                        "details": details
                    });
                    if !json_mode {
                        eprintln!(
                            "  {profile_name}:\n    \u{2713} Host wrapper updated (\u{2192} {cli_version})"
                        );
                    }
                    any_updated = true;
                    needs_reload = true;
                }
                Err(e) => {
                    result["wrapperRepair"] = json!({
                        "updated": false,
                        "error": e
                    });
                    if !json_mode {
                        eprintln!("  {profile_name}:\n    \u{26a0} Wrapper repair failed: {e}");
                    }
                    any_failed = true;
                }
            }
        } else {
            result["wrapperRepair"] = json!({ "updated": false, "reason": "already current" });
            if !json_mode {
                eprintln!("  {profile_name}:\n    \u{b7} Host wrapper already current");
            }
        }

        // Step 2: Sync extension
        match attempt_profile_extension_sync(profile_name) {
            Ok(sync) => {
                let ext_updated = sync
                    .get("updated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if ext_updated {
                    any_updated = true;
                    // Extension sync already sends reload when updated
                    needs_reload = false;
                    let target = sync
                        .get("targetVersion")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if !json_mode {
                        eprintln!("    \u{2713} Extension synced ({target})");
                    }
                } else if !json_mode {
                    eprintln!("    \u{b7} Extension already in sync");
                }
                result["extensionSync"] = sync;
            }
            Err(e) => {
                result["extensionSync"] = json!({ "attempted": true, "ok": false, "error": e });
                if !json_mode {
                    eprintln!("    \u{26a0} Extension sync failed: {e}");
                }
                any_failed = true;
            }
        }

        // Step 3: Send reload if wrapper was repaired but extension sync didn't reload
        if needs_reload {
            match send_request(
                "reload",
                Value::Object(Map::new()),
                Some(profile_name),
                false,
            ) {
                Ok(r) => {
                    result["reload"] = json!({ "attempted": true, "ok": r.ok });
                    if !json_mode {
                        eprintln!("    \u{2713} Reloaded");
                    }
                }
                Err(e) => {
                    result["reload"] = json!({ "attempted": true, "ok": false, "error": e });
                    if !json_mode {
                        eprintln!("    \u{26a0} Reload failed: {e}");
                    }
                    any_failed = true;
                }
            }
        }

        profile_results.push(result);
    }

    // Summary and next steps
    let data = json!({
        "cliVersion": cli_version,
        "anyUpdated": any_updated,
        "anyFailed": any_failed,
        "profiles": profile_results
    });

    if json_mode {
        return render_local_command(matches, "upgrade", data);
    }

    eprintln!();
    if any_failed {
        eprintln!("\u{26a0}\u{fe0f} Upgrade applied but some steps failed.");
        eprintln!();
        eprintln!("Next steps:");
        eprintln!("  1. Run 'tabctl ping' to check if the host reconnected.");
        eprintln!(
            "  2. If still failing, reload the extension manually or run 'tabctl setup --browser <edge|chrome>'."
        );
    } else if any_updated {
        eprintln!("\u{2705} Upgrade complete.");
        eprintln!();
        eprintln!("Next steps:");
        eprintln!("  Run 'tabctl ping' to verify all components are in sync.");
    } else {
        eprintln!("\u{2705} All profiles up to date.");
    }
    Ok(())
}

pub(super) fn run_graphql_query(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let query_str = sub
        .get_one::<String>("graphql")
        .ok_or("Missing GraphQL query")?;
    let profile = matches.get_one::<String>("profile").map(|s| s.as_str());

    let snapshot_response = send_request("list", json!({"all": true}), profile, false)?;
    let snapshot = snapshot_response.data.unwrap_or(json!({}));

    let sender = std::sync::Arc::new(CliCommandSender {
        profile: profile.map(String::from),
    });

    let result = tabctl_graphql::execute(query_str, None, snapshot, sender)?;
    render_local_command(matches, "query", result)
}

struct CliCommandSender {
    profile: Option<String>,
}

impl tabctl_graphql::CommandSender for CliCommandSender {
    fn send(&self, action: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let response = send_request(action, params, self.profile.as_deref(), false)?;
        if !response.ok {
            let msg = response
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "Request failed".to_string());
            return Err(msg);
        }
        Ok(response.data.unwrap_or(json!({})))
    }

    fn snapshot(&self) -> Result<serde_json::Value, String> {
        let response = send_request("list", json!({"all": true}), self.profile.as_deref(), false)?;
        if !response.ok {
            let msg = response
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "Snapshot request failed".to_string());
            return Err(msg);
        }
        Ok(response.data.unwrap_or(json!({})))
    }
}
