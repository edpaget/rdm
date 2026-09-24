//! SIGTERM arriving while a [`Session`] waits in `recv_line` tears the
//! session down: `SessionError::Interrupted` is returned and the child's
//! whole process group is killed and reaped — what lets a nextest
//! slow-timeout SIGTERM still clean up a workflow host.
//!
//! This raises a real, process-wide SIGTERM, which every concurrent session
//! or `run_bounded` in the same process would observe. It therefore lives
//! alone in its own test binary: under plain `cargo test` the tests of one
//! binary run as threads of a single process, so no other run may share it.

mod common;

use std::time::Duration;

use common::{ReapGuard, fixture, gone_within, read_pid, wait_for_file};
use rdm_devtools::process::{ProcessSpec, Session, SessionError};
use signal_hook::consts::SIGTERM;
use tempfile::TempDir;

#[test]
fn sigterm_during_recv_line_interrupts_and_kills_the_group() {
    let dir = TempDir::new().unwrap();
    let pidfile = dir.path().join("grandchild.pid");
    let spec = ProcessSpec::new(fixture())
        .arg("spawn-grandchild")
        .arg(&pidfile)
        .timeout(Duration::from_secs(30));
    let mut session = Session::spawn(&spec).expect("spawn");
    let pid = session.pid();
    let mut guard = ReapGuard::default();
    guard.track(pid);
    assert_eq!(session.recv_line().expect("ready line"), "ready");
    assert!(wait_for_file(&pidfile, Duration::from_secs(3)));
    let grandchild = read_pid(&pidfile).unwrap();
    guard.track(grandchild);

    // The child sleeps after `ready`, so the next receive blocks until the
    // signal lands.
    let raiser = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(300));
        signal_hook::low_level::raise(SIGTERM).expect("raise SIGTERM");
    });
    let err = session.recv_line().unwrap_err();
    raiser.join().expect("raiser thread");

    assert!(matches!(err, SessionError::Interrupted(SIGTERM)), "{err:?}");
    assert!(session.is_closed(), "the session is torn down");
    assert!(gone_within(pid, Duration::from_secs(3)), "child reaped");
    assert!(
        gone_within(grandchild, Duration::from_secs(3)),
        "the whole process group is killed"
    );
    guard.clear();
}
