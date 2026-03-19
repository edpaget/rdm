use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

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
        let status = Command::new("cargo")
            .args(["build", "-p", "rdm-cli"])
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "cargo build failed");

        let binary = env!("CARGO_MANIFEST_DIR").replace("rdm-mcp", "target/debug/rdm");

        let mut child = Command::new(&binary)
            .args(["--root", root.to_str().unwrap(), "mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn rdm mcp");

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
        "rdm_roadmap_create",
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
        "rdm_roadmap_create",
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
