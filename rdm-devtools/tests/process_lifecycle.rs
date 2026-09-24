//! Library-level lifecycle tests for `rdm_devtools::process::run_bounded`,
//! driven through real fixture processes.

mod common;

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{ReapGuard, check_teardown, fixture, read_pid, wait_for_file};
use rdm_devtools::process::{Hooks, ProcessSpec, RunError, Teardown, run_bounded};
use tempfile::TempDir;

/// A non-secret marker file that the run's cleanup hook removes.
fn marker(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("not-a-credential");
    std::fs::write(&path, "test").expect("write marker");
    path
}

fn remove(path: &Path) -> impl FnOnce() -> io::Result<()> + '_ {
    move || std::fs::remove_file(path)
}

fn spec(args: &[&str]) -> ProcessSpec {
    ProcessSpec::new(fixture())
        .args(args)
        .timeout(Duration::from_secs(10))
}

#[test]
fn success_returns_stdout_and_runs_cleanup() {
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let child = Cell::new(0);
    let out = run_bounded(
        &spec(&["print", "hello"]),
        Hooks::new().cleanup(remove(&m)).on_spawn(|p| child.set(p)),
    )
    .expect("success");
    assert_eq!(out.stdout, b"hello\n");
    assert!(out.status.success());
    check_teardown(&m, &[child.get()]).unwrap();
}

#[test]
fn nonzero_exit_errors_and_runs_cleanup() {
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let child = Cell::new(0);
    let err = run_bounded(
        &spec(&["exit", "3"]),
        Hooks::new().cleanup(remove(&m)).on_spawn(|p| child.set(p)),
    )
    .unwrap_err();
    assert!(
        matches!(err, RunError::NonZeroExit { code: Some(3), .. }),
        "{err:?}"
    );
    check_teardown(&m, &[child.get()]).unwrap();
}

/// Runs `spawn-grandchild` until the timeout under `teardown`, returning the
/// error and the child/grandchild pids (tracked in `guard`).
fn timeout_scenario(
    dir: &TempDir,
    m: &Path,
    teardown: Teardown,
    guard: &mut ReapGuard,
) -> (RunError, Vec<u32>) {
    let pidfile = dir.path().join("grandchild.pid");
    let child = Cell::new(0);
    let err = run_bounded(
        &spec(&["spawn-grandchild", pidfile.to_str().unwrap()])
            .timeout(Duration::from_millis(1500))
            .teardown(teardown),
        Hooks::new().cleanup(remove(m)).on_spawn(|p| child.set(p)),
    )
    .unwrap_err();
    guard.track(child.get());
    assert!(
        wait_for_file(&pidfile, Duration::from_secs(1)),
        "fixture never wrote the grandchild pid"
    );
    let grandchild = read_pid(&pidfile).unwrap();
    guard.track(grandchild);
    (err, vec![child.get(), grandchild])
}

#[test]
fn timeout_kills_group_reaps_and_runs_cleanup() {
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let mut guard = ReapGuard::default();
    let (err, pids) = timeout_scenario(&dir, &m, Teardown::Correct, &mut guard);
    assert!(matches!(err, RunError::TimedOut(_)), "{err:?}");
    check_teardown(&m, &pids).unwrap();
    guard.clear();
}

#[test]
fn output_cap_kills_and_runs_cleanup() {
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let child = Cell::new(0);
    let err = run_bounded(
        &spec(&["flood"]).stdout_cap(1024 * 1024),
        Hooks::new().cleanup(remove(&m)).on_spawn(|p| child.set(p)),
    )
    .unwrap_err();
    assert!(matches!(err, RunError::OutputCap(1_048_576)), "{err:?}");
    check_teardown(&m, &[child.get()]).unwrap();
}

#[test]
fn spawn_failure_runs_cleanup() {
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let missing = dir.path().join("no-such-program");
    let err = run_bounded(
        &ProcessSpec::new(&missing),
        Hooks::new().cleanup(remove(&m)),
    )
    .unwrap_err();
    assert!(matches!(err, RunError::Spawn(_)), "{err:?}");
    check_teardown(&m, &[]).unwrap();
}

