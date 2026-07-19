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

fn create_task(dir: &TempDir, slug: &str, tags: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            slug,
            "--title",
            slug,
            "--tags",
            tags,
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();
}

fn create_roadmap(dir: &TempDir, slug: &str, tags: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            slug,
            "--title",
            slug,
            "--tags",
            tags,
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();
}

fn stdout_of(dir: &TempDir, args: &[&str]) -> Vec<u8> {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

#[test]
fn lists_tags_for_project() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_task(&dir, "t1", "cli");
    create_task(&dir, "t2", "cli");
    create_task(&dir, "t3", "web-ui");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tag", "list", "--project", "fbm"])
        .assert()
        .success();
}

#[test]
fn human_output_shows_tag_with_count() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    for slug in ["t1", "t2", "t3"] {
        create_task(&dir, slug, "cli");
    }
    for slug in ["w1", "w2", "w3", "w4"] {
        create_task(&dir, slug, "web-ui");
    }
    create_roadmap(&dir, "r1", "web-ui");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tag", "list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cli (3)"))
        .stdout(predicate::str::contains("web-ui (5)"));
}

#[test]
fn empty_project_reports_no_tags() {
    let dir = TempDir::new().unwrap();
    init(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tag", "list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No tags in use."));
}

#[test]
fn json_format_emits_array_of_tag_counts() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    for slug in ["t1", "t2", "t3"] {
        create_task(&dir, slug, "cli");
    }
    create_task(&dir, "b1", "bug");

    let out = stdout_of(
        &dir,
        &["tag", "list", "--project", "fbm", "--format", "json"],
    );
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let arr = value.as_array().expect("expected a JSON array");
    assert_eq!(arr.len(), 2, "got: {value}");
    assert_eq!(arr[0]["tag"], "cli");
    assert_eq!(arr[0]["count"], 3);
    assert_eq!(arr[0]["tasks"], 3);
    assert_eq!(arr[0]["roadmaps"], 0);
    assert_eq!(arr[1]["tag"], "bug");
    assert_eq!(arr[1]["count"], 1);
}

#[test]
fn json_format_empty_project_emits_empty_array() {
    let dir = TempDir::new().unwrap();
    init(&dir);

    let out = stdout_of(
        &dir,
        &["tag", "list", "--project", "fbm", "--format", "json"],
    );
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(value, serde_json::json!([]));
}

#[test]
fn tag_on_one_item_is_counted_once() {
    let dir = TempDir::new().unwrap();
    init(&dir);
    create_task(&dir, "t1", "cli,cli");

    let out = stdout_of(
        &dir,
        &["tag", "list", "--project", "fbm", "--format", "json"],
    );
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let arr = value.as_array().unwrap();
    assert_eq!(arr.len(), 1, "got: {value}");
    assert_eq!(arr[0]["count"], 1);
}

#[test]
fn unknown_project_fails_with_actionable_error() {
    let dir = TempDir::new().unwrap();
    init(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tag", "list", "--project", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nope"));
}

#[test]
fn resolves_default_project_without_flag() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["init", "--default-project", "fbm"])
        .assert()
        .success();
    create_task(&dir, "t1", "cli");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tag", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cli (1)"));
}

#[test]
fn table_format_is_rejected_with_actionable_error() {
    let dir = TempDir::new().unwrap();
    init(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tag", "list", "--project", "fbm", "--format", "table"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--format table is not supported"));
}
