//! Storage abstraction layer for plan repo data.
//!
//! This module provides a [`Store`] trait that decouples rdm-core from the
//! filesystem, enabling in-memory backends for testing and future git backends.

mod memory;

pub use memory::MemoryStore;

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
}
