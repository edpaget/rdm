//! Shared staged-write/delete overlay for [`Store`](super::Store) backends.
//!
//! Both the in-memory and filesystem stores buffer pending writes and deletes
//! in memory until commit, dispatching reads staged-first for
//! read-your-own-writes consistency. [`StagedOverlay`] owns that state machine
//! so each backend only supplies its committed-lookup behavior (via closures)
//! and its structurally-distinct `list`/`commit` logic.

use std::collections::BTreeMap;

use crate::error::{Error, Result};

/// A staged entry: either a pending write or a pending delete.
#[derive(Clone, Debug)]
pub enum StagedEntry {
    /// Content to be written on commit.
    Write(String),
    /// Marker for a pending deletion.
    Delete,
}

/// An overlay of staged (uncommitted) writes and deletes over a committed
/// backend.
///
/// The overlay holds the pending changes and resolves reads staged-first; the
/// committed backend is supplied per-call as a closure, so the overlay works
/// over either an in-memory map or the filesystem without a trait. Commit and
/// directory listing are backend-specific and live in the owning store —
/// [`StagedOverlay::iter`], [`StagedOverlay::get`], and
/// [`StagedOverlay::drain`] expose the staged map for those paths.
#[derive(Clone, Debug, Default)]
pub struct StagedOverlay {
    staged: BTreeMap<String, StagedEntry>,
}

impl StagedOverlay {
    /// Creates a new, empty overlay.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stages a write (create or overwrite) for `key`.
    pub fn write(&mut self, key: &str, content: String) {
        self.staged
            .insert(key.to_string(), StagedEntry::Write(content));
    }

