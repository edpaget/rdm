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

    // Land a real commit so the fixture starts from a clean, committed state.
    run(&[
        "--root",
        root_str,
        "commit",
        "-m",
        "seed: plan repo fixture",
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
        // Document review tools
        "rdm_review_requests",
        "rdm_review_show",
        "rdm_review_address_comment",
        "rdm_review_complete",
        // Worktree tools — the spawned `rdm` binary is built via `cargo build -p
        // rdm-cli`, whose default features enable `git` (and `rdm-mcp?/git`), so
        // these are always registered regardless of this test crate's features.
        "rdm_worktree_add",
        "rdm_worktree_list",
        "rdm_worktree_current",
        "rdm_worktree_remove",
        // Git status/commit/discard tools — same always-on-git rationale as
        // the worktree tools above.
        "rdm_status",
        "rdm_commit",
        "rdm_discard",
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
        // Review resolution tools mutate the review file.
        "rdm_review_address_comment",
        "rdm_review_complete",
        // `rdm_worktree_add` / `rdm_worktree_remove` mutate the project repo and
        // so carry readOnlyHint=false; `rdm_worktree_list` is read-only. The
        // spawned `rdm` binary always has the git feature on (see `tools_list`),
        // so these tools are always present.
        "rdm_worktree_add",
        "rdm_worktree_remove",
        // `rdm_commit` / `rdm_discard` mutate the plan repo; `rdm_status` is
        // read-only (verified as such by being absent from this list, per the
        // `else` branch below).
        "rdm_commit",
        "rdm_discard",
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
fn phase_update_records_blocked_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Park the phase as blocked with a stage-tagged escalation reason.
    let response = h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "1",
            "status": "blocked",
            "reason": "[plan] AC 2 is ambiguous about which crate owns parsing"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("[plan] AC 2 is ambiguous about which crate owns parsing"),
        "Expected the blocked reason in update response: {text}"
    );

    // The reason persists and is surfaced by phase show.
    let show = h.call_tool(
        "rdm_phase_show",
        serde_json::json!({"project": "test-proj", "roadmap": "auth", "phase": "1"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("[plan] AC 2 is ambiguous about which crate owns parsing"),
        "Expected the blocked reason in show response: {show_text}"
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

#[test]
fn search_ambiguous_status_without_kind_matches_both_kinds() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // `needs-review` is valid for both phases and tasks. Move one of each there.
    h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "2",
            "status": "needs-review"
        }),
    );
    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "status": "needs-review"
        }),
    );

    // With no `kind`, the shared status must surface BOTH the phase and the task.
    let response = h.call_tool(
        "rdm_search",
        serde_json::json!({
            "query": "",
            "project": "test-proj",
            "status": "needs-review"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Implement Auth"),
        "needs-review phase should appear without a kind filter: {text}"
    );
    assert!(
        text.contains("Fix login bug"),
        "needs-review task should appear without a kind filter: {text}"
    );
}

#[test]
fn search_ambiguous_status_both_phase_and_task_without_kind() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Move phase 2 (Implement Auth) to needs-review
    h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "2",
            "status": "needs-review"
        }),
    );

    // Move fix-login-bug task to needs-review
    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "status": "needs-review"
        }),
    );

    // With no `kind`, both phase and task must be returned
    let response = h.call_tool(
        "rdm_search",
        serde_json::json!({
            "query": "",
            "project": "test-proj",
            "status": "needs-review"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Implement Auth"),
        "needs-review phase should appear without a kind filter: {text}"
    );
    assert!(
        text.contains("Fix login bug"),
        "needs-review task should appear without a kind filter: {text}"
    );

    // Verify kind="task" narrows to only the task
    let response = h.call_tool(
        "rdm_search",
        serde_json::json!({
            "query": "",
            "project": "test-proj",
            "status": "needs-review",
            "kind": "task"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Fix login bug"),
        "task should appear with kind=task filter: {text}"
    );
    assert!(
        !text.contains("Implement Auth"),
        "phase should not appear with kind=task filter: {text}"
    );

    // Verify kind="phase" narrows to only the phase
    let response = h.call_tool(
        "rdm_search",
        serde_json::json!({
            "query": "",
            "project": "test-proj",
            "status": "needs-review",
            "kind": "phase"
        }),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Implement Auth"),
        "phase should appear with kind=phase filter: {text}"
    );
    assert!(
        !text.contains("Fix login bug"),
        "task should not appear with kind=phase filter: {text}"
    );
}

// ==================== GitStore integration tests ====================

/// Run a git command in `root`, clearing `GIT_DIR` / `GIT_WORK_TREE` /
/// `GIT_INDEX_FILE` / object-directory overrides so the command targets the
/// temp-dir repo rather than a parent repo leaked via environment variables
/// (e.g. inside pre-commit hooks, which may point `GIT_INDEX_FILE` at the
/// source repo's index — a leaked value would make a `git add` here clobber
/// that index with the temp repo's tree).
fn git_cmd(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
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

/// Get the HEAD commit's full message.
fn git_last_commit_message(root: &std::path::Path) -> String {
    let output = git_cmd(root, &["log", "-1", "--format=%B"]);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn git_mutation_stages_without_commit() {
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

    // Verify the roadmap is readable (data was written to disk, even though
    // nothing was committed).
    let show = h.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "billing"}),
    );
    let show_text = result_text(&show);
    assert!(
        show_text.contains("Billing System"),
        "Expected roadmap to be readable after a staged mutation: {show_text}"
    );
    drop(h);

    let after = git_head_sha(tmp.path());
    assert_eq!(
        before, after,
        "Expected NO new git commits from an MCP mutation (before={before}, after={after})"
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

// ==================== Document review tools ====================

/// Run the rdm CLI against `root` and return its stdout (asserting success).
fn run_rdm_capture(root: &std::path::Path, args: &[&str]) -> String {
    build_once();
    let binary = env!("CARGO_MANIFEST_DIR").replace("rdm-mcp", "target/debug/rdm");
    let mut full_args = vec!["--root", root.to_str().unwrap()];
    full_args.extend_from_slice(args);
    let output = Command::new(&binary)
        .args(&full_args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run rdm {}: {e}", args.join(" ")));
    assert!(
        output.status.success(),
        "rdm {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Seed the standard plan repo plus a submitted request-changes review on
/// `task/fix-login-bug` with two comments: #1 anchored to the quote
/// "special chars", #2 whole-document. Returns the review id.
fn setup_review_fixture(root: &std::path::Path) -> String {
    setup_plan_repo(root);
    let started = run_rdm_capture(
        root,
        &[
            "review",
            "start",
            "--on",
            "task/fix-login-bug",
            "--author",
            "reviewer",
            "--no-edit",
            "--project",
            "test-proj",
            "--format",
            "json",
        ],
    );
    let started: serde_json::Value = serde_json::from_str(&started).expect("review JSON");
    let id = started["id"].as_str().expect("review id").to_string();
    run_rdm_capture(
        root,
        &[
            "review",
            "comment",
            &id,
            "--quote",
            "special chars",
            "--body",
            "Spell out which characters break login.",
            "--no-edit",
            "--project",
            "test-proj",
        ],
    );
    run_rdm_capture(
        root,
        &[
            "review",
            "comment",
            &id,
            "--body",
            "Overall: add reproduction steps to the task body.",
            "--no-edit",
            "--project",
            "test-proj",
        ],
    );
    run_rdm_capture(
        root,
        &[
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
            "--body",
            "Please tighten the task description.",
            "--no-edit",
            "--project",
            "test-proj",
        ],
    );
    id
}

/// Parse a tool response's text content as JSON.
fn result_json(response: &serde_json::Value) -> serde_json::Value {
    serde_json::from_str(result_text(response)).expect("tool response should be JSON")
}

/// Extract the `Commit: <sha>` trailer from a mutation tool's text output.
fn commit_trailer(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .find_map(|l| l.strip_prefix("Commit: "))
        .map(str::to_string)
}

#[test]
fn review_requests_lists_only_request_changes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    // An approved review must not appear in the queue.
    let approved = run_rdm_capture(
        tmp.path(),
        &[
            "review",
            "start",
            "--on",
            "roadmap/auth",
            "--author",
            "reviewer",
            "--no-edit",
            "--project",
            "test-proj",
            "--format",
            "json",
        ],
    );
    let approved: serde_json::Value = serde_json::from_str(&approved).unwrap();
    let approved_id = approved["id"].as_str().unwrap().to_string();
    run_rdm_capture(
        tmp.path(),
        &[
            "review",
            "submit",
            &approved_id,
            "--verdict",
            "approve",
            "--body",
            "Looks good.",
            "--no-edit",
            "--project",
            "test-proj",
        ],
    );

    let mut h = McpTestHarness::spawn(tmp.path());
    let response = h.call_tool(
        "rdm_review_requests",
        serde_json::json!({"project": "test-proj"}),
    );
    let queue = result_json(&response);
    let arr = queue.as_array().expect("array of change requests");
    assert_eq!(arr.len(), 1, "only the request-changes review: {queue}");
    let entry = &arr[0];
    assert_eq!(entry["id"], serde_json::json!(id));
    assert_eq!(entry["target"]["kind"], "task");
    assert_eq!(entry["target"]["slug"], "fix-login-bug");
    assert_eq!(entry["target_ref"], "task/fix-login-bug");
    assert_eq!(entry["author"], "reviewer");
    assert_eq!(
        entry["summary"].as_str().map(str::trim),
        Some("Please tighten the task description.")
    );
    assert_eq!(entry["open_comment_count"], 2);
    assert!(
        entry["created_commit"].as_str().is_some(),
        "git-backed store must record created_commit: {entry}"
    );
}

#[test]
fn review_requests_target_filters() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Kind-only filter: no roadmap-targeted change requests exist.
    let response = h.call_tool(
        "rdm_review_requests",
        serde_json::json!({"project": "test-proj", "target_kind": "roadmap"}),
    );
    assert_eq!(result_json(&response).as_array().unwrap().len(), 0);

    // Exact target filter matches the fixture review.
    let response = h.call_tool(
        "rdm_review_requests",
        serde_json::json!({
            "project": "test-proj",
            "target_kind": "task",
            "target_id": "fix-login-bug",
        }),
    );
    assert_eq!(result_json(&response).as_array().unwrap().len(), 1);

    // target_id without target_kind is rejected.
    let response = h.call_tool(
        "rdm_review_requests",
        serde_json::json!({"project": "test-proj", "target_id": "fix-login-bug"}),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert!(result_text(&response).contains("target_kind"));
}

#[test]
fn review_show_returns_resolution_and_documents() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_show",
        serde_json::json!({"project": "test-proj", "review_id": id}),
    );
    let v = result_json(&response);
    let review = &v["review"];
    assert_eq!(review["id"], serde_json::json!(id));
    assert_eq!(review["state"], "submitted");
    assert_eq!(review["verdict"], "request-changes");

    // Comment 1: anchored, resolves against the created_commit body.
    let c1 = &review["comments"][0];
    assert_eq!(c1["anchor"]["anchor_type"], "text-quote");
    assert_eq!(c1["resolution"]["state"], "resolved");
    assert_eq!(c1["resolution"]["quote"], "special chars");
    assert_eq!(c1["resolution"]["body"], "original");
    // Comment 2: whole-document, always unresolved.
    let c2 = &review["comments"][1];
    assert!(c2.get("anchor").is_none(), "whole-doc comment: {c2}");
    assert_eq!(c2["resolution"]["state"], "unresolved");

    // Referenced documents are inlined: the target with both body versions.
    let docs = v["documents"].as_array().expect("documents array");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["ref"], "task/fix-login-bug");
    let current = docs[0]["current_body"].as_str().expect("current body");
    assert!(current.contains("special chars"), "got: {current}");
    let original = docs[0]["body_at_created_commit"]
        .as_str()
        .expect("historical body");
    assert!(original.contains("special chars"), "got: {original}");
    // The resolution range indexes body_at_created_commit.
    let start = c1["resolution"]["range_start"].as_u64().unwrap() as usize;
    let end = c1["resolution"]["range_end"].as_u64().unwrap() as usize;
    assert_eq!(&original[start..end], "special chars");
}

#[test]
fn review_show_unknown_anchor_round_trips() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());

    // Write a review fixture carrying an anchor_type this build does not
    // model (as a newer rdm would), and commit it like any plan mutation.
    let review_path = tmp
        .path()
        .join("projects/test-proj/reviews/2026-07-01-1200-ffff.md");
    std::fs::create_dir_all(review_path.parent().unwrap()).unwrap();
    std::fs::write(
        &review_path,
        r#"---
id: 2026-07-01-1200-ffff
author: fixture
target:
  kind: task
  slug: fix-login-bug
state: submitted
verdict: request-changes
created: 2026-07-01T12:00:00Z
submitted: 2026-07-01T12:30:00Z
comments:
- id: 1
  status: open
  anchor:
    anchor_type: line-range
    start: 3
    end: 7
  body: Uses a line-range anchor from a newer rdm.
---
Fixture review with an unknown anchor type.
"#,
    )
    .unwrap();
    let add = git_cmd(tmp.path(), &["add", "-A"]);
    assert!(add.status.success());
    let commit = git_cmd(
        tmp.path(),
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "fixture: unknown-anchor review",
        ],
    );
    assert!(commit.status.success());

    let mut h = McpTestHarness::spawn(tmp.path());
    let response = h.call_tool(
        "rdm_review_show",
        serde_json::json!({"project": "test-proj", "review_id": "2026-07-01-1200-ffff"}),
    );
    assert!(
        response["result"]["isError"].is_null()
            || response["result"]["isError"] == serde_json::json!(false),
        "unknown anchor must not error: {response}"
    );
    let v = result_json(&response);
    let c1 = &v["review"]["comments"][0];
    // The unknown anchor round-trips verbatim (tagged union preserved)...
    assert_eq!(c1["anchor"]["anchor_type"], "line-range");
    assert_eq!(c1["anchor"]["start"], 3);
    assert_eq!(c1["anchor"]["end"], 7);
    // ...and resolves as unresolved (whole-document treatment).
    assert_eq!(c1["resolution"]["state"], "unresolved");

    // It also passes through the queue without error.
    let response = h.call_tool(
        "rdm_review_requests",
        serde_json::json!({"project": "test-proj"}),
    );
    let queue = result_json(&response);
    assert_eq!(queue.as_array().unwrap().len(), 1);
    assert_eq!(queue[0]["open_comment_count"], 1);
}

