//! A signal arriving during `prepare` stops the run before anything spawns.
//!
//! This raises a real, process-wide SIGINT, which every concurrent
//! `run_bounded` in the same process would observe. It therefore lives alone
//! in its own test binary: under plain `cargo test` the tests of one binary
//! run as threads of a single process, so no other run may share it.

mod common;

use std::cell::Cell;
use std::time::Duration;

use common::{check_teardown, fixture};
use rdm_devtools::process::{Hooks, ProcessSpec, RunError, run_bounded};
use signal_hook::consts::SIGINT;
use tempfile::TempDir;

#[test]
fn signal_during_prepare_never_spawns_and_runs_cleanup() {
    let dir = TempDir::new().unwrap();
    let m = dir.path().join("not-a-credential");
    std::fs::write(&m, "test").expect("write marker");
    let spawned = Cell::new(false);
    let err = run_bounded(
        &ProcessSpec::new(fixture())
            .args(["print", "never"])
            .timeout(Duration::from_secs(10)),
        Hooks::new()
            .prepare(|| signal_hook::low_level::raise(SIGINT))
            .cleanup(|| std::fs::remove_file(&m))
            .on_spawn(|_| spawned.set(true)),
    )
    .unwrap_err();
    assert!(matches!(err, RunError::Interrupted(SIGINT)), "{err:?}");
    assert!(!spawned.get(), "a signal during prepare must not spawn");
    check_teardown(&m, &[]).unwrap();
}
