use clap::{value_parser, Arg, ArgAction, ArgMatches, Command};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use tabctl_shared::windows_pipe_path;
use tabctl_shared::{
    normalize_path_for_current_platform, path_to_platform_string, Browser, ProfileEntry,
    ProfileRegistry, RequestEnvelope, ResponseEnvelope, SocketEndpoint,
};

use sha2::{Digest, Sha256};

const WSL_TCP_PORT_FILENAME: &str = "tcp-port";
#[cfg(target_os = "linux")]
const WSL_PIPE_ENDPOINT_FILENAME: &str = "pipe-endpoint";
const AUTH_TOKEN_FILENAME: &str = "auth-token";
const CLI_RESPONSE_TIMEOUT_MS: u64 = 35_000;
const EXTENSION_ACTIVE_DIR_NAME: &str = "extension";
const EXTENSION_RELEASES_DIR_NAME: &str = "extension-releases";
const EXTENSION_VERSIONS_DIR_NAME: &str = "extension-versions";
const EXTENSION_VERSION_MARKER_FILE: &str = ".tabctl-version";

#[derive(Debug, Clone)]
struct ExtensionSyncResult {
    updated: bool,
    target_version: String,
    installed_version: Option<String>,
    active_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoSyncMode {
    Auto,
    ReleaseLike,
    Off,
}

#[path = "api.rs"]
mod api;
#[path = "commands.rs"]
mod commands;
#[path = "local.rs"]
mod local;
#[path = "output.rs"]
mod output;
#[path = "route.rs"]
mod route;
#[path = "setup.rs"]
mod setup;
#[path = "transport.rs"]
mod transport;

pub use api::run;

use api::*;
use commands::*;
use local::*;
use output::*;
use route::*;
use setup::*;
use transport::*;

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
    fn routes_ping_without_params() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "ping"])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "ping");
        assert_eq!(routed.params, json!({}));
    }

    #[test]
    fn routes_history_limit_to_host_params() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "history", "--limit", "5"])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert_eq!(routed.action, "history");
        assert_eq!(routed.params["limit"], json!(5));
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
                "ping",
            ])
            .expect("parse command");
        let routed = route_command(&matches).expect("route command");
        assert!(routed.json);
        assert!(!routed.pretty);
        assert_eq!(routed.profile.as_deref(), Some("edge-work"));
        assert_eq!(routed.action, "ping");
    }

    #[test]
    fn query_command_accepts_graphql_string() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "query", "query { ping { ok } }"])
            .expect("parse command");
        let (command, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "query");
        assert_eq!(
            sub.get_one::<String>("graphql").map(String::as_str),
            Some("query { ping { ok } }")
        );
    }

    #[test]
    fn removed_browser_commands_are_unknown() {
        for command in ["list", "open", "close", "raw"] {
            assert!(
                build_cli()
                    .try_get_matches_from(["tabctl", command])
                    .is_err(),
                "expected {command} to be removed from the public CLI"
            );
        }
    }

    #[test]
    fn compact_response_payload_prefers_data_for_success() {
        let response = ResponseEnvelope {
            ok: true,
            action: Some("list".to_string()),
            request_id: Some("req-1".to_string()),
            component: Some("host".to_string()),
            version: Some("1.0.0".to_string()),
            progress: None,
            data: Some(json!({"windows":[{"windowId": 1}]})),
            error: None,
        };
        assert_eq!(
            compact_response_payload(&response),
            json!({"windows":[{"windowId": 1}]})
        );
    }

    #[test]
    fn compact_response_payload_uses_minimal_error_shape() {
        let response = ResponseEnvelope {
            ok: false,
            action: Some("list".to_string()),
            request_id: Some("req-2".to_string()),
            component: Some("host".to_string()),
            version: Some("1.0.0".to_string()),
            progress: None,
            data: None,
            error: Some(tabctl_shared::ProtocolError {
                message: "boom".to_string(),
                hint: Some("retry".to_string()),
            }),
        };
        assert_eq!(
            compact_response_payload(&response),
            json!({"ok": false, "error": {"message": "boom", "hint": "retry"}})
        );
    }

    #[test]
    fn compact_response_payload_omits_empty_error_hint() {
        let response = ResponseEnvelope {
            ok: false,
            action: Some("list".to_string()),
            request_id: Some("req-3".to_string()),
            component: Some("host".to_string()),
            version: Some("1.0.0".to_string()),
            progress: None,
            data: None,
            error: Some(tabctl_shared::ProtocolError {
                message: "boom".to_string(),
                hint: None,
            }),
        };
        assert_eq!(
            compact_response_payload(&response),
            json!({"ok": false, "error": {"message": "boom"}})
        );
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
    fn parses_setup_command_with_local_extension_dir() {
        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "setup",
                "--browser",
                "edge",
                "--extension-dir",
                "/tmp/tabctl-extension",
            ])
            .expect("parse command");
        let (_, sub) = matches.subcommand().expect("subcommand");
        assert_eq!(
            sub.get_one::<String>("extension-dir").map(String::as_str),
            Some("/tmp/tabctl-extension")
        );
    }

    #[test]
    fn setup_extension_dir_override_uses_cli_and_env() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-extension-dir-{}", request_id()));
        fs::create_dir_all(&dir).expect("create extension dir");
        fs::write(
            dir.join("manifest.json"),
            r#"{"manifest_version":3,"name":"Tab Control","version":"0.6.0"}"#,
        )
        .expect("write manifest");
        let dir_str = dir.to_str().expect("utf-8 extension dir");

        let matches = build_cli()
            .try_get_matches_from([
                "tabctl",
                "setup",
                "--browser",
                "edge",
                "--extension-dir",
                dir_str,
            ])
            .expect("parse command");
        let (_, sub) = matches.subcommand().expect("subcommand");
        let resolved = resolve_setup_extension_dir_override(sub)
            .expect("resolve local extension dir from cli")
            .expect("local extension dir from cli should be set");
        assert_eq!(
            resolved,
            fs::canonicalize(&dir).expect("canonicalize cli dir")
        );

        with_env_vars(&[("TABCTL_SETUP_EXTENSION_DIR", Some(dir_str))], || {
            let matches = build_cli()
                .try_get_matches_from(["tabctl", "setup", "--browser", "edge"])
                .expect("parse command");
            let (_, sub) = matches.subcommand().expect("subcommand");
            let resolved = resolve_setup_extension_dir_override(sub)
                .expect("resolve local extension dir from env")
                .expect("local extension dir from env should be set");
            assert_eq!(
                resolved,
                fs::canonicalize(&dir).expect("canonicalize env dir")
            );
        });

        let _ = fs::remove_dir_all(&dir);
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

    #[test]
    fn compare_base_versions_handles_prerelease_ordering() {
        // pre-release < release with same triplet
        assert_eq!(
            compare_base_versions("0.6.0-alpha.5", "0.6.0"),
            Some(std::cmp::Ordering::Less)
        );
        // higher triplet wins regardless of pre-release
        assert_eq!(
            compare_base_versions("0.6.1-rc.1", "0.6.0"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_base_versions("0.5.2", "0.6.0-alpha.1"),
            Some(std::cmp::Ordering::Less)
        );
        // pre-release numeric ordering
        assert_eq!(
            compare_base_versions("0.6.0-alpha.10", "0.6.0-alpha.9"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_base_versions("0.6.0-alpha.9", "0.6.0-alpha.10"),
            Some(std::cmp::Ordering::Less)
        );
        // same pre-release
        assert_eq!(
            compare_base_versions("0.6.0-alpha.10", "0.6.0-alpha.10"),
            Some(std::cmp::Ordering::Equal)
        );
        // rc > alpha (lexicographic)
        assert_eq!(
            compare_base_versions("0.6.0-rc.1", "0.6.0-alpha.10"),
            Some(std::cmp::Ordering::Greater)
        );
    }

    #[test]
    fn strip_dev_suffix_preserves_prerelease_tags() {
        assert_eq!(
            strip_dev_suffix("0.6.0-alpha.10-dev.f4ad4314"),
            "0.6.0-alpha.10"
        );
        assert_eq!(
            strip_dev_suffix("0.6.0-alpha.10-dev.f4ad4314.dirty"),
            "0.6.0-alpha.10"
        );
        assert_eq!(strip_dev_suffix("0.6.0-alpha.10"), "0.6.0-alpha.10");
        assert_eq!(strip_dev_suffix("0.6.0"), "0.6.0");
        assert_eq!(strip_dev_suffix("1.0.0-rc.1-dev.abc123"), "1.0.0-rc.1");
    }

    #[test]
    fn chromium_extension_id_from_digest_maps_to_ap_alphabet() {
        let zeros = chromium_extension_id_from_digest(&[0u8; 32]);
        assert_eq!(zeros, "a".repeat(32));

        let ff = chromium_extension_id_from_digest(&[0xffu8; 32]);
        assert_eq!(ff, "p".repeat(32));
    }

    #[test]
    fn derive_extension_id_from_extension_path_returns_32_ap_chars() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-derive-ext-id-{}", request_id()));
        fs::create_dir_all(&dir).expect("create extension dir");
        fs::write(
            dir.join("manifest.json"),
            r#"{"manifest_version":3,"name":"Tab Control","version":"0.6.0"}"#,
        )
        .expect("write manifest");

        let id = derive_extension_id_from_extension_path(&dir).expect("derive extension id");
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|ch| ('a'..='p').contains(&ch)));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn should_runtime_auto_sync_skips_reload() {
        assert!(should_runtime_auto_sync("ping"));
        assert!(!should_runtime_auto_sync("reload"));
        assert!(should_runtime_auto_sync("query"));
    }

    #[test]
    fn effective_auto_sync_mode_defaults_to_auto() {
        with_env_vars(&[("TABCTL_AUTO_SYNC_MODE", None)], || {
            assert_eq!(effective_auto_sync_mode(), AutoSyncMode::Auto);
        });
    }

    #[test]
    fn effective_auto_sync_mode_accepts_release_like_values() {
        with_env_vars(&[("TABCTL_AUTO_SYNC_MODE", Some("release-like"))], || {
            assert_eq!(effective_auto_sync_mode(), AutoSyncMode::ReleaseLike);
        });
        with_env_vars(&[("TABCTL_AUTO_SYNC_MODE", Some("true"))], || {
            assert_eq!(effective_auto_sync_mode(), AutoSyncMode::ReleaseLike);
        });
    }

    #[test]
    fn effective_auto_sync_mode_accepts_off_values() {
        with_env_vars(&[("TABCTL_AUTO_SYNC_MODE", Some("off"))], || {
            assert_eq!(effective_auto_sync_mode(), AutoSyncMode::Off);
        });
        with_env_vars(&[("TABCTL_AUTO_SYNC_MODE", Some("0"))], || {
            assert_eq!(effective_auto_sync_mode(), AutoSyncMode::Off);
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

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_config_dir_uses_windows_roaming_appdata_in_wsl() {
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_CONFIG_DIR", None),
                ("XDG_CONFIG_HOME", None),
                (
                    "PATH",
                    Some("/usr/bin:/mnt/c/Users/TestUser/AppData/Local/Microsoft/WindowsApps"),
                ),
            ],
            || {
                let config_dir = resolve_config_dir().expect("resolve config dir");
                assert_eq!(
                    config_dir,
                    "/mnt/c/Users/TestUser/AppData/Roaming/tabctl".to_string()
                );
            },
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_data_dir_uses_windows_local_appdata_in_wsl() {
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_DATA_DIR", None),
                ("TABCTL_STATE_DIR", None),
                ("XDG_STATE_HOME", None),
                (
                    "PATH",
                    Some("/usr/bin:/mnt/c/Users/TestUser/AppData/Local/Microsoft/WindowsApps"),
                ),
            ],
            || {
                let data_dir = resolve_data_dir(None).expect("resolve data dir");
                assert_eq!(
                    data_dir,
                    "/mnt/c/Users/TestUser/AppData/Local/tabctl".to_string()
                );
            },
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn host_wrapper_repair_is_disabled_in_wsl() {
        with_env_vars(&[("WSL_INTEROP", Some("1"))], || {
            assert!(!can_repair_host_wrapper());
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn attempt_profile_repair_fails_fast_in_wsl() {
        with_env_vars(&[("WSL_INTEROP", Some("1"))], || {
            let entry = ProfileEntry {
                browser: Browser::Chrome,
                extension_id: "dkfnfgfelacbfclhenpgdckfefmfddbd".to_string(),
                node_path: "/mnt/c/dev/ekroon/tabctl/rust/target/debug/tabctl".to_string(),
                host_path:
                    r"C:\Users\TestUser\AppData\Local\tabctl\profiles\chrome\tabctl-host.cmd"
                        .to_string(),
                data_dir: r"C:\Users\TestUser\AppData\Local\tabctl\profiles\chrome".to_string(),
                user_data_dir: None,
            };
            let err = attempt_profile_repair("chrome", &entry).expect_err("repair should fail");
            assert!(
                err.contains("unsupported from WSL"),
                "unexpected error: {err}"
            );
        });
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
    fn translates_windows_data_dir_into_wsl_candidate() {
        let candidates = wsl_file_candidates(
            r"C:\Users\TestUser\AppData\Local\tabctl\profiles\chrome",
            "pipe-endpoint",
        );
        assert!(
            candidates.iter().any(|path| {
                path == &PathBuf::from(
                    "/mnt/c/Users/TestUser/AppData/Local/tabctl/profiles/chrome/pipe-endpoint",
                )
            }),
            "expected translated WSL candidate, got: {candidates:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn keeps_explicit_pipe_socket_in_wsl() {
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_SOCKET", Some("pipe://tabctl-test")),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert_eq!(
                    endpoint,
                    SocketEndpoint::Pipe {
                        path: r"\\.\pipe\tabctl-test".to_string(),
                    }
                );
            },
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_explicit_tcp_socket_in_wsl() {
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_SOCKET", Some("tcp://127.0.0.1:39006")),
                ("TABCTL_TCP_PORT", Some("39007")),
            ],
            || {
                let err = resolve_socket_endpoint(None).expect_err("resolve endpoint should fail");
                assert!(err.contains("disabled in WSL"), "unexpected error: {err}");
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
    fn wsl_named_pipe_endpoint_is_required_when_not_discovered() {
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
                let err = resolve_socket_endpoint(None).expect_err("resolve endpoint should fail");
                assert!(err.contains("pipe-endpoint"), "unexpected error: {err}");
            },
        );
        if let Err(err) = std::fs::remove_dir_all(&temp_root) {
            if err.kind() != std::io::ErrorKind::NotFound {
                panic!("remove temp directory: {err}");
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolves_wsl_pipe_endpoint_from_pipe_endpoint_file() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp_root = std::env::temp_dir().join(format!(
            "tabctl-wsl-pipe-endpoint-test-{}-{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&temp_root);
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        std::fs::write(
            temp_root.join(WSL_PIPE_ENDPOINT_FILENAME),
            "\\\\.\\pipe\\tabctl-test-bridge\n",
        )
        .expect("write pipe endpoint file");

        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
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
                    SocketEndpoint::Pipe {
                        path: r"\\.\pipe\tabctl-test-bridge".to_string(),
                    }
                );
            },
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipe_transport_returns_error_without_pipe_endpoint_file() {
        let temp_root =
            std::env::temp_dir().join(format!("tabctl-wsl-pipe-missing-{}", request_id()));
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        with_env_vars(
            &[
                ("WSL_INTEROP", Some("1")),
                ("TABCTL_SOCKET", None),
                (
                    "TABCTL_DATA_DIR",
                    Some(temp_root.to_str().expect("temp path should be valid utf-8")),
                ),
            ],
            || {
                let err =
                    resolve_socket_endpoint(None).expect_err("missing pipe endpoint should fail");
                assert!(err.contains("pipe-endpoint"), "unexpected error: {err}");
            },
        );
        let _ = std::fs::remove_dir_all(&temp_root);
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

    #[cfg(windows)]
    #[test]
    fn resolves_windows_pipe_endpoint_for_mixed_separator_variants() {
        let canonical = resolve_windows_pipe_endpoint(r"C:\Users\tester\AppData\Local\tabctl");
        let mixed = resolve_windows_pipe_endpoint(r"C:/Users/tester/AppData/Local/tabctl");
        assert_eq!(canonical, mixed);
    }

    #[cfg(windows)]
    #[test]
    fn windows_pipe_connect_error_includes_profile_data_dir_and_hint() {
        let err = std::io::Error::from_raw_os_error(5);
        let rendered = format_windows_pipe_connect_error(
            Some("edge"),
            r"C:\Users\tester\AppData\Local\tabctl\profiles\edge",
            r"\\.\pipe\tabctl-test",
            &err,
        );
        assert!(rendered.contains(r"\\.\pipe\tabctl-test"));
        assert!(rendered.contains("profile: edge"));
        assert!(rendered.contains(r"data dir: C:\Users\tester\AppData\Local\tabctl\profiles\edge"));
        assert!(rendered.contains("named-pipe ACLs") || rendered.contains("Windows denied access"));
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
    fn run_setup_without_extension_id_derives_from_local_extension_dir() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-e2e-derive-{}", request_id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        let data_dir = dir.join("data");
        let udd = dir.join("user-data");
        let local_extension_dir = dir.join("local-extension");
        fs::create_dir_all(&local_extension_dir).expect("create local extension dir");
        fs::write(
            local_extension_dir.join("manifest.json"),
            r#"{"manifest_version":3,"name":"Tab Control","version":"0.6.0"}"#,
        )
        .expect("write local extension manifest");
        fs::write(
            local_extension_dir.join("background.js"),
            "self.addEventListener('install', () => {});",
        )
        .expect("write local extension background");

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
                        "--extension-dir",
                        local_extension_dir.to_str().unwrap(),
                        "--user-data-dir",
                        udd.to_str().unwrap(),
                    ])
                    .expect("parse args");

                let (_, sub) = matches.subcommand().expect("subcommand");
                let result = run_setup(&matches, sub);
                assert!(result.is_ok(), "run_setup failed: {:?}", result.err());

                let wrapper = data_dir
                    .join("profiles")
                    .join("edge")
                    .join(if cfg!(windows) {
                        "tabctl-host.cmd"
                    } else {
                        "tabctl-host.sh"
                    });
                assert!(wrapper.exists(), "wrapper script should be created");

                let manifest_path = udd
                    .join("NativeMessagingHosts")
                    .join(format!("{HOST_NAME}.json"));
                assert!(manifest_path.exists(), "native manifest should be created");
                let manifest_json: Value = serde_json::from_str(
                    &fs::read_to_string(&manifest_path).expect("read native manifest"),
                )
                .expect("parse native manifest");

                let profiles = config_dir.join("profiles.json");
                assert!(profiles.exists(), "profiles.json should be created");
                let registry: Value =
                    serde_json::from_str(&fs::read_to_string(&profiles).expect("read profiles"))
                        .expect("parse profiles");
                let extension_id = registry["profiles"]["edge"]["extensionId"]
                    .as_str()
                    .expect("derived extension id should be string");
                assert_eq!(extension_id.len(), 32);
                assert!(extension_id.chars().all(|ch| ('a'..='p').contains(&ch)));
                assert_eq!(
                    manifest_json["allowed_origins"][0],
                    json!(format!("chrome-extension://{extension_id}/"))
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

    #[cfg(windows)]
    #[test]
    fn test_transport_windows_falls_back_to_named_pipe_without_auth_token() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp_root = std::env::temp_dir().join(format!(
            "tabctl-windows-pipe-fallback-test-{}-{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&temp_root);
        std::fs::create_dir_all(&temp_root).expect("create temp directory");
        std::fs::write(temp_root.join(WSL_TCP_PORT_FILENAME), "39023\n").expect("write port file");

        with_env_vars(
            &[
                ("TABCTL_TRANSPORT", None),
                ("TABCTL_TCP_PORT", None),
                ("TABCTL_SOCKET", None),
                ("TABCTL_AUTH_TOKEN", None),
                (
                    "TABCTL_DATA_DIR",
                    Some(temp_root.to_str().expect("temp path should be valid utf-8")),
                ),
            ],
            || {
                let endpoint = resolve_socket_endpoint(None).expect("resolve endpoint");
                assert_eq!(
                    endpoint,
                    resolve_windows_pipe_endpoint(temp_root.to_str().unwrap())
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
                    step.as_str() == Some("Inspect the active profile and resolved paths: tabctl profile-show --json")
                }),
                "manual steps should include profile inspection guidance"
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
                ("TABCTL_SETUP_FETCH_EXTENSION", Some("0")),
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
                assert!(
                    steps.iter().any(|step| {
                        step.as_str() == Some("Inspect the active profile and resolved paths: tabctl profile-show --json")
                    }),
                    "manual steps should include profile inspection guidance"
                );
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_upgrade_command() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "upgrade"])
            .expect("parse upgrade command");
        let (command, _sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "upgrade");
    }

    #[test]
    fn parses_update_alias() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "update"])
            .expect("parse update alias");
        let (command, _sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "upgrade");
    }

    #[test]
    fn upgrade_with_profile_flag() {
        let matches = build_cli()
            .try_get_matches_from(["tabctl", "--profile", "edge", "upgrade"])
            .expect("parse upgrade with profile");
        let profile = matches.get_one::<String>("profile");
        assert_eq!(profile.map(|s| s.as_str()), Some("edge"));
        let (command, _sub) = matches.subcommand().expect("subcommand");
        assert_eq!(command, "upgrade");
    }

    #[test]
    fn upgrade_missing_profiles_json_returns_error() {
        let dir = std::env::temp_dir().join(format!("tabctl-test-upgrade-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).unwrap();

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let matches = build_cli()
                    .try_get_matches_from(["tabctl", "--json", "upgrade"])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                let result = run_upgrade(&matches, sub);
                assert!(result.is_err());
                assert!(
                    result.unwrap_err().contains("profiles.json not found"),
                    "should report missing profiles.json"
                );
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn upgrade_unknown_profile_returns_error() {
        let dir =
            std::env::temp_dir().join(format!("tabctl-test-upgrade-unk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let config_dir = dir.join("config");
        fs::create_dir_all(&config_dir).unwrap();

        let seed = json!({
            "default": "edge",
            "profiles": {
                "edge": {
                    "browser": "edge",
                    "extensionId": "abc",
                    "nodePath": "/bin/tabctl",
                    "hostPath": "/host.sh",
                    "dataDir": "/data"
                }
            }
        });
        fs::write(
            config_dir.join("profiles.json"),
            serde_json::to_string_pretty(&seed).unwrap(),
        )
        .unwrap();

        with_env_vars(
            &[("TABCTL_CONFIG_DIR", Some(config_dir.to_str().unwrap()))],
            || {
                let matches = build_cli()
                    .try_get_matches_from([
                        "tabctl",
                        "--profile",
                        "nonexistent",
                        "--json",
                        "upgrade",
                    ])
                    .expect("parse command");
                let (_, sub) = matches.subcommand().expect("subcommand");
                let result = run_upgrade(&matches, sub);
                assert!(result.is_err());
                assert!(
                    result.unwrap_err().contains("not found"),
                    "should report profile not found"
                );
            },
        );

        let _ = fs::remove_dir_all(&dir);
    }

    struct HangingStream {
        delay: Duration,
        writes: Vec<u8>,
    }

    impl Read for HangingStream {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            thread::sleep(self.delay);
            Ok(0)
        }
    }

    impl Write for HangingStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn send_request_over_stream_times_out_when_no_response_arrives() {
        with_env_vars(&[("TABCTL_RESPONSE_TIMEOUT_MS", Some("10"))], || {
            let stream = HangingStream {
                delay: Duration::from_millis(50),
                writes: Vec::new(),
            };
            let err =
                send_request_over_stream(stream, "ping", Value::Object(Map::new()), false, None)
                    .expect_err("request should time out");
            assert!(
                err.contains("Request timed out after 10ms"),
                "unexpected timeout error: {err}"
            );
        });
    }

    #[test]
    fn cached_snapshot_from_browser_state_extracts_snapshot_payload() {
        let response = ResponseEnvelope {
            ok: true,
            action: Some("browser-state-latest".to_string()),
            request_id: Some("req-1".to_string()),
            component: Some("host".to_string()),
            version: Some("0.6.0".to_string()),
            progress: None,
            data: Some(json!({
                "snapshotId": 1,
                "snapshot": {
                    "generatedAt": 123,
                    "windows": []
                }
            })),
            error: None,
        };

        let snapshot = cached_snapshot_from_browser_state(&response).expect("snapshot");
        assert_eq!(snapshot["generatedAt"], json!(123));
        assert_eq!(snapshot["windows"], json!([]));
    }

    #[test]
    fn cached_snapshot_from_browser_state_returns_none_without_snapshot_field() {
        let response = ResponseEnvelope {
            ok: true,
            action: Some("browser-state-latest".to_string()),
            request_id: Some("req-1".to_string()),
            component: Some("host".to_string()),
            version: Some("0.6.0".to_string()),
            progress: None,
            data: Some(json!({ "snapshotId": 1 })),
            error: None,
        };

        assert!(cached_snapshot_from_browser_state(&response).is_none());
    }

    #[test]
    fn graphql_cache_refresh_is_limited_to_structural_mutations() {
        assert!(should_refresh_graphql_snapshot_cache(
            "mutation { updateGroup(groupId: 1, title: \"Work\") { title } }"
        ));
        assert!(should_refresh_graphql_snapshot_cache(
            "mutation { moveTab(tabIds: [1], index: 0) { movedTabs } }"
        ));
        assert!(should_refresh_graphql_snapshot_cache(
            "mutation { undoAction(latest: true) { txid } }"
        ));
        assert!(should_refresh_graphql_snapshot_cache(
            "mutation { closeTabs(tabIds: [1], confirm: true) { txid } }"
        ));
        assert!(!should_refresh_graphql_snapshot_cache(
            "mutation { focusTab(tabId: 1) { success } }"
        ));
    }
}
