//! A best-effort, age-bounded advisory file lock.
//!
//! Two places in rdm want the same narrow guarantee: *most of the time*,
//! keep two concurrent processes out of a short critical section, and **never**
//! block indefinitely if the holder was killed. The scoped-commit path wants
//! it around read-HEAD → build-tree → update-ref; the filesystem store's
//! flush wants it around verify-baselines → rename.
//!
//! Both are *advisory by construction*: failing to take the lock proceeds
//! anyway rather than erroring, because in each case a real correctness
//! mechanism sits underneath (the compare-and-swap on HEAD; the content-digest
//! precondition). The lock only narrows the window in the common case. That is
//! also why the session journal does **not** use it: a journal rewrite has no
//! such underlayer, so `journal::compact` needs exclusion the kernel enforces
//! rather than a bound on how long a stale file may stand, and it takes a
//! `File::lock` on a lock file of its own instead (see
//! `rdm_core::session::journal`). A caller here whose critical section ends in
//! an irreversible step can ask [`AdvisoryLock::still_held`] before taking it
//! rather than trusting `stale_after` not to have dispossessed it mid-run.
//!
//! Living here rather than in each backend means the wait/staleness state
//! machine exists once. The *durations* stay with the caller, since the commit
//! path shortens them under `cfg(test)` and the flush path does not.
//!
//! # Why this touches the filesystem directly
//!
//! This module is category (b) of the direct-I/O carve-out in
//! `docs/principles.md` § 1 — *out-of-band coordination state*. It is
//! mechanism, not plan data: it knows no domain type, holds no
//! [`Store`](crate::store::Store), and its lock files live outside the plan
//! tree and are never committed. It cannot route through a `Store` even in
//! principle, because one of its two callers is the filesystem store's own
//! flush — a lock taken through the flush it guards would be circular.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// A held (or deliberately unheld) advisory lock, released on drop.
///
/// [`AdvisoryLock::held`] reports which: an unheld guard is the degraded
/// "proceed anyway" case, not an error. [`AdvisoryLock::still_held`] reports
/// whether the lock this guard took is *still* the one on disk, which is a
/// weaker and more useful question once staleness takeover is in play — see
/// its own documentation.
#[derive(Debug)]
pub struct AdvisoryLock {
    path: Option<PathBuf>,
    /// The bytes written into the lock file when this guard created it.
    ///
    /// Ownership evidence. Age-bounded takeover means a guard can stop holding
    /// its lock without ever being told, so "the lock file exists" says
    /// nothing; "the lock file still contains *my* token" does.
    token: String,
}

/// Mints a token unique to one guard.
///
/// Uniqueness has to hold across processes (two `rdm` runs on one repo) and
/// within one (two guards on different lock files, or the same file in
/// sequence), so it combines the process id, the wall clock, and a
/// process-global counter. It is evidence of identity only — nothing reads it
/// back for meaning — so a clock that jumps costs nothing.
fn mint_token() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("rdm-lock {} {nanos} {seq}\n", std::process::id())
}

