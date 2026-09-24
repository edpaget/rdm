#![warn(missing_docs)]
//! The filesystem side of [`rdm_core::transcript`]: a [`TranscriptSource`]
//! over a Claude Code projects directory on disk.
//!
//! All knowledge of the session layout stays in `rdm-core`. This crate only
//! lists directories and reads files, and resolves where the projects
//! directory is ([`projects_root`]).

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rdm_core::transcript::{
    EntryKind, TranscriptEntry, TranscriptError, TranscriptPath, TranscriptSource,
};

/// The projects directory could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectsRootError {
    /// Neither `CLAUDE_CONFIG_DIR` nor `HOME` is set to a non-empty value.
    NoConfigDir,
}

impl fmt::Display for ProjectsRootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectsRootError::NoConfigDir => f.write_str(
                "cannot find Claude Code's data directory: neither CLAUDE_CONFIG_DIR nor HOME is set; set CLAUDE_CONFIG_DIR to the directory that holds `projects/` (usually ~/.claude)",
            ),
        }
    }
}

impl std::error::Error for ProjectsRootError {}

/// The Claude Code projects directory for the given environment values:
/// `<CLAUDE_CONFIG_DIR>/projects` when `claude_config_dir` is non-empty,
/// otherwise `<HOME>/.claude/projects`.
///
/// # Errors
///
/// Returns [`ProjectsRootError::NoConfigDir`] when both values are missing or
/// empty.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// use rdm_transcript::projects_root;
///
/// assert_eq!(
///     projects_root(Some("/cfg".into()), Some("/home/me".into())),
///     Ok(PathBuf::from("/cfg/projects")),
/// );
/// assert_eq!(
///     projects_root(None, Some("/home/me".into())),
///     Ok(PathBuf::from("/home/me/.claude/projects")),
/// );
/// assert!(projects_root(None, None).is_err());
/// ```
pub fn projects_root(
    claude_config_dir: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, ProjectsRootError> {
    let non_empty = |v: Option<OsString>| v.filter(|v| !v.is_empty());
    if let Some(dir) = non_empty(claude_config_dir) {
        return Ok(PathBuf::from(dir).join("projects"));
    }
    if let Some(home) = non_empty(home) {
        return Ok(PathBuf::from(home).join(".claude").join("projects"));
    }
    Err(ProjectsRootError::NoConfigDir)
}

/// A [`TranscriptSource`] reading a projects directory on disk.
///
/// # Examples
///
/// ```
/// use rdm_core::transcript::{TranscriptPath, TranscriptSource};
/// use rdm_transcript::FsTranscriptSource;
///
/// let dir = tempfile::tempdir()?;
/// std::fs::create_dir_all(dir.path().join("-proj-b"))?;
/// std::fs::create_dir_all(dir.path().join("-proj-a/s1/subagents"))?;
/// let src = FsTranscriptSource::new(dir.path().to_path_buf());
/// let slugs: Vec<String> = src.list(&TranscriptPath::root())?.into_iter().map(|e| e.name).collect();
/// assert_eq!(slugs, ["-proj-a", "-proj-b"], "sorted by name");
/// // A missing directory lists as empty; a missing file is an error.
/// assert!(src.list(&TranscriptPath::parse("-proj-a/s1/workflows")?)?.is_empty());
/// assert!(src.read(&TranscriptPath::parse("-proj-a/s1.jsonl")?).is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsTranscriptSource {
    root: PathBuf,
}

impl FsTranscriptSource {
    /// A source rooted at `projects_root` (the `projects/` directory itself).
    #[must_use]
    pub fn new(projects_root: PathBuf) -> Self {
        Self {
            root: projects_root,
        }
    }

    /// A source rooted at the projects directory the process environment
    /// names; see [`projects_root`].
    ///
    /// # Errors
    ///
    /// Returns [`ProjectsRootError::NoConfigDir`] when neither
    /// `CLAUDE_CONFIG_DIR` nor `HOME` is set.
    pub fn from_env() -> Result<Self, ProjectsRootError> {
        projects_root(
            std::env::var_os("CLAUDE_CONFIG_DIR"),
            std::env::var_os("HOME"),
        )
        .map(Self::new)
    }

    /// The projects directory this source reads.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_of(&self, rel: &TranscriptPath) -> PathBuf {
        let mut path = self.root.clone();
        path.extend(rel.segments());
        path
    }
}

fn io_error(path: &Path, err: &io::Error) -> TranscriptError {
    TranscriptError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    }
}

