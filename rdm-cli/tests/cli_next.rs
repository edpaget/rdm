use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn init(dir: &TempDir) {
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
}

fn create_roadmap(dir: &TempDir, slug: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            slug,
            "--title",
            slug,
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

fn create_phase(dir: &TempDir, roadmap: &str, slug: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            slug,
            "--title",
            slug,
            "--roadmap",
            roadmap,
            "--project",
            "fbm",
        ])
        .assert()
        .success();
}

fn create_phase_with_estimate(
    dir: &TempDir,
    roadmap: &str,
    slug: &str,
    difficulty: &str,
    model: &str,
) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            slug,
            "--title",
            slug,
            "--roadmap",
            roadmap,
            "--project",
            "fbm",
            "--difficulty",
            difficulty,
            "--model",
            model,
        ])
        .assert()
        .success();
}

fn set_phase_status(dir: &TempDir, roadmap: &str, stem: &str, status: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            stem,
            "--status",
            status,
            "--roadmap",
            roadmap,
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn next_fresh_roadmap_prints_phase_one() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_roadmap(&dir, "two-way");
    create_phase(&dir, "two-way", "core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["next", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-1-core"));
}

#[test]
fn next_fresh_roadmap_json() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_roadmap(&dir, "two-way");
    create_phase(&dir, "two-way", "core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "next",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"result\": \"phase\""))
        .stdout(predicate::str::contains("\"stem\": \"phase-1-core\""))
        .stdout(predicate::str::contains("\"number\": 1"));
}

#[test]
fn next_complete_roadmap_nothing_actionable() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_roadmap(&dir, "two-way");
    create_phase(&dir, "two-way", "core");
    set_phase_status(&dir, "two-way", "phase-1-core", "done");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["next", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing actionable in two-way."));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "next",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"result\": \"nothing\""));
}

#[test]
fn next_blocked_on_unmet_dependency() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    // Dependency roadmap with an open phase.
    create_roadmap(&dir, "dep");
    create_phase(&dir, "dep", "groundwork");
    // Dependent roadmap.
    create_roadmap(&dir, "two-way");
    create_phase(&dir, "two-way", "core");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "depend",
            "two-way",
            "--on",
            "dep",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "next",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"result\": \"blocked-on-dependencies\"",
        ))
        .stdout(predicate::str::contains("dep"));
}

#[test]
fn next_surfaces_difficulty_and_model() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_roadmap(&dir, "two-way");
    create_phase_with_estimate(&dir, "two-way", "core", "hard", "large");

    // Text output includes the difficulty/model lines.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["next", "--roadmap", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("difficulty: hard"))
        .stdout(predicate::str::contains("model: large"));

    // JSON output includes the difficulty/model keys and values.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "next",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"difficulty\": \"hard\""))
        .stdout(predicate::str::contains("\"model\": \"large\""));
}

#[test]
fn next_markdown_format_succeeds() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_roadmap(&dir, "two-way");
    create_phase(&dir, "two-way", "core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "next",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-1-core"));
}

#[test]
fn next_table_format_is_rejected() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_roadmap(&dir, "two-way");
    create_phase(&dir, "two-way", "core");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "next",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
            "--format",
            "table",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("table"));
}

#[test]
fn next_without_roadmap_flag_fails() {
    let dir = TempDir::new().unwrap();
    init(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["next", "--project", "fbm"])
        .assert()
        .failure()
        // clap's required-argument error, not a project-wide scan.
        .stderr(predicate::str::contains("--roadmap"))
        .stderr(predicate::str::contains("scan").not());
}
