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
            Error::ProjectNotFound(name) => write!(f, "project not found: {name}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::FrontmatterParse(e) => Some(e),
            Error::ConfigParse(e) => Some(e),
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

/// A convenient `Result` type for rdm-core.
pub type Result<T> = std::result::Result<T, Error>;
