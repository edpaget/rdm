use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    // Isolate from any RDM_PROJECT set in the ambient environment.
    cmd.env_remove("RDM_PROJECT");
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

fn create_task(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            slug,
            "--title",
            title,
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn prints_all_sections_human() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_task(&dir, "some-task", "Some Task");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["backlog", "report", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stale tasks"))
        .stdout(predicate::str::contains("Duplicate clusters"))
        .stdout(predicate::str::contains("Tag clusters"))
        .stdout(predicate::str::contains("Archivable roadmaps"));
}

#[test]
fn json_structure() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_task(&dir, "some-task", "Some Task");

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["backlog", "report", "--project", "fbm", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let obj = value.as_object().expect("expected a JSON object");
    assert!(obj.contains_key("stale_tasks"));
    assert!(obj.contains_key("duplicate_clusters"));
    assert!(obj.contains_key("tag_clusters"));
    assert!(obj.contains_key("archivable_roadmaps"));
}

#[test]
fn older_than_zero_flags_fresh() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_task(&dir, "fresh-task", "Fresh Task");

    // Default threshold (60 days): a task created today is not stale.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["backlog", "report", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("fresh-task").not());

    // --older-than 0: a task created today (0 days old) is flagged.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["backlog", "report", "--project", "fbm", "--older-than", "0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("fresh-task"));
}

#[test]
fn help_documents_default_threshold() {
    rdm()
        .args(["backlog", "report", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("60"));
}

#[test]
fn performs_zero_writes() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_task(&dir, "some-task", "Some Task");

    // Land the setup so `status` starts clean.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "chore(plan): create some-task"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["backlog", "report", "--project", "fbm"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No uncommitted changes."));
}

#[test]
fn table_format_rejected() {
    let dir = TempDir::new().unwrap();
    init(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["backlog", "report", "--project", "fbm", "--format", "table"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("table"));
}

#[test]
fn without_project_fails() {
    let dir = TempDir::new().unwrap();
    init(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["backlog", "report"])
        .assert()
        .failure();
}