impl TranscriptSource for FsTranscriptSource {
    fn root_display(&self) -> String {
        self.root.display().to_string()
    }

    fn list(&self, dir: &TranscriptPath) -> Result<Vec<TranscriptEntry>, TranscriptError> {
        let path = self.path_of(dir);
        let read = match fs::read_dir(&path) {
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(&path, &e)),
        };
        let mut entries = Vec::new();
        for entry in read {
            let entry = entry.map_err(|e| io_error(&path, &e))?;
            // A name that is not UTF-8 cannot be a session, run or agent id.
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            // Follow symlinks; a dangling one is skipped.
            let Ok(meta) = fs::metadata(entry.path()) else {
                continue;
            };
            let kind = if meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            entries.push(TranscriptEntry { name, kind });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn read(&self, file: &TranscriptPath) -> Result<String, TranscriptError> {
        let path = self.path_of(file);
        let bytes = fs::read(&path).map_err(|e| io_error(&path, &e))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn claude_config_dir_takes_precedence_over_home() {
        assert_eq!(
            projects_root(Some("/cfg".into()), Some("/home/u".into())),
            Ok(PathBuf::from("/cfg/projects"))
        );
    }

    #[test]
    fn home_is_the_fallback_and_empty_values_are_unset() {
        assert_eq!(
            projects_root(Some(OsString::new()), Some("/home/u".into())),
            Ok(PathBuf::from("/home/u/.claude/projects"))
        );
        assert_eq!(
            projects_root(None, Some("/home/u".into())),
            Ok(PathBuf::from("/home/u/.claude/projects"))
        );
    }

    #[test]
    fn neither_set_is_an_actionable_error() {
        let err = projects_root(None, Some(OsString::new())).err();
        assert_eq!(err, Some(ProjectsRootError::NoConfigDir));
        let msg = err.map(|e| e.to_string()).unwrap_or_default();
        assert!(msg.contains("CLAUDE_CONFIG_DIR"), "{msg}");
    }

    fn tree() -> TempDir {
        let dir = TempDir::new().unwrap_or_else(|e| panic!("tempdir: {e}"));
        for sub in ["-b", "-a/s1/subagents"] {
            fs::create_dir_all(dir.path().join(sub)).unwrap_or_else(|e| panic!("{e}"));
        }
        for file in ["-a/z.jsonl", "-a/m.jsonl"] {
            fs::write(dir.path().join(file), "").unwrap_or_else(|e| panic!("{e}"));
        }
        dir
    }

    #[test]
    fn listing_is_sorted_by_name_with_kinds() {
        let dir = tree();
        let src = FsTranscriptSource::new(dir.path().to_path_buf());
        let root = src.list(&TranscriptPath::root()).unwrap_or_default();
        let names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["-a", "-b"]);
        let slug = src
            .list(&TranscriptPath::parse("-a").unwrap_or_default())
            .unwrap_or_default();
        assert_eq!(
            slug,
            [
                TranscriptEntry {
                    name: "m.jsonl".into(),
                    kind: EntryKind::File
                },
                TranscriptEntry {
                    name: "s1".into(),
                    kind: EntryKind::Dir
                },
                TranscriptEntry {
                    name: "z.jsonl".into(),
                    kind: EntryKind::File
                },
            ]
        );
    }

    #[test]
    fn a_missing_directory_lists_empty() {
        let dir = tree();
        let src = FsTranscriptSource::new(dir.path().join("does-not-exist"));
        assert_eq!(src.list(&TranscriptPath::root()), Ok(Vec::new()));
        let src = FsTranscriptSource::new(dir.path().to_path_buf());
        assert_eq!(
            src.list(&TranscriptPath::parse("-a/s1/workflows").unwrap_or_default()),
            Ok(Vec::new())
        );
    }

    #[test]
    fn non_utf8_bytes_read_lossily_and_missing_files_are_io_errors() {
        let dir = tree();
        fs::write(dir.path().join("-b/x.jsonl"), b"ok \xff end").unwrap_or_else(|e| panic!("{e}"));
        let src = FsTranscriptSource::new(dir.path().to_path_buf());
        assert_eq!(
            src.read(&TranscriptPath::parse("-b/x.jsonl").unwrap_or_default()),
            Ok("ok \u{fffd} end".to_owned())
        );
        let missing = src.read(&TranscriptPath::parse("-b/none.jsonl").unwrap_or_default());
        match missing {
            Err(TranscriptError::Io { path, .. }) => {
                assert!(path.ends_with("none.jsonl"), "{path}")
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }
}
