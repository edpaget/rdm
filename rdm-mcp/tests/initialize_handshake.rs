use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn initialize_handshake() {
    // Build the binary first
    let status = Command::new("cargo")
        .args(["build", "-p", "rdm-cli"])
        .status()
        .expect("failed to run cargo build");
    assert!(status.success(), "cargo build failed");

    let binary = env!("CARGO_MANIFEST_DIR").replace("rdm-mcp", "target/debug/rdm");

    let mut child = Command::new(&binary)
        .args(["--root", "/tmp/rdm-mcp-test", "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn rdm mcp");

    let mut stdin = child.stdin.take().expect("no stdin");
    let stdout = child.stdout.take().expect("no stdout");
    let mut reader = BufReader::new(stdout);

    // Send initialize request
    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "0.1.0"
            }
        }
    });
    let msg = serde_json::to_string(&init_request).unwrap();
    writeln!(stdin, "{msg}").expect("failed to write initialize");
    stdin.flush().unwrap();

    // Read initialize response
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("failed to read response");
    let response: serde_json::Value =
        serde_json::from_str(line.trim()).expect("invalid JSON response");

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    let result = &response["result"];
    assert_eq!(result["serverInfo"]["name"], "rdm-mcp");
    assert!(!result["serverInfo"]["version"].as_str().unwrap().is_empty());
    assert!(!result["protocolVersion"].as_str().unwrap().is_empty());

    // Send initialized notification
    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let msg = serde_json::to_string(&initialized).unwrap();
    writeln!(stdin, "{msg}").expect("failed to write initialized");
    stdin.flush().unwrap();

    // Close stdin to signal EOF, server should exit cleanly
    drop(stdin);

    let output = child.wait().expect("failed to wait for child");
    assert!(
        output.success(),
        "rdm mcp exited with non-zero status: {output}"
    );
}
