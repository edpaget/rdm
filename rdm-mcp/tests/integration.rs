use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::Once;

static BUILD_ONCE: Once = Once::new();

/// Build `rdm-cli` exactly once per test process, even when tests run in parallel.
///
/// Skips the build if the binary already exists on disk (e.g. when running
/// inside a pre-commit hook that already compiled everything).
fn build_once() {
    BUILD_ONCE.call_once(|| {
        let binary = env!("CARGO_MANIFEST_DIR").replace("rdm-mcp", "target/debug/rdm");
        if std::path::Path::new(&binary).exists() {
            return;
        }
        let status = Command::new("cargo")
            .args(["build", "-p", "rdm-cli"])
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "cargo build failed");
    });
}

/// Test helper that manages an rdm mcp subprocess.
struct McpTestHarness {
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    child: Option<std::process::Child>,
    next_id: u64,
}

impl McpTestHarness {
    /// Build the rdm binary and spawn `rdm mcp --root <dir>`.
    fn spawn(root: &std::path::Path) -> Self {
        Self::spawn_with_env(root, &[])
    }

    /// Spawn with additional environment variables set on the child process.
    fn spawn_with_env(root: &std::path::Path, env: &[(&str, &str)]) -> Self {
        build_once();

        let binary = env!("CARGO_MANIFEST_DIR").replace("rdm-mcp", "target/debug/rdm");

        let mut cmd = Command::new(&binary);
        cmd.args(["--root", root.to_str().unwrap(), "mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for &(k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().expect("failed to spawn rdm mcp");

        let stdin = child.stdin.take().expect("no stdin");
        let stdout = child.stdout.take().expect("no stdout");
        let reader = BufReader::new(stdout);

        let mut harness = Self {
            stdin,
            reader,
            child: Some(child),
            next_id: 1,
        };

        harness.initialize();
        harness
    }

    /// Perform the MCP initialize handshake.
    fn initialize(&mut self) {
        let response = self.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "0.1.0"
                }
            }),
        );

        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response["result"]["serverInfo"]["name"].as_str().is_some());

        // Send initialized notification
        self.notify("notifications/initialized", serde_json::json!({}));
    }

    /// Send a JSON-RPC request and return the response.
    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let msg = serde_json::to_string(&request).unwrap();
        writeln!(self.stdin, "{msg}").expect("failed to write request");
        self.stdin.flush().unwrap();

        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .expect("failed to read response");
        serde_json::from_str(line.trim()).expect("invalid JSON response")
    }

    /// Send a JSON-RPC notification (no response expected).
    fn notify(&mut self, method: &str, params: serde_json::Value) {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let msg = serde_json::to_string(&notification).unwrap();
        writeln!(self.stdin, "{msg}").expect("failed to write notification");
        self.stdin.flush().unwrap();
    }

    /// Invoke an MCP tool and return the result object.
    fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        self.request(
            "tools/call",
            serde_json::json!({
                "name": name,
                "arguments": arguments
            }),
        )
    }
}