#[test]
fn update_tools_never_report_a_commit_trailer() {
    // MCP mutations only stage; they never carry a Commit: trailer.
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Login fails when the password contains a quote character.",
        }),
    );
    assert!(
        commit_trailer(result_text(&response)).is_none(),
        "rdm_task_update must never report a Commit: trailer"
    );

    let response = h.call_tool(
        "rdm_phase_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "phase": "1",
            "status": "in-progress",
        }),
    );
    assert!(
        commit_trailer(result_text(&response)).is_none(),
        "rdm_phase_update must never report a Commit: trailer"
    );

    let response = h.call_tool(
        "rdm_roadmap_update",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "body": "Implement authentication, now with provenance.",
        }),
    );
    assert!(
        commit_trailer(result_text(&response)).is_none(),
        "rdm_roadmap_update must never report a Commit: trailer"
    );
}

#[test]
fn review_address_comment_rejects_bogus_status() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 1,
            "status": "bogus",
            "reply": "nope",
        }),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    let text = result_text(&response);
    assert!(
        text.contains("addressed") && text.contains("wont-fix"),
        "rejection must name the accepted status values: {text}"
    );
}

#[test]
fn review_requests_rejects_bogus_target_kind() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_requests",
        serde_json::json!({"project": "test-proj", "target_kind": "bogus"}),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    let text = result_text(&response);
    assert!(
        text.contains("roadmap, phase, or task"),
        "rejection must name the accepted kinds: {text}"
    );
}

