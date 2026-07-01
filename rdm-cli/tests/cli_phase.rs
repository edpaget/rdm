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
            predicate::str::contains("| # | Phase | Status | Difficulty | Model | Stem |")
                .and(predicate::str::contains(
                    "| 1 | Core Valuation | not-started | - | - | phase-1-core |",
                ))
                .and(predicate::str::contains(
                    "| 2 | Keeper Service | not-started | - | - | phase-2-service |",
                )),
        );
}

#[test]
fn phase_create_with_difficulty_and_model() {
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
            "--difficulty",
            "hard",
            "--model",
            "large",
            "--no-edit",
        ])
        .assert()
        .success();

    // Human show reflects both
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
            predicate::str::contains("Difficulty: hard")
                .and(predicate::str::contains("Model: large")),
        );

    // JSON show reflects both
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
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"difficulty\": \"hard\"")
                .and(predicate::str::contains("\"model\": \"large\"")),
        );
}

#[test]
fn phase_update_sets_and_clears_difficulty_and_model() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Set both
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--difficulty",
            "easy",
            "--model",
            "small",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
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
            predicate::str::contains("Difficulty: easy")
                .and(predicate::str::contains("Model: small")),
        );

    // Clear both
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--clear-difficulty",
            "--clear-model",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
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
            predicate::str::contains("Difficulty:")
                .not()
                .and(predicate::str::contains("Model:").not()),
        );
}

#[test]
fn phase_update_status_and_estimate_in_one_invocation() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core Valuation");

    // Status + estimate in a single invocation (the consolidated single-write
    // path). stdout still reports the status transition unchanged.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--status",
            "in-progress",
            "--difficulty",
            "hard",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Updated 'phase-1-core' → in-progress",
        ));

    // Both the status change and the estimate (hard → derived large) landed.
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
                .and(predicate::str::contains("Difficulty: hard"))
                .and(predicate::str::contains("Model: large")),
        );
}

#[test]
fn phase_update_difficulty_conflicts_with_clear() {
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
            "--difficulty",
            "hard",
            "--clear-difficulty",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .failure();
}

#[test]
fn phase_update_model_conflicts_with_clear() {
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
            "--model",
            "large",
            "--clear-model",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .failure();
}

#[test]
fn phase_list_shows_difficulty_and_model_columns() {
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
            "--difficulty",
            "moderate",
            "--model",
            "medium",
            "--no-edit",
        ])
        .assert()
        .success();

    // Human list shows the populated columns
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["phase", "list", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("| # | Phase | Status | Difficulty | Model | Stem |").and(
                predicate::str::contains(
                    "| 1 | Core Valuation | not-started | moderate | medium | phase-1-core |",
                ),
            ),
        );

    // JSON list carries the fields
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "list",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"difficulty\": \"moderate\"")
                .and(predicate::str::contains("\"model\": \"medium\"")),
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

fn head_sha(dir: &std::path::Path) -> String {
    let repo = gix::open(dir).unwrap();
    let mut head = repo.head().unwrap();
    head.peel_to_commit().unwrap().id.to_string()
}

#[test]
fn phase_show_at_revision_returns_historical_body() {
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
            "original-phase-body",
            "--no-edit",
        ])
        .assert()
        .success();

    let old_sha = head_sha(dir.path());

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "new-phase-body",
            "--no-edit",
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
            "--at",
            &old_sha,
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("original-phase-body")
                .and(predicate::str::contains(format!("Revision: {old_sha}")))
                .and(predicate::str::contains("new-phase-body").not()),
        );
}

#[test]
fn phase_show_at_unknown_revision_errors() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Core");

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
            "--at",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not known to the store"));
}

#[test]
fn phase_show_at_revision_missing_path_errors() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);

    // Capture anchor SHA *before* the phase exists.
    let pre_sha = head_sha(dir.path());

    create_phase(&dir, "core", "Core");

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
            "--at",
            &pre_sha,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not present at revision"));
}

#[test]
fn phase_update_body_flag_beats_stdin() {
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
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "inline body wins",
        ])
        .write_stdin("piped body loses")
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains("inline body wins"),
        "expected inline body in file, got: {content}"
    );
    assert!(
        !content.contains("piped body loses"),
        "stdin must be ignored when --body is provided, got: {content}"
    );
}

#[test]
fn phase_update_empty_body_refuses_clobber() {
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
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "existing content",
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
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "",
            "--no-edit",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--clear-body"));

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        content.contains("existing content"),
        "body should be unchanged after refused clobber, got: {content}"
    );
}

#[test]
fn phase_update_clear_body_succeeds() {
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
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "existing content",
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
            "phase-1-core",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--clear-body",
            "--no-edit",
        ])
        .assert()
        .success();

    let phase_file = dir
        .path()
        .join("projects/fbm/roadmaps/two-way/phase-1-core.md");
    let content = fs::read_to_string(&phase_file).unwrap();
    assert!(
        !content.contains("existing content"),
        "body should be empty after --clear-body, got: {content}"
    );
}

#[test]
fn phase_update_empty_body_ok_when_already_empty() {
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
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn phase_update_clear_body_conflicts_with_body_flag() {
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
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--body",
            "x",
            "--clear-body",
            "--no-edit",
        ])
        .assert()
        .failure();
}

#[test]
fn phase_update_title_renames_in_place() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Old Phase Title");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--title",
            "New Phase Title",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // New title reflected via `show`; stem/number unchanged.
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
            predicate::str::contains("New Phase Title")
                .and(predicate::str::contains("phase-1-core")),
        );
}

#[test]
fn phase_update_empty_title_rejected() {
    let dir = TempDir::new().unwrap();
    init_with_roadmap(&dir);
    create_phase(&dir, "core", "Keep This Title");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "phase-1-core",
            "--title",
            "   ",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("title cannot be empty"));

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
        .stdout(predicate::str::contains("Keep This Title"));
}