impl Drop for McpTestHarness {
    fn drop(&mut self) {
        // Close stdin to signal EOF
        drop(std::mem::replace(
            &mut self.stdin,
            // Safety: we're in Drop, need a placeholder. Use /dev/null.
            Command::new("true")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap()
                .stdin
                .take()
                .unwrap(),
        ));
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

/// Set up a plan repo with sample data for testing.
fn setup_plan_repo(root: &std::path::Path) {
    build_once();
    let binary = env!("CARGO_MANIFEST_DIR").replace("rdm-mcp", "target/debug/rdm");

    let run = |args: &[&str]| {
        let status = Command::new(&binary)
            .args(args)
            .status()
            .unwrap_or_else(|e| panic!("failed to run rdm {}: {e}", args.join(" ")));
        assert!(
            status.success(),
            "rdm {} failed with status {status}",
            args.join(" ")
        );
    };

    let root_str = root.to_str().unwrap();

    // Initialize plan repo
    run(&["--root", root_str, "init"]);

    // Create a project
    run(&[
        "--root",
        root_str,
        "project",
        "create",
        "test-proj",
        "--title",
        "Test Project",
    ]);

    // Create a roadmap
    run(&[
        "--root",
        root_str,
        "roadmap",
        "create",
        "auth",
        "--title",
        "Authentication",
        "--body",
        "Implement authentication system.",
        "--no-edit",
        "--project",
        "test-proj",
    ]);

    // Create phases
    run(&[
        "--root",
        root_str,
        "phase",
        "create",
        "design",
        "--title",
        "Design Auth",
        "--number",
        "1",
        "--body",
        "Design the auth flow.",
        "--no-edit",
        "--roadmap",
        "auth",
        "--project",
        "test-proj",
    ]);
    run(&[
        "--root",
        root_str,
        "phase",
        "create",
        "implement",
        "--title",
        "Implement Auth",
        "--number",
        "2",
        "--body",
        "Build the auth endpoints.",
        "--no-edit",
        "--roadmap",
        "auth",
        "--project",
        "test-proj",
    ]);

    // Create a task
    run(&[
        "--root",
        root_str,
        "task",
        "create",
        "fix-login-bug",
        "--title",
        "Fix login bug",
        "--body",
        "Login fails when password contains special chars.",
        "--no-edit",
        "--project",
        "test-proj",
    ]);
}

#[test]
fn initialize_handshake() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let _harness = McpTestHarness::spawn(tmp.path());
    // If we get here, the handshake succeeded
}

#[test]
fn tools_list() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.request("tools/list", serde_json::json!({}));
    let tools = response["result"]["tools"].as_array().expect("tools array");

    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

    let expected = [
        // Init tool
        "rdm_init",
        // Read-only tools
        "rdm_project_list",
        "rdm_roadmap_list",
        "rdm_roadmap_show",
        "rdm_phase_list",
        "rdm_phase_show",
        "rdm_task_list",
        "rdm_task_show",
        "rdm_search",
        // Mutation tools
        "rdm_project_create",
        "rdm_roadmap_create",
        "rdm_roadmap_update",
        "rdm_phase_create",
        "rdm_phase_update",
        "rdm_task_create",
        "rdm_task_update",
        "rdm_task_promote",
    ];

    for name in &expected {
        assert!(
            tool_names.contains(name),
            "Missing tool: {name}. Found: {tool_names:?}"
        );
    }
    assert_eq!(tool_names.len(), expected.len());
}

#[test]
fn project_list() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool("rdm_project_list", serde_json::json!({}));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("test-proj"),
        "Expected 'test-proj' in: {text}"
    );
}

#[test]
fn roadmap_list() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "test-proj"}),
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Authentication"),
        "Expected 'Authentication' in: {text}"
    );
}

#[test]
fn roadmap_show() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "auth"}),
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Authentication"),
        "Expected 'Authentication' in: {text}"
    );
    assert!(
        text.contains("Design Auth"),
        "Expected phase title in: {text}"
    );
}

#[test]
fn phase_list() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_phase_list",
        serde_json::json!({"project": "test-proj", "roadmap": "auth"}),
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Design Auth"),
        "Expected 'Design Auth' in: {text}"
    );
    assert!(
        text.contains("Implement Auth"),
        "Expected 'Implement Auth' in: {text}"
    );
}

#[test]
fn phase_show() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Test by phase number
    let response = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({"project": "test-proj", "roadmap": "auth", "phase": "1"}),
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Design Auth"),
        "Expected 'Design Auth' in: {text}"
    );
    assert!(
        text.contains("Design the auth flow"),
        "Expected body in: {text}"
    );
}

#[test]
fn task_list() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool("rdm_task_list", serde_json::json!({"project": "test-proj"}));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Fix login bug"),
        "Expected 'Fix login bug' in: {text}"
    );
}

#[test]
fn task_show() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_task_show",
        serde_json::json!({"project": "test-proj", "task": "fix-login-bug"}),
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Fix login bug"),
        "Expected 'Fix login bug' in: {text}"
    );
    assert!(
        text.contains("special chars"),
        "Expected body content in: {text}"
    );
}

#[test]
fn search() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_search",
        serde_json::json!({"query": "auth", "project": "test-proj"}),
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Authentication") || text.contains("auth"),
        "Expected search results for 'auth' in: {text}"
    );
}

