use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn init_with_project(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["project", "create", "fbm"])
        .assert()
        .success();
}

#[test]
fn index_generates_file() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "phase",
            "create",
            "core",
            "--roadmap",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated INDEX.md"));

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(content.contains("# Plan Index"));
    assert!(content.contains("[fbm](projects/fbm/INDEX.md)"));
    assert!(content.contains("not started"));
    // Details are in per-project index, not root
    assert!(!content.contains("## Project: fbm"));
}

#[test]
fn index_idempotent() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();
    let first = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let first_project = std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();
    let second = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let second_project = std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();

    assert_eq!(first, second, "top-level INDEX.md should be idempotent");
    assert_eq!(
        first_project, second_project,
        "project-level INDEX.md should be idempotent"
    );
}

#[test]
fn index_deterministic_sorting() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["project", "create", "zzz"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["project", "create", "aaa"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let aaa_pos = content.find("[aaa]").unwrap();
    let zzz_pos = content.find("[zzz]").unwrap();
    assert!(aaa_pos < zzz_pos);
}

#[test]
fn index_task_priority_sorting() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "task",
            "create",
            "low-task",
            "--project",
            "fbm",
            "--priority",
            "low",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "task",
            "create",
            "crit-task",
            "--project",
            "fbm",
            "--priority",
            "critical",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "task",
            "create",
            "high-task",
            "--project",
            "fbm",
            "--priority",
            "high",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    // Task details are in per-project index, not root
    let project_content =
        std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();
    let crit_pos = project_content.find("crit-task").unwrap();
    let high_pos = project_content.find("high-task").unwrap();
    let low_pos = project_content.find("low-task").unwrap();
    assert!(crit_pos < high_pos, "critical should come before high");
    assert!(high_pos < low_pos, "high should come before low");

    // Root index just shows task count
    let root = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(root.contains("| 3 |"));
}

#[test]
fn index_dependency_graph() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    // Create roadmap with dependencies by writing the file directly
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();

    // Write a roadmap with dependencies manually
    let roadmap_path = dir.path().join("projects/fbm/roadmaps/beta");
    std::fs::create_dir_all(&roadmap_path).unwrap();
    std::fs::write(
        roadmap_path.join("roadmap.md"),
        "---\nproject: fbm\nroadmap: beta\ntitle: Beta\nphases: []\ndependencies:\n  - alpha\n---\n",
    )
    .unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    // Dependency graph is in per-project index, not root
    let project_content =
        std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();
    assert!(project_content.contains("Dependency Graph"));
    assert!(project_content.contains("**beta** → alpha"));

    // Root index just links to project
    let root = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(root.contains("[fbm](projects/fbm/INDEX.md)"));
}

#[test]
fn mutation_auto_generates_index() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();

    // project create should auto-generate index
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(content.contains("# Plan Index"));
    assert!(content.contains("[fbm](projects/fbm/INDEX.md)"));
}

#[test]
fn no_index_flag_suppresses() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    // Create a roadmap so project-level index has content
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();

    // Generate full index so both levels exist
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let root_before = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let project_before = std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();

    // Mutate with --no-index: create a phase
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "phase",
            "create",
            "core",
            "--roadmap",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    let root_after = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let project_after = std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();

    assert_eq!(
        root_before, root_after,
        "--no-index should prevent top-level INDEX.md regeneration"
    );
    assert_eq!(
        project_before, project_after,
        "--no-index should prevent project-level INDEX.md regeneration"
    );
}

#[test]
fn index_after_phase_update() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "phase",
            "create",
            "core",
            "--roadmap",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // Generate initial index
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();
    let before = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(before.contains("not started"));

    // Update phase to done (auto-generates index)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--roadmap",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    let after = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(
        after.contains("complete"),
        "index should reflect phase status change"
    );
}

#[test]
fn mutation_only_rewrites_targeted_project_index() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();

    // Create two projects with roadmaps
    for (proj, roadmap) in &[("proj-a", "alpha"), ("proj-b", "beta")] {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .arg("--no-index")
            .args(["project", "create", proj])
            .assert()
            .success();
        rdm()
            .arg("--root")
            .arg(dir.path())
            .arg("--no-index")
            .args(["roadmap", "create", roadmap, "--project", proj])
            .assert()
            .success();
    }

    // Generate full index so both project INDEX.md files exist
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let proj_b_index_before =
        std::fs::read_to_string(dir.path().join("projects/proj-b/INDEX.md")).unwrap();

    // Mutate proj-a (auto-regenerates index for proj-a only)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--roadmap",
            "alpha",
            "--project",
            "proj-a",
        ])
        .assert()
        .success();

    // proj-b's INDEX.md should be unchanged
    let proj_b_index_after =
        std::fs::read_to_string(dir.path().join("projects/proj-b/INDEX.md")).unwrap();
    assert_eq!(
        proj_b_index_before, proj_b_index_after,
        "proj-b INDEX.md should not be rewritten when proj-a is mutated"
    );

    // Top-level INDEX.md should reflect the mutation (proj-a now has a phase)
    let root = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(root.contains("[proj-a]"));
    assert!(root.contains("[proj-b]"));
    assert!(root.contains("not started")); // proj-a's phase is not-started

    // proj-a's INDEX.md should reflect the new phase (phase count goes from 0 to 1)
    let proj_a_index =
        std::fs::read_to_string(dir.path().join("projects/proj-a/INDEX.md")).unwrap();
    assert!(
        proj_a_index.contains("alpha"),
        "proj-a INDEX.md should reference its roadmap"
    );
    assert!(
        proj_a_index.contains("not started"),
        "proj-a INDEX.md should show progress for the roadmap with a phase"
    );
}

#[test]
fn index_after_promote() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    // Create a task
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["task", "create", "big-feature", "--project", "fbm"])
        .assert()
        .success();

    // Generate index before promote
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let root_before = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let project_before = std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();
    assert!(
        project_before.contains("big-feature"),
        "task should appear in project index before promote"
    );

    // Promote task to roadmap (auto-generates index)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "promote",
            "big-feature",
            "--roadmap-slug",
            "big-feature-roadmap",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // Both index levels should reflect the promotion
    let root_after = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let project_after = std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();

    assert!(
        root_after.contains("[fbm]"),
        "top-level should still list the project"
    );
    assert!(
        project_after.contains("big-feature-roadmap"),
        "project index should contain the new roadmap after promote"
    );
    // The promoted task should no longer appear as a standalone task
    assert_ne!(
        root_before, root_after,
        "top-level index should change after promote"
    );
}
