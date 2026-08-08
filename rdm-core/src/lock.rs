//! A best-effort, age-bounded advisory file lock.
//!
//! Two places in rdm want the same narrow guarantee: *most of the time*, keep
//! two concurrent processes out of a short critical section, and **never**
//! block indefinitely if the holder was killed. The scoped-commit path wants
//! it around read-HEAD → build-tree → update-ref; the filesystem store's
//! flush wants it around verify-baselines → rename.
//!
//! Both are *advisory by construction*: failing to take the lock proceeds
//! anyway rather than erroring, because in each case a real correctness
//! mechanism sits underneath (the compare-and-swap on HEAD; the content-digest
//! precondition). The lock only narrows the window in the common case.
//!
//! Living here rather than in each backend means the wait/staleness state
//! machine exists once. The *durations* stay with the caller, since the commit
//! path shortens them under `cfg(test)` and the flush path does not.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A held (or deliberately unheld) advisory lock, released on drop.
///
/// [`AdvisoryLock::held`] reports which: an unheld guard is the degraded
/// "proceed anyway" case, not an error.
#[derive(Debug)]
pub struct AdvisoryLock {
    path: Option<PathBuf>,
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
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return Self { path: None };
        }
        let deadline = Instant::now() + wait;
        loop {
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
            {
                Ok(_) => return Self { path: Some(path) },
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
                        return Self { path: None };
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    /// Returns whether this guard actually holds the lock.
    #[must_use]
    pub fn held(&self) -> bool {
        self.path.is_some()
    }

    /// Returns the lock file this guard holds, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Drop for AdvisoryLock {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
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
        assert_eq!(std::fs::read(&path).unwrap(), Vec::<u8>::new());
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
