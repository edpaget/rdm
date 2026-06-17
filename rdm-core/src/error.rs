/// Errors that can occur in rdm-core operations.
#[derive(Debug)]
pub enum Error {
    /// An I/O error occurred.
    Io(std::io::Error),
    /// Failed to parse YAML frontmatter.
    FrontmatterParse(serde_yaml::Error),
    /// The document is missing a frontmatter block.
    FrontmatterMissing,
    /// Failed to parse the config file.
    ConfigParse(toml::de::Error),
    /// The config file was not found.
    ConfigNotFound,
    /// The plan repo is already initialized.
    AlreadyInitialized,
    /// The specified project was not found.
    ProjectNotFound(String),
    /// The specified roadmap was not found.
    RoadmapNotFound(String),
    /// The specified phase was not found.
    PhaseNotFound(String),
    /// The specified task was not found.
    TaskNotFound(String),
    /// A slug already exists.
    DuplicateSlug(String),
    /// Adding a dependency would create a cycle.
    CyclicDependency(String),
    /// No project was specified and no default project is configured.
    ProjectNotSpecified,
    /// Failed to serialize the config file.
    ConfigSerialize(toml::ser::Error),
    /// A relative path is invalid.
    InvalidPath(String),
    /// A specified phase stem is not part of the source roadmap.
    InvalidPhaseSelection(String),
    /// The roadmap has incomplete phases and cannot be archived without force.
    RoadmapHasIncompletePhases(String),
    /// A config value is not valid for the given key.
    InvalidConfigValue {
        /// The configuration key.
        key: String,
        /// The invalid value that was provided.
        value: String,
        /// A description of the valid values.
        valid: String,
    },
    /// A git operation failed.
    Git(String),
    /// The revision exists, but the requested path is not present in that
    /// revision (e.g. the file was added later or deleted at that point).
    BodyAtRevisionMissing {
        /// The relative path that was requested.
        path: String,
        /// The revision (commit SHA) that was searched.
        sha: String,
    },
    /// The requested revision does not exist in the backing store.
    RevisionUnknown {
        /// The revision (commit SHA) that could not be resolved.
        sha: String,
    },
    /// The storage backend has no notion of history (e.g. an unborn HEAD or a
    /// backend that opted out of revision-scoped reads).
    HistoryUnavailable,
    /// An update tried to replace a non-empty body with an empty string
    /// without an explicit opt-in. Callers must use [`BodyUpdate::Clear`] to
    /// confirm the clobber (the CLI exposes this as `--clear-body`, the
    /// HTTP/MCP surfaces as `clear_body: true`).
    ///
    /// [`BodyUpdate::Clear`]: crate::ops::BodyUpdate::Clear
    BodyClobberRefused,
    /// An update request set both a value and its `clear_*` flag for the same
    /// field — for example `--body` together with `--clear-body`. The two are
    /// contradictory; pass exactly one.
    ConflictingUpdate {
        /// The field name (`"body"`, `"priority"`, or `"tags"`).
        field: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::FrontmatterParse(e) => write!(f, "failed to parse frontmatter: {e}"),
            Error::FrontmatterMissing => {
                write!(f, "document is missing frontmatter delimiters (---)")
            }
            Error::ConfigParse(e) => write!(f, "failed to parse config: {e}"),
            Error::ConfigNotFound => write!(f, "rdm.toml not found — run `rdm init` first"),
            Error::AlreadyInitialized => {
                write!(f, "plan repo is already initialized (rdm.toml exists)")
            }
            Error::ProjectNotFound(name) => {
                write!(
                    f,
                    "project not found: {name} — create it with `rdm project create`"
                )
            }
            Error::RoadmapNotFound(name) => {
                write!(
                    f,
                    "roadmap not found: {name} — create it with `rdm roadmap create`"
                )
            }
            Error::PhaseNotFound(name) => {
                write!(f, "phase not found: {name}")
            }
            Error::TaskNotFound(name) => {
                write!(f, "task not found: {name}")
            }
            Error::DuplicateSlug(slug) => {
                write!(f, "'{slug}' already exists — choose a different name")
            }
            Error::CyclicDependency(msg) => {
                write!(f, "cyclic dependency: {msg}")
            }
            Error::ProjectNotSpecified => {
                write!(
                    f,
                    "no project specified — use --project or set default_project in rdm.toml"
                )
            }
            Error::ConfigSerialize(e) => write!(f, "failed to serialize config: {e}"),
            Error::InvalidPath(msg) => write!(f, "invalid path: {msg}"),
            Error::InvalidPhaseSelection(msg) => {
                write!(f, "invalid phase selection: {msg}")
            }
            Error::RoadmapHasIncompletePhases(slug) => {
                write!(
                    f,
                    "roadmap '{slug}' has incomplete phases — pass --force to archive anyway"
                )
            }
            Error::InvalidConfigValue { key, value, valid } => {
                write!(
                    f,
                    "invalid value '{value}' for '{key}' — valid values: {valid}"
                )
            }
            Error::Git(msg) => write!(f, "git error: {msg}"),
            Error::BodyAtRevisionMissing { path, sha } => {
                write!(f, "path '{path}' is not present at revision {sha}")
            }
            Error::RevisionUnknown { sha } => {
                write!(f, "revision '{sha}' is not known to the store")
            }
            Error::HistoryUnavailable => {
                write!(
                    f,
                    "the store has no history available (unborn HEAD or backend without revision support)"
                )
            }
            Error::BodyClobberRefused => {
                write!(
                    f,
                    "refusing to overwrite non-empty body with an empty value without explicit opt-in"
                )
            }
            Error::ConflictingUpdate { field } => {
                write!(f, "cannot set both '{field}' and 'clear_{field}'")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::FrontmatterParse(e) => Some(e),
            Error::ConfigParse(e) => Some(e),
            Error::ConfigSerialize(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_yaml::Error> for Error {
    fn from(e: serde_yaml::Error) -> Self {
        Error::FrontmatterParse(e)
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::ConfigParse(e)
    }
}

impl From<toml::ser::Error> for Error {
    fn from(e: toml::ser::Error) -> Self {
        Error::ConfigSerialize(e)
    }
}

/// A convenient `Result` type for rdm-core.
pub type Result<T> = std::result::Result<T, Error>;
