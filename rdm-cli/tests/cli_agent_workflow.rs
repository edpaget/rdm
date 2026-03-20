use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

/// Creates a rich fixture for agent workflow tests:
/// - Project "acme" with title "Acme Corp"
/// - Roadmap "backend" with body
/// - 3 phases: design (done), impl (in-progress), review (not-started), each with body
/// - Task "fix-auth" with priority high, tags "bug,auth", and body
/// - Task "add-caching" with body
fn setup_rich_fixture(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme", "--title", "Acme Corp"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "backend",
            "--title",
            "Backend Rewrite",
            "--body",
            "ROADMAP_BODY_MARKER: Rewrite the backend services for scale.",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    // Phase 1: design (done)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--body",
            "PHASE_DESIGN_MARKER: Architectural design phase.",
            "--roadmap",
            "backend",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "done",
            "--no-edit",
            "--roadmap",
            "backend",
            "--project",
            "acme",
        ])
        .assert()
        .success();

    // Phase 2: impl (in-progress)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "impl",
            "--title",
            "Implementation",
            "--body",
            "PHASE_IMPL_MARKER: Build the core services.",
            "--roadmap",
            "backend",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-2-impl",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "backend",
            "--project",
            "acme",
        ])
        .assert()
        .success();

    // Phase 3: review (not-started)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "review",
            "--title",
            "Review",
            "--body",
            "PHASE_REVIEW_MARKER: Code review and QA.",
            "--roadmap",
            "backend",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    // Task: fix-auth
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-auth",
            "--title",
            "Fix Auth Bug",
            "--body",
            "TASK_AUTH_MARKER: Authentication tokens expire too early.",
            "--priority",
            "high",
            "--tags",
            "bug,auth",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    // Task: add-caching
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "add-caching",
            "--title",
            "Add Caching",
            "--body",
            "TASK_CACHE_MARKER: Add Redis caching layer.",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
}

fn run_json(dir: &TempDir, args: &[&str]) -> Value {
    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["--format", "json"])
        .args(args)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    serde_json::from_str(&stdout).expect("should be valid JSON")
}

// ---------------------------------------------------------------------------
// Test 1: Full discovery flow
// ---------------------------------------------------------------------------

