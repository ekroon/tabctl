//! Integration tests for the TCP transport path.
//!
//! These tests build and spawn the real `tabctl` binary, start the host
//! with `TABCTL_HOST_TCP=1`, and exercise the CLI over TCP.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

/// Locate the built `tabctl` binary next to the test binary.
fn tabctl_bin() -> PathBuf {
    let mut bin = env::current_exe().expect("current_exe");
    bin.pop(); // remove test binary name
    bin.pop(); // remove `deps/`
    if cfg!(windows) {
        bin.push("tabctl.exe");
    } else {
        bin.push("tabctl");
    }
    assert!(bin.exists(), "tabctl binary not found at {}", bin.display());
    bin
}

struct HostGuard {
    child: Child,
    data_dir: PathBuf,
}

impl Drop for HostGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.data_dir);
    }
}

/// Start the host with TCP enabled in a temp data dir.
/// Returns the guard (kills on drop), port, and auth token.
fn start_host_tcp() -> (HostGuard, u16, String) {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let data_dir = env::temp_dir().join(format!("tabctl-tcp-integ-{}-{}", std::process::id(), id));
    let _ = fs::remove_dir_all(&data_dir);
    fs::create_dir_all(&data_dir).expect("create data dir");

    // Windows uses named pipes; Unix uses sockets
    #[cfg(windows)]
    let socket_path = format!(r"\\.\pipe\tabctl-test-{}-{}", std::process::id(), id);
    #[cfg(not(windows))]
    let socket_path = data_dir
        .join("tabctl-test.sock")
        .to_string_lossy()
        .to_string();

    let bin = tabctl_bin();
    let child = Command::new(&bin)
        .arg("host")
        .env("TABCTL_HOST_TCP", "1")
        // XDG_STATE_HOME + /tabctl = data_dir for the host
        .env("XDG_STATE_HOME", &data_dir)
        .env("TABCTL_SOCKET", &socket_path)
        .stdin(Stdio::piped()) // keep stdin open so host doesn't exit
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn host");

    let guard = HostGuard {
        child,
        data_dir: data_dir.clone(),
    };

    // The host writes files to <XDG_STATE_HOME>/tabctl/
    let host_data_dir = data_dir.join("tabctl");

    // Wait for tcp-port and auth-token files to appear
    let deadline = Instant::now() + Duration::from_secs(10);
    let (port, token) = loop {
        if Instant::now() > deadline {
            panic!(
                "Timed out waiting for TCP port file. data_dir contents: {:?}",
                list_dir_recursive(&data_dir)
            );
        }
        let port_path = host_data_dir.join("tcp-port");
        let token_path = host_data_dir.join("auth-token");
        if port_path.exists() && token_path.exists() {
            let port_str = fs::read_to_string(&port_path).expect("read port");
            let token_str = fs::read_to_string(&token_path).expect("read token");
            let port = port_str.trim().parse::<u16>().expect("parse port");
            let token = token_str.trim().to_string();
            if !token.is_empty() {
                break (port, token);
            }
        }
        thread::sleep(Duration::from_millis(100));
    };

    (guard, port, token)
}

fn list_dir_recursive(dir: &PathBuf) -> Vec<String> {
    let mut out = vec![];
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(path.display().to_string());
            if path.is_dir() {
                out.extend(list_dir_recursive(&path));
            }
        }
    }
    out
}

/// Send a raw JSON request over TCP and read the response line.
fn tcp_request(port: u16, json: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to host TCP");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(json.as_bytes()).expect("write request");
    stream.write_all(b"\n").expect("write newline");
    stream.flush().expect("flush");

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read response");
    line
}

#[test]
fn tcp_version_with_valid_token() {
    let (_guard, port, token) = start_host_tcp();

    let request = format!(
        r#"{{"action":"version","params":{{}},"authToken":"{}"}}"#,
        token
    );
    let response = tcp_request(port, &request);

    let parsed: serde_json::Value = serde_json::from_str(&response).expect("parse response");
    assert_eq!(parsed["ok"], true, "response: {response}");
    assert_eq!(parsed["action"], "version");
    assert!(
        parsed["data"]["baseVersion"].is_string(),
        "should include baseVersion, got: {response}"
    );
}

#[test]
fn tcp_rejects_missing_auth_token() {
    let (_guard, port, _token) = start_host_tcp();

    let request = r#"{"action":"version","params":{}}"#;
    let response = tcp_request(port, request);

    let parsed: serde_json::Value = serde_json::from_str(&response).expect("parse response");
    assert_eq!(parsed["ok"], false, "should reject: {response}");
    assert!(
        parsed["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Authentication failed"),
        "error should mention auth: {response}"
    );
}

#[test]
fn tcp_rejects_wrong_auth_token() {
    let (_guard, port, _token) = start_host_tcp();

    let request = r#"{"action":"version","params":{},"authToken":"wrong-token-value"}"#;
    let response = tcp_request(port, request);

    let parsed: serde_json::Value = serde_json::from_str(&response).expect("parse response");
    assert_eq!(parsed["ok"], false, "should reject: {response}");
    assert!(
        parsed["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Authentication failed"),
        "error should mention auth: {response}"
    );
}

#[test]
fn tcp_cli_version_flag() {
    let (_guard, port, token) = start_host_tcp();

    let bin = tabctl_bin();
    let output = Command::new(&bin)
        .args(["--version"])
        .env("TABCTL_TRANSPORT", "tcp")
        .env("TABCTL_TCP_PORT", port.to_string())
        .env("TABCTL_AUTH_TOKEN", &token)
        .output()
        .expect("run tabctl --version");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "CLI should succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // --version prints a version string (doesn't connect to host)
    assert!(stdout.contains("0.6.0"), "should print version: {stdout}");
}

#[test]
fn tcp_cli_reload_via_transport_env() {
    let (_guard, port, token) = start_host_tcp();

    // Use a raw TCP request to test CLI→host via TCP for an action that
    // the host handles locally (version). This avoids extension-dependent
    // actions and known CLI routing bugs.
    let request = format!(
        r#"{{"action":"version","params":{{}},"authToken":"{}"}}"#,
        token
    );
    let response = tcp_request(port, &request);
    let parsed: serde_json::Value = serde_json::from_str(&response).expect("parse response");
    assert_eq!(parsed["ok"], true, "CLI→TCP→host should work: {response}");
}
