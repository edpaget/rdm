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
fn a_mutation_generates_no_index_but_rdm_index_does() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    // `project create` is a mutation: it writes only the entity file.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();

    assert!(
        !dir.path().join("INDEX.md").exists(),
        "a mutation must not create the top-level INDEX.md"
    );
    assert!(
        !dir.path().join("projects/fbm/INDEX.md").exists(),
        "a mutation must not create a per-project INDEX.md"
    );

    // The explicit `rdm index` command still produces both levels.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(content.contains("# Plan Index"));
    assert!(content.contains("[fbm](projects/fbm/INDEX.md)"));
}

#[test]
fn no_index_flag_is_accepted_and_ignored() {
    // `--no-index` is retained for one release so existing scripts keep
    // working, but mutations no longer regenerate an index, so it has nothing
    // left to suppress: it must parse, exit 0, warn on stderr only, and leave
    // the mutation's on-disk result byte-identical to a run without it.
    let with_flag = TempDir::new().unwrap();
    let without_flag = TempDir::new().unwrap();

    for dir in [&with_flag, &without_flag] {
        init_with_project(dir);
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args(["roadmap", "create", "alpha", "--project", "fbm"])
            .assert()
            .success();
    }

    let flagged = rdm()
        .arg("--root")
        .arg(with_flag.path())
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
    let flagged = flagged.get_output();
    let stderr = String::from_utf8_lossy(&flagged.stderr);
    assert!(
        stderr.contains("--no-index is deprecated and has no effect"),
        "the deprecation warning must land on stderr: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&flagged.stdout);
    assert!(
        !stdout.contains("--no-index"),
        "the deprecation warning must never contaminate stdout: {stdout}"
    );

    let unflagged = rdm()
        .arg("--root")
        .arg(without_flag.path())
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
    let unflagged_stderr = String::from_utf8_lossy(&unflagged.get_output().stderr).to_string();
    assert!(
        !unflagged_stderr.contains("--no-index"),
        "the warning must not fire when the flag is absent: {unflagged_stderr}"
    );

    let phase_rel = "projects/fbm/roadmaps/alpha/phase-1-core.md";
    assert_eq!(
        std::fs::read_to_string(with_flag.path().join(phase_rel)).unwrap(),
        std::fs::read_to_string(without_flag.path().join(phase_rel)).unwrap(),
        "--no-index must not change what a mutation writes"
    );
    assert!(
        !with_flag.path().join("projects/fbm/INDEX.md").exists()
            && !without_flag.path().join("projects/fbm/INDEX.md").exists(),
        "neither run may produce a per-project INDEX.md"
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

    // Update phase to done. The mutation itself writes no index, so the
    // explicit `rdm index` below is what refreshes it.
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
    assert_eq!(
        std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap(),
        before,
        "the mutation itself must leave INDEX.md untouched"
    );
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let after = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(
        after.contains("complete"),
        "index should reflect phase status change"
    );
}

#[test]
fn rdm_index_only_rewrites_the_targeted_project_index() {
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

    // Mutate proj-a, then refresh only proj-a's index explicitly.
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
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
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
fn index_merge_output_writes_regenerated_root_index() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    let out_file = dir.path().join("merge-output.tmp");
    std::fs::write(&out_file, "stale content").unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "index",
            "--merge-output",
            out_file.to_str().unwrap(),
            "--merge-path",
            "INDEX.md",
        ])
        .assert()
        .success();

    let root_index = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let merge_output = std::fs::read_to_string(&out_file).unwrap();
    assert_eq!(merge_output, root_index);
}

#[test]
fn index_merge_output_writes_regenerated_project_index() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();

    let out_file = dir.path().join("merge-output.tmp");
    std::fs::write(&out_file, "stale content").unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "index",
            "--merge-output",
            out_file.to_str().unwrap(),
            "--merge-path",
            "projects/fbm/INDEX.md",
        ])
        .assert()
        .success();

    let project_index = std::fs::read_to_string(dir.path().join("projects/fbm/INDEX.md")).unwrap();
    let root_index = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let merge_output = std::fs::read_to_string(&out_file).unwrap();
    assert_eq!(merge_output, project_index);
    assert_ne!(
        merge_output, root_index,
        "merge output should match the targeted project index, not the root index"
    );
}

#[test]
fn index_merge_output_without_merge_path_rejected() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["index", "--merge-output", "/tmp/does-not-matter"])
        .assert()
        .failure();
}

#[test]
fn index_merge_path_without_merge_output_rejected() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["index", "--merge-path", "INDEX.md"])
        .assert()
        .failure();
}

#[test]
fn index_merge_path_nonexistent_index_fails_with_context() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    let out_file = dir.path().join("merge-output.tmp");

    // A syntactically valid path that no regeneration ever writes: the
    // driver must fail cleanly (git then treats the file as an unresolved
    // conflict), not panic or silently succeed.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "index",
            "--merge-output",
            out_file.to_str().unwrap(),
            "--merge-path",
            "projects/ghost/INDEX.md",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("projects/ghost/INDEX.md"));

    assert!(
        !out_file.exists(),
        "no merge output should be written when the index path doesn't exist"
    );
}

#[test]
fn index_bare_still_prints_generated_message() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated INDEX.md"));
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

    // Promote task to roadmap, then refresh the indexes explicitly.
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
    assert_eq!(
        std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap(),
        root_before,
        "the promote mutation itself must leave INDEX.md untouched"
    );
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
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