/// Helper to extract text from a successful MCP tool call response.
fn result_text(response: &serde_json::Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("expected text in result content")
}

#[test]
fn tools_list_includes_mutation_tools() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.request("tools/list", serde_json::json!({}));
    let tools = response["result"]["tools"].as_array().expect("tools array");

    let mutation_tools = [
        "rdm_init",
        "rdm_project_create",
        "rdm_roadmap_create",
        "rdm_roadmap_update",
        "rdm_phase_create",
        "rdm_phase_update",
        "rdm_task_create",
        "rdm_task_update",
        "rdm_task_promote",
    ];

    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for name in &mutation_tools {
        assert!(
            tool_names.contains(name),
            "Missing mutation tool: {name}. Found: {tool_names:?}"
        );
    }

    // Verify read_only_hint annotations
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let read_only = tool["annotations"]["readOnlyHint"].as_bool();
        if mutation_tools.contains(&name) {
            assert_eq!(
                read_only,
                Some(false),
                "Mutation tool {name} should have readOnlyHint=false"
            );
        } else {
            assert_eq!(
                read_only,
                Some(true),
                "Read-only tool {name} should have readOnlyHint=true"
            );
        }
    }
}

#[test]
fn project_create() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_project_create",
        serde_json::json!({
            "name": "billing",
            "title": "Billing System"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("billing"),
        "Expected project name in create response: {text}"
    );

    // Verify it persists via rdm_project_list
    let list = h.call_tool("rdm_project_list", serde_json::json!({}));
    let list_text = result_text(&list);
    assert!(
        list_text.contains("billing"),
        "Expected 'billing' in project list: {list_text}"
    );
}

#[test]
fn project_create_title_defaults_to_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_project_create",
        serde_json::json!({
            "name": "my-proj"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("my-proj"),
        "Expected project name in create response: {text}"
    );
}

#[test]
fn project_create_duplicate() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // test-proj already exists from setup_plan_repo
    let response = h.call_tool(
        "rdm_project_create",
        serde_json::json!({
            "name": "test-proj",
            "title": "Duplicate"
        }),
    );
    let is_error = response["result"]["isError"].as_bool().unwrap_or(false);
    assert!(is_error, "Expected error for duplicate project: {response}");
}

#[test]
fn roadmap_create() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "billing",
            "title": "Billing System",
            "body": "Implement billing and invoicing."
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Billing System"),
        "Expected 'Billing System' in create response: {text}"
    );

    // Verify it persists via rdm_roadmap_show
    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "billing"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("Billing System"),
        "Expected 'Billing System' in show response: {show_text}"
    );
}

#[test]
fn phase_create() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_phase_create",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "slug": "testing",
            "title": "Test Auth",
            "number": 3,
            "body": "Write integration tests for auth."
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Test Auth"),
        "Expected 'Test Auth' in create response: {text}"
    );

    // Verify via phase_show
    let show = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({"project": "test-proj", "roadmap": "auth", "phase": "3"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("Test Auth"),
        "Expected 'Test Auth' in show response: {show_text}"
    );
}

#[test]
fn phase_update() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "1",
            "status": "done"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("done"),
        "Expected 'done' status in update response: {text}"
    );

    // Verify persisted
    let show = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({"project": "test-proj", "roadmap": "auth", "phase": "1"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("done"),
        "Expected 'done' in show response: {show_text}"
    );
}

#[test]
fn task_create() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_task_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "add-logging",
            "title": "Add structured logging",
            "priority": "high",
            "tags": ["observability", "infra"],
            "body": "Add structured JSON logging throughout the app."
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Add structured logging"),
        "Expected title in create response: {text}"
    );

    // Verify via task_show
    let show = h.call_tool(
        "rdm_task_show",
        serde_json::json!({"project": "test-proj", "task": "add-logging"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("Add structured logging"),
        "Expected title in show response: {show_text}"
    );
    assert!(
        show_text.contains("high"),
        "Expected 'high' priority in show response: {show_text}"
    );
}

#[test]
fn task_update() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "status": "done"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("done"),
        "Expected 'done' in update response: {text}"
    );

    // Verify persisted
    let show = h.call_tool(
        "rdm_task_show",
        serde_json::json!({"project": "test-proj", "task": "fix-login-bug"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("done"),
        "Expected 'done' in show response: {show_text}"
    );
}

#[test]
fn task_promote() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_task_promote",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "roadmap_slug": "login-fix"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Fix login bug") || text.contains("login-fix"),
        "Expected roadmap info in promote response: {text}"
    );

    // Verify roadmap was created
    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "login-fix"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("Fix login bug") || show_text.contains("login-fix"),
        "Expected promoted roadmap in show response: {show_text}"
    );
}

