use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use std::process::{self, Command, Stdio};
use std::thread;

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(1);
}

fn main() {
    let exe_path = env::current_exe().unwrap_or_else(|err| {
        fail(&format!("tabctl-host: cannot resolve exe path: {err}"));
    });
    let exe_dir = exe_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let cfg_path = exe_dir.join("host-launcher.cfg");

    let file = File::open(&cfg_path).unwrap_or_else(|err| {
        fail(&format!(
            "tabctl-host: cannot open {}: {err}",
            cfg_path.display()
        ));
    });
    let mut lines = BufReader::new(file).lines();

    let node_path = match lines.next() {
        Some(Ok(line)) => line.trim().to_owned(),
        Some(Err(err)) => fail(&format!(
            "tabctl-host: cannot read {}: {err}",
            cfg_path.display()
        )),
        None => fail(&format!(
            "tabctl-host: missing node path in {}",
            cfg_path.display()
        )),
    };

    let host_path = match lines.next() {
        Some(Ok(line)) => line.trim().to_owned(),
        Some(Err(err)) => fail(&format!(
            "tabctl-host: cannot read {}: {err}",
            cfg_path.display()
        )),
        None => fail(&format!(
            "tabctl-host: missing host path in {}",
            cfg_path.display()
        )),
    };

    for line in lines {
        let line = line.unwrap_or_else(|err| {
            fail(&format!(
                "tabctl-host: cannot read {}: {err}",
                cfg_path.display()
            ))
        });
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            if eq > 0 {
                let key = trimmed[..eq].trim();
                let value = trimmed[eq + 1..].trim();
                env::set_var(key, value);
            }
        }
    }

    let mut child = Command::new(&node_path)
        .arg(&host_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|err| fail(&format!("tabctl-host: failed to start: {err}")));

    let mut node_in = child
        .stdin
        .take()
        .unwrap_or_else(|| fail("tabctl-host: stdin pipe: unavailable"));
    let mut node_out = child
        .stdout
        .take()
        .unwrap_or_else(|| fail("tabctl-host: stdout pipe: unavailable"));

    let stdin_to_child = thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let _ = io::copy(&mut stdin, &mut node_in);
    });
    let child_to_stdout = thread::spawn(move || {
        let mut stdout = io::stdout().lock();
        let _ = io::copy(&mut node_out, &mut stdout);
    });

    let status = child.wait().unwrap_or_else(|_| process::exit(1));
    let _ = child_to_stdout.join();
    drop(stdin_to_child);
    match status.code() {
        Some(code) => process::exit(code),
        None => process::exit(1),
    }
}
