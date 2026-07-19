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

#[test]
fn list_empty_project() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No roadmaps found."));
}

#[test]
fn list_with_progress() {
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
        .args(["list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "two-way — Two-Way Players (1/1 done)",
        ));
}

#[test]
fn list_all_projects() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    // Create two projects with roadmaps
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "alpha", "--title", "Alpha Project"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "beta", "--title", "Beta Project"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "r1",
            "--title",
            "Road One",
            "--project",
            "alpha",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "r2",
            "--title",
            "Road Two",
            "--project",
            "beta",
        ])
        .assert()
        .success();

    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--all"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Project: alpha"));
    assert!(stdout.contains("Project: beta"));
    assert!(stdout.contains("r1 — Road One"));
    assert!(stdout.contains("r2 — Road Two"));
}

#[test]
fn list_no_project_fails() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    rdm()
        .env_remove("RDM_PROJECT")
        .arg("--root")
        .arg(dir.path())
        .args(["list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no project specified"));
}

fn create_tagged_roadmap(dir: &TempDir, project: &str, slug: &str, tags: Option<&str>) {
    let mut cmd = rdm();
    cmd.arg("--root").arg(dir.path()).args([
        "roadmap",
        "create",
        slug,
        "--title",
        slug,
        "--project",
        project,
        "--no-edit",
    ]);
    if let Some(tags) = tags {
        cmd.args(["--tags", tags]);
    }
    cmd.assert().success();
}

#[test]
fn list_filters_by_tag() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_roadmap(&dir, "fbm", "auth-rm", Some("auth"));
    create_tagged_roadmap(&dir, "fbm", "misc-rm", None);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "fbm", "--tag", "auth"])
        .assert()
        .success()
        .stdout(predicate::str::contains("auth-rm").and(predicate::str::contains("misc-rm").not()));
}

#[test]
fn list_multi_tag_is_and() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_roadmap(&dir, "fbm", "both-rm", Some("bug,ui"));
    create_tagged_roadmap(&dir, "fbm", "bug-only-rm", Some("bug"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "fbm", "--tag", "bug", "--tag", "ui"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("both-rm").and(predicate::str::contains("bug-only-rm").not()),
        );
}

#[test]
fn list_all_applies_tag_filter_per_project() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "other"])
        .assert()
        .success();
    create_tagged_roadmap(&dir, "fbm", "auth-rm", Some("auth"));
    create_tagged_roadmap(&dir, "other", "other-rm", None);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--all", "--tag", "auth"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("auth-rm")
                .and(predicate::str::contains("other-rm").not())
                // Every project still prints its header, even with no matches.
                .and(predicate::str::contains("Project: other")),
        );
}

#[test]
fn list_json_applies_tag_filter_before_serialization() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_roadmap(&dir, "fbm", "auth-rm", Some("auth"));
    create_tagged_roadmap(&dir, "fbm", "misc-rm", None);

    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "--format",
            "json",
            "list",
            "--project",
            "fbm",
            "--tag",
            "auth",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 1, "expected one roadmap, got {json}");
    assert_eq!(arr[0]["slug"], "auth-rm");
}

#[test]
fn list_shows_tags_suffix_when_tagged() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_tagged_roadmap(&dir, "fbm", "tagged-rm", Some("bug,ui"));
    create_tagged_roadmap(&dir, "fbm", "plain-rm", None);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[tags: bug, ui]"));
}
