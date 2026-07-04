//! Storage abstraction layer for plan repo data.
//!
//! This module provides a [`Store`] trait that decouples rdm-core from the
//! filesystem, enabling in-memory backends for testing and future git backends.

mod memory;
mod overlay;

pub use memory::MemoryStore;
pub use overlay::{StagedEntry, StagedOverlay};

use crate::error::{Error, Result};

/// A validated relative path within a store.
///
/// `RelPath` guarantees the path contains no leading `/`, no `..` components,
/// no `\` characters, no `.` components, and is not empty (except for the
/// special root sentinel). Double slashes and trailing slashes are normalized.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(String);

impl RelPath {
    /// Creates a new `RelPath` from the given string, validating and normalizing it.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidPath` if the path is empty, starts with `/`,
    /// contains `\` or `..` or `.` components.
    pub fn new(path: &str) -> Result<Self> {
        if path.is_empty() {
            return Err(Error::InvalidPath("path must not be empty".to_string()));
        }
        if path.starts_with('/') {
            return Err(Error::InvalidPath(
                "path must not start with '/'".to_string(),
            ));
        }
        if path.contains('\\') {
            return Err(Error::InvalidPath("path must not contain '\\'".to_string()));
        }

        // Normalize: collapse double slashes, strip trailing slash
        let normalized: String = path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");

        if normalized.is_empty() {
            return Err(Error::InvalidPath("path must not be empty".to_string()));
        }

        // Validate components
        for component in normalized.split('/') {
            if component == ".." {
                return Err(Error::InvalidPath(
                    "path must not contain '..' components".to_string(),
                ));
            }
            if component == "." {
                return Err(Error::InvalidPath(
                    "path must not contain '.' components".to_string(),
                ));
            }
        }

        Ok(Self(normalized))
    }

