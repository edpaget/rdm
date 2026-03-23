use crate::config::Config;
use crate::error::Result;
use crate::store::Store;

use super::PlanRepo;

impl<S: Store> PlanRepo<S> {
    // -- Init (delegates to crate::ops::init) --

    /// Initializes a new plan repo with the given store and default config.
    ///
    /// Creates `rdm.toml` (with default values) and `INDEX.md`.
    /// Equivalent to `init_with_config(store, Config::default())`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AlreadyInitialized`] if `rdm.toml` already exists, or
    /// [`Error::Io`] if file creation fails.
    pub fn init(mut store: S) -> Result<Self> {
        crate::ops::init::init(&mut store)?;
        Ok(PlanRepo { store })
    }

    /// Initializes a new plan repo with the given store and config.
    ///
    /// Creates `rdm.toml` (populated from `config`) and `INDEX.md`.
    /// The config is validated before any files are written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AlreadyInitialized`] if `rdm.toml` already exists,
    /// [`Error::InvalidConfigValue`] if the config fails validation, or
    /// [`Error::Io`] if file creation fails.
    pub fn init_with_config(mut store: S, config: Config) -> Result<Self> {
        crate::ops::init::init_with_config(&mut store, config)?;
        Ok(PlanRepo { store })
    }
}
