use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use tabctl_shared::{
    Browser, ProfileEntry, ProfileRegistry, RequestEnvelope, ResponseEnvelope, SocketEndpoint,
};

#[cfg(windows)]
use sha2::{Digest, Sha256};

const WSL_TCP_PORT_FILENAME: &str = "tcp-port";
#[cfg(target_os = "linux")]
const WSL_TCP_PORT_FALLBACK: u16 = 39_001;
const AUTH_TOKEN_FILENAME: &str = "auth-token";

pub fn run<I, T>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let matches = build_cli()
        .try_get_matches_from(args)
        .map_err(|e| e.to_string())?;
    if matches.get_flag("version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(("help", sub)) = matches.subcommand() {
        return run_help(&matches, sub);
    }
    if let Some(("version", sub)) = matches.subcommand() {
        return run_version(&matches, sub);
    }
    if let Some(("setup", sub)) = matches.subcommand() {
        return run_setup(&matches, sub);
    }
    if let Some(("extension-fetch", sub)) = matches.subcommand() {
        return run_extension_fetch(&matches, sub);
    }
    if let Some(("doctor", sub)) = matches.subcommand() {
        return run_doctor(&matches, sub);
    }
    if let Some(("policy", sub)) = matches.subcommand() {
        return run_policy(&matches, sub);
    }
    if let Some(("skill", sub)) = matches.subcommand() {
        return run_skill(&matches, sub);
    }
    if let Some(("profile-list", sub)) = matches.subcommand() {
        return run_profile_list(&matches, sub);
    }
    if let Some(("profile-show", sub)) = matches.subcommand() {
        return run_profile_show(&matches, sub);
    }
    if let Some(("profile-switch", sub)) = matches.subcommand() {
        return run_profile_switch(&matches, sub);
    }
    if let Some(("profile-remove", sub)) = matches.subcommand() {
        return run_profile_remove(&matches, sub);
    }
    let routed = route_command(&matches)?;
    let response = send_request(
        &routed.action,
        routed.params,
        routed.profile.as_deref(),
        routed.progress,
    )?;
    render_response(&response, routed.json, routed.pretty)
}

#[derive(Debug)]
struct RoutedCommand {
    action: String,
    params: Value,
    json: bool,
    pretty: bool,
    progress: bool,
    profile: Option<String>,
}

fn build_cli() -> Command {
    Command::new("tabctl")
        .disable_help_subcommand(true)
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("pretty")
                .long("pretty")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("no-pretty")
                .long("no-pretty")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("profile")
                .long("profile")
                .value_name("name")
                .global(true),
        )
        .arg(
            Arg::new("progress")
                .long("progress")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .short('v')
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .subcommand(command_with_scope("ping"))
        .subcommand(command_list())
        .subcommand(
            command_with_scope("group-list")
                .visible_alias("groups")
                .visible_alias("group"),
        )
        .subcommand(command_analyze())
        .subcommand(command_dedupe())
        .subcommand(command_inspect())
        .subcommand(command_with_scope("focus"))
        .subcommand(command_with_scope("refresh"))
        .subcommand(command_open())
        .subcommand(command_group_update())
        .subcommand(command_group_ungroup())
        .subcommand(command_group_assign())
        .subcommand(command_with_scope("group-gather"))
        .subcommand(command_move_tab())
        .subcommand(command_move_group())
        .subcommand(command_merge_window())
        .subcommand(command_with_scope("archive"))
        .subcommand(command_close())
        .subcommand(command_report())
        .subcommand(command_setup())
        .subcommand(command_doctor())
        .subcommand(command_policy())
        .subcommand(command_skill())
        .subcommand(command_profile_list())
        .subcommand(command_profile_show())
        .subcommand(command_profile_switch())
        .subcommand(command_profile_remove())
        .subcommand(command_help())
        .subcommand(command_version())
        .subcommand(command_screenshot())
        .subcommand(command_undo())
        .subcommand(
            Command::new("history").arg(
                Arg::new("limit")
                    .long("limit")
                    .value_parser(value_parser!(u64))
                    .value_name("n"),
            ),
        )
        .subcommand(
            Command::new("extension-fetch")
                .arg(
                    Arg::new("version")
                        .long("version")
                        .value_name("version|tag"),
                )
                .arg(
                    Arg::new("repo")
                        .long("repo")
                        .value_name("owner/repo")
                        .default_value("ekroon/tabctl"),
                )
                .arg(
                    Arg::new("asset")
                        .long("asset")
                        .value_name("name")
                        .default_value("tabctl-extension.zip"),
                )
                .arg(Arg::new("out").long("out").value_name("path")),
        )
        .subcommand(command_with_scope("reload"))
}

fn run_extension_fetch(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let source = resolve_extension_release_source(
        sub.get_one::<String>("version").map(|v| v.as_str()),
        sub.get_one::<String>("repo").map(|v| v.as_str()),
        sub.get_one::<String>("asset").map(|v| v.as_str()),
        sub.get_one::<String>("out").map(PathBuf::from),
    )?;
    let payload = download_extension_asset(&source)?;
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
    } else if let Some(path) = payload.get("path").and_then(Value::as_str) {
        println!("{path}");
    }
    Ok(())
}

fn render_local_command(matches: &ArgMatches, action: &str, data: Value) -> Result<(), String> {
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

fn command_list() -> Command {
    command_with_scope("list").arg(Arg::new("groups").long("groups").action(ArgAction::SetTrue))
}

fn command_help() -> Command {
    Command::new("help").arg(Arg::new("command").value_name("command").index(1))
}

fn command_version() -> Command {
    Command::new("version")
}

fn command_doctor() -> Command {
    Command::new("doctor").arg(Arg::new("fix").long("fix").action(ArgAction::SetTrue))
}

fn command_policy() -> Command {
    Command::new("policy").arg(Arg::new("init").long("init").action(ArgAction::SetTrue))
}

fn command_skill() -> Command {
    Command::new("skill")
        .arg(
            Arg::new("agent")
                .long("agent")
                .action(ArgAction::Append)
                .value_name("name"),
        )
        .arg(Arg::new("global").long("global").action(ArgAction::SetTrue))
}

fn command_profile_list() -> Command {
    Command::new("profile-list")
}

fn command_profile_show() -> Command {
    Command::new("profile-show")
}

fn command_profile_switch() -> Command {
    Command::new("profile-switch").arg(Arg::new("name").value_name("name").required(true))
}

fn command_profile_remove() -> Command {
    Command::new("profile-remove").arg(Arg::new("name").value_name("name").required(true))
}

fn resolve_extension_release_source(
    version_input: Option<&str>,
    repo_input: Option<&str>,
    asset_input: Option<&str>,
    output_path_input: Option<PathBuf>,
) -> Result<ExtensionReleaseSource, String> {
    let version_input = version_input.unwrap_or(env!("CARGO_PKG_VERSION"));
    let tag = if version_input.starts_with('v') {
        version_input.to_string()
    } else {
        format!("v{version_input}")
    };
    let version = tag.trim_start_matches('v');
    let repo = repo_input.unwrap_or("ekroon/tabctl");
    let asset = asset_input.unwrap_or("tabctl-extension.zip");
    let output_path = if let Some(path) = output_path_input {
        path
    } else {
        PathBuf::from(resolve_data_dir(None)?)
            .join("extension")
            .join(version)
            .join(asset)
    };
    let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
    Ok(ExtensionReleaseSource {
        repo: repo.to_string(),
        tag,
        asset: asset.to_string(),
        path: output_path,
        url,
    })
}

fn download_extension_asset(source: &ExtensionReleaseSource) -> Result<Value, String> {
    if let Some(parent) = source.path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create output directory: {e}"))?;
    }
    let status = ProcessCommand::new("curl")
        .arg("--fail")
        .arg("--location")
        .arg("--silent")
        .arg("--output")
        .arg(&source.path)
        .arg(&source.url)
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute curl: {e}"))?;
    if !status.success() {
        return Err(format!(
            "Failed to download extension asset from {}",
            source.url
        ));
    }
    Ok(source.to_json())
}

fn command_setup() -> Command {
    Command::new("setup")
        .arg(
            Arg::new("browser")
                .long("browser")
                .required(true)
                .value_name("edge|chrome"),
        )
        .arg(
            Arg::new("skip-extension-download")
                .long("skip-extension-download")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("release-repo")
                .long("release-repo")
                .value_name("owner/repo"),
        )
        .arg(
            Arg::new("release-tag")
                .long("release-tag")
                .value_name("tag|version"),
        )
        .arg(
            Arg::new("release-version")
                .long("release-version")
                .value_name("version"),
        )
        .arg(
            Arg::new("release-asset")
                .long("release-asset")
                .value_name("name"),
        )
        .arg(
            Arg::new("extension-id")
                .long("extension-id")
                .value_name("id"),
        )
        .arg(Arg::new("node").long("node").value_name("path"))
        .arg(Arg::new("name").long("name").value_name("name"))
        .arg(
            Arg::new("user-data-dir")
                .long("user-data-dir")
                .value_name("path"),
        )
}

fn run_help(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let mut cli = build_cli();
    let help_text = if let Some(command) = sub.get_one::<String>("command") {
        let cmd = cli
            .find_subcommand_mut(command)
            .ok_or_else(|| format!("Unknown command: {command}"))?;
        let mut buf = Vec::new();
        cmd.write_long_help(&mut buf).map_err(|e| e.to_string())?;
        String::from_utf8(buf).map_err(|e| e.to_string())?
    } else {
        let mut buf = Vec::new();
        cli.write_long_help(&mut buf).map_err(|e| e.to_string())?;
        String::from_utf8(buf).map_err(|e| e.to_string())?
    };

    if matches.get_flag("json") {
        let data = json!({
            "command": sub.get_one::<String>("command"),
            "text": help_text
        });
        return render_local_command(matches, "help", data);
    }
    println!("{help_text}");
    Ok(())
}

fn run_version(matches: &ArgMatches, _sub: &ArgMatches) -> Result<(), String> {
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

fn load_profile_registry(
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
        return Err("profiles.json not found. Run `tabctl setup --browser <edge|chrome> --extension-id <id>` first.".to_string());
    }
    let content = fs::read_to_string(&profiles_path)
        .map_err(|e| format!("failed to read profiles.json: {e}"))?;
    let registry = serde_json::from_str::<ProfileRegistry>(&content)
        .map_err(|e| format!("failed to parse profiles.json: {e}"))?;
    Ok((config_dir, profiles_path, registry))
}

fn save_profile_registry(
    config_dir: &str,
    profiles_path: &Path,
    registry: &ProfileRegistry,
) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    let content = serde_json::to_string_pretty(registry).map_err(|e| e.to_string())?;
    fs::write(profiles_path, content).map_err(|e| format!("failed to write profiles.json: {e}"))
}

