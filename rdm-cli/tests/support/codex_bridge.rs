//! Narrow JSON callback transport: production JavaScript executes in Node, all
//! regression scenarios and expectations stay in individually named Rust tests.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        for _ in 0..20 {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Give production cancellation handlers a chance to stop their own
        // detached subprocess groups before terminating the transport group.
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", self.0.id())])
            .status();
        for _ in 0..50 {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.0.id())])
            .status();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn invoke(
    module: &str,
    export: &str,
    args: Value,
    callback: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<Value, String> {
    invoke_request(
        json!({"module": repository().join(module), "export": export, "args": args}),
        callback,
    )
}

pub fn invoke_request(
    request: Value,
    mut callback: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let isolation = tempfile::tempdir().unwrap();
    let mut command = Command::new("node");
    command
        .arg(repository().join("rdm-cli/tests/support/codex-bridge.mjs"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", isolation.path())
        .env("CODEX_HOME", isolation.path().join("codex"))
        .env("XDG_CONFIG_HOME", isolation.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(env) = request.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            command.env(key, value.as_str().unwrap());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut process = Process(command.spawn().expect(
        "Codex Rust tests require Node.js on PATH; install Node.js before running cargo nextest",
    ));
    let mut input = process.0.stdin.take().unwrap();
    let output = process.0.stdout.take().unwrap();
    let stderr = process.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let errors = std::thread::spawn(move || std::io::read_to_string(stderr).unwrap_or_default());
    writeln!(input, "{request}").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = match receiver.recv_timeout(remaining) {
            Ok(Ok(line)) => line,
            other => {
                drop(input);
                drop(process);
                return Err(format!(
                    "Node bridge ended or timed out: {other:?}; {}",
                    errors.join().unwrap()
                ));
            }
        };
        let event: Value = serde_json::from_str(&line)
            .map_err(|error| format!("Invalid bridge output: {error}: {line}"))?;
        if event["type"] == "result" {
            return if let Some(error) = event["error"].as_str() {
                Err(error.to_owned())
            } else {
                Ok(event["value"].clone())
            };
        }
        let result = callback(event["name"].as_str().unwrap(), event["args"].clone());
        let reply = match result {
            Ok(value) => json!({"type":"reply", "id":event["id"], "value":value}),
            Err(error) => json!({"type":"reply", "id":event["id"], "error":error}),
        };
        writeln!(input, "{reply}").unwrap();
    }
}