impl AdvisoryLock {
    /// Tries to take the lock file at `path`, waiting up to `wait` for a
    /// contended one and taking over any lock older than `stale_after`.
    ///
    /// Never errors and never blocks past `wait`: every failure mode — an
    /// uncreatable parent directory, a live holder, an expired deadline —
    /// degrades to an unheld guard so the caller proceeds unlocked.
    ///
    /// # Panics
    ///
    /// Never.
    pub fn acquire(path: PathBuf, wait: Duration, stale_after: Duration) -> Self {
        let token = mint_token();
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return Self::unheld(token);
        }
        let deadline = Instant::now() + wait;
        loop {
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
            {
                Ok(mut file) => {
                    // The token is what later lets this guard tell its own
                    // lock from a successor's. A guard that cannot record it
                    // could never answer `still_held`, so it releases the file
                    // and degrades to unheld rather than holding a lock whose
                    // ownership it cannot prove.
                    if file.write_all(token.as_bytes()).is_err() || file.flush().is_err() {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Self::unheld(token);
                    }
                    return Self {
                        path: Some(path),
                        token,
                    };
                }
                Err(_) => {
                    // Age-bounded takeover: a lock left by a killed process
                    // must never block a deadline-bounded caller forever.
                    let stale = std::fs::metadata(&path)
                        .and_then(|md| md.modified())
                        .map(|m| m.elapsed().map(|d| d > stale_after).unwrap_or(false))
                        .unwrap_or(false);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Self::unheld(token);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    /// An acquisition that did not take the lock.
    fn unheld(token: String) -> Self {
        Self { path: None, token }
    }

    /// Returns whether this guard actually took the lock.
    ///
    /// Answers a question about *acquisition*, not about the present: see
    /// [`AdvisoryLock::still_held`] for the latter.
    #[must_use]
    pub fn held(&self) -> bool {
        self.path.is_some()
    }

    /// Returns whether the lock this guard took is still the one on disk.
    ///
    /// [`AdvisoryLock::acquire`] takes over a lock file older than
    /// `stale_after` without any evidence that its holder released or died —
    /// deliberately, because a lock left behind by a killed process must never
    /// block a deadline-bounded caller forever. The cost is that a holder
    /// whose critical section legitimately outruns `stale_after` can be
    /// dispossessed *while still running*, and nothing tells it so.
    ///
    /// A caller whose critical section ends in an irreversible step (a
    /// `rename` over shared state, an unlink) can ask this immediately before
    /// that step and decline to take it. That converts the takeover from a
    /// silent double-holder into a detected one, at the cost of one `read`.
    /// It is not a substitute for holding the lock: it narrows nothing on its
    /// own, and it can still be raced in the instant after it returns. What it
    /// rules out is the *long* overlap — the one the takeover threshold
    /// creates by construction, where the successor has already been running
    /// for its own critical section.
    ///
    /// Always `false` for a guard that never took the lock.
    #[must_use]
    pub fn still_held(&self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };
        std::fs::read(path).is_ok_and(|found| found == self.token.as_bytes())
    }

    /// Returns the lock file this guard holds, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Drop for AdvisoryLock {
    fn drop(&mut self) {
        // Only release a lock file that is still this guard's own. Deleting a
        // successor's file after a staleness takeover would hand the lock to a
        // third party while the successor believed it held it.
        let held = self.still_held();
        if let Some(path) = self.path.take()
            && held
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const WAIT: Duration = Duration::from_millis(80);
    const STALE: Duration = Duration::from_secs(30);

    fn lock_path(dir: &TempDir) -> PathBuf {
        dir.path().join("sub").join("x.lock")
    }

    #[test]
    fn a_free_lock_is_taken_and_released_on_drop() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(&dir);
        {
            let lock = AdvisoryLock::acquire(path.clone(), WAIT, STALE);
            assert!(lock.held());
            assert!(path.exists());
        }
        assert!(!path.exists(), "dropping the guard must release the lock");
    }

    #[test]
    fn a_fresh_foreign_lock_degrades_to_unheld_after_the_wait() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"held").unwrap();

        {
            let lock = AdvisoryLock::acquire(path.clone(), WAIT, STALE);
            assert!(!lock.held(), "a fresh foreign lock must not be stolen");
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"held",
            "releasing an unheld lock must not delete the holder's file"
        );
    }

    #[test]
    fn a_stale_lock_is_taken_over_immediately() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"pid 1 (killed)").unwrap();
        let when = std::time::SystemTime::now() - (STALE + Duration::from_secs(5));
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();

        let started = Instant::now();
        let lock = AdvisoryLock::acquire(path.clone(), WAIT, STALE);
        assert!(lock.held(), "a stale lock must be taken over");
        assert!(started.elapsed() < WAIT, "takeover must be immediate");
        assert!(
            lock.still_held(),
            "the taker must have replaced the dead holder's contents with its own token"
        );
        assert_ne!(
            std::fs::read(&path).unwrap(),
            b"pid 1 (killed)".to_vec(),
            "the dead holder's bytes must not survive the takeover"
        );
    }

    #[test]
    fn a_dispossessed_holder_reports_that_it_no_longer_holds_the_lock() {
        // The residual staleness takeover creates by construction: a holder
        // whose critical section outruns `stale_after` keeps its guard while a
        // successor takes the file. `held` cannot see that; `still_held` must.
        let dir = TempDir::new().unwrap();
        let path = lock_path(&dir);
        let first = AdvisoryLock::acquire(path.clone(), WAIT, STALE);
        assert!(first.held() && first.still_held());

        // Age the lock past staleness, then let a second guard take it over.
        let when = std::time::SystemTime::now() - (STALE + Duration::from_secs(5));
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();
        let second = AdvisoryLock::acquire(path.clone(), WAIT, STALE);

        assert!(second.held() && second.still_held());
        assert!(first.held(), "the first guard still believes it acquired");
        assert!(
            !first.still_held(),
            "but it must be able to discover it was dispossessed"
        );

        drop(first);
        assert!(
            second.still_held(),
            "releasing a dispossessed guard must not delete the successor's lock"
        );
        assert!(path.exists());
    }

    #[test]
    fn two_guards_on_one_path_never_share_a_token() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(&dir);
        let first = AdvisoryLock::acquire(path.clone(), WAIT, STALE);
        let first_bytes = std::fs::read(&path).unwrap();
        drop(first);
        let second = AdvisoryLock::acquire(path.clone(), WAIT, STALE);
        assert!(second.still_held());
        assert_ne!(
            first_bytes,
            std::fs::read(&path).unwrap(),
            "a reused lock path must not hand a released guard's token to its successor"
        );
    }

    #[test]
    fn an_unusable_lock_directory_degrades_to_unheld() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("sub"), b"not a directory").unwrap();
        let lock = AdvisoryLock::acquire(lock_path(&dir), WAIT, STALE);
        assert!(
            !lock.held(),
            "an uncreatable lock directory must degrade to unlocked, never error or hang"
        );
    }
}