#[test]
fn end_to_end_workflow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let binary = env!("CARGO_MANIFEST_DIR").replace("rdm-mcp", "target/debug/rdm");

    // Minimal setup: just init + project create
    let run = |args: &[&str]| {
        let status = Command::new(&binary)
            .args(args)
            .status()
            .unwrap_or_else(|e| panic!("failed to run rdm {}: {e}", args.join(" ")));
        assert!(
            status.success(),
            "rdm {} failed with status {status}",
            args.join(" ")
        );
    };
    let root_str = root.to_str().unwrap();
    run(&["--root", root_str, "init"]);
    run(&[
        "--root", root_str, "project", "create", "e2e", "--title", "E2E Test",
    ]);

    let mut h = McpTestHarness::spawn(root);

    // 1. Create roadmap
    let resp = h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "e2e",
            "slug": "onboarding",
            "title": "User Onboarding",
            "body": "Implement the onboarding flow."
        }),
    );
    assert!(
        result_text(&resp).contains("User Onboarding"),
        "roadmap create failed: {}",
        result_text(&resp)
    );

    // 2. Create phases
    let resp = h.call_tool(
        "rdm_phase_create",
        serde_json::json!({
            "project": "e2e",
            "roadmap": "onboarding",
            "slug": "design",
            "title": "Design Onboarding",
            "number": 1,
            "body": "Design the onboarding UX."
        }),
    );
    assert!(
        result_text(&resp).contains("Design Onboarding"),
        "phase 1 create failed"
    );

    let resp = h.call_tool(
        "rdm_phase_create",
        serde_json::json!({
            "project": "e2e",
            "roadmap": "onboarding",
            "slug": "build",
            "title": "Build Onboarding",
            "number": 2,
            "body": "Implement the onboarding screens."
        }),
    );
    assert!(
        result_text(&resp).contains("Build Onboarding"),
        "phase 2 create failed"
    );

    // 3. Verify roadmap shows phases
    let resp = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "e2e", "roadmap": "onboarding"}),
    );
    let text = result_text(&resp);
    assert!(
        text.contains("Design Onboarding"),
        "roadmap show missing phase 1"
    );
    assert!(
        text.contains("Build Onboarding"),
        "roadmap show missing phase 2"
    );

    // 4. Update phase status
    let resp = h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "e2e",
            "roadmap": "onboarding",
            "phase": "1",
            "status": "in-progress"
        }),
    );
    assert!(
        result_text(&resp).contains("in-progress"),
        "phase update to in-progress failed"
    );

    let resp = h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "e2e",
            "roadmap": "onboarding",
            "phase": "1",
            "status": "done"
        }),
    );
    assert!(
        result_text(&resp).contains("done"),
        "phase update to done failed"
    );

    // Verify phase status persisted
    let resp = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({"project": "e2e", "roadmap": "onboarding", "phase": "1"}),
    );
    assert!(
        result_text(&resp).contains("done"),
        "phase show should reflect done status"
    );

    // 5. Create a task
    let resp = h.call_tool(
        "rdm_task_create",
        serde_json::json!({
            "project": "e2e",
            "slug": "fix-tooltip",
            "title": "Fix tooltip positioning",
            "body": "Tooltips overflow on mobile screens."
        }),
    );
    assert!(
        result_text(&resp).contains("Fix tooltip positioning"),
        "task create failed"
    );

    // 6. Search for the task
    let resp = h.call_tool(
        "rdm_search",
        serde_json::json!({"query": "tooltip", "project": "e2e"}),
    );
    assert!(
        result_text(&resp).contains("tooltip") || result_text(&resp).contains("Tooltip"),
        "search should find tooltip task: {}",
        result_text(&resp)
    );

    // 7. Promote task to roadmap
    let resp = h.call_tool(
        "rdm_task_promote",
        serde_json::json!({
            "project": "e2e",
            "task": "fix-tooltip",
            "roadmap_slug": "tooltip-fix"
        }),
    );
    let text = result_text(&resp);
    assert!(
        text.contains("tooltip") || text.contains("Tooltip"),
        "promote response should reference tooltip: {text}"
    );

    // 8. Verify promoted roadmap exists
    let resp = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "e2e", "roadmap": "tooltip-fix"}),
    );
    let text = result_text(&resp);
    assert!(
        text.contains("Fix tooltip positioning") || text.contains("tooltip-fix"),
        "promoted roadmap should exist: {text}"
    );
}