#[test]
fn agent_discovery_workflow() {
    let dir = TempDir::new().unwrap();
    setup_rich_fixture(&dir);

    // 1. project list → "acme" present
    let projects = run_json(&dir, &["project", "list"]);
    let arr = projects.as_array().expect("project list is array");
    assert!(
        arr.iter().any(|v| v.as_str() == Some("acme")),
        "project list should contain 'acme'"
    );

    // 2. roadmap list → "backend" present with progress
    let roadmaps = run_json(&dir, &["roadmap", "list", "--project", "acme"]);
    let arr = roadmaps.as_array().expect("roadmap list is array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["slug"], "backend");
    assert_eq!(arr[0]["title"], "Backend Rewrite");
    let progress = arr[0]["progress"].as_str().unwrap();
    assert!(
        progress.contains("1/3"),
        "progress should show 1/3 done, got: {progress}"
    );

    // 3. roadmap show → body present, phases are summaries
    let roadmap = run_json(&dir, &["roadmap", "show", "backend", "--project", "acme"]);
    assert_eq!(roadmap["slug"], "backend");
    let body = roadmap["body"].as_str().unwrap();
    assert!(
        body.contains("ROADMAP_BODY_MARKER"),
        "roadmap body should contain marker"
    );
    let phases = roadmap["phases"].as_array().expect("phases is array");
    assert_eq!(phases.len(), 3);
    // Verify phase statuses
    let statuses: Vec<&str> = phases
        .iter()
        .map(|p| p["status"].as_str().unwrap())
        .collect();
    assert!(statuses.contains(&"done"));
    assert!(statuses.contains(&"in-progress"));
    assert!(statuses.contains(&"not-started"));
    // Phase summaries should NOT have body
    for phase in phases {
        assert!(
            phase.get("body").is_none(),
            "phase summaries in roadmap show should not include body"
        );
    }

    // 4. phase show → body present, navigation links
    let phase = run_json(
        &dir,
        &[
            "phase",
            "show",
            "phase-2-impl",
            "--roadmap",
            "backend",
            "--project",
            "acme",
        ],
    );
    let phase_body = phase["body"].as_str().unwrap();
    assert!(
        phase_body.contains("PHASE_IMPL_MARKER"),
        "phase body should contain marker"
    );
    assert_eq!(phase["status"], "in-progress");
    // Navigation: prev/next
    assert_eq!(
        phase["prev_phase"].as_str().unwrap(),
        "phase-1-design",
        "prev_phase should be phase-1-design"
    );
    assert_eq!(
        phase["next_phase"].as_str().unwrap(),
        "phase-3-review",
        "next_phase should be phase-3-review"
    );

    // 5. task list → 2 tasks
    let tasks = run_json(&dir, &["task", "list", "--project", "acme"]);
    let arr = tasks.as_array().expect("task list is array");
    assert_eq!(arr.len(), 2);
    let slugs: Vec<&str> = arr.iter().map(|t| t["slug"].as_str().unwrap()).collect();
    assert!(slugs.contains(&"fix-auth"));
    assert!(slugs.contains(&"add-caching"));

    // 6. task show → body, tags, priority
    let task = run_json(&dir, &["task", "show", "fix-auth", "--project", "acme"]);
    assert_eq!(task["title"], "Fix Auth Bug");
    assert_eq!(task["priority"], "high");
    let task_body = task["body"].as_str().unwrap();
    assert!(
        task_body.contains("TASK_AUTH_MARKER"),
        "task body should contain marker"
    );
    let tags = task["tags"].as_array().expect("tags is array");
    let tag_strs: Vec<&str> = tags.iter().map(|t| t.as_str().unwrap()).collect();
    assert!(tag_strs.contains(&"bug"));
    assert!(tag_strs.contains(&"auth"));

    // 7. describe phase → returns schema
    let describe = {
        let assert = rdm()
            .args(["describe", "phase", "--format", "json"])
            .assert()
            .success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        serde_json::from_str::<Value>(&stdout).unwrap()
    };
    assert_eq!(describe["name"], "phase");
    assert!(!describe["fields"].as_array().unwrap().is_empty());

    // 8. tree → hierarchical structure
    let tree = run_json(&dir, &["tree", "--project", "acme"]);
    assert_eq!(tree["name"], "acme");
    assert_eq!(tree["kind"], "project");
    let children = tree["children"].as_array().expect("tree has children");
    // Find roadmap child
    let roadmap_child = children
        .iter()
        .find(|c| c["kind"] == "roadmap")
        .expect("should have roadmap child");
    assert_eq!(roadmap_child["name"], "backend");
    let roadmap_children = roadmap_child["children"].as_array().unwrap();
    let phase_children: Vec<&Value> = roadmap_children
        .iter()
        .filter(|c| c["kind"] == "phase")
        .collect();
    assert_eq!(
        phase_children.len(),
        3,
        "roadmap should have 3 phase children"
    );
    // Task children at project level
    let task_children: Vec<&Value> = children.iter().filter(|c| c["kind"] == "task").collect();
    assert_eq!(
        task_children.len(),
        2,
        "project should have 2 task children"
    );
}

// ---------------------------------------------------------------------------
// Test 2: JSON parity — body content survives round-trip
// ---------------------------------------------------------------------------

#[test]
fn json_output_contains_all_source_content() {
    let dir = TempDir::new().unwrap();
    setup_rich_fixture(&dir);

    // Roadmap body
    let roadmap = run_json(&dir, &["roadmap", "show", "backend", "--project", "acme"]);
    assert!(
        roadmap["body"]
            .as_str()
            .unwrap()
            .contains("ROADMAP_BODY_MARKER"),
        "roadmap JSON should contain body marker"
    );

    // Phase body
    let phase = run_json(
        &dir,
        &[
            "phase",
            "show",
            "phase-1-design",
            "--roadmap",
            "backend",
            "--project",
            "acme",
        ],
    );
    assert!(
        phase["body"]
            .as_str()
            .unwrap()
            .contains("PHASE_DESIGN_MARKER"),
        "phase JSON should contain body marker"
    );

    // Task body
    let task = run_json(&dir, &["task", "show", "add-caching", "--project", "acme"]);
    assert!(
        task["body"].as_str().unwrap().contains("TASK_CACHE_MARKER"),
        "task JSON should contain body marker"
    );
}

// ---------------------------------------------------------------------------
// Test 3: describe covers all entities
// ---------------------------------------------------------------------------

#[test]
fn describe_returns_schema_for_all_entities() {
    // List all entities
    let output = rdm()
        .args(["describe", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let entities: Value = serde_json::from_slice(&output.stdout).unwrap();
    let arr = entities.as_array().unwrap();
    assert_eq!(arr.len(), 4, "should have 4 entity types");
    let names: Vec<&str> = arr.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["project", "roadmap", "phase", "task"]);

    // Each entity has non-empty fields
    for name in &["project", "roadmap", "phase", "task"] {
        let output = rdm()
            .args(["describe", name, "--format", "json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "describe {name} should succeed");
        let entity: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(entity["name"].as_str().unwrap(), *name);
        let fields = entity["fields"].as_array().unwrap();
        assert!(
            !fields.is_empty(),
            "describe {name} should have non-empty fields"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: Navigation completeness — project → roadmap → phase → body
// ---------------------------------------------------------------------------

#[test]
fn navigation_from_project_to_phase_body() {
    let dir = TempDir::new().unwrap();
    setup_rich_fixture(&dir);

    // Step 1: discover projects
    let projects = run_json(&dir, &["project", "list"]);
    let project_name = projects.as_array().unwrap()[0].as_str().unwrap();
    assert_eq!(project_name, "acme");

    // Step 2: list roadmaps for the discovered project
    let roadmaps = run_json(&dir, &["roadmap", "list", "--project", project_name]);
    let roadmap_slug = roadmaps.as_array().unwrap()[0]["slug"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(roadmap_slug, "backend");

    // Step 3: show roadmap to discover phases
    let roadmap = run_json(
        &dir,
        &["roadmap", "show", &roadmap_slug, "--project", project_name],
    );
    let phases = roadmap["phases"].as_array().unwrap();
    assert!(!phases.is_empty());
    // Pick the in-progress phase by examining the phases array
    let in_progress = phases
        .iter()
        .find(|p| p["status"] == "in-progress")
        .expect("should have an in-progress phase");
    let phase_stem = in_progress["stem"].as_str().unwrap().to_string();

    // Step 4: show phase to get body
    let phase = run_json(
        &dir,
        &[
            "phase",
            "show",
            &phase_stem,
            "--roadmap",
            &roadmap_slug,
            "--project",
            project_name,
        ],
    );
    let body = phase["body"].as_str().expect("phase should have body");
    assert!(
        !body.is_empty(),
        "phase body should be non-empty at the end of the navigation chain"
    );
    assert!(
        body.contains("PHASE_IMPL_MARKER"),
        "navigated phase body should contain expected content"
    );
}