    /// Returns the root sentinel, used for listing the top-level directory.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Joins this path with a child segment.
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidPath` if the resulting path is invalid.
    pub fn join(&self, child: &str) -> Result<Self> {
        if self.0.is_empty() {
            Self::new(child)
        } else {
            Self::new(&format!("{}/{child}", self.0))
        }
    }

    /// Returns the parent directory, or `None` if this is a single-component path or root.
    pub fn parent(&self) -> Option<Self> {
        if self.0.is_empty() {
            return None;
        }
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Some(Self(parent.to_string())),
            None => Some(Self::root()),
        }
    }

    /// Returns the final component of the path, or `None` for root.
    pub fn file_name(&self) -> Option<&str> {
        if self.0.is_empty() {
            return None;
        }
        Some(self.0.rsplit('/').next().unwrap_or(&self.0))
    }

    /// Returns the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RelPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The kind of a directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirEntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
}

/// A single entry in a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The name of this entry (final path component, not a full path).
    pub name: String,
    /// Whether this entry is a file or directory.
    pub kind: DirEntryKind,
}

/// A staged change to be committed atomically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Write (create or overwrite) a file.
    Write {
        /// The path to write.
        path: RelPath,
        /// The file content.
        content: String,
    },
    /// Delete a file.
    Delete {
        /// The path to delete.
        path: RelPath,
    },
}

/// An abstract storage backend for plan repo data.
///
/// Implementations provide staged writes with atomic commit semantics.
/// Reads see staged (uncommitted) writes for read-your-own-writes consistency.
pub trait Store {
    /// Reads the content of a file.
    ///
    /// Returns staged content if present, otherwise committed content.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not exist.
    fn read(&self, path: &RelPath) -> Result<String>;

    /// Checks whether a file exists (staged or committed).
    fn exists(&self, path: &RelPath) -> bool;

    /// Lists the entries in a directory, sorted by name.
    ///
    /// Returns files and subdirectories that are direct children of the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not represent a directory.
    fn list(&self, path: &RelPath) -> Result<Vec<DirEntry>>;

    /// Stages a write (create or overwrite) for the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path is invalid.
    fn write(&mut self, path: &RelPath, content: String) -> Result<()>;

    /// Stages a deletion for the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not exist.
    fn delete(&mut self, path: &RelPath) -> Result<()>;

    /// Commits all staged changes atomically, merging them into the committed state.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit fails.
    fn commit(&mut self) -> Result<()>;

    /// Discards all staged changes without committing.
    fn discard(&mut self);

    /// Returns the last-modified time of a committed file, if the backend
    /// tracks one.
    ///
    /// The default implementation returns `Ok(None)`; history-less or
    /// in-memory backends keep it. Filesystem-backed stores override it to
    /// return the real on-disk mtime, and `Ok(None)` when the file is absent.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend tracks mtimes but the lookup fails for
    /// a reason other than the file being absent.
    fn modified(&self, _path: &RelPath) -> Result<Option<std::time::SystemTime>> {
        Ok(None)
    }
}

/// A [`Store`] that also exposes read access to committed history.
///
/// Version-control-backed stores (such as the git store) and the in-memory
/// test store implement this in addition to [`Store`]; a minimal,
/// history-less backend can implement just [`Store`]. Functions that read a
/// document as of a specific revision — [`crate::io::load_roadmap_at`],
/// [`crate::io::load_phase_at`], [`crate::io::load_task_at`] — are bounded by
/// `VersionedStore` rather than the base [`Store`].
pub trait VersionedStore: Store {
    /// Returns an opaque identifier for the current committed state.
    ///
    /// For git-backed stores this is the HEAD commit SHA. For the in-memory
    /// store this is a synthetic `mem-N` token that advances on each
    /// [`Store::commit`] call (see [`MemoryStore`] for details).
    ///
    /// The returned value is opaque — callers must treat it as a string,
    /// not assume any specific format.
    ///
    /// # Errors
    ///
    /// Returns [`Error::HistoryUnavailable`] when the backend has no
    /// committed state to identify (e.g. an unborn HEAD in a fresh git
    /// repository), or when the backend has opted out of revision tracking.
    fn head_sha(&self) -> Result<String>;

    /// Reads the content of a file as it existed at a specific revision.
    ///
    /// Unlike [`Store::read`], this method ignores staged changes and reads
    /// from history.
    ///
    /// # Errors
    ///
    /// - [`Error::RevisionUnknown`] if `sha` does not name a known revision.
    /// - [`Error::BodyAtRevisionMissing`] if `sha` exists but `path` is not
    ///   present at that revision (added later, deleted at that point, etc).
    /// - [`Error::HistoryUnavailable`] if the backend has no notion of
    ///   history.
    fn fetch_body_at(&self, path: &RelPath, sha: &str) -> Result<String>;
}

impl Store for Box<dyn VersionedStore + Send + Sync> {
    fn read(&self, path: &RelPath) -> Result<String> {
        (**self).read(path)
    }

    fn exists(&self, path: &RelPath) -> bool {
        (**self).exists(path)
    }

    fn list(&self, path: &RelPath) -> Result<Vec<DirEntry>> {
        (**self).list(path)
    }

    fn write(&mut self, path: &RelPath, content: String) -> Result<()> {
        (**self).write(path, content)
    }

    fn delete(&mut self, path: &RelPath) -> Result<()> {
        (**self).delete(path)
    }

    fn commit(&mut self) -> Result<()> {
        (**self).commit()
    }

    fn commit_with_message(&mut self, message: &str) -> Result<()> {
        (**self).commit_with_message(message)
    }

    fn discard(&mut self) {
        (**self).discard();
    }

    fn modified(&self, path: &RelPath) -> Result<Option<std::time::SystemTime>> {
        (**self).modified(path)
    }
}

impl VersionedStore for Box<dyn VersionedStore + Send + Sync> {
    fn head_sha(&self) -> Result<String> {
        (**self).head_sha()
    }

    fn fetch_body_at(&self, path: &RelPath, sha: &str) -> Result<String> {
        (**self).fetch_body_at(path, sha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relpath_valid_simple() {
        let p = RelPath::new("foo/bar.md").unwrap();
        assert_eq!(p.as_str(), "foo/bar.md");
    }

    #[test]
    fn relpath_valid_single_component() {
        let p = RelPath::new("README.md").unwrap();
        assert_eq!(p.as_str(), "README.md");
    }

    #[test]
    fn relpath_normalizes_trailing_slash() {
        let p = RelPath::new("foo/bar/").unwrap();
        assert_eq!(p.as_str(), "foo/bar");
    }

    #[test]
    fn relpath_normalizes_double_slashes() {
        let p = RelPath::new("foo//bar//baz").unwrap();
        assert_eq!(p.as_str(), "foo/bar/baz");
    }

    #[test]
    fn relpath_rejects_empty() {
        assert!(RelPath::new("").is_err());
    }

    #[test]
    fn relpath_rejects_leading_slash() {
        assert!(RelPath::new("/foo").is_err());
    }

    #[test]
    fn relpath_rejects_dotdot() {
        assert!(RelPath::new("foo/../bar").is_err());
    }

    #[test]
    fn relpath_rejects_dot_component() {
        assert!(RelPath::new("foo/./bar").is_err());
    }

    #[test]
    fn relpath_rejects_backslash() {
        assert!(RelPath::new("foo\\bar").is_err());
    }

    #[test]
    fn relpath_join() {
        let p = RelPath::new("foo").unwrap();
        let joined = p.join("bar/baz.md").unwrap();
        assert_eq!(joined.as_str(), "foo/bar/baz.md");
    }

    #[test]
    fn relpath_join_from_root() {
        let p = RelPath::root();
        let joined = p.join("foo.md").unwrap();
        assert_eq!(joined.as_str(), "foo.md");
    }

    #[test]
    fn relpath_parent() {
        let p = RelPath::new("foo/bar/baz.md").unwrap();
        let parent = p.parent().unwrap();
        assert_eq!(parent.as_str(), "foo/bar");

        let grandparent = parent.parent().unwrap();
        assert_eq!(grandparent.as_str(), "foo");

        let root = grandparent.parent().unwrap();
        assert_eq!(root.as_str(), "");

        assert!(root.parent().is_none());
    }

    #[test]
    fn relpath_file_name() {
        assert_eq!(
            RelPath::new("foo/bar.md").unwrap().file_name(),
            Some("bar.md")
        );
        assert_eq!(RelPath::new("bar.md").unwrap().file_name(), Some("bar.md"));
        assert_eq!(RelPath::root().file_name(), None);
    }

    #[test]
    fn boxed_versioned_store_delegates_read_write_commit() {
        let inner = MemoryStore::new();
        let mut boxed: Box<dyn VersionedStore + Send + Sync> = Box::new(inner);

        let path = RelPath::new("foo.md").unwrap();
        Store::write(&mut boxed, &path, "hello".to_string()).unwrap();
        Store::commit(&mut boxed).unwrap();

        assert_eq!(Store::read(&boxed, &path).unwrap(), "hello");
        assert!(Store::exists(&boxed, &path));
    }

    #[test]
    fn boxed_versioned_store_delegates_commit_with_message() {
        use std::sync::{Arc, Mutex};

        /// Wraps [`MemoryStore`] and records the message passed to
        /// `commit_with_message` into a shared cell. This distinguishes the
        /// boxed delegation override from the trait's default body: the
        /// default discards the message and calls `commit()`, so if the
        /// `impl Store for Box<dyn VersionedStore + Send + Sync>` override
        /// were removed, nothing would be recorded and this test would fail.
        struct MessageRecordingStore {
            inner: MemoryStore,
            recorded: Arc<Mutex<Option<String>>>,
        }

        impl Store for MessageRecordingStore {
            fn read(&self, path: &RelPath) -> Result<String> {
                self.inner.read(path)
            }
            fn exists(&self, path: &RelPath) -> bool {
                self.inner.exists(path)
            }
            fn list(&self, path: &RelPath) -> Result<Vec<DirEntry>> {
                self.inner.list(path)
            }
            fn write(&mut self, path: &RelPath, content: String) -> Result<()> {
                self.inner.write(path, content)
            }
            fn delete(&mut self, path: &RelPath) -> Result<()> {
                self.inner.delete(path)
            }
            fn commit(&mut self) -> Result<()> {
                self.inner.commit()
            }
            fn commit_with_message(&mut self, message: &str) -> Result<()> {
                *self.recorded.lock().unwrap() = Some(message.to_string());
                self.inner.commit()
            }
            fn discard(&mut self) {
                self.inner.discard();
            }
        }

        impl VersionedStore for MessageRecordingStore {
            fn head_sha(&self) -> Result<String> {
                self.inner.head_sha()
            }
            fn fetch_body_at(&self, path: &RelPath, sha: &str) -> Result<String> {
                self.inner.fetch_body_at(path, sha)
            }
        }

        let recorded: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let mut boxed: Box<dyn VersionedStore + Send + Sync> = Box::new(MessageRecordingStore {
            inner: MemoryStore::new(),
            recorded: Arc::clone(&recorded),
        });

        let path = RelPath::new("foo.md").unwrap();
        Store::write(&mut boxed, &path, "hello".to_string()).unwrap();
        Store::commit_with_message(&mut boxed, "custom message").unwrap();

        assert_eq!(
            recorded.lock().unwrap().as_deref(),
            Some("custom message"),
            "boxed delegation must forward the commit message to the inner store"
        );
        assert_eq!(Store::read(&boxed, &path).unwrap(), "hello");
    }

    #[test]
    fn boxed_versioned_store_delegates_head_sha_and_fetch_body_at() {
        let mut plain = MemoryStore::new();
        let path = RelPath::new("foo.md").unwrap();
        plain.write(&path, "hello".to_string()).unwrap();
        plain.commit().unwrap();
        let plain_sha = plain.head_sha().unwrap();
        let plain_body = plain.fetch_body_at(&path, &plain_sha).unwrap();

        let boxed: Box<dyn VersionedStore + Send + Sync> = Box::new(plain);
        assert_eq!(VersionedStore::head_sha(&boxed).unwrap(), plain_sha);
        assert_eq!(
            VersionedStore::fetch_body_at(&boxed, &path, &plain_sha).unwrap(),
            plain_body
        );
    }
}