#[test]
fn error_missing_project() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "nonexistent"}),
    );
    let result = &response["result"];
    assert_eq!(
        result["isError"],
        serde_json::json!(true),
        "Expected isError=true for missing project. Full result: {result}"
    );
}

#[test]
fn init_via_mcp() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Start with an empty directory — no setup_plan_repo
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool("rdm_init", serde_json::json!({}));
    let text = result_text(&response);
    assert!(
        text.contains("initialized"),
        "Expected 'initialized' in init response: {text}"
    );

    // Verify we can now list projects (should succeed with empty list)
    let response = h.call_tool("rdm_project_list", serde_json::json!({}));
    let result = &response["result"];
    assert!(
        result["isError"].is_null() || result["isError"] == false,
        "Expected project_list to succeed after init. Result: {result}"
    );
}

#[test]
fn init_with_default_project() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_init",
        serde_json::json!({"default_project": "my-proj"}),
    );
    let text = result_text(&response);
    assert!(
        text.contains("my-proj"),
        "Expected 'my-proj' in init response: {text}"
    );

    // Verify the project was created
    let response = h.call_tool("rdm_project_list", serde_json::json!({}));
    let text = result_text(&response);
    assert!(
        text.contains("my-proj"),
        "Expected 'my-proj' in project list: {text}"
    );
}

#[test]
fn init_already_initialized() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool("rdm_init", serde_json::json!({}));
    let result = &response["result"];
    assert_eq!(
        result["isError"],
        serde_json::json!(true),
        "Expected isError=true for double init. Full result: {result}"
    );
}

#[test]
fn error_uninitialized_repo() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Empty directory — no init
    let mut h = McpTestHarness::spawn(tmp.path());

    // Creating a roadmap on an uninitialized repo should fail with an actionable error
    let response = h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "auth",
            "title": "Auth"
        }),
    );
    let result = &response["result"];
    assert_eq!(
        result["isError"],
        serde_json::json!(true),
        "Expected isError=true for uninitialized repo. Full result: {result}"
    );
    let text = result["content"][0]["text"].as_str().unwrap();
    // Should get a meaningful error (project not found since no projects exist)
    assert!(!text.is_empty(), "Error should have a message: {text}");
}

// ==================== Roadmap priority tests ====================

#[test]
fn roadmap_create_with_priority() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "urgent",
            "title": "Urgent Fix",
            "priority": "high"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Urgent Fix"),
        "Expected title in response: {text}"
    );
    assert!(
        text.contains("high"),
        "Expected priority in response: {text}"
    );
}

#[test]
fn roadmap_update_priority() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_roadmap_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "priority": "critical"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("critical"),
        "Expected 'critical' in update response: {text}"
    );

    // Verify via show
    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "auth"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("critical"),
        "Expected 'critical' in show response: {show_text}"
    );
}

#[test]
fn roadmap_update_clear_priority() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Set priority first
    h.call_tool(
        "rdm_roadmap_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "priority": "high"
        }),
    );

    // Clear it
    let response = h.call_tool(
        "rdm_roadmap_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "clear_priority": true
        }),
    );
    let text = result_text(&response);
    assert!(
        !text.contains("Priority:"),
        "Expected no priority in response after clearing: {text}"
    );
}

#[test]
fn roadmap_list_with_priority_filter() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Create a high-priority roadmap
    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "urgent",
            "title": "Urgent",
            "priority": "high"
        }),
    );

    let response = h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "test-proj", "priority": "high"}),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Urgent"),
        "Expected 'Urgent' in filtered list: {text}"
    );
    assert!(
        !text.contains("Authentication"),
        "Should not contain non-high roadmaps: {text}"
    );
}