#[test]
fn review_show_unknown_review_id_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_show",
        serde_json::json!({"project": "test-proj", "review_id": "9999-99-99-0000-dead"}),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert!(
        result_text(&response).contains("9999-99-99-0000-dead"),
        "error must name the missing review id: {}",
        result_text(&response)
    );
}

#[test]
fn review_address_comment_unknown_review_id_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": "9999-99-99-0000-dead",
            "comment_id": 1,
            "status": "addressed",
            "reply": "nope",
        }),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert!(
        result_text(&response).contains("9999-99-99-0000-dead"),
        "error must name the missing review id: {}",
        result_text(&response)
    );
}

#[test]
fn review_complete_unknown_review_id_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_complete",
        serde_json::json!({"project": "test-proj", "review_id": "9999-99-99-0000-dead"}),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert!(
        result_text(&response).contains("9999-99-99-0000-dead"),
        "error must name the missing review id: {}",
        result_text(&response)
    );
}

#[test]
fn review_address_comment_records_explicit_applied_commit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Apply the edit the comment asked for; it only stages...
    let update = h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Login fails when the password contains quotes or backslashes.",
        }),
    );
    assert!(
        commit_trailer(result_text(&update)).is_none(),
        "rdm_task_update must not report a Commit: trailer"
    );

    // ...so land it explicitly and thread its Commit: trailer.
    let commit = h.call_tool("rdm_commit", serde_json::json!({}));
    let edit_sha = commit_trailer(result_text(&commit))
        .expect("rdm_commit response must carry a Commit: trailer");
    assert_eq!(edit_sha, git_head_sha(tmp.path()));

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 1,
            "status": "addressed",
            "applied_commit": edit_sha,
            "reply": "Spelled out the failing characters; anchor resolved cleanly.",
        }),
    );
    let v = result_json(&response);
    assert_eq!(v["status"], "addressed");
    assert_eq!(v["applied_commit"], serde_json::json!(edit_sha));
    assert_eq!(
        v["reply"],
        "Spelled out the failing characters; anchor resolved cleanly."
    );
    assert_eq!(v["review_state"], "submitted");
    assert_eq!(v["open_comment_count"], 1);
    // The review-file update this call produced was only staged — there is
    // no real plan-repo commit to report, so the JSON carries no "commit" key.
    assert!(
        v.get("commit").is_none(),
        "response must not carry a stale commit field: {v}"
    );
}

