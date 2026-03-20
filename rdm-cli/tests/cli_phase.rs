use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn create_phase(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            slug,
            "--title",
            title,
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

fn init_with_roadmap(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "two-way",
            "--title",
            "Two-Way Players",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

#[test]
fn phase_create_auto_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase 'phase-1-core'"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "service",
            "--title",
            "Keeper Service",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase 'phase-2-service'"));
}

#[test]
fn phase_create_explicit_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--number",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-5-core"));
}

#[test]
fn phase_show() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Status: not-started")),
        );
}

#[test]
fn phase_update_to_done() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated 'phase-1-core' → done"));

    // Verify completed date is set
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Status: done").and(predicate::str::contains("Completed:")),
        );
}

#[test]
fn phase_update_done_then_back() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "done",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));

    // Verify completed date is cleared
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Status: in-progress")
                .and(predicate::str::contains("Completed:").not()),
        );
}

#[test]
fn phase_list() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| # | Phase | Status | Stem |")
                .and(predicate::str::contains(
                    "| 1 | Core Valuation | not-started | phase-1-core |",
                ))
                .and(predicate::str::contains(
                    "| 2 | Keeper Service | not-started | phase-2-service |",
                )),
        );
}

#[test]
fn phase_list_empty() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No phases yet."));
}

#[test]
fn phase_show_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Stem: phase-1-core")),
        );
}

#[test]
fn phase_show_by_number_not_found() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "99",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("phase not found: 99"));
}

#[test]
fn phase_update_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

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
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated 'phase-1-core' → done"));
}

#[test]
fn phase_create_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Phase description here.",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Phase description here."));
}

#[test]
fn phase_update_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "Updated body content.",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated body content."));
}

#[test]
fn phase_create_with_stdin_pipe() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .write_stdin("piped phase content")
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains("piped phase content"),
        "expected piped content in file, got: {content}"
    );
}

#[test]
fn phase_remove_by_stem() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "remove",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed phase 'phase-1-core'"));

    // Verify it no longer appears in phase list
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Keeper Service")
                .and(predicate::str::contains("Core Valuation").not()),
        );
}

#[test]
fn phase_remove_by_number() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "remove",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed phase 'phase-1-core'"));

    // Verify phase list is empty
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No phases yet."));
}

#[test]
fn phase_remove_not_found() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "remove",
            "phase-99-nope",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("phase not found"));
}

#[test]
fn phase_show_body_and_no_body() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Append body text to the phase file
    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    fs::write(
        &phase_file,
        format!("{content}\n## Details\n\nPhase body content.\n"),
    )
    .unwrap();

    // show includes body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Phase body content.")),
        );

    // show --no-body suppresses body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-body",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Phase 1: Core Valuation")
                .and(predicate::str::contains("Phase body content.").not()),
        );
}

#[test]
fn phase_create_no_edit_skips_editor() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "no-edit",
            "--title",
            "No Edit Phase",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created phase"));
}

#[test]
fn phase_update_no_edit_skips_editor() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));
}

#[test]
fn phase_update_without_status_preserves_existing() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // First set status to in-progress
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "in-progress",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Update body only, without --status
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "New body content.",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("in-progress"));

    // Verify body was updated and status preserved
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("New body content."))
        .stdout(predicate::str::contains("in-progress"));
}

#[test]
fn phase_update_without_status_and_without_body() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Update with neither --status nor --body should succeed (no-op)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("not-started"));
}

#[test]
fn phase_show_includes_navigation_hints() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");
    create_phase(&dir, "ui", "UI Layer");

    // Middle phase has both prev and next
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-2-service",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Prev: rdm phase show phase-1-core")
                .and(predicate::str::contains("Next: rdm phase show phase-3-ui")),
        );
}

#[test]
fn phase_show_first_phase_no_prev() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Next: rdm phase show phase-2-service")
                .and(predicate::str::contains("Prev:").not()),
        );
}

#[test]
fn phase_update_done_to_done_updates_commit() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Mark done with --commit abc
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--commit",
            "abc123",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Mark done again with --commit def
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--commit",
            "def456",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Verify show output has def456
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("def456"));
}

#[test]
fn phase_update_done_to_done_no_commit_preserves() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Mark done with --commit abc
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--commit",
            "abc123",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Mark done again without --commit
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
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();

    // Verify abc123 is preserved
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("abc123"));
}

#[test]
fn phase_show_last_phase_no_next() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");
    create_phase(&dir, "service", "Keeper Service");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-2-service",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Prev: rdm phase show phase-1-core")
                .and(predicate::str::contains("Next:").not()),
        );
}
