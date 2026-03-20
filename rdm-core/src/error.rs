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
    /// The specified git remote was not found.
    RemoteNotFound(String),
    /// A git remote with the given name already exists.
    DuplicateRemote(String),
    /// A git push was rejected (non-fast-forward).
    PushRejected(String),
    /// Local and remote branches have diverged.
    BranchesDiverged(String),
    /// A merge conflict occurred during pull.
    MergeConflict(String),
    /// No merge is in progress.
    NoMergeInProgress,
    /// A file is not in the unmerged list.
    NotConflicted(String),
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
            Error::RemoteNotFound(name) => {
                write!(
                    f,
                    "remote not found: {name} — use `rdm remote add` to create one"
                )
            }
            Error::DuplicateRemote(name) => {
                write!(f, "remote '{name}' already exists")
            }
            Error::PushRejected(msg) => {
                write!(
                    f,
                    "push rejected: {msg} — pull first with `rdm remote pull`, then push again"
                )
            }
            Error::BranchesDiverged(msg) => {
                write!(
                    f,
                    "branches have diverged: {msg} — resolve manually with `git rebase` or `git merge`"
                )
            }
            Error::MergeConflict(msg) => {
                write!(
                    f,
                    "merge conflict: {msg} — run `rdm conflicts` to see details, then `rdm resolve <file>`"
                )
            }
            Error::NoMergeInProgress => {
                write!(f, "no merge in progress — nothing to resolve")
            }
            Error::NotConflicted(path) => {
                write!(f, "file '{path}' is not in the unmerged list")
            }
            Error::InvalidConfigValue { key, value, valid } => {
                write!(
                    f,
                    "invalid value '{value}' for '{key}' — valid values: {valid}"
                )
            }
            Error::Git(msg) => write!(f, "git error: {msg}"),
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