#[test]
fn review_address_comment_omitted_applied_commit_stays_null() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 1,
            "status": "addressed",
            "reply": "Addressed without naming the commit.",
        }),
    );
    let v = result_json(&response);
    // MCP mutations only stage: there is no commit to default to, so
    // omitting applied_commit must leave it null rather than guessing.
    assert!(
        v["applied_commit"].is_null(),
        "omitted applied_commit must stay null: {v}"
    );
    assert!(v.get("commit").is_none(), "no commit field: {v}");
}

#[test]
fn review_address_comment_wont_fix_never_defaults_applied_commit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 2,
            "status": "wont-fix",
            "reply": "Out of scope for this task; tracked separately.",
        }),
    );
    let v = result_json(&response);
    assert_eq!(v["status"], "wont-fix");
    assert!(
        v["applied_commit"].is_null(),
        "wont-fix must never stamp a provenance commit: {v}"
    );
}

#[test]
fn review_address_comment_reply_only_leaves_comment_open() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 1,
            "reply": "The quoted text drifted — which behavior did you mean?",
        }),
    );
    let v = result_json(&response);
    assert_eq!(v["status"], "open");
    assert!(
        v["applied_commit"].is_null(),
        "no default on reply-only: {v}"
    );
    assert_eq!(v["open_comment_count"], 2);
}