    /// Stages a deletion for `key`.
    ///
    /// `committed_exists` is consulted only when `key` is not already staged;
    /// it reports whether the committed backend holds the file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] with [`std::io::ErrorKind::NotFound`] and a
    /// `file not found: {key}` message if the file exists neither in the
    /// staged overlay (as a write) nor in the committed backend.
    pub fn delete(&mut self, key: &str, committed_exists: impl FnOnce() -> bool) -> Result<()> {
        if !self.exists(key, committed_exists) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("file not found: {key}"),
            )));
        }
        self.staged.insert(key.to_string(), StagedEntry::Delete);
        Ok(())
    }

    /// Discards all staged changes.
    pub fn discard(&mut self) {
        self.staged.clear();
    }

    /// Reads `key`, staged-first.
    ///
    /// A staged write returns its buffered content; a staged delete reports the
    /// file as missing; otherwise the read falls through to `committed_read`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] with [`std::io::ErrorKind::NotFound`] and a
    /// `file not found: {key}` message if `key` is staged for deletion.
    /// Otherwise propagates any error from `committed_read`.
    pub fn read(
        &self,
        key: &str,
        committed_read: impl FnOnce() -> Result<String>,
    ) -> Result<String> {
        if let Some(entry) = self.staged.get(key) {
            return match entry {
                StagedEntry::Write(content) => Ok(content.clone()),
                StagedEntry::Delete => Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("file not found: {key}"),
                ))),
            };
        }
        committed_read()
    }

    /// Reports whether `key` exists, staged-first.
    ///
    /// A staged write is present; a staged delete is absent; otherwise the
    /// answer comes from `committed_exists`.
    pub fn exists(&self, key: &str, committed_exists: impl FnOnce() -> bool) -> bool {
        if let Some(entry) = self.staged.get(key) {
            return matches!(entry, StagedEntry::Write(_));
        }
        committed_exists()
    }

    /// Returns the staged entry for `key`, if any.
    ///
    /// Intended for backend `list` implementations that need to consult staged
    /// state while walking committed entries.
    pub fn get(&self, key: &str) -> Option<&StagedEntry> {
        self.staged.get(key)
    }

    /// Iterates over the staged entries in key order.
    ///
    /// Intended for backend `list` implementations.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &StagedEntry)> {
        self.staged.iter()
    }

    /// Removes and returns all staged entries, leaving the overlay empty.
    ///
    /// Intended for backend `commit` implementations, which apply the drained
    /// entries to their committed backend.
    pub fn drain(&mut self) -> BTreeMap<String, StagedEntry> {
        std::mem::take(&mut self.staged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_returns_staged_content() {
        let mut overlay = StagedOverlay::new();
        overlay.write("f.md", "new".to_string());
        let content = overlay
            .read("f.md", || panic!("committed_read should not be called"))
            .unwrap();
        assert_eq!(content, "new");
    }

    #[test]
    fn read_falls_through_to_committed_when_absent() {
        let overlay = StagedOverlay::new();
        let content = overlay
            .read("f.md", || Ok("committed".to_string()))
            .unwrap();
        assert_eq!(content, "committed");
    }

    #[test]
    fn read_staged_delete_returns_not_found() {
        let mut overlay = StagedOverlay::new();
        overlay.delete("f.md", || true).unwrap();
        let err = overlay
            .read("f.md", || panic!("committed_read should not be called"))
            .unwrap_err();
        match err {
            Error::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Io NotFound, got {other:?}"),
        }
    }

    #[test]
    fn exists_reflects_staged_write_and_delete() {
        let mut overlay = StagedOverlay::new();
        assert!(!overlay.exists("f.md", || false));
        overlay.write("f.md", "x".to_string());
        assert!(overlay.exists("f.md", || panic!("not consulted when staged")));
        overlay.delete("f.md", || true).unwrap();
        assert!(!overlay.exists("f.md", || panic!("not consulted when staged")));
    }

    #[test]
    fn exists_falls_through_to_committed_when_absent() {
        let overlay = StagedOverlay::new();
        assert!(overlay.exists("f.md", || true));
        assert!(!overlay.exists("f.md", || false));
    }

    #[test]
    fn delete_nonexistent_returns_error() {
        let mut overlay = StagedOverlay::new();
        let err = overlay.delete("nope.md", || false).unwrap_err();
        match err {
            Error::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Io NotFound, got {other:?}"),
        }
    }

    #[test]
    fn delete_committed_file_succeeds() {
        let mut overlay = StagedOverlay::new();
        overlay.delete("f.md", || true).unwrap();
        assert!(matches!(overlay.get("f.md"), Some(StagedEntry::Delete)));
    }

    #[test]
    fn discard_clears_staged_changes() {
        let mut overlay = StagedOverlay::new();
        overlay.write("a.md", "x".to_string());
        overlay.discard();
        assert!(overlay.get("a.md").is_none());
        // After discard, reads fall through to committed.
        let content = overlay
            .read("a.md", || Ok("committed".to_string()))
            .unwrap();
        assert_eq!(content, "committed");
    }

    #[test]
    fn drain_returns_entries_and_empties_overlay() {
        let mut overlay = StagedOverlay::new();
        overlay.write("a.md", "a".to_string());
        overlay.delete("b.md", || true).unwrap();
        let drained = overlay.drain();
        assert_eq!(drained.len(), 2);
        assert!(matches!(drained.get("a.md"), Some(StagedEntry::Write(_))));
        assert!(matches!(drained.get("b.md"), Some(StagedEntry::Delete)));
        // Overlay is now empty.
        assert_eq!(overlay.iter().count(), 0);
        assert!(overlay.drain().is_empty());
    }

    #[test]
    fn write_after_delete_resurrects_key() {
        // A write staged over a prior delete shadows it: reads return the new
        // content and the key is no longer reported missing.
        let mut overlay = StagedOverlay::new();
        overlay.delete("f.md", || true).unwrap();
        overlay.write("f.md", "resurrected".to_string());
        let content = overlay
            .read("f.md", || panic!("committed_read should not be called"))
            .unwrap();
        assert_eq!(content, "resurrected");
        assert!(overlay.exists("f.md", || panic!("not consulted when staged")));
    }

    #[test]
    fn iter_yields_staged_entries_in_key_order() {
        let mut overlay = StagedOverlay::new();
        overlay.write("b.md", "b".to_string());
        overlay.write("a.md", "a".to_string());
        let keys: Vec<&str> = overlay.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["a.md", "b.md"]);
    }
}
