use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
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