#[test]
fn review_address_comment_rejects_open_status() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 1,
            "status": "open",
            "reply": "nope",
        }),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert!(result_text(&response).contains("omit status"));
}

#[test]
fn review_complete_refuses_open_comments_listing_offenders() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Resolve comment 1, leave comment 2 open.
    h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 1,
            "status": "addressed",
            "reply": "Done.",
        }),
    );
    let response = h.call_tool(
        "rdm_review_complete",
        serde_json::json!({"project": "test-proj", "review_id": id}),
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    let text = result_text(&response);
    assert!(
        text.contains("1 comment(s) still open: 2"),
        "refusal must name the open comment ids: {text}"
    );

    // The review is still submitted.
    let show = h.call_tool(
        "rdm_review_show",
        serde_json::json!({"project": "test-proj", "review_id": id}),
    );
    assert_eq!(result_json(&show)["review"]["state"], "submitted");
}

#[test]
fn review_complete_succeeds_when_all_resolved() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 1,
            "status": "addressed",
            "reply": "Done.",
        }),
    );
    h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 2,
            "status": "wont-fix",
            "reply": "Reproduction steps live in the linked issue.",
        }),
    );
    let response = h.call_tool(
        "rdm_review_complete",
        serde_json::json!({"project": "test-proj", "review_id": id}),
    );
    let text = result_text(&response);
    assert!(text.contains("addressed"), "got: {text}");

    let show = h.call_tool(
        "rdm_review_show",
        serde_json::json!({"project": "test-proj", "review_id": id}),
    );
    assert_eq!(result_json(&show)["review"]["state"], "addressed");
    // And it drops out of the change-request queue.
    let queue = h.call_tool(
        "rdm_review_requests",
        serde_json::json!({"project": "test-proj"}),
    );
    assert_eq!(result_json(&queue).as_array().unwrap().len(), 0);
}

