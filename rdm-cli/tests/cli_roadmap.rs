use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

fn init_with_project(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm", "--title", "FBM"])
        .assert()
        .success();
}

#[test]
fn roadmap_create_and_show() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

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
        .success()
        .stdout(predicate::str::contains("Created roadmap 'two-way'"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "show", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Two-Way Players")
                .and(predicate::str::contains("No phases yet.")),
        );
}

#[test]
fn roadmap_show_with_phases() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

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
        .args(["roadmap", "show", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("0/1 phases done")
                .and(predicate::str::contains("Core Valuation")),
        );
}

#[test]
fn roadmap_list() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

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

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "draft",
            "--title",
            "Draft Strategy",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "fbm"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("two-way") && stdout.contains("Two-Way Players"));
    assert!(stdout.contains("draft") && stdout.contains("Draft Strategy"));
}

#[test]
fn roadmap_list_empty() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No roadmaps found."));
}

#[test]
fn roadmap_list_with_progress() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

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
            "create",
            "ui",
            "--title",
            "UI",
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
            "1",
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
        .args(["roadmap", "list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1/2 done"));
}

#[test]
fn roadmap_create_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

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
            "--body",
            "Roadmap body content.",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "show", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Roadmap body content."));
}

#[test]
fn roadmap_show_body_and_no_body() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

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

    // Append body text to the roadmap file
    let roadmap_file = dir.path().join("projects/fbm/roadmaps/two-way/roadmap.md");
    let content = fs::read_to_string(&roadmap_file).unwrap();
    fs::write(
        &roadmap_file,
        format!("{content}\n## Overview\n\nBody text here.\n"),
    )
    .unwrap();

    // show includes body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "show", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Two-Way Players")
                .and(predicate::str::contains("Body text here.")),
        );

    // show --no-body suppresses body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "show",
            "two-way",
            "--project",
            "fbm",
            "--no-body",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Two-Way Players")
                .and(predicate::str::contains("Body text here.").not()),
        );
}

#[test]
fn roadmap_create_missing_project() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "create", "slug", "--project", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("project not found"));
}

#[test]
fn roadmap_create_no_edit_skips_editor() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "no-edit-rm",
            "--title",
            "No Edit Roadmap",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created roadmap 'no-edit-rm'"));
}

// -- Dependency tests --

fn init_with_two_roadmaps(dir: &TempDir) {
    init_with_project(dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "alpha",
            "--title",
            "Alpha",
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
            "roadmap",
            "create",
            "beta",
            "--title",
            "Beta",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn roadmap_depend_and_deps() {
    let dir = TempDir::new().unwrap();
    init_with_two_roadmaps(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "depend",
            "beta",
            "--on",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added dependency: beta → alpha"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "deps", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("beta → alpha"));
}

#[test]
fn roadmap_undepend() {
    let dir = TempDir::new().unwrap();
    init_with_two_roadmaps(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "depend",
            "beta",
            "--on",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "undepend",
            "beta",
            "--on",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed dependency: beta → alpha"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "deps", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No dependencies found."));
}

#[test]
fn roadmap_depend_cycle_rejected() {
    let dir = TempDir::new().unwrap();
    init_with_two_roadmaps(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "depend",
            "beta",
            "--on",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "depend",
            "alpha",
            "--on",
            "beta",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cyclic dependency"));
}

#[test]
fn roadmap_depend_nonexistent_target() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "alpha",
            "--title",
            "Alpha",
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
            "roadmap",
            "depend",
            "alpha",
            "--on",
            "nonexistent",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("roadmap not found"));
}

#[test]
fn roadmap_deps_empty() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "deps", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No dependencies found."));
}
