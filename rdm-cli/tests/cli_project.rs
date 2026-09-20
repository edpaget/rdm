use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn init_repo(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
}

#[test]
fn project_create_success() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "project",
            "create",
            "fbm",
            "--title",
            "Fantasy Baseball Manager",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created project 'fbm'"));

    assert!(dir.path().join("projects/fbm/project.md").exists());
}

#[test]
fn project_create_duplicate_fails() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn project_list_empty() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No projects yet."));
}

#[test]
fn project_list_shows_projects() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "aaa"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "zzz"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("aaa").and(predicate::str::contains("zzz")));
}

#[test]
fn project_update_sets_and_reads_back_source() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "project",
            "update",
            "acme",
            "--source-repo",
            "/src/acme",
            "--source-branch",
            "main",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("/src/acme"))
        .stdout(predicate::str::contains("branch: main"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "show", "acme", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"repo\": \"/src/acme\""))
        .stdout(predicate::str::contains("\"default_branch\": \"main\""));
}

#[test]
fn project_update_branch_only_keeps_the_configured_repo() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "update", "acme", "--source-repo", "/src/acme"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "update", "acme", "--source-branch", "trunk"])
        .assert()
        .success()
        .stdout(predicate::str::contains("/src/acme"))
        .stdout(predicate::str::contains("branch: trunk"));
}

#[test]
fn project_update_branch_without_repo_is_actionable() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "update", "acme", "--source-branch", "main"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no source repository configured"))
        .stderr(predicate::str::contains("--source-repo"));
}

#[test]
fn project_update_clear_source_removes_it() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "update", "acme", "--source-repo", "/src/acme"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "update", "acme", "--clear-source"])
        .assert()
        .success()
        .stdout(predicate::str::contains("source cleared"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "show", "acme", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"source\"").not());
}

#[test]
fn project_update_clear_source_conflicts_with_repo() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "project",
            "update",
            "acme",
            "--clear-source",
            "--source-repo",
            "/src/acme",
        ])
        .assert()
        .failure();
}

#[test]
fn project_update_with_no_flags_is_actionable() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "update", "acme"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to update"))
        .stderr(predicate::str::contains("--clear-source"));
}

#[test]
fn project_update_unknown_project_is_actionable() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "update", "ghost", "--source-repo", "/src"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ghost"));
}