// ==================== Git status/commit/discard tools ====================

#[test]
fn status_empty_on_clean_tree() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool("rdm_status", serde_json::json!({}));
    let statuses = result_json(&response);
    assert_eq!(
        statuses.as_array().unwrap().len(),
        0,
        "expected no staged changes on a freshly committed repo: {statuses}"
    );
}

#[test]
fn status_reports_staged_changes() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Updated body for status test.",
        }),
    );

    let response = h.call_tool("rdm_status", serde_json::json!({}));
    let statuses = result_json(&response);
    let arr = statuses.as_array().expect("array of statuses");
    assert_eq!(arr.len(), 1, "expected exactly one staged change: {arr:?}");
    assert!(
        arr[0]["path"].as_str().unwrap().contains("fix-login-bug"),
        "expected the task file path: {arr:?}"
    );
    assert_eq!(arr[0]["change"], "modified");
}

#[test]
fn commit_clean_tree_is_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let before = git_head_sha(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool("rdm_commit", serde_json::json!({}));
    let text = result_text(&response);
    assert!(
        text.contains("Nothing to commit."),
        "expected no-op message: {text}"
    );
    drop(h);
    assert_eq!(before, git_head_sha(tmp.path()), "HEAD must not move");
}

#[test]
fn commit_lands_real_commit() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let before = git_head_sha(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Updated body for commit test.",
        }),
    );
    let response = h.call_tool("rdm_commit", serde_json::json!({}));
    let text = result_text(&response);
    assert!(text.contains("Committed 1 file(s)."), "got: {text}");
    let sha = commit_trailer(text).expect("rdm_commit must report a Commit: trailer");
    drop(h);

    let after = git_head_sha(tmp.path());
    assert_ne!(before, after, "HEAD must advance");
    assert_eq!(
        sha, after,
        "trailer must name the commit rdm_commit produced"
    );
}

#[test]
fn commit_default_message_matches_cli_generator() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Updated body for default-message test.",
        }),
    );
    h.call_tool("rdm_commit", serde_json::json!({}));
    drop(h);

    // Mirrors GitRepo::default_commit_message's single-file shape: "rdm:
    // update <path>" — the same shape the CLI's `rdm commit` produces.
    let msg = git_last_commit_message(tmp.path());
    assert!(
        msg.starts_with("rdm: update "),
        "expected default single-file message shape, got: {msg}"
    );
    assert!(msg.contains("fix-login-bug"), "got: {msg}");
}

#[test]
fn commit_explicit_message_used_verbatim() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Updated body for explicit-message test.",
        }),
    );
    h.call_tool(
        "rdm_commit",
        serde_json::json!({"message": "custom commit message"}),
    );
    drop(h);

    let msg = git_last_commit_message(tmp.path());
    assert!(
        msg.starts_with("custom commit message"),
        "expected verbatim explicit message, got: {msg}"
    );
}