#[test]
fn prepare_failure_runs_cleanup_and_never_spawns() {
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let spawned = Cell::new(false);
    let err = run_bounded(
        &spec(&["print", "never"]),
        Hooks::new()
            .prepare(|| Err(io::Error::other("partial private copy")))
            .cleanup(remove(&m))
            .on_spawn(|_| spawned.set(true)),
    )
    .unwrap_err();
    assert!(matches!(err, RunError::Prepare(_)), "{err:?}");
    assert!(!spawned.get(), "a failed prepare must not spawn");
    check_teardown(&m, &[]).unwrap();
}

#[test]
fn cleanup_runs_exactly_once() {
    for args in [&["print", "ok"][..], &["exit", "5"][..], &["sleep"][..]] {
        let calls = Cell::new(0);
        let _ = run_bounded(
            &spec(args).timeout(Duration::from_millis(300)),
            Hooks::new().cleanup(|| {
                calls.set(calls.get() + 1);
                Ok(())
            }),
        );
        assert_eq!(calls.get(), 1, "cleanup count for {args:?}");
    }
}

#[test]
fn cleanup_error_supersedes_success() {
    let err = run_bounded(
        &spec(&["print", "ok"]),
        Hooks::new().cleanup(|| Err(io::Error::other("marker removal failed"))),
    )
    .unwrap_err();
    assert!(matches!(err, RunError::Cleanup(_)), "{err:?}");
}

#[test]
fn stderr_is_drained_not_returned() {
    // Far more than any pipe buffer: an undrained stderr would deadlock.
    let out = run_bounded(&spec(&["stderr-flood", "4194304"]), Hooks::new()).expect("success");
    assert_eq!(out.stdout, b"done\n");
}

#[test]
fn child_env_is_spec_env_only() {
    let hostile = [
        ("RDM_ROOT", "/hostile/rdm-root"),
        ("HOME", "/hostile/home"),
        ("NODE_OPTIONS", "--hostile"),
        ("GIT_CONFIG_GLOBAL", "/hostile/gitconfig"),
    ];
    let names: Vec<&str> = hostile.iter().map(|(k, _)| *k).collect();
    let before: Vec<_> = names.iter().map(std::env::var_os).collect();

    let mut s = spec(&["print-env"]).args(&names);
    for (k, v) in hostile {
        s = s.env(k, v);
    }
    let out = run_bounded(&s, Hooks::new()).expect("success");
    let expected: String = hostile.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
    let after: Vec<_> = names.iter().map(std::env::var_os).collect();
    assert_eq!(
        before, after,
        "the runner must not touch its own environment"
    );

    let cleared = spec(&["print-env", "PATH", "HOME", "ONLY"])
        .env_clear()
        .env("ONLY", "1");
    let out = run_bounded(&cleared, Hooks::new()).expect("success");
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "PATH unset\nHOME unset\nONLY=1\n"
    );

    let removed = spec(&["print-env", "PATH"]).env_remove("PATH");
    let out = run_bounded(&removed, Hooks::new()).expect("success");
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "PATH unset\n");
}

#[test]
fn broken_cleanup_mutant_is_detected() {
    // Leaking the process group: the checker must name the surviving pid.
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let mut guard = ReapGuard::default();
    let (err, pids) = timeout_scenario(&dir, &m, Teardown::SkipGroupKill, &mut guard);
    assert!(matches!(err, RunError::TimedOut(_)), "{err:?}");
    let verdict = check_teardown(&m, &pids);
    guard.kill_all();
    let msg = verdict.expect_err("a leaked grandchild must be detected");
    assert!(msg.contains(&pids[1].to_string()), "{msg}");

    // Skipping cleanup on the failure path: the checker must name the marker.
    let dir = TempDir::new().unwrap();
    let m = marker(&dir);
    let mut guard = ReapGuard::default();
    let (err, pids) = timeout_scenario(&dir, &m, Teardown::SkipCleanupOnError, &mut guard);
    assert!(matches!(err, RunError::TimedOut(_)), "{err:?}");
    let verdict = check_teardown(&m, &pids);
    guard.kill_all();
    let msg = verdict.expect_err("a skipped cleanup must be detected");
    assert!(msg.contains("not-a-credential"), "{msg}");
}
