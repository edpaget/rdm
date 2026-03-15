use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use assert_cmd::Command;
use predicates::prelude::*;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Returns a unique subdirectory under `.tmp/` in the project root for each test.
fn test_dir(name: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".tmp")
        .join(format!("{name}-{id}"));
    // Clean up from any prior run, then create fresh
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

#[test]
fn init_creates_plan_repo() {
    let dir = test_dir("init");
    rdm()
        .arg("--root")
        .arg(&dir)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized plan repo"));

    assert!(dir.join("rdm.toml").exists());
    assert!(dir.join("projects").is_dir());
    assert!(dir.join("INDEX.md").exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn init_twice_fails() {
    let dir = test_dir("init-twice");
    rdm().arg("--root").arg(&dir).arg("init").assert().success();

    rdm()
        .arg("--root")
        .arg(&dir)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already initialized"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn no_subcommand_shows_usage() {
    rdm()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}
