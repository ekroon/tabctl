use super::*;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

static BROWSER: OnceLock<SharedBrowser> = OnceLock::new();
static BOOTSTRAP_PID: AtomicU32 = AtomicU32::new(0);

extern "C" {
    fn atexit(function: extern "C" fn()) -> std::ffi::c_int;
    fn kill(pid: std::ffi::c_int, signal: std::ffi::c_int) -> std::ffi::c_int;
    fn waitpid(
        pid: std::ffi::c_int,
        status: *mut std::ffi::c_int,
        options: std::ffi::c_int,
    ) -> std::ffi::c_int;
    fn _exit(status: std::ffi::c_int) -> !;
}

extern "C" fn cleanup_bootstrap() {
    let pid = BOOTSTRAP_PID.swap(0, Ordering::SeqCst);
    if pid != 0 && pid <= i32::MAX as u32 {
        unsafe {
            kill(pid as i32, 15);
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let mut status = 0;
            let waited = unsafe { waitpid(pid as i32, &mut status, 1) };
            if waited == pid as i32 {
                if status != 0 {
                    eprintln!("Browser fixture cleanup failed: wait status {status}");
                    unsafe { _exit(1) }
                }
                return;
            }
            if waited < 0 {
                eprintln!(
                    "Failed to wait for browser fixture: {}",
                    std::io::Error::last_os_error()
                );
                unsafe { _exit(1) }
            }
            if Instant::now() >= deadline {
                unsafe {
                    kill(pid as i32, 9);
                }
                eprintln!("Browser fixture cleanup timed out");
                unsafe { _exit(1) }
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

pub struct SharedBrowser {
    pub tabctl_bin: PathBuf,
    pub root: PathBuf,
    pub profile_name: String,
    pub config_home: PathBuf,
    pub state_home: PathBuf,
    pub fixture_root: PathBuf,
    pub extension_id: String,
}

impl SharedBrowser {
    pub fn run(&self, args: &[&str]) -> Value {
        self.run_result(args)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    pub fn run_result(&self, args: &[&str]) -> Result<Value, String> {
        self.run_timeout(args, Duration::from_secs(30))
    }

    pub fn run_timeout(&self, args: &[&str], timeout: Duration) -> Result<Value, String> {
        run_tabctl_json_with_timeout(
            &self.tabctl_bin,
            &self.root,
            &self.profile_name,
            &self.config_home,
            &self.state_home,
            args,
            timeout,
        )
    }

    pub fn output(&self, args: &[&str]) -> Output {
        run_tabctl_output(
            &self.tabctl_bin,
            &self.root,
            &self.profile_name,
            &self.config_home,
            &self.state_home,
            args,
            Duration::from_secs(30),
        )
        .expect("run isolated CLI")
    }

    pub fn run_query(&self, query: &str) -> Value {
        self.run_query_result(query)
            .unwrap_or_else(|error| panic!("{error}\nquery: {query}"))
    }

    pub fn run_query_result(&self, query: &str) -> Result<Value, String> {
        self.run_result(&["query", query])
    }

    pub fn wait_for_host_ready(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            match self.run_timeout(&["ping"], Duration::from_secs(5)) {
                Ok(ping)
                    if ping.get("runtimeId").and_then(Value::as_str)
                        == Some(self.extension_id.as_str()) =>
                {
                    return
                }
                result if Instant::now() >= deadline => {
                    panic!("Browser did not reconnect: {result:?}")
                }
                _ => thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    pub fn create_test_window(&self, urls: &[&str], group: Option<&str>) -> (i64, Vec<i64>) {
        let urls = urls
            .iter()
            .map(|url| gql_string(url))
            .collect::<Vec<_>>()
            .join(", ");
        let open = self.run_query(&format!(
            "mutation {{ openTabs(urls: [{urls}], newWindow: true) {{ windowId tabs {{ tabId }} }} }}"
        ));
        let data = &response_data(&open)["openTabs"];
        let window_id = data["windowId"].as_i64().expect("created window ID");
        let tab_ids = data["tabs"]
            .as_array()
            .expect("created tabs")
            .iter()
            .map(|tab| tab["tabId"].as_i64().expect("created tab ID"))
            .collect::<Vec<_>>();
        if let Some(title) = group {
            let ids = tab_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            self.run_query(&format!(
                "mutation {{ assignToGroup(tabIds: [{ids}], groupTitle: {}) {{ groupId }} }}",
                gql_string(title)
            ));
        }
        (window_id, tab_ids)
    }

    pub fn close_test_window(&self, window_id: i64) {
        let result = self.seed_browser("window-remove", json!({"windowId":window_id}));
        if result.get("error").is_some() {
            eprintln!("Test window cleanup: {result}");
        }
    }

    pub fn seed_browser(&self, action: &str, params: Value) -> Value {
        self.socket_request(self.fixture_root.join("control.sock"), action, params)
    }

    pub fn send_host_request(&self, action: &str, params: Value) -> Value {
        let socket_path = self
            .state_home
            .join("tabctl/profiles")
            .join(&self.profile_name)
            .join("tabctl.sock");
        self.socket_request(socket_path, action, params)
    }

    fn socket_request(&self, socket_path: PathBuf, action: &str, params: Value) -> Value {
        let mut stream = UnixStream::connect(&socket_path).expect("connect isolated host");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("set host read timeout");
        let request = json!({"id":format!("itest-{}",now_ms()),"action":action,"params":params});
        serde_json::to_writer(&mut stream, &request).expect("encode host request");
        stream.write_all(b"\n").expect("terminate host request");
        stream.flush().expect("flush host request");
        for line in BufReader::new(stream).lines() {
            let line = line.expect("read host response");
            if line.trim().is_empty() {
                continue;
            }
            let response: Value = serde_json::from_str(&line).expect("decode host response");
            if response.get("progress").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if response.get("ok").and_then(Value::as_bool) == Some(true) {
                return response["data"].clone();
            }
            return response;
        }
        panic!("Host closed without responding to {action}");
    }

    pub fn window_tabs(&self, window_id: i64) -> Vec<Value> {
        let snapshot = self.send_host_request("snapshot", json!({}));
        snapshot["windows"]
            .as_array()
            .expect("live windows")
            .iter()
            .find(|window| window["windowId"].as_i64() == Some(window_id))
            .and_then(|window| window["tabs"].as_array())
            .cloned()
            .unwrap_or_default()
    }
}

fn init_browser() -> SharedBrowser {
    let root = repo_root();
    let tabctl_bin = rust_tabctl_bin(&root);
    assert!(tabctl_bin.exists(), "Run npm run build first");
    let fixture_root = create_sandbox();
    let node = std::env::var("TABCTL_NODE_EXEC").unwrap_or_else(|_| "node".into());
    let mut child = Command::new(node)
        .arg(root.join("scripts/ci/integration-bootstrap.js"))
        .current_dir(&root)
        .env("TABCTL_BIN", &tabctl_bin)
        .env("TABCTL_FIXTURE_ROOT", &fixture_root)
        .env("TABCTL_EXTENSION_DIR", root.join("dist/extension"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start isolated browser fixture");
    BOOTSTRAP_PID.store(child.id(), Ordering::SeqCst);
    unsafe {
        atexit(cleanup_bootstrap);
    }
    let stdout = child.stdout.take().expect("fixture stdout");
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("read fixture ready signal");
            if let Ok(ready) = serde_json::from_str::<Value>(&line) {
                if ready["ok"] == true && ready["profile"].is_string() {
                    tx.send(ready).expect("deliver fixture ready signal");
                    return;
                }
            }
        }
    });
    let ready = rx
        .recv_timeout(Duration::from_secs(90))
        .expect("browser fixture startup failed");
    assert_eq!(ready["tmpDir"].as_str(), fixture_root.to_str());
    let browser = SharedBrowser {
        tabctl_bin,
        root,
        profile_name: ready["profile"].as_str().expect("fixture profile").into(),
        config_home: fixture_root.join("c"),
        state_home: fixture_root.join("s"),
        extension_id: ready["extensionId"]
            .as_str()
            .expect("fixture extension ID")
            .into(),
        fixture_root,
    };
    std::mem::forget(child);
    browser
}

pub fn shared_browser() -> &'static SharedBrowser {
    BROWSER.get_or_init(init_browser)
}