#[test]
fn discard_without_confirm_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Should survive the rejected discard.",
        }),
    );

    let response = h.call_tool("rdm_discard", serde_json::json!({}));
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert!(result_text(&response).contains("confirm"));

    // Data survives: the staged edit is still readable.
    let show = h.call_tool(
        "rdm_task_show",
        serde_json::json!({"project": "test-proj", "task": "fix-login-bug"}),
    );
    assert!(
        result_text(&show).contains("Should survive the rejected discard."),
        "staged edit must not be lost by a rejected discard"
    );
}

#[test]
fn discard_with_confirm_reverts() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let before = git_head_sha(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Should be discarded.",
        }),
    );

    let response = h.call_tool("rdm_discard", serde_json::json!({"confirm": true}));
    let text = result_text(&response);
    assert!(text.contains("Discarded 1 file(s)."), "got: {text}");

    let show = h.call_tool(
        "rdm_task_show",
        serde_json::json!({"project": "test-proj", "task": "fix-login-bug"}),
    );
    assert!(
        !result_text(&show).contains("Should be discarded."),
        "discarded edit must be reverted"
    );
    drop(h);
    assert_eq!(
        before,
        git_head_sha(tmp.path()),
        "discard never touches HEAD — nothing was committed"
    );
}

#[test]
fn discard_clean_tree_is_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let response = h.call_tool("rdm_discard", serde_json::json!({"confirm": true}));
    let text = result_text(&response);
    assert!(
        text.contains("Nothing to discard."),
        "expected no-op message: {text}"
    );
}

#[test]
fn review_address_comment_wont_fix_honors_explicit_applied_commit() {
    // The plan explicitly preserves pass-through of an explicitly supplied
    // applied_commit even for wont-fix — only the auto-default was removed.
    let tmp = tempfile::TempDir::new().unwrap();
    let id = setup_review_fixture(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_update",
        serde_json::json!({
            "project": "test-proj",
            "task": "fix-login-bug",
            "body": "Some unrelated context captured for the record.",
        }),
    );
    let commit = h.call_tool("rdm_commit", serde_json::json!({}));
    let sha = commit_trailer(result_text(&commit)).expect("Commit: trailer");

    let response = h.call_tool(
        "rdm_review_address_comment",
        serde_json::json!({
            "project": "test-proj",
            "review_id": id,
            "comment_id": 2,
            "status": "wont-fix",
            "applied_commit": sha,
            "reply": "Documented in this commit instead of fixing.",
        }),
    );
    let v = result_json(&response);
    assert_eq!(v["status"], "wont-fix");
    assert_eq!(
        v["applied_commit"],
        serde_json::json!(sha),
        "explicit applied_commit must pass through even for wont-fix: {v}"
    );
}

#[test]
fn stage_status_commit_discard_roundtrip() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let head_after_setup = git_head_sha(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    // Mutate: stages only, no commit.
    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "billing",
            "title": "Billing System"
        }),
    );
    assert_eq!(
        head_after_setup,
        git_head_sha(tmp.path()),
        "mutation must not advance HEAD"
    );

    // rdm_status is non-empty.
    let status = h.call_tool("rdm_status", serde_json::json!({}));
    let arr = result_json(&status);
    assert!(
        !arr.as_array().unwrap().is_empty(),
        "expected staged changes: {arr}"
    );

    // rdm_commit lands one commit; status empties.
    let commit_response = h.call_tool("rdm_commit", serde_json::json!({}));
    let landed_sha =
        commit_trailer(result_text(&commit_response)).expect("Commit: trailer after rdm_commit");
    assert_ne!(
        head_after_setup, landed_sha,
        "HEAD must advance exactly once"
    );
    assert_eq!(landed_sha, git_head_sha(tmp.path()));

    let status = h.call_tool("rdm_status", serde_json::json!({}));
    assert!(
        result_json(&status).as_array().unwrap().is_empty(),
        "status must be empty right after commit"
    );

    // A separate mutate...
    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "invoicing",
            "title": "Invoicing"
        }),
    );
    assert_eq!(
        landed_sha,
        git_head_sha(tmp.path()),
        "second mutation must not advance HEAD either"
    );

    // ...reverted by rdm_discard: HEAD unchanged since the rdm_commit above,
    // and the item is gone.
    let discard = h.call_tool("rdm_discard", serde_json::json!({"confirm": true}));
    assert!(result_text(&discard).contains("Discarded"));
    drop(h);

    assert_eq!(
        landed_sha,
        git_head_sha(tmp.path()),
        "discard never touches HEAD"
    );

    let mut h2 = McpTestHarness::spawn(tmp.path());
    let show = h2.call_tool(
        "rdm_roadmap_show",
        serde_json::json!({"project": "test-proj", "roadmap": "invoicing"}),
    );
    assert_eq!(
        show["result"]["isError"],
        serde_json::json!(true),
        "discarded roadmap must be gone: {show}"
    );
}