#[test]
fn roadmap_list_with_sort_priority() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Create a critical roadmap
    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "critical-rm",
            "title": "Critical Work",
            "priority": "critical"
        }),
    );

    let response = h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "test-proj", "sort": "priority"}),
    );
    let text = result_text(&response);
    let critical_pos = text
        .find("Critical Work")
        .expect("Critical Work should appear");
    let auth_pos = text
        .find("Authentication")
        .expect("Authentication should appear");
    assert!(
        critical_pos < auth_pos,
        "Critical should sort before non-priority roadmap"
    );
}

// ==================== Tag tests (expand-tag-support phase 4) ====================

#[test]
fn roadmap_create_with_tags_persists_them() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "tagged-rm",
            "title": "Tagged Roadmap",
            "tags": ["bug", "ui"],
        }),
    );

    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "tagged-rm"}),
    );
    let text = result_text(&show);
    assert!(text.contains("Tagged Roadmap"));
    assert!(text.contains("bug"), "show should display tags: {text}");
    assert!(text.contains("ui"));
}

#[test]
fn roadmap_update_replaces_tags() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "rm",
            "title": "RM",
            "tags": ["a", "b"],
        }),
    );
    h.call_tool(
        "rdm_roadmap_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "rm",
            "tags": ["c"],
        }),
    );
    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "rm"}),
    );
    let text = result_text(&show);
    // Tags render as `Tags: a, b, c` — assert the new tag appears in that
    // context so we don't false-positive on incidental letters.
    assert!(
        text.contains("Tags: c"),
        "Tags: line should show 'c': {text}"
    );
    assert!(
        !text.contains("Tags: a") && !text.contains(", a") && !text.contains(", b"),
        "old tags 'a'/'b' should be removed: {text}"
    );
}

#[test]
fn roadmap_update_clear_tags() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "rm",
            "title": "RM",
            "tags": ["a"],
        }),
    );
    h.call_tool(
        "rdm_roadmap_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "rm",
            "clear_tags": true,
        }),
    );
    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "rm"}),
    );
    let text = result_text(&show);
    assert!(
        !text.contains("Tags:"),
        "Tags: line should be omitted when no tags: {text}"
    );
}

#[test]
fn roadmap_update_conflicting_tag_fields_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_roadmap_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "tags": ["x"],
            "clear_tags": true,
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("cannot set both 'tags' and 'clear_tags'"),
        "expected conflict error, got: {text}"
    );
}

#[test]
fn roadmap_list_filter_by_tag() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "tagged-rm",
            "title": "Tagged",
            "tags": ["needle"],
        }),
    );
    let response = h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "test-proj", "tag": "needle"}),
    );
    let text = result_text(&response);
    assert!(text.contains("Tagged"));
    assert!(
        !text.contains("Authentication"),
        "untagged roadmap should be excluded: {text}"
    );
}

#[test]
fn phase_create_with_tags_persists_them() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_phase_create",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "slug": "review",
            "title": "Review",
            "number": 3,
            "tags": ["audit", "security"],
        }),
    );
    let show = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "phase-3-review",
        }),
    );
    let text = result_text(&show);
    assert!(text.contains("audit"), "tags should appear in show: {text}");
    assert!(text.contains("security"));
}

#[test]
fn phase_update_replaces_tags_and_clear_tags() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "phase-1-design",
            "tags": ["temp"],
        }),
    );
    let show1 = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "phase-1-design",
        }),
    );
    assert!(result_text(&show1).contains("temp"));

    h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "phase-1-design",
            "clear_tags": true,
        }),
    );
    let show2 = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "phase-1-design",
        }),
    );
    let text = result_text(&show2);
    assert!(
        !text.contains("Tags:"),
        "Tags: line should be omitted after clear_tags: {text}"
    );
}