fn run_profile_list(matches: &ArgMatches, _sub: &ArgMatches) -> Result<(), String> {
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

fn run_profile_show(matches: &ArgMatches, _sub: &ArgMatches) -> Result<(), String> {
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

fn run_profile_switch(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
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

fn run_profile_remove(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
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

fn run_doctor(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
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

struct DoctorProfileEvaluation {
    report: Value,
    healthy: bool,
    repaired: bool,
    connectivity_healthy: bool,
}

fn evaluate_doctor_profile(
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

    let healthy = static_healthy && connectivity_healthy;
    report["healthy"] = Value::Bool(healthy);
    Ok(DoctorProfileEvaluation {
        report,
        healthy,
        repaired,
        connectivity_healthy,
    })
}

fn run_policy(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
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

fn run_skill(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let mut args = vec![
        "skills".to_string(),
        "add".to_string(),
        "https://github.com/ekroon/tabctl".to_string(),
        "--skill".to_string(),
        "tabctl".to_string(),
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

fn browser_name(browser: &Browser) -> &'static str {
    match browser {
        Browser::Edge => "edge",
        Browser::Chrome => "chrome",
    }
}

fn command_path_healthy(path: &str) -> bool {
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

fn profile_connectivity_report(profile_name: &str, entry: &ProfileEntry) -> Value {
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

fn connectivity_manual_steps(profile_name: &str, entry: &ProfileEntry) -> Vec<String> {
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

fn profile_health_report(profile_name: &str, entry: &ProfileEntry) -> Result<Value, String> {
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

fn attempt_profile_repair(profile_name: &str, entry: &ProfileEntry) -> Result<Value, String> {
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

fn run_setup(matches: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let browser = sub
        .get_one::<String>("browser")
        .map(|v| v.as_str())
        .ok_or_else(|| "Missing --browser".to_string())?;
    if browser != "edge" && browser != "chrome" {
        return Err("Missing or invalid --browser (edge|chrome)".to_string());
    }
    let release_source = resolve_setup_release_source(sub)?;
    let mut setup_warnings = Vec::new();
    let extension_asset = if should_skip_extension_download(sub) {
        json!({
            "downloaded": false,
            "reason": "skipped",
            "source": release_source.to_json()
        })
    } else {
        match download_extension_asset(&release_source) {
            Ok(payload) => json!({ "downloaded": true, "asset": payload }),
            Err(error_message) => {
                let warning = json!({
                    "code": "extension_download_failed",
                    "message": error_message,
                    "url": release_source.url.clone(),
                });
                setup_warnings.push(warning.clone());
                json!({
                    "downloaded": false,
                    "reason": "download-failed",
                    "source": release_source.to_json(),
                    "fallback": {
                        "path": release_source.path.display().to_string()
                    },
                    "warning": warning
                })
            }
        }
    };
    let data_dir = resolve_data_dir(None)?;
    let runtime_env = if cfg!(windows) {
        "native-win32"
    } else if cfg!(target_os = "macos") {
        "native-darwin"
    } else {
        "native-linux"
    };
    let wrapper_path = resolve_tabctl_binary_path();
    let extension_id = sub.get_one::<String>("extension-id");

    let mut actual_wrapper_path = wrapper_path.clone();
    let mut actual_manifest_path = data_dir.clone();
    let mut is_default_profile = false;
    let mut profile_registry = None::<Value>;

    #[cfg(windows)]
    let mut registry_key_value = None::<String>;

    if let Some(ext_id) = extension_id {
        let profile_name = sub
            .get_one::<String>("name")
            .map(|s| s.as_str())
            .unwrap_or(browser);

        let profile_data_dir = PathBuf::from(&data_dir).join("profiles").join(profile_name);

        let wrapper_file = write_host_wrapper(&wrapper_path, profile_name, &profile_data_dir)?;

        let user_data_dir = sub.get_one::<String>("user-data-dir").map(|s| s.as_str());
        let manifest_path = write_native_manifest(browser, &wrapper_file, ext_id, user_data_dir)?;

        #[cfg(windows)]
        {
            registry_key_value = Some(write_registry_key(browser, &manifest_path)?);
        }

        actual_wrapper_path = wrapper_file.display().to_string();
        actual_manifest_path = manifest_path.display().to_string();

        let registry = register_profile(
            &data_dir,
            profile_name,
            browser,
            ext_id,
            &wrapper_file,
            &manifest_path,
        )?;
        is_default_profile = registry
            .get("default")
            .and_then(|v| v.as_str())
            .map(|d| d == profile_name)
            .unwrap_or(false);
        profile_registry = Some(registry);
    }

    let mut data = json!({
        "profileName": browser,
        "browser": browser,
        "runtimeEnv": runtime_env,
        "dataDir": data_dir,
        "wrapperPath": actual_wrapper_path,
        "manifestPath": actual_manifest_path,
        "hostArgs": ["host"],
        "extensionReleaseAsset": extension_asset,
        "warnings": setup_warnings
    });
    if let Some(id) = extension_id {
        data["extensionId"] = json!(id);
        data["allowedOrigins"] = json!([format!("chrome-extension://{id}/")]);
        data["isDefault"] = json!(is_default_profile);
        if let Some(ref reg) = profile_registry {
            data["profileRegistry"] = reg.clone();
        }
        #[cfg(windows)]
        if let Some(ref rk) = registry_key_value {
            data["registryKey"] = json!(rk);
        }
    }
    let setup_payload = json!({
        "ok": true,
        "action": "setup",
        "data": data
    });
    if matches.get_flag("json") {
        if !matches.get_flag("no-pretty") {
            println!(
                "{}",
                serde_json::to_string_pretty(&setup_payload).map_err(|e| e.to_string())?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(&setup_payload).map_err(|e| e.to_string())?
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&setup_payload["data"]).map_err(|e| e.to_string())?
        );
    }
    Ok(())
}

/// Resolve the path to the `tabctl` binary.
/// Prefers the current running executable; falls back to searching PATH.
fn resolve_tabctl_binary_path() -> String {
    if let Ok(exe) = std::env::current_exe() {
        let path_str = if let Ok(canonical) = dunce::canonicalize(&exe) {
            canonical.display().to_string()
        } else {
            exe.display().to_string()
        };
        return path_str;
    }
    // Fallback: look up "tabctl" in PATH
    let cmd = if cfg!(windows) { "where" } else { "which" };
    if let Ok(output) = ProcessCommand::new(cmd).arg("tabctl").output() {
        if output.status.success() {
            if let Ok(path) = String::from_utf8(output.stdout) {
                let trimmed = path.trim().to_string();
                if !trimmed.is_empty() {
                    return trimmed;
                }
            }
        }
    }
    "tabctl".to_string()
}

#[derive(Clone, Debug)]
struct ExtensionReleaseSource {
    repo: String,
    tag: String,
    asset: String,
    path: PathBuf,
    url: String,
}

impl ExtensionReleaseSource {
    fn to_json(&self) -> Value {
        json!({
            "repo": self.repo,
            "tag": self.tag,
            "asset": self.asset,
            "url": self.url,
            "path": self.path.display().to_string(),
            "version": self.tag.trim_start_matches('v'),
        })
    }
}

fn resolve_setup_release_source(sub: &ArgMatches) -> Result<ExtensionReleaseSource, String> {
    let tag_override = sub
        .get_one::<String>("release-tag")
        .cloned()
        .or_else(|| sub.get_one::<String>("release-version").cloned())
        .or_else(|| std::env::var("TABCTL_RELEASE_TAG").ok());
    resolve_extension_release_source(
        tag_override.as_deref(),
        resolve_setup_release_override(sub, "release-repo", "TABCTL_RELEASE_REPO").as_deref(),
        resolve_setup_release_override(sub, "release-asset", "TABCTL_RELEASE_ASSET").as_deref(),
        None,
    )
}

fn resolve_setup_release_override(
    sub: &ArgMatches,
    cli_key: &str,
    env_key: &str,
) -> Option<String> {
    sub.get_one::<String>(cli_key)
        .cloned()
        .or_else(|| std::env::var(env_key).ok())
}

fn should_skip_extension_download(sub: &ArgMatches) -> bool {
    sub.get_flag("skip-extension-download")
        || std::env::var("TABCTL_SETUP_FETCH_EXTENSION")
            .ok()
            .as_deref()
            == Some("0")
}

fn command_with_scope(name: &'static str) -> Command {
    Command::new(name)
        .arg(
            Arg::new("tab")
                .long("tab")
                .action(ArgAction::Append)
                .value_name("id"),
        )
        .arg(Arg::new("group").long("group").value_name("name"))
        .arg(
            Arg::new("group-id")
                .long("group-id")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("ungrouped")
                .long("ungrouped")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("window")
                .long("window")
                .value_name("id|active|last-focused|new"),
        )
        .arg(Arg::new("all").long("all").action(ArgAction::SetTrue))
        .arg(
            Arg::new("limit")
                .long("limit")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("offset")
                .long("offset")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("no-page")
                .long("no-page")
                .action(ArgAction::SetTrue),
        )
}

fn command_with_scope_no_all(name: &'static str) -> Command {
    Command::new(name)
        .arg(
            Arg::new("tab")
                .long("tab")
                .action(ArgAction::Append)
                .value_name("id"),
        )
        .arg(Arg::new("group").long("group").value_name("name"))
        .arg(
            Arg::new("group-id")
                .long("group-id")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("ungrouped")
                .long("ungrouped")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("window")
                .long("window")
                .value_name("id|active|last-focused|new"),
        )
        .arg(
            Arg::new("limit")
                .long("limit")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("offset")
                .long("offset")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("no-page")
                .long("no-page")
                .action(ArgAction::SetTrue),
        )
}

fn command_open() -> Command {
    command_with_scope("open")
        .arg(
            Arg::new("url")
                .long("url")
                .action(ArgAction::Append)
                .value_name("url"),
        )
        .arg(Arg::new("color").long("color").value_name("name"))
        .arg(
            Arg::new("before-tab")
                .long("before-tab")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("after-tab")
                .long("after-tab")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("after-group")
                .long("after-group")
                .value_name("name"),
        )
        .arg(
            Arg::new("new-window")
                .long("new-window")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("window-group")
                .long("window-group")
                .value_name("name"),
        )
        .arg(
            Arg::new("window-tab")
                .long("window-tab")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("window-url")
                .long("window-url")
                .value_name("substring"),
        )
        .arg(
            Arg::new("new-group")
                .long("new-group")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("allow-duplicates")
                .long("allow-duplicates")
                .action(ArgAction::SetTrue),
        )
}

fn command_group_update() -> Command {
    command_with_scope("group-update")
        .arg(Arg::new("title").long("title").value_name("name"))
        .arg(Arg::new("color").long("color").value_name("name"))
        .arg(
            Arg::new("collapsed")
                .long("collapsed")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("expanded")
                .long("expanded")
                .action(ArgAction::SetTrue),
        )
}

fn command_group_ungroup() -> Command {
    command_with_scope("group-ungroup")
}

fn command_group_assign() -> Command {
    command_with_scope("group-assign")
        .arg(Arg::new("create").long("create").action(ArgAction::SetTrue))
        .arg(Arg::new("color").long("color").value_name("name"))
        .arg(
            Arg::new("collapsed")
                .long("collapsed")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("expanded")
                .long("expanded")
                .action(ArgAction::SetTrue),
        )
}

fn command_merge_window() -> Command {
    Command::new("merge-window")
        .arg(
            Arg::new("from")
                .long("from")
                .required(true)
                .value_parser(value_parser!(i64)),
        )
        .arg(
            Arg::new("to")
                .long("to")
                .required(true)
                .value_parser(value_parser!(i64)),
        )
        .arg(
            Arg::new("close-source")
                .long("close-source")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("confirm")
                .long("confirm")
                .action(ArgAction::SetTrue),
        )
}

fn command_close() -> Command {
    command_with_scope_no_all("close")
        .arg(Arg::new("apply").long("apply").value_name("analysisId"))
        .arg(
            Arg::new("confirm")
                .long("confirm")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue),
        )
}

fn command_report() -> Command {
    command_with_scope("report")
        .arg(Arg::new("format").long("format").value_name("json|md|csv"))
        .arg(Arg::new("out").long("out").value_name("path"))
}

fn command_analyze() -> Command {
    command_with_scope("analyze")
        .arg(
            Arg::new("stale-days")
                .long("stale-days")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("window-title")
                .long("window-title")
                .action(ArgAction::SetTrue),
        )
}

fn command_dedupe() -> Command {
    command_with_scope("dedupe")
        .arg(
            Arg::new("stale-days")
                .long("stale-days")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("include-stale")
                .long("include-stale")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("window-title")
                .long("window-title")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("confirm")
                .long("confirm")
                .action(ArgAction::SetTrue),
        )
}

fn command_inspect() -> Command {
    command_with_scope("inspect")
        .arg(
            Arg::new("signal-config")
                .long("signal-config")
                .value_name("path"),
        )
        .arg(
            Arg::new("signal")
                .long("signal")
                .action(ArgAction::Append)
                .value_name("id"),
        )
        .arg(
            Arg::new("selector")
                .long("selector")
                .action(ArgAction::Append)
                .value_name("name=css|json"),
        )
        .arg(
            Arg::new("selector-attr")
                .long("selector-attr")
                .value_name("attr"),
        )
        .arg(
            Arg::new("signal-concurrency")
                .long("signal-concurrency")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("signal-timeout-ms")
                .long("signal-timeout-ms")
                .value_parser(value_parser!(u64))
                .value_name("ms"),
        )
        .arg(
            Arg::new("wait-for")
                .long("wait-for")
                .value_name("load|dom|settle|none"),
        )
        .arg(
            Arg::new("wait-timeout-ms")
                .long("wait-timeout-ms")
                .value_parser(value_parser!(u64))
                .value_name("ms"),
        )
}

fn command_screenshot() -> Command {
    command_with_scope("screenshot")
        .arg(Arg::new("mode").long("mode").value_name("viewport|full"))
        .arg(Arg::new("format").long("format").value_name("png|jpeg"))
        .arg(
            Arg::new("quality")
                .long("quality")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("tile-max-dim")
                .long("tile-max-dim")
                .value_parser(value_parser!(u64))
                .value_name("px"),
        )
        .arg(
            Arg::new("max-bytes")
                .long("max-bytes")
                .value_parser(value_parser!(u64))
                .value_name("n"),
        )
        .arg(
            Arg::new("wait-for")
                .long("wait-for")
                .value_name("load|dom|settle|none"),
        )
        .arg(
            Arg::new("wait-timeout-ms")
                .long("wait-timeout-ms")
                .value_parser(value_parser!(u64))
                .value_name("ms"),
        )
        .arg(Arg::new("out").long("out").value_name("dir"))
}

fn command_move_tab() -> Command {
    command_with_scope("move-tab")
        .arg(
            Arg::new("before-tab")
                .long("before-tab")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("after-tab")
                .long("after-tab")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("before-group")
                .long("before-group")
                .value_name("name"),
        )
        .arg(
            Arg::new("after-group")
                .long("after-group")
                .value_name("name"),
        )
        .arg(
            Arg::new("new-window")
                .long("new-window")
                .action(ArgAction::SetTrue),
        )
}

fn command_move_group() -> Command {
    command_with_scope("move-group")
        .arg(
            Arg::new("before-tab")
                .long("before-tab")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("after-tab")
                .long("after-tab")
                .value_parser(value_parser!(i64))
                .value_name("id"),
        )
        .arg(
            Arg::new("before-group")
                .long("before-group")
                .value_name("name"),
        )
        .arg(
            Arg::new("after-group")
                .long("after-group")
                .value_name("name"),
        )
        .arg(
            Arg::new("new-window")
                .long("new-window")
                .action(ArgAction::SetTrue),
        )
}

fn command_undo() -> Command {
    Command::new("undo")
        .arg(Arg::new("txid").value_name("txid").index(1))
        .arg(Arg::new("txid-flag").long("txid").value_name("txid"))
        .arg(Arg::new("latest").long("latest").action(ArgAction::SetTrue))
}

fn route_command(matches: &ArgMatches) -> Result<RoutedCommand, String> {
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

fn collect_scope_params(sub: &ArgMatches) -> Map<String, Value> {
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
    if sub.get_flag("no-page") {
        params.insert("page".to_string(), Value::Bool(false));
    }
    params
}

fn copy_opt_string(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Some(value) = sub.get_one::<String>(src) {
        out.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn copy_opt_i64(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Some(value) = sub.get_one::<i64>(src) {
        out.insert(key.to_string(), Value::from(*value));
    }
}

fn copy_opt_u64(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Some(value) = sub.get_one::<u64>(src) {
        out.insert(key.to_string(), Value::from(*value));
    }
}

fn copy_opt_bool(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Ok(Some(value)) = sub.try_get_one::<bool>(src) {
        if *value {
            out.insert(key.to_string(), Value::Bool(true));
        }
    }
}

fn copy_many_strings(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Some(values) = sub.get_many::<String>(src) {
        let entries: Vec<Value> = values.map(|v| Value::String(v.to_string())).collect();
        if !entries.is_empty() {
            out.insert(key.to_string(), Value::Array(entries));
        }
    }
}

fn copy_many_i64(sub: &ArgMatches, src: &str, out: &mut Map<String, Value>, key: &str) {
    if let Some(values) = sub.get_many::<String>(src) {
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
fn parse_selector_args<'a>(
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

fn resolve_socket_endpoint(profile: Option<&str>) -> Result<SocketEndpoint, String> {
    if let Ok(path) = std::env::var("TABCTL_SOCKET") {
        if !path.trim().is_empty() {
            let endpoint = SocketEndpoint::parse(&path)?;
            #[cfg(target_os = "linux")]
            if matches!(endpoint, SocketEndpoint::Pipe { .. }) && is_wsl_environment() {
                if let Some(tcp) = discover_wsl_tcp_endpoint(profile) {
                    return Ok(tcp);
                }
            }
            return Ok(endpoint);
        }
    }
    // Check TABCTL_TRANSPORT=tcp for explicit TCP transport selection
    if let Ok(transport) = std::env::var("TABCTL_TRANSPORT") {
        if transport.trim().eq_ignore_ascii_case("tcp") {
            if let Ok(port_str) = std::env::var("TABCTL_TCP_PORT") {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    if port > 0 {
                        return Ok(SocketEndpoint::Tcp {
                            host: "127.0.0.1".to_string(),
                            port,
                        });
                    }
                }
            }
            let data_dir = resolve_data_dir(profile)?;
            let port_path = PathBuf::from(&data_dir).join(WSL_TCP_PORT_FILENAME);
            if let Some(port) = read_tcp_port_file(&port_path) {
                return Ok(SocketEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port,
                });
            }
            return Err("TABCTL_TRANSPORT=tcp but no tcp-port file found. Is the host running with TABCTL_HOST_TCP=1?".to_string());
        }
    }
    #[cfg(target_os = "linux")]
    if is_wsl_environment() {
        if let Some(tcp) = discover_wsl_tcp_endpoint(profile) {
            return Ok(tcp);
        }
        return Ok(SocketEndpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: WSL_TCP_PORT_FALLBACK,
        });
    }
    let data_dir = resolve_data_dir(profile)?;
    #[cfg(windows)]
    {
        Ok(resolve_windows_pipe_endpoint(&data_dir))
    }
    #[cfg(not(windows))]
    {
        SocketEndpoint::parse(&format!("{data_dir}/tabctl.sock"))
    }
}

#[cfg(target_os = "linux")]
fn is_wsl_environment() -> bool {
    std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/version")
            .map(|content| content.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn discover_wsl_tcp_endpoint(profile: Option<&str>) -> Option<SocketEndpoint> {
    if let Ok(value) = std::env::var("TABCTL_TCP_PORT") {
        if let Ok(port) = value.trim().parse::<u16>() {
            if port > 0 {
                return Some(SocketEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port,
                });
            }
        }
    }
    let data_dir = resolve_data_dir(profile).ok()?;
    discover_wsl_tcp_port_from_data_dir(&data_dir).map(|port| SocketEndpoint::Tcp {
        host: "127.0.0.1".to_string(),
        port,
    })
}

#[cfg(target_os = "linux")]
fn discover_wsl_tcp_port_from_data_dir(data_dir: &str) -> Option<u16> {
    for path in wsl_tcp_port_candidates(data_dir) {
        if let Some(port) = read_tcp_port_file(&path) {
            return Some(port);
        }
    }
    None
}

#[cfg(any(target_os = "linux", test))]
fn resolve_windows_username_from_path() -> Option<String> {
    let path_env = std::env::var("PATH").ok()?;
    for entry in path_env.split(':') {
        let lower = entry.to_lowercase();
        if let Some(pos) = lower.find("/mnt/c/users/") {
            let after_prefix = &entry[pos + "/mnt/c/Users/".len()..];
            let username = after_prefix.split('/').next()?;
            if !username.is_empty() {
                return Some(username.to_string());
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", test))]
fn resolve_windows_appdata_local() -> Option<PathBuf> {
    let username = resolve_windows_username_from_path()?;
    let path = PathBuf::from(format!("/mnt/c/Users/{username}/AppData/Local"));
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

#[cfg(any(target_os = "linux", test))]
fn wsl_file_candidates(data_dir: &str, filename: &str) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(data_dir).join(filename)];
    let Some(relative_suffix) = tabctl_relative_suffix(Path::new(data_dir)) else {
        return candidates;
    };
    let Some(appdata_local) = resolve_windows_appdata_local() else {
        return candidates;
    };
    candidates.push(
        appdata_local
            .join("tabctl")
            .join(&relative_suffix)
            .join(filename),
    );
    candidates.push(
        appdata_local
            .join("tabctl-state")
            .join("tabctl")
            .join(&relative_suffix)
            .join(filename),
    );
    candidates
}

#[cfg(target_os = "linux")]
fn wsl_tcp_port_candidates(data_dir: &str) -> Vec<PathBuf> {
    wsl_file_candidates(data_dir, WSL_TCP_PORT_FILENAME)
}

#[cfg(any(target_os = "linux", test))]
fn tabctl_relative_suffix(path: &Path) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    let mut found = false;
    for component in path.components() {
        if found {
            relative.push(component.as_os_str());
            continue;
        }
        if component.as_os_str() == "tabctl" {
            found = true;
        }
    }
    found.then_some(relative)
}

fn read_tcp_port_file(path: &Path) -> Option<u16> {
    let content = fs::read_to_string(path).ok()?;
    let port = content.trim().parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

#[cfg(windows)]
fn resolve_windows_pipe_endpoint(data_dir: &str) -> SocketEndpoint {
    let mut hasher = Sha256::new();
    hasher.update(data_dir.as_bytes());
    let digest = hasher.finalize();
    let hash = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    SocketEndpoint::Pipe {
        path: format!(r"\\.\pipe\tabctl-{hash}"),
    }
}

fn resolve_data_dir(profile: Option<&str>) -> Result<String, String> {
    if let Ok(path) = std::env::var("TABCTL_DATA_DIR") {
        if !path.trim().is_empty() {
            return Ok(path);
        }
    }
    let config_dir = resolve_config_dir()?;
    if let Some(profile_name) = profile {
        let profiles_path = PathBuf::from(&config_dir).join("profiles.json");
        if let Ok(contents) = fs::read_to_string(profiles_path) {
            if let Ok(registry) = serde_json::from_str::<ProfileRegistry>(&contents) {
                if let Some(profile_entry) = registry.profiles.get(profile_name) {
                    return Ok(profile_entry.data_dir.clone());
                }
            }
        }
        return Err(format!(
            "Profile \"{profile_name}\" not found in profiles.json"
        ));
    }
    if let Ok(path) = std::env::var("TABCTL_STATE_DIR") {
        if !path.trim().is_empty() {
            return Ok(path);
        }
    }
    if let Ok(path) = std::env::var("XDG_STATE_HOME") {
        return Ok(format!("{path}/tabctl"));
    }
    let home = dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
    Ok(format!("{}/.local/state/tabctl", home.display()))
}

fn resolve_config_dir() -> Result<String, String> {
    if let Ok(path) = std::env::var("TABCTL_CONFIG_DIR") {
        return Ok(path);
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(format!("{path}/tabctl"));
    }
    let home = dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
    Ok(format!("{}/.config/tabctl", home.display()))
}

fn resolve_effective_profile(profile: Option<&str>) -> Option<String> {
    if let Some(name) = profile {
        return Some(name.to_string());
    }
    let config_dir = resolve_config_dir().ok()?;
    let profiles_path = PathBuf::from(config_dir).join("profiles.json");
    let contents = fs::read_to_string(profiles_path).ok()?;
    let registry = serde_json::from_str::<ProfileRegistry>(&contents).ok()?;
    registry
        .default
        .or_else(|| registry.profiles.keys().next().cloned())
}

fn read_auth_token(profile: Option<&str>) -> Option<String> {
    if let Ok(token) = std::env::var("TABCTL_AUTH_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }

    let data_dir = resolve_data_dir(profile).ok()?;

    #[cfg(target_os = "linux")]
    if is_wsl_environment() {
        for path in wsl_file_candidates(&data_dir, AUTH_TOKEN_FILENAME) {
            if let Ok(content) = fs::read_to_string(&path) {
                let token = content.trim().to_string();
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
        return None;
    }

    let path = PathBuf::from(&data_dir).join(AUTH_TOKEN_FILENAME);
    fs::read_to_string(&path).ok().and_then(|content| {
        let token = content.trim().to_string();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    })
}

fn register_profile(
    data_dir: &str,
    profile_name: &str,
    browser: &str,
    extension_id: &str,
    wrapper_path: &Path,
    _manifest_path: &Path,
) -> Result<Value, String> {
    let config_dir = resolve_config_dir()?;
    let profiles_path = PathBuf::from(&config_dir).join("profiles.json");

    let mut registry = if profiles_path.exists() {
        let contents = fs::read_to_string(&profiles_path)
            .map_err(|e| format!("failed to read profiles.json: {e}"))?;
        serde_json::from_str::<ProfileRegistry>(&contents)
            .map_err(|e| format!("failed to parse profiles.json: {e}"))?
    } else {
        ProfileRegistry {
            default: None,
            profiles: HashMap::new(),
        }
    };

    let browser_enum = match browser {
        "edge" => Browser::Edge,
        "chrome" => Browser::Chrome,
        _ => return Err(format!("unsupported browser: {browser}")),
    };

    let profile_data_dir = PathBuf::from(data_dir).join("profiles").join(profile_name);

    let entry = ProfileEntry {
        browser: browser_enum,
        extension_id: extension_id.to_string(),
        node_path: resolve_tabctl_binary_path(),
        host_path: wrapper_path.display().to_string(),
        data_dir: profile_data_dir.display().to_string(),
        user_data_dir: None,
    };

    // First registered profile becomes the default
    if registry.default.is_none() || registry.profiles.is_empty() {
        registry.default = Some(profile_name.to_string());
    }

    registry.profiles.insert(profile_name.to_string(), entry);

    fs::create_dir_all(&config_dir).map_err(|e| format!("failed to create config dir: {e}"))?;

    let content = serde_json::to_string_pretty(&registry).map_err(|e| e.to_string())?;
    fs::write(&profiles_path, content)
        .map_err(|e| format!("failed to write profiles.json: {e}"))?;

    serde_json::to_value(&registry).map_err(|e| e.to_string())
}

const HOST_NAME: &str = "com.erwinkroon.tabctl";

fn resolve_manifest_dir(browser: &str) -> Result<PathBuf, String> {
    match browser {
        "edge" | "chrome" => {}
        _ => return Err(format!("unsupported browser: {browser}")),
    }

    #[cfg(target_os = "windows")]
    {
        let data_dir = resolve_data_dir(None)?;
        Ok(PathBuf::from(data_dir))
    }

    #[cfg(target_os = "macos")]
    {
        let home =
            dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
        let subdir = match browser {
            "edge" => "Microsoft Edge",
            _ => "Google/Chrome",
        };
        Ok(home
            .join("Library/Application Support")
            .join(subdir)
            .join("NativeMessagingHosts"))
    }

    #[cfg(target_os = "linux")]
    {
        let home =
            dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
        let subdir = match browser {
            "edge" => "microsoft-edge",
            _ => "google-chrome",
        };
        Ok(home
            .join(".config")
            .join(subdir)
            .join("NativeMessagingHosts"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err(format!("unsupported platform"))
}

fn write_host_wrapper(
    tabctl_binary_path: &str,
    profile_name: &str,
    wrapper_dir: &Path,
) -> Result<PathBuf, String> {
    fs::create_dir_all(wrapper_dir).map_err(|e| format!("failed to create wrapper dir: {e}"))?;
    let config_dir = resolve_config_dir()?;
    let data_dir = resolve_data_dir(None)?;

    #[cfg(unix)]
    let (filename, content) = {
        let script = format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nexport TABCTL_PROFILE=\"{profile_name}\"\nexport TABCTL_CONFIG_DIR=\"{config_dir}\"\nexport TABCTL_DATA_DIR=\"{data_dir}\"\nexec \"{tabctl_binary_path}\" host\n"
        );
        ("tabctl-host.sh", script)
    };

    #[cfg(windows)]
    let (filename, content) = {
        let script = format!(
            "@echo off\r\nset TABCTL_PROFILE={profile_name}\r\nset TABCTL_CONFIG_DIR={config_dir}\r\nset TABCTL_DATA_DIR={data_dir}\r\n\"{tabctl_binary_path}\" host\r\n"
        );
        ("tabctl-host.cmd", script)
    };

    let wrapper_path = wrapper_dir.join(filename);
    fs::write(&wrapper_path, &content)
        .map_err(|e| format!("failed to write wrapper script: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&wrapper_path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("failed to set wrapper permissions: {e}"))?;
    }

    Ok(wrapper_path)
}

#[cfg(windows)]
fn write_registry_key(browser: &str, manifest_path: &Path) -> Result<String, String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let subkey = match browser {
        "edge" => format!("Software\\Microsoft\\Edge\\NativeMessagingHosts\\{HOST_NAME}"),
        "chrome" => format!("Software\\Google\\Chrome\\NativeMessagingHosts\\{HOST_NAME}"),
        _ => return Err(format!("Unsupported browser for registry: {browser}")),
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(&subkey)
        .map_err(|e| format!("Failed to create registry key: {e}"))?;

    key.set_value("", &manifest_path.display().to_string())
        .map_err(|e| format!("Failed to set registry value: {e}"))?;

    Ok(format!("HKCU\\{subkey}"))
}

fn write_native_manifest(
    browser: &str,
    wrapper_path: &Path,
    extension_id: &str,
    user_data_dir: Option<&str>,
) -> Result<PathBuf, String> {
    let manifest_dir = if let Some(udd) = user_data_dir {
        PathBuf::from(udd).join("NativeMessagingHosts")
    } else {
        resolve_manifest_dir(browser)?
    };
    fs::create_dir_all(&manifest_dir).map_err(|e| format!("failed to create manifest dir: {e}"))?;

    let abs_wrapper =
        dunce::canonicalize(wrapper_path).unwrap_or_else(|_| wrapper_path.to_path_buf());

    let manifest = json!({
        "name": HOST_NAME,
        "description": "tabctl native host",
        "path": abs_wrapper.display().to_string(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")]
    });

    let manifest_path = manifest_dir.join(format!("{HOST_NAME}.json"));
    let content = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(&manifest_path, content)
        .map_err(|e| format!("failed to write native manifest: {e}"))?;

    Ok(manifest_path)
}

fn request_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("req-{now}-{}", std::process::id())
}

fn send_request(
    action: &str,
    params: Value,
    profile: Option<&str>,
    show_progress: bool,
) -> Result<ResponseEnvelope, String> {
    let effective_profile = resolve_effective_profile(profile);
    let resolved_profile = effective_profile.as_deref();
    let endpoint = resolve_socket_endpoint(resolved_profile)?;
    match endpoint {
        SocketEndpoint::Unix { path } => {
            #[cfg(unix)]
            {
                let stream = UnixStream::connect(path)
                    .map_err(|e| format!("Failed to connect to host: {e}"))?;
                send_request_over_stream(stream, action, params, show_progress, None)
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                Err("Unix socket transport is unsupported on this target".to_string())
            }
        }
        SocketEndpoint::Tcp { host, port } => {
            let auth_token = read_auth_token(resolved_profile);
            let stream = TcpStream::connect((host.as_str(), port))
                .map_err(|e| format!("Failed to connect to host: {e}"))?;
            send_request_over_stream(stream, action, params, show_progress, auth_token)
        }
        SocketEndpoint::Pipe { path } => {
            #[cfg(windows)]
            {
                let stream = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(|e| format!("Failed to connect to host: {e}"))?;
                send_request_over_stream(stream, action, params, show_progress, None)
            }
            #[cfg(not(windows))]
            {
                let _ = path;
                Err("Named pipe transport is unsupported on this target".to_string())
            }
        }
    }
}

fn send_request_over_stream<S>(
    mut stream: S,
    action: &str,
    params: Value,
    show_progress: bool,
    auth_token: Option<String>,
) -> Result<ResponseEnvelope, String>
where
    S: std::io::Read + Write,
{
    let request = RequestEnvelope {
        id: Some(request_id()),
        action: action.to_string(),
        params,
        auth_token,
    };
    serde_json::to_writer(&mut stream, &request)
        .map_err(|e| format!("Failed to encode request: {e}"))?;
    stream
        .write_all(b"\n")
        .map_err(|e| format!("Failed to send request: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush request: {e}"))?;

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line.map_err(|e| format!("Failed to read response: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let response: ResponseEnvelope =
            serde_json::from_str(&line).map_err(|e| format!("Invalid response payload: {e}"))?;
        if response.progress.unwrap_or(false) {
            if show_progress {
                let data = response.data.unwrap_or(json!({}));
                eprintln!("[tabctl] progress: {}", data);
            }
            continue;
        }
        return Ok(response);
    }
    Err("No response received".to_string())
}

fn render_response(
    response: &ResponseEnvelope,
    json_mode: bool,
    pretty: bool,
) -> Result<(), String> {
    if json_mode {
        if pretty {
            println!(
                "{}",
                serde_json::to_string_pretty(response).map_err(|e| e.to_string())?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(response).map_err(|e| e.to_string())?
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_env_vars<F>(vars: &[(&str, Option<&str>)], run: F)
    where
        F: FnOnce(),
    {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved: Vec<(String, Option<OsString>)> = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), std::env::var_os(key)))
            .collect();
        for (key, value) in vars {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        for (key, value) in saved {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    #[test]
    fn resolve_effective_profile_prefers_explicit_profile() {
        assert_eq!(
            resolve_effective_profile(Some("edge-smoke")),
            Some("edge-smoke".to_string())
        );
    }

    #[test]
    fn resolve_effective_profile_uses_registry_default() {
        let dir =
            std::env::temp_dir().join(format!("tabctl-test-effective-profile-{}", request_id()));
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let profiles = json!({
            "default": "edge",
            "profiles": {
                "edge": {
                    "browser": "edge",
                    "extensionId": "mpglnmehddpkinfhheeahiicfieegcon",
                    "nodePath": "/tmp/tabctl",
                    "hostPath": "/tmp/tabctl-host.sh",
                    "dataDir": "/tmp/tabctl/profiles/edge"
                }
            }
        });
        fs::write(
            config_dir.join("profiles.json"),
            serde_json::to_string_pretty(&profiles).expect("serialize profiles"),
        )
        .expect("write profiles.json");

        with_env_vars(
            &[
                (
                    "TABCTL_CONFIG_DIR",
                    Some(config_dir.to_str().expect("config dir path")),
                ),
                ("XDG_CONFIG_HOME", None),
            ],
            || {
                assert_eq!(resolve_effective_profile(None), Some("edge".to_string()));
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn routes_group_alias_to_group_list_action() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "group", "--window", "12"])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "group-list");
        assert_eq!(routed.params["windowId"], "12");
    }

    #[test]
    fn maps_close_flags_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "close", "--tab", "1", "--confirm", "--dry-run"])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "close");
        assert_eq!(routed.params["tabIds"], json!([1]));
        assert_eq!(routed.params["confirmed"], json!(true));
        assert_eq!(routed.params["dryRun"], json!(true));
    }

    #[test]
    fn close_rejects_all_scope_flag() {
        let err = build_cli()
            .try_get_matches_from(["tabctl", "close", "--all", "--confirm"])
            .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("--all"),
            "expected error mentioning --all, got: {message}"
        );
    }

    #[test]
    fn supports_dedupe_alias_with_analyze_action() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "dedupe", "--window", "active"])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "analyze");
        assert_eq!(routed.params["dedupe"], json!(true));
    }

    #[test]
    fn routes_global_rendering_flags_and_profile() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "--json",
                "--no-pretty",
                "--profile",
                "edge-work",
                "list",
                "--all",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert!(routed.json);
        assert!(!routed.pretty);
        assert_eq!(routed.profile.as_deref(), Some("edge-work"));
        assert_eq!(routed.params["all"], json!(true));
    }

    #[test]
    fn maps_no_page_and_valid_tab_ids_only() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "list",
                "--tab",
                "11",
                "--tab",
                "bad",
                "--tab",
                "14",
                "--no-page",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.params["tabIds"], json!([11, 14]));
        assert_eq!(routed.params["page"], json!(false));
    }

    #[test]
    fn list_groups_flag_routes_to_group_list_action() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "list", "--groups", "--window", "12"])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "group-list");
        assert_eq!(routed.params["windowId"], "12");
    }

    #[test]
    fn parses_doctor_fix_flag() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "doctor", "--fix"])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "doctor");
        assert!(sub.get_flag("fix"));
    }

    #[test]
    fn parses_policy_init_flag() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "policy", "--init"])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "policy");
        assert!(sub.get_flag("init"));
    }

    #[test]
    fn parses_skill_flags() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "skill",
                "--global",
                "--agent",
                "opencode",
                "--agent",
                "github-copilot",
            ])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "skill");
        assert!(sub.get_flag("global"));
        let agents = sub
            .get_many::<String>("agent")
            .expect("agents")
            .map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(agents, vec!["opencode", "github-copilot"]);
    }

    #[test]
    fn policy_init_creates_default_file() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-policy-init-{}", request_id()));
        let config_dir = dir.join("config");
        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let matches = build_cli()
                    .try_get_matches_from(["tabctl", "policy", "--init"])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                run_policy(&matches, sub).expect("run policy");

                let policy_path = config_dir.join("policy.json");
                assert!(policy_path.exists(), "policy.json should be created");
                let content = fs::read_to_string(policy_path).expect("read policy");
                let policy: Value = serde_json::from_str(&content).expect("parse policy");
                assert_eq!(policy["protect"]["pinned"], json!(true));
                assert_eq!(policy["protect"]["groupTitles"], json!(["🔒"]));
            },
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_help_and_version_subcommands() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "help", "open"])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "help");
        assert_eq!(
            sub.get_one::<String>("command").map(String::as_str),
            Some("open")
        );

        let matches = build_cli()
            .try_get_matches_from(["tabctl", "version"])
            .expect("parse command");
        let (command, _) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "version");
    }

    #[test]
    fn parses_profile_management_subcommands() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "profile-list"])
            .expect("parse command");
        assert_eq!(matches.subcommand().expect("subcommand").0, "profile-list");

        let matches = build_cli()
            .try_get_matches_from(["tabctl", "profile-show"])
            .expect("parse command");
        assert_eq!(matches.subcommand().expect("subcommand").0, "profile-show");

        let matches = build_cli()
            .try_get_matches_from(["tabctl", "profile-switch", "work"])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "profile-switch");
        assert_eq!(
            sub.get_one::<String>("name").map(String::as_str),
            Some("work")
        );

        let matches = build_cli()
            .try_get_matches_from(["tabctl", "profile-remove", "work"])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "profile-remove");
        assert_eq!(
            sub.get_one::<String>("name").map(String::as_str),
            Some("work")
        );
    }

    #[test]
    fn profile_switch_updates_default_profile() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-profile-switch-{}", request_id()));
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let profiles_path = config_dir.join("profiles.json");
        fs::write(
            &profiles_path,
            serde_json::to_string_pretty(&json!({
                "default": "edge",
                "profiles": {
                    "edge": {
                        "browser": "edge",
                        "extensionId": "ext-edge",
                        "nodePath": "/usr/bin/tabctl",
                        "hostPath": "/tmp/tabctl-edge-host.sh",
                        "dataDir": "/tmp/tabctl/profiles/edge"
                    },
                    "chrome": {
                        "browser": "chrome",
                        "extensionId": "ext-chrome",
                        "nodePath": "/usr/bin/tabctl",
                        "hostPath": "/tmp/tabctl-chrome-host.sh",
                        "dataDir": "/tmp/tabctl/profiles/chrome"
                    }
                }
            }))
            .expect("serialize profiles"),
        )
        .expect("write profiles");

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let matches = build_cli()
                    .try_get_matches_from(["tabctl", "profile-switch", "chrome"])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                run_profile_switch(&matches, sub).expect("run profile-switch");
            },
        );

        let updated: Value = serde_json::from_str(
            &fs::read_to_string(&profiles_path).expect("read updated profiles"),
        )
        .expect("parse updated profiles");
        assert_eq!(updated["default"], json!("chrome"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_remove_reassigns_default_profile() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-profile-remove-{}", request_id()));
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let profiles_path = config_dir.join("profiles.json");
        fs::write(
            &profiles_path,
            serde_json::to_string_pretty(&json!({
                "default": "edge",
                "profiles": {
                    "edge": {
                        "browser": "edge",
                        "extensionId": "ext-edge",
                        "nodePath": "/usr/bin/tabctl",
                        "hostPath": "/tmp/tabctl-edge-host.sh",
                        "dataDir": "/tmp/tabctl/profiles/edge"
                    },
                    "chrome": {
                        "browser": "chrome",
                        "extensionId": "ext-chrome",
                        "nodePath": "/usr/bin/tabctl",
                        "hostPath": "/tmp/tabctl-chrome-host.sh",
                        "dataDir": "/tmp/tabctl/profiles/chrome"
                    }
                }
            }))
            .expect("serialize profiles"),
        )
        .expect("write profiles");

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let matches = build_cli()
                    .try_get_matches_from(["tabctl", "profile-remove", "edge"])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                run_profile_remove(&matches, sub).expect("run profile-remove");
            },
        );

        let updated: Value = serde_json::from_str(
            &fs::read_to_string(&profiles_path).expect("read updated profiles"),
        )
        .expect("parse updated profiles");
        assert_eq!(updated["default"], json!("chrome"));
        assert!(updated["profiles"].get("edge").is_none());
        assert!(updated["profiles"].get("chrome").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_extension_fetch_command() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "extension-fetch", "--version", "0.5.3"])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "extension-fetch");
        assert_eq!(
            sub.get_one::<String>("version").map(String::as_str),
            Some("0.5.3")
        );
    }

    #[test]
    fn parses_setup_command_with_extension_toggle() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "setup",
                "--browser",
                "edge",
                "--skip-extension-download",
            ])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "setup");
        assert_eq!(
            sub.get_one::<String>("browser").map(String::as_str),
            Some("edge")
        );
        assert!(sub.get_flag("skip-extension-download"));
    }

    #[test]
    fn parses_setup_release_override_flags() {
        with_env_vars(
            &[
                ("TABCTL_RELEASE_REPO", None),
                ("TABCTL_RELEASE_TAG", None),
                ("TABCTL_RELEASE_ASSET", None),
            ],
            || {
                let matches = build_cli()
                    .try_get_matches_from([
                        "tabctl",
                        "setup",
                        "--browser",
                        "edge",
                        "--release-repo",
                        "octo/tabctl",
                        "--release-version",
                        "v1.2.3",
                        "--release-asset",
                        "custom.zip",
                    ])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                let source =
                    resolve_setup_release_source(sub).expect("resolve setup release source");
                assert_eq!(source.repo, "octo/tabctl");
                assert_eq!(source.tag, "v1.2.3");
                assert_eq!(source.asset, "custom.zip");
            },
        );
    }

    #[test]
    fn setup_release_overrides_use_env_defaults() {
        with_env_vars(
            &[
                ("TABCTL_RELEASE_REPO", Some("env/tabctl")),
                ("TABCTL_RELEASE_TAG", Some("2.0.0")),
                ("TABCTL_RELEASE_ASSET", Some("env-extension.zip")),
            ],
            || {
                let matches = build_cli()
                    .try_get_matches_from(["tabctl", "setup", "--browser", "edge"])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                let source =
                    resolve_setup_release_source(sub).expect("resolve setup release source");
                assert_eq!(source.repo, "env/tabctl");
                assert_eq!(source.tag, "v2.0.0");
                assert_eq!(source.asset, "env-extension.zip");
            },
        );
    }

    #[test]
    fn setup_release_cli_overrides_env() {
        with_env_vars(
            &[
                ("TABCTL_RELEASE_REPO", Some("env/tabctl")),
                ("TABCTL_RELEASE_TAG", Some("2.0.0")),
                ("TABCTL_RELEASE_ASSET", Some("env-extension.zip")),
            ],
            || {
                let matches = build_cli()
                    .try_get_matches_from([
                        "tabctl",
                        "setup",
                        "--browser",
                        "edge",
                        "--release-repo",
                        "cli/tabctl",
                        "--release-tag",
                        "3.0.1",
                        "--release-asset",
                        "cli-extension.zip",
                    ])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                let source =
                    resolve_setup_release_source(sub).expect("resolve setup release source");
                assert_eq!(source.repo, "cli/tabctl");
                assert_eq!(source.tag, "v3.0.1");
                assert_eq!(source.asset, "cli-extension.zip");
            },
        );
    }

    #[test]
    fn setup_release_version_flag_overrides_env_tag() {
        with_env_vars(&[("TABCTL_RELEASE_TAG", Some("2.0.0"))], || {
            let matches = build_cli()
                .try_get_matches_from([
                    "tabctl",
                    "setup",
                    "--browser",
                    "edge",
                    "--release-version",
                    "4.1.0",
                ])
                .expect("parse command");
            let (_, sub) = matches.subcommand().expect("subcommand");
            let source = resolve_setup_release_source(sub).expect("resolve setup release source");
            assert_eq!(source.tag, "v4.1.0");
        });
    }

    #[test]
    fn setup_release_tag_takes_priority_over_release_version() {
        with_env_vars(&[("TABCTL_RELEASE_TAG", None)], || {
            let matches = build_cli()
                .try_get_matches_from([
                    "tabctl",
                    "setup",
                    "--browser",
                    "edge",
                    "--release-tag",
                    "v5.0.0",
                    "--release-version",
                    "4.9.9",
                ])
                .expect("parse command");
            let (_, sub) = matches.subcommand().expect("subcommand");
            let source = resolve_setup_release_source(sub).expect("resolve setup release source");
            assert_eq!(source.tag, "v5.0.0");
        });
    }

    #[test]
    fn setup_skip_extension_download_uses_env_toggle() {
        with_env_vars(&[("TABCTL_SETUP_FETCH_EXTENSION", Some("0"))], || {
            let matches = build_cli()
                .try_get_matches_from(["tabctl", "setup", "--browser", "edge"])
                .expect("parse command");
            let (_, sub) = matches.subcommand().expect("subcommand");
            assert!(should_skip_extension_download(sub));
        });
    }

    #[test]
    fn setup_skip_extension_download_only_uses_zero_env_toggle() {
        with_env_vars(&[("TABCTL_SETUP_FETCH_EXTENSION", Some("1"))], || {
            let matches = build_cli()
                .try_get_matches_from(["tabctl", "setup", "--browser", "edge"])
                .expect("parse command");
            let (_, sub) = matches.subcommand().expect("subcommand");
            assert!(!should_skip_extension_download(sub));
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn discovers_wsl_tcp_port_from_data_dir_file() {
        let temp_root = std::env::temp_dir().join(format!("tabctl-cli-test-{}", request_id()));
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        let port_file = temp_root.join(WSL_TCP_PORT_FILENAME);
        std::fs::write(&port_file, "39001\n").expect("write port file");
        let port = discover_wsl_tcp_port_from_data_dir(
            temp_root.to_str().expect("temp path should be valid utf-8"),
        );
        std::fs::remove_dir_all(&temp_root).expect("remove temp directory");
        assert_eq!(port, Some(39001));
    }

    #[test]
    fn rejects_invalid_wsl_tcp_port_file_values() {
        let temp_root = std::env::temp_dir().join(format!("tabctl-cli-test-{}", request_id()));
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        let port_file = temp_root.join(WSL_TCP_PORT_FILENAME);
        std::fs::write(&port_file, "not-a-port").expect("write invalid port");
        let port = read_tcp_port_file(&port_file);
        std::fs::remove_dir_all(&temp_root).expect("remove temp directory");
        assert_eq!(port, None);
    }

    #[test]
    fn resolves_windows_username_from_path_env() {
        with_env_vars(
            &[(
                "PATH",
                Some("/usr/bin:/mnt/c/Users/TestUser/AppData/Local/bin:/usr/local/bin"),
            )],
            || {
                let username = resolve_windows_username_from_path();
                assert_eq!(username, Some("TestUser".to_string()));
            },
        );
    }

    #[test]
    fn resolves_windows_username_case_insensitive_prefix() {
        with_env_vars(
            &[(
                "PATH",
                Some("/usr/bin:/mnt/C/Users/MyUser/AppData/Local/bin"),
            )],
            || {
                let username = resolve_windows_username_from_path();
                assert_eq!(username, Some("MyUser".to_string()));
            },
        );
    }

    #[test]
    fn returns_none_when_path_has_no_windows_entry() {
        with_env_vars(&[("PATH", Some("/usr/bin:/usr/local/bin"))], || {
            let username = resolve_windows_username_from_path();
            assert_eq!(username, None);
        });
    }

    #[test]
    fn wsl_file_candidates_returns_data_dir_only_without_tabctl_segment() {
        with_env_vars(
            &[("PATH", Some("/mnt/c/Users/TestUser/AppData/Local/bin"))],
            || {
                let candidates = wsl_file_candidates("/some/path/without/marker", "tcp-port");
                assert_eq!(
                    candidates,
                    vec![PathBuf::from("/some/path/without/marker/tcp-port")]
                );
            },
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolves_pipe_socket_to_wsl_tcp_endpoint() {
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_SOCKET", Some("pipe://tabctl-test")),
                ("TABCTL_TCP_PORT", Some("39005")),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert_eq!(
                    endpoint,
                    SocketEndpoint::Tcp {
                        host: "127.0.0.1".to_string(),
                        port: 39005,
                    }
                );
            },
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn keeps_explicit_tcp_socket_in_wsl() {
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_SOCKET", Some("tcp://127.0.0.1:39006")),
                ("TABCTL_TCP_PORT", Some("39007")),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert_eq!(
                    endpoint,
                    SocketEndpoint::Tcp {
                        host: "127.0.0.1".to_string(),
                        port: 39006,
                    }
                );
            },
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn returns_invalid_socket_error_before_wsl_fallback() {
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_SOCKET", Some("tcp://127.0.0.1")),
                ("TABCTL_TCP_PORT", Some("39008")),
            ],
            || {
                let err = resolve_socket_endpoint(None).expect_err("invalid socket should fail");
                assert!(err.contains("TCP endpoint must include host and port"));
            },
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn falls_back_to_default_wsl_tcp_port_when_not_discovered() {
        let temp_root = std::env::temp_dir().join(format!("tabctl-cli-test-{}", request_id()));
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_SOCKET", None),
                ("TABCTL_TCP_PORT", None),
                (
                    "TABCTL_DATA_DIR",
                    Some(temp_root.to_str().expect("temp path should be valid utf-8")),
                ),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert_eq!(
                    endpoint,
                    SocketEndpoint::Tcp {
                        host: "127.0.0.1".to_string(),
                        port: WSL_TCP_PORT_FALLBACK,
                    }
                );
            },
        );
        if let Err(err) = std::fs::remove_dir_all(&temp_root) {
            if err.kind() != std::io::ErrorKind::NotFound {
                panic!("remove temp directory: {err}");
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn resolves_windows_pipe_endpoint_from_data_dir_hash() {
        let endpoint = resolve_windows_pipe_endpoint(r"C:\Users\tester\AppData\Local\tabctl");
        assert_eq!(
            endpoint,
            SocketEndpoint::Pipe {
                path: r"\\.\pipe\tabctl-f9bd75adcc15".to_string()
            }
        );
    }

    #[test]
    fn resolve_manifest_dir_rejects_invalid_browser() {
        let result = resolve_manifest_dir("firefox");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported browser"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_manifest_dir_edge_macos() {
        let path = resolve_manifest_dir("edge").expect("should resolve");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with("Library/Application Support/Microsoft Edge/NativeMessagingHosts"),
            "unexpected path: {path_str}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_manifest_dir_chrome_macos() {
        let path = resolve_manifest_dir("chrome").expect("should resolve");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with("Library/Application Support/Google/Chrome/NativeMessagingHosts"),
            "unexpected path: {path_str}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_manifest_dir_edge_linux() {
        let path = resolve_manifest_dir("edge").expect("should resolve");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with(".config/microsoft-edge/NativeMessagingHosts"),
            "unexpected path: {path_str}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_manifest_dir_chrome_linux() {
        let path = resolve_manifest_dir("chrome").expect("should resolve");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with(".config/google-chrome/NativeMessagingHosts"),
            "unexpected path: {path_str}"
        );
    }

    #[test]
    fn host_name_constant_is_correct() {
        assert_eq!(HOST_NAME, "com.erwinkroon.tabctl");
    }

    #[test]
    fn write_host_wrapper_creates_file_in_dir() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-wrapper-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let result = write_host_wrapper("/usr/local/bin/tabctl", "default", &dir);
        assert!(
            result.is_ok(),
            "write_host_wrapper failed: {:?}",
            result.err()
        );

        let path = result.unwrap();
        assert!(path.exists(), "wrapper file should exist");
        assert_eq!(path.parent().unwrap(), dir);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_host_wrapper_unix_filename_and_content() {
        let dir =
            std::env::temp_dir().join(format!("tabctl-test-wrapper-unix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let path = write_host_wrapper("/opt/bin/tabctl", "work", &dir).unwrap();
        assert!(path.to_string_lossy().ends_with("tabctl-host.sh"));

        let content = fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("TABCTL_PROFILE=\"work\""),
            "should contain profile name"
        );
        assert!(
            content.contains("TABCTL_CONFIG_DIR=\""),
            "should export config dir"
        );
        assert!(
            content.contains("TABCTL_DATA_DIR=\""),
            "should export data dir"
        );
        assert!(
            content.contains("\"/opt/bin/tabctl\" host"),
            "should contain binary path"
        );
        assert!(
            content.starts_with("#!/usr/bin/env bash\n"),
            "should have shebang"
        );
        assert!(
            !content.contains("\r\n"),
            "unix wrapper should use LF line endings"
        );

        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "wrapper should be executable owner-only");

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn write_host_wrapper_windows_filename_and_content() {
        let dir =
            std::env::temp_dir().join(format!("tabctl-test-wrapper-win-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let path =
            write_host_wrapper("C:\\Program Files\\tabctl\\tabctl.exe", "personal", &dir).unwrap();
        assert!(path.to_string_lossy().ends_with("tabctl-host.cmd"));

        let content = fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("TABCTL_PROFILE=personal"),
            "should contain profile name"
        );
        assert!(
            content.contains("\"C:\\Program Files\\tabctl\\tabctl.exe\" host"),
            "should contain binary path"
        );
        assert!(
            content.starts_with("@echo off\r\n"),
            "should start with @echo off"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_native_manifest_creates_valid_json() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-manifest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create a dummy wrapper file so canonicalize can resolve it
        let wrapper = dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();

        let result = write_native_manifest(
            "chrome",
            &wrapper,
            "abcdefghijklmnopqrstuvwxyz012345",
            Some(dir.to_str().unwrap()),
        );
        assert!(
            result.is_ok(),
            "write_native_manifest failed: {:?}",
            result.err()
        );

        let manifest_path = result.unwrap();
        assert!(manifest_path.exists(), "manifest file should exist");
        assert!(
            manifest_path
                .to_string_lossy()
                .ends_with(&format!("{HOST_NAME}.json")),
            "unexpected manifest filename: {}",
            manifest_path.display()
        );

        let content = fs::read_to_string(&manifest_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("manifest should be valid JSON");

        assert_eq!(parsed["name"], HOST_NAME);
        assert_eq!(parsed["description"], "tabctl native host");
        assert_eq!(parsed["type"], "stdio");

        let origins = parsed["allowed_origins"]
            .as_array()
            .expect("allowed_origins should be array");
        assert_eq!(origins.len(), 1);
        assert_eq!(
            origins[0],
            "chrome-extension://abcdefghijklmnopqrstuvwxyz012345/"
        );

        assert!(
            parsed["path"].as_str().unwrap().contains("tabctl-host.sh"),
            "path should reference wrapper script"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn write_registry_key_creates_and_reads_back() {
        use winreg::enums::*;
        use winreg::RegKey;

        let test_subkey = format!("Software\\tabctl-test\\NativeMessagingHosts\\{}", HOST_NAME);
        let manifest_path = std::path::PathBuf::from("C:\\test\\com.erwinkroon.tabctl.json");

        // Write via our function is browser-specific, so test directly with registry
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(&test_subkey).expect("create test key");
        key.set_value("", &manifest_path.display().to_string())
            .expect("set test value");

        let readback: String = key.get_value("").expect("read test value");
        assert_eq!(readback, manifest_path.display().to_string());

        // Cleanup
        let _ = hkcu.delete_subkey_all("Software\\tabctl-test");
    }

    #[test]
    fn register_profile_creates_new_profiles_json() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-reg-new-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let wrapper = data_dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();
        let manifest = data_dir.join("com.erwinkroon.tabctl.json");
        fs::write(&manifest, "{}").unwrap();

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let result = register_profile(
                    data_dir.to_str().unwrap(),
                    "edge",
                    "edge",
                    "mpglnmehddpkinfhheeahiicfieegcon",
                    &wrapper,
                    &manifest,
                );
                assert!(
                    result.is_ok(),
                    "register_profile failed: {:?}",
                    result.err()
                );
                let registry = result.unwrap();
                assert_eq!(registry["default"], "edge");
                assert!(registry["profiles"]["edge"].is_object());
                assert_eq!(registry["profiles"]["edge"]["browser"], "edge");
                assert_eq!(
                    registry["profiles"]["edge"]["extensionId"],
                    "mpglnmehddpkinfhheeahiicfieegcon"
                );

                let profiles_path = config_dir.join("profiles.json");
                assert!(profiles_path.exists(), "profiles.json should be created");
                let on_disk: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(&profiles_path).unwrap()).unwrap();
                assert_eq!(on_disk["default"], "edge");
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_profile_preserves_existing_profiles() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-reg-add-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        fs::create_dir_all(&config_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();

        // Seed with an existing profile
        let seed = json!({
            "default": "edge",
            "profiles": {
                "edge": {
                    "browser": "edge",
                    "extensionId": "aaaa",
                    "nodePath": "/bin/tabctl",
                    "hostPath": "/tmp/host.sh",
                    "dataDir": "/tmp/data/profiles/edge"
                }
            }
        });
        fs::write(
            config_dir.join("profiles.json"),
            serde_json::to_string_pretty(&seed).unwrap(),
        )
        .unwrap();

        let wrapper = data_dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();
        let manifest = data_dir.join("manifest.json");
        fs::write(&manifest, "{}").unwrap();

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let result = register_profile(
                    data_dir.to_str().unwrap(),
                    "chrome-work",
                    "chrome",
                    "bbbbbbbb",
                    &wrapper,
                    &manifest,
                );
                assert!(
                    result.is_ok(),
                    "register_profile failed: {:?}",
                    result.err()
                );
                let registry = result.unwrap();
                // Default should still be edge (first registered)
                assert_eq!(registry["default"], "edge");
                // Both profiles should exist
                assert!(registry["profiles"]["edge"].is_object());
                assert!(registry["profiles"]["chrome-work"].is_object());
                assert_eq!(registry["profiles"]["chrome-work"]["browser"], "chrome");
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_profile_updates_existing_entry() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-reg-upd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        fs::create_dir_all(&config_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();

        // Seed with an existing profile
        let seed = json!({
            "default": "edge",
            "profiles": {
                "edge": {
                    "browser": "edge",
                    "extensionId": "old-ext-id",
                    "nodePath": "/old/bin/tabctl",
                    "hostPath": "/old/host.sh",
                    "dataDir": "/old/data/profiles/edge"
                }
            }
        });
        fs::write(
            config_dir.join("profiles.json"),
            serde_json::to_string_pretty(&seed).unwrap(),
        )
        .unwrap();

        let wrapper = data_dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();
        let manifest = data_dir.join("manifest.json");
        fs::write(&manifest, "{}").unwrap();

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let result = register_profile(
                    data_dir.to_str().unwrap(),
                    "edge",
                    "edge",
                    "new-ext-id",
                    &wrapper,
                    &manifest,
                );
                assert!(
                    result.is_ok(),
                    "register_profile failed: {:?}",
                    result.err()
                );
                let registry = result.unwrap();
                // Should still be one profile, updated
                assert_eq!(registry["profiles"].as_object().unwrap().len(), 1);
                assert_eq!(registry["profiles"]["edge"]["extensionId"], "new-ext-id");
                // Default preserved
                assert_eq!(registry["default"], "edge");
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_host_wrapper_creates_nested_dirs() {
        let dir = std::env::temp_dir()
            .join(format!("tabctl-test-nested-{}", std::process::id()))
            .join("a")
            .join("b")
            .join("c");
        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap().parent().unwrap());

        let result = write_host_wrapper("/usr/bin/tabctl", "deep", &dir);
        assert!(
            result.is_ok(),
            "should create nested dirs: {:?}",
            result.err()
        );
        assert!(dir.exists(), "nested wrapper dir should exist");

        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap().parent().unwrap());
    }

    #[test]
    fn write_native_manifest_creates_nmh_subdir() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-nmh-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let wrapper = dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();

        let udd = dir.join("fake-user-data");
        // udd and its NativeMessagingHosts child should not exist yet
        assert!(!udd.exists());

        let result =
            write_native_manifest("edge", &wrapper, "extid123", Some(udd.to_str().unwrap()));
        assert!(
            result.is_ok(),
            "should create NMH subdir: {:?}",
            result.err()
        );

        let nmh = udd.join("NativeMessagingHosts");
        assert!(
            nmh.exists(),
            "NativeMessagingHosts subdir should be created"
        );
        assert!(nmh.join(format!("{HOST_NAME}.json")).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_native_manifest_wrapper_path_is_absolute() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-abs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let wrapper = dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();

        let manifest_path =
            write_native_manifest("chrome", &wrapper, "testextid", Some(dir.to_str().unwrap()))
                .unwrap();

        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let path_in_manifest = content["path"].as_str().unwrap();
        assert!(
            PathBuf::from(path_in_manifest).is_absolute(),
            "manifest path should be absolute, got: {path_in_manifest}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_profile_rejects_unsupported_browser() {
        let dir =
            std::env::temp_dir().join(format!("tabctl-test-badbrowser-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let wrapper = data_dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();
        let manifest = data_dir.join("manifest.json");
        fs::write(&manifest, "{}").unwrap();

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let result = register_profile(
                    data_dir.to_str().unwrap(),
                    "firefox-profile",
                    "firefox",
                    "extid",
                    &wrapper,
                    &manifest,
                );
                assert!(result.is_err());
                assert!(result.unwrap_err().contains("unsupported browser"));
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_profile_data_dir_includes_profile_name() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-datadir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let wrapper = data_dir.join("tabctl-host.sh");
        fs::write(&wrapper, "#!/bin/bash\n").unwrap();
        let manifest = data_dir.join("manifest.json");
        fs::write(&manifest, "{}").unwrap();

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let registry = register_profile(
                    data_dir.to_str().unwrap(),
                    "my-profile",
                    "chrome",
                    "extid",
                    &wrapper,
                    &manifest,
                )
                .unwrap();

                let data_dir_val = registry["profiles"]["my-profile"]["dataDir"]
                    .as_str()
                    .unwrap();
                assert!(
                    data_dir_val.ends_with("profiles/my-profile")
                        || data_dir_val.ends_with("profiles\\my-profile"),
                    "dataDir should end with profiles/<name>, got: {data_dir_val}"
                );
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_setup_end_to_end_writes_all_files() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        let udd = dir.join("user-data");

        with_env_vars(
            &[
                ("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap())),
                ("TABCTL_DATA_DIR", Some(data_dir.to_str().unwrap())),
                ("TABCTL_SETUP_FETCH_EXTENSION", Some("0")),
            ],
            || {
                let matches = build_cli()
                    .try_get_matches_from([
                        "tabctl",
                        "--json",
                        "setup",
                        "--browser",
                        "edge",
                        "--skip-extension-download",
                        "--extension-id",
                        "testextensionid1234567890abcde",
                        "--user-data-dir",
                        udd.to_str().unwrap(),
                    ])
                    .expect("parse args");

                let (_, sub) = matches.subcommand().expect("subcommand");
                let result = run_setup(&matches, sub);
                assert!(result.is_ok(), "run_setup failed: {:?}", result.err());

                // Wrapper script should exist under data/profiles/edge/
                let wrapper = data_dir
                    .join("profiles")
                    .join("edge")
                    .join(if cfg!(windows) {
                        "tabctl-host.cmd"
                    } else {
                        "tabctl-host.sh"
                    });
                assert!(wrapper.exists(), "wrapper script should be created");

                // Native manifest in user-data-dir/NativeMessagingHosts/
                let manifest = udd
                    .join("NativeMessagingHosts")
                    .join(format!("{HOST_NAME}.json"));
                assert!(manifest.exists(), "native manifest should be created");

                // profiles.json should exist
                let profiles = config_dir.join("profiles.json");
                assert!(profiles.exists(), "profiles.json should be created");

                let reg: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(&profiles).unwrap()).unwrap();
                assert_eq!(reg["default"], "edge");
                assert_eq!(
                    reg["profiles"]["edge"]["extensionId"],
                    "testextensionid1234567890abcde"
                );
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_auth_token_from_file() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-auth-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(AUTH_TOKEN_FILENAME), "  secret-token-123\n").unwrap();

        with_env_vars(
            &[
                ("TABCTL_AUTH_TOKEN", None),
                ("TABCTL_DATA_DIR", Some(dir.to_str().unwrap())),
            ],
            || {
                let token = read_auth_token(None);
                assert_eq!(token.as_deref(), Some("secret-token-123"));
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_auth_token_env_override() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-auth-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(AUTH_TOKEN_FILENAME), "file-token").unwrap();

        with_env_vars(
            &[
                ("TABCTL_AUTH_TOKEN", Some("env-token-override")),
                ("TABCTL_DATA_DIR", Some(dir.to_str().unwrap())),
            ],
            || {
                let token = read_auth_token(None);
                assert_eq!(token.as_deref(), Some("env-token-override"));
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_auth_token_missing_file_returns_none() {
        let dir =
            std::env::temp_dir().join(format!("tabctl-test-auth-miss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        with_env_vars(
            &[
                ("TABCTL_AUTH_TOKEN", None),
                ("TABCTL_DATA_DIR", Some(dir.to_str().unwrap())),
            ],
            || {
                let token = read_auth_token(None);
                assert!(token.is_none());
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wsl_file_candidates_includes_auth_token_filename() {
        let candidates = wsl_file_candidates("/tmp/tabctl/data", AUTH_TOKEN_FILENAME);
        assert!(candidates[0].ends_with(AUTH_TOKEN_FILENAME));
    }

    #[test]
    fn test_transport_tcp_with_port_env() {
        with_env_vars(
            &[
                ("TABCTL_TRANSPORT", Some("tcp")),
                ("TABCTL_TCP_PORT", Some("39005")),
                ("TABCTL_SOCKET", None),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert_eq!(
                    endpoint,
                    SocketEndpoint::Tcp {
                        host: "127.0.0.1".to_string(),
                        port: 39005,
                    }
                );
            },
        );
    }

    #[test]
    fn test_transport_tcp_with_port_file() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp_root = std::env::temp_dir().join(format!(
            "tabctl-tcp-port-test-{}-{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&temp_root);
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        let port_file = temp_root.join(WSL_TCP_PORT_FILENAME);
        std::fs::write(&port_file, "39010\n").expect("write port file");
        assert!(port_file.exists(), "port file must exist before test");
        with_env_vars(
            &[
                ("TABCTL_TRANSPORT", Some("tcp")),
                ("TABCTL_TCP_PORT", None),
                ("TABCTL_SOCKET", None),
                (
                    "TABCTL_DATA_DIR",
                    Some(temp_root.to_str().expect("temp path should be valid utf-8")),
                ),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert_eq!(
                    endpoint,
                    SocketEndpoint::Tcp {
                        host: "127.0.0.1".to_string(),
                        port: 39010,
                    }
                );
            },
        );
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn test_transport_tcp_missing_port_returns_error() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp_root = std::env::temp_dir().join(format!(
            "tabctl-tcp-noport-test-{}-{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&temp_root);
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        with_env_vars(
            &[
                ("TABCTL_TRANSPORT", Some("tcp")),
                ("TABCTL_TCP_PORT", None),
                ("TABCTL_SOCKET", None),
                (
                    "TABCTL_DATA_DIR",
                    Some(temp_root.to_str().expect("temp path should be valid utf-8")),
                ),
            ],
            || {
                let err = resolve_socket_endpoint(None).expect_err("should fail without port file");
                assert!(
                    err.contains("TABCTL_HOST_TCP"),
                    "error should mention TABCTL_HOST_TCP, got: {err}"
                );
            },
        );
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[cfg(not(windows))]
    #[test]
    fn test_transport_non_tcp_ignored() {
        let temp_root = std::env::temp_dir().join(format!("tabctl-cli-test-{}", request_id()));
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        with_env_vars(
            &[
                ("TABCTL_TRANSPORT", Some("socket")),
                ("TABCTL_TCP_PORT", None),
                ("TABCTL_SOCKET", None),
                (
                    "TABCTL_DATA_DIR",
                    Some(temp_root.to_str().expect("temp path should be valid utf-8")),
                ),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert!(
                    matches!(endpoint, SocketEndpoint::Unix { .. }),
                    "expected Unix endpoint, got: {endpoint:?}"
                );
            },
        );
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn maps_inspect_flags_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "--progress",
                "inspect",
                "--tab",
                "42",
                "--signal",
                "page-meta",
                "--signal",
                "selector",
                "--selector",
                "links=a[href]",
                "--selector-attr",
                "href-url",
                "--signal-concurrency",
                "4",
                "--signal-timeout-ms",
                "5000",
                "--wait-for",
                "settle",
                "--wait-timeout-ms",
                "3000",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "inspect");
        assert_eq!(routed.params["tabIds"], json!([42]));
        assert_eq!(routed.params["signals"], json!(["page-meta", "selector"]));
        assert_eq!(routed.params["selectorAttr"], json!("href-url"));
        assert_eq!(routed.params["signalConcurrency"], json!(4));
        assert_eq!(routed.params["signalTimeoutMs"], json!(5000));
        assert_eq!(routed.params["waitFor"], json!("settle"));
        assert_eq!(routed.params["waitTimeoutMs"], json!(3000));
        assert_eq!(routed.params["progress"], json!(true));
        // selectorSpecs should contain one entry with default attr from --selector-attr
        let specs = routed.params["selectorSpecs"].as_array().unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["name"], json!("links"));
        assert_eq!(specs[0]["selector"], json!("a[href]"));
        assert_eq!(specs[0]["attr"], json!("href-url"));
    }

    #[test]
    fn inspect_selector_json_format() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "inspect",
                "--tab",
                "1",
                "--selector",
                r#"{"name":"price","selector":".amount","attr":"text","all":true}"#,
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        let specs = routed.params["selectorSpecs"].as_array().unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["name"], json!("price"));
        assert_eq!(specs[0]["selector"], json!(".amount"));
        assert_eq!(specs[0]["attr"], json!("text"));
        assert_eq!(specs[0]["all"], json!(true));
    }

    #[test]
    fn inspect_selector_invalid_format_rejected() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "inspect",
                "--tab",
                "1",
                "--selector",
                "noequalssign",
            ])
            .expect("parse command");
        let err = route_command(&matches).unwrap_err();
        assert!(err.contains("Invalid --selector format"), "got: {err}");
    }

    #[test]
    fn maps_screenshot_flags_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "--progress",
                "screenshot",
                "--tab",
                "7",
                "--mode",
                "full",
                "--format",
                "jpeg",
                "--quality",
                "80",
                "--tile-max-dim",
                "4096",
                "--max-bytes",
                "500000",
                "--wait-for",
                "load",
                "--wait-timeout-ms",
                "2000",
                "--out",
                "/tmp/shots",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "screenshot");
        assert_eq!(routed.params["tabIds"], json!([7]));
        assert_eq!(routed.params["mode"], json!("full"));
        assert_eq!(routed.params["format"], json!("jpeg"));
        assert_eq!(routed.params["quality"], json!(80));
        assert_eq!(routed.params["tileMaxDim"], json!(4096));
        assert_eq!(routed.params["maxBytes"], json!(500000));
        assert_eq!(routed.params["waitFor"], json!("load"));
        assert_eq!(routed.params["waitTimeoutMs"], json!(2000));
        assert_eq!(routed.params["out"], json!("/tmp/shots"));
        assert_eq!(routed.params["progress"], json!(true));
    }

    #[test]
    fn maps_analyze_flags_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "analyze",
                "--window",
                "active",
                "--stale-days",
                "14",
                "--window-title",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "analyze");
        assert_eq!(routed.params["staleDays"], json!(14));
        assert_eq!(routed.params["windowTitle"], json!(true));
        assert!(routed.params.get("dedupe").is_none());
    }

    #[test]
    fn maps_dedupe_flags_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "dedupe",
                "--all",
                "--stale-days",
                "7",
                "--include-stale",
                "--window-title",
                "--confirm",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "analyze");
        assert_eq!(routed.params["dedupe"], json!(true));
        assert_eq!(routed.params["staleDays"], json!(7));
        assert_eq!(routed.params["includeStale"], json!(true));
        assert_eq!(routed.params["windowTitle"], json!(true));
        assert_eq!(routed.params["confirmed"], json!(true));
    }

    #[test]
    fn maps_move_tab_flags_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "move-tab",
                "--tab",
                "5",
                "--after-tab",
                "10",
                "--before-group",
                "Work",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "move-tab");
        assert_eq!(routed.params["tabIds"], json!([5]));
        assert_eq!(routed.params["afterTabId"], json!(10));
        assert_eq!(routed.params["beforeGroupTitle"], json!("Work"));
    }

    #[test]
    fn maps_move_group_flags_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "move-group",
                "--group",
                "Research",
                "--new-window",
                "--after-group",
                "Archive",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "move-group");
        assert_eq!(routed.params["groupTitle"], json!("Research"));
        assert_eq!(routed.params["newWindow"], json!(true));
        assert_eq!(routed.params["afterGroupTitle"], json!("Archive"));
    }

    #[test]
    fn connectivity_report_includes_manual_steps_when_ping_fails() {
        let entry = ProfileEntry {
            browser: Browser::Edge,
            extension_id: "mpglnmehddpkinfhheeahiicfieegcon".to_string(),
            node_path: std::env::current_exe()
                .expect("resolve current exe")
                .display()
                .to_string(),
            host_path: "/tmp/tabctl-host.sh".to_string(),
            data_dir: "/tmp/tabctl/profiles/edge".to_string(),
            user_data_dir: None,
        };

        with_env_vars(&[("TABCTL_SOCKET", Some("tcp://127.0.0.1"))], || {
            let report = profile_connectivity_report("edge", &entry);
            assert_eq!(report["healthy"], json!(false));
            assert_eq!(report["checks"]["pingOk"], json!(false));
            let steps = report["manualSteps"]
                .as_array()
                .expect("manualSteps should be an array");
            assert!(
                steps.iter().any(|step| {
                    step.as_str() == Some("Verify connection: tabctl --profile edge ping")
                }),
                "manual steps should include profile-scoped ping verification"
            );
            assert!(
                steps.iter().any(|step| {
                    step.as_str().map(|value| {
                        value.contains(
                            "tabctl setup --browser edge --name edge --extension-id mpglnmehddpkinfhheeahiicfieegcon",
                        )
                    }) == Some(true)
                }),
                "manual steps should include setup remediation with extension id"
            );
        });
    }

    #[test]
    fn doctor_fix_reports_manual_remediation_for_connectivity_only_issue() {
        let dir =
            std::env::temp_dir().join(format!("tabctl-test-doctor-connectivity-{}", request_id()));
        let home_dir = dir.join("home");
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        let profile_dir = data_dir.join("profiles").join("edge");
        fs::create_dir_all(&profile_dir).expect("create profile dir");
        let wrapper = profile_dir.join(if cfg!(windows) {
            "tabctl-host.cmd"
        } else {
            "tabctl-host.sh"
        });
        fs::write(&wrapper, "echo ok\n").expect("write wrapper");

        with_env_vars(
            &[
                ("TABCTL_SOCKET", Some("tcp://127.0.0.1")),
                (
                    "TABCTL_CONFIG_DIR",
                    Some(config_dir.to_str().expect("config path")),
                ),
                (
                    "TABCTL_DATA_DIR",
                    Some(data_dir.to_str().expect("data path")),
                ),
                ("HOME", Some(home_dir.to_str().expect("home path"))),
                ("XDG_CONFIG_HOME", None),
                ("XDG_STATE_HOME", None),
            ],
            || {
                let extension_id = "mpglnmehddpkinfhheeahiicfieegcon";
                let manifest_dir = resolve_manifest_dir("edge").expect("resolve manifest dir");
                fs::create_dir_all(&manifest_dir).expect("create manifest dir");
                let manifest = json!({
                    "name": HOST_NAME,
                    "description": "tabctl native host",
                    "path": wrapper.display().to_string(),
                    "type": "stdio",
                    "allowed_origins": [format!("chrome-extension://{extension_id}/")]
                });
                fs::write(
                    manifest_dir.join(format!("{HOST_NAME}.json")),
                    serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
                )
                .expect("write manifest");

                let entry = ProfileEntry {
                    browser: Browser::Edge,
                    extension_id: extension_id.to_string(),
                    node_path: std::env::current_exe()
                        .expect("resolve current exe")
                        .display()
                        .to_string(),
                    host_path: wrapper.display().to_string(),
                    data_dir: profile_dir.display().to_string(),
                    user_data_dir: None,
                };

                let evaluation =
                    evaluate_doctor_profile("edge", &entry, true).expect("evaluate doctor profile");
                assert!(
                    !evaluation.healthy,
                    "connectivity failure should remain unhealthy"
                );
                assert!(
                    !evaluation.connectivity_healthy,
                    "connectivity should be marked unhealthy"
                );
                assert_eq!(evaluation.report["repair"]["attempted"], json!(true));
                assert_eq!(
                    evaluation.report["connectivity"]["checks"]["pingOk"],
                    json!(false)
                );
                let steps = evaluation.report["connectivity"]["manualSteps"]
                    .as_array()
                    .expect("manualSteps should exist for connectivity failure");
                assert!(
                    steps.iter().any(|step| {
                        step.as_str() == Some("Verify connection: tabctl --profile edge ping")
                    }),
                    "manual steps should include ping verification"
                );
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
