//! Plan repo initialization.

use crate::config::Config;
use crate::error::{Error, Result};
use crate::store::Store;

/// Initializes a new plan repo with the default config.
///
/// Creates `rdm.toml` (with default values).
/// Equivalent to `init_with_config(store, Config::default())`.
///
/// # Errors
///
/// Returns [`Error::AlreadyInitialized`] if `rdm.toml` already exists, or
/// [`Error::Io`] if file creation fails.
pub fn init(store: &mut impl Store) -> Result<()> {
    init_with_config(store, Config::default())
}

/// Initializes a new plan repo with the given config.
///
/// Creates `rdm.toml` (populated from `config`). The config is validated
/// before any files are written.
///
/// No `INDEX.md` is created: the generated indexes are not maintained by any
/// mutation, so seeding a fresh repo with one would plant a file that is
/// stale from the first write onward — and a changeset mixing that derived
/// path with entity writes would commit a reconciled index the working tree
/// never receives. Run [`crate::ops::index::generate_index`] (the `rdm index`
/// command) to produce one explicitly.
///
/// # Errors
///
/// Returns [`Error::AlreadyInitialized`] if `rdm.toml` already exists,
/// [`Error::InvalidConfigValue`] if the config fails validation, or
/// [`Error::Io`] if file creation fails.
pub fn init_with_config(store: &mut impl Store, config: Config) -> Result<()> {
    config.validate()?;

    if store.exists(&crate::paths::config_path()) {
        return Err(Error::AlreadyInitialized);
    }

    let toml_str = config.to_toml()?;
    let config_path = crate::paths::config_path();
    store.write(&config_path, toml_str)?;

    store.commit()?;
    Ok(())
}