#[test]
fn phase_list_filter_by_tag() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Tag phase 1 with 'needle'.
    h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "phase-1-design",
            "tags": ["needle"],
        }),
    );

    let response = h.call_tool(
        "rdm_phase_list",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "tag": "needle",
        }),
    );
    let text = result_text(&response);
    assert!(text.contains("Design Auth"));
    assert!(
        !text.contains("Implement Auth"),
        "untagged phase should be excluded: {text}"
    );
}

#[test]
fn search_filter_by_tag() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "matchable",
            "title": "Matchable Roadmap",
            "tags": ["needle"],
        }),
    );
    let response = h.call_tool(
        "rdm_search",
        serde_json::json!({
            "query": "Roadmap",
            "project": "test-proj",
            "tags": ["needle"],
        }),
    );
    let text = result_text(&response);
    assert!(text.contains("Matchable"));
    assert!(
        !text.contains("Authentication"),
        "untagged roadmap should be excluded by tag filter: {text}"
    );
}

#[test]
fn search_tag_filter_ands_multiple_tags() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Has both `alpha` and `beta` — should match an AND filter on those two.
    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "both",
            "title": "Both Tags",
            "tags": ["alpha", "beta"],
        }),
    );
    // Has only `alpha` — should be excluded when both `alpha` AND `beta` are required.
    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "only-alpha",
            "title": "Only Alpha",
            "tags": ["alpha"],
        }),
    );

    let response = h.call_tool(
        "rdm_search",
        serde_json::json!({
            "query": "",
            "project": "test-proj",
            "tags": ["alpha", "beta"],
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Both Tags"),
        "roadmap with both tags should match: {text}"
    );
    assert!(
        !text.contains("Only Alpha"),
        "roadmap with only `alpha` must be excluded by AND filter: {text}"
    );
}

// ==================== GitStore integration tests ====================

/// Run a git command in `root`, clearing `GIT_DIR` / `GIT_WORK_TREE` so the
/// command targets the temp-dir repo rather than a parent repo leaked via
/// environment variables (e.g. inside pre-commit hooks).
fn git_cmd(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("failed to run git command")
}

/// Get the HEAD commit SHA in a repo, or empty string if no commits.
fn git_head_sha(root: &std::path::Path) -> String {
    let output = git_cmd(root, &["rev-parse", "HEAD"]);
    if !output.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Get git log output (one line per commit) from a repo.
fn git_log(root: &std::path::Path) -> String {
    let output = git_cmd(root, &["log", "--oneline"]);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn git_mutation_creates_commit() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());

    let before = git_head_sha(tmp.path());

    let mut h = McpTestHarness::spawn(tmp.path());
    let response = h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "billing",
            "title": "Billing System"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Billing System"),
        "Expected creation to succeed: {text}"
    );
    drop(h);

    let after = git_head_sha(tmp.path());
    assert_ne!(before, after, "Expected HEAD to advance after mutation");

    let log = git_log(tmp.path());
    assert!(
        log.contains("rdm:"),
        "Expected auto-commit message with 'rdm:' prefix in log:\n{log}"
    );
}

#[test]
fn git_staging_mode_defers_commit() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());

    let before = git_head_sha(tmp.path());

    let mut h = McpTestHarness::spawn_with_env(tmp.path(), &[("RDM_STAGE", "true")]);
    let response = h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "billing",
            "title": "Billing System"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Billing System"),
        "Expected creation to succeed in staging mode: {text}"
    );

    // Verify the roadmap is readable (data was written to disk)
    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "billing"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("Billing System"),
        "Expected roadmap to be readable in staging mode: {show_text}"
    );
    drop(h);

    let after = git_head_sha(tmp.path());
    assert_eq!(
        before, after,
        "Expected NO new git commits in staging mode (before={before}, after={after})"
    );
}

#[test]
fn git_read_tools_no_commit() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());

    let before = git_head_sha(tmp.path());

    let mut h = McpTestHarness::spawn(tmp.path());

    // Run several read-only tools
    h.call_tool("rdm_project_list", serde_json::json!({}));
    h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "test-proj"}),
    );
    h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "auth"}),
    );
    h.call_tool("rdm_task_list", serde_json::json!({"project": "test-proj"}));
    drop(h);

    let after = git_head_sha(tmp.path());
    assert_eq!(
        before, after,
        "Expected no git commits from read-only tools (before={before}, after={after})"
    );
}