// ==================== Tag filtering + Tags rendering (tagging-support) ====================

#[test]
fn task_list_filter_by_tag_excludes_untagged() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "tagged-task",
            "title": "Tagged Task",
            "tags": ["needle"],
        }),
    );
    h.call_tool(
        "rdm_task_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "plain-task",
            "title": "Plain Task",
        }),
    );

    let response = h.call_tool(
        "rdm_task_list",
        serde_json::json!({"project": "test-proj", "tag": "needle"}),
    );
    let text = result_text(&response);
    assert!(text.contains("tagged-task"), "tagged task missing: {text}");
    assert!(
        !text.contains("plain-task"),
        "untagged task should be excluded: {text}"
    );
}

#[test]
fn task_list_renders_tags_column_only_when_tagged() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_task_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "plain-task",
            "title": "Plain Task",
        }),
    );
    let resp_before = h.call_tool("rdm_task_list", serde_json::json!({"project": "test-proj"}));
    let before = result_text(&resp_before);
    assert!(
        !before.contains("Tags"),
        "Tags column should be absent when nothing is tagged: {before}"
    );

    h.call_tool(
        "rdm_task_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "tagged-task",
            "title": "Tagged Task",
            "tags": ["bug", "ui"],
        }),
    );
    let resp_after = h.call_tool("rdm_task_list", serde_json::json!({"project": "test-proj"}));
    let after = result_text(&resp_after);
    assert!(
        after.contains("Tags"),
        "Tags column should appear once a task is tagged: {after}"
    );
    assert!(
        after.contains("bug, ui"),
        "tags should render comma-joined: {after}"
    );
}

#[test]
fn roadmap_list_renders_tags_suffix_only_when_tagged() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    let resp_before = h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "test-proj"}),
    );
    let before = result_text(&resp_before);
    assert!(
        !before.contains("[tags:"),
        "no tags suffix expected before tagging: {before}"
    );

    h.call_tool(
        "rdm_roadmap_create",
        serde_json::json!({
            "project": "test-proj",
            "slug": "tagged-rm",
            "title": "Tagged Roadmap",
            "tags": ["bug", "ui"],
        }),
    );
    let resp_after = h.call_tool(
        "rdm_roadmap_list",
        serde_json::json!({"project": "test-proj"}),
    );
    let after = result_text(&resp_after);
    assert!(
        after.contains("[tags: bug, ui]"),
        "roadmap list should carry a tags suffix: {after}"
    );
}

#[test]
fn phase_list_filter_by_tag_excludes_untagged() {
    let tmp = tempfile::TempDir::new().unwrap();
    setup_plan_repo(tmp.path());
    let mut h = McpTestHarness::spawn(tmp.path());

    h.call_tool(
        "rdm_phase_create",
        serde_json::json!({
            "project": "test-proj",
            "roadmap": "auth",
            "slug": "audited",
            "title": "Audited Phase",
            "number": 9,
            "tags": ["needle"],
        }),
    );
    let response = h.call_tool(
        "rdm_phase_list",
        serde_json::json!({"project": "test-proj", "roadmap": "auth", "tag": "needle"}),
    );
    let text = result_text(&response);
    assert!(
        text.contains("Audited Phase"),
        "tagged phase missing: {text}"
    );
    assert!(
        !text.contains("phase-1"),
        "untagged phases should be excluded: {text}"
    );
}
