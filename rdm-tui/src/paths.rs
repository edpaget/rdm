//! Plan repo root resolution.
//!
//! Thin wrapper over [`rdm_core::root`]. The TUI has no `--root` CLI flag, so
//! it reads `RDM_ROOT` itself before delegating the priority chain and path
//! expansion to core, ensuring it locates the same plan repo the CLI would.

use std::path::PathBuf;

use anyhow::Result;
use rdm_core::config::GlobalConfig;

/// Loads the global config from disk, returning `Default` if the file is missing.
///
/// Delegates to [`rdm_core::root::load_global_config`].
pub(crate) fn load_global_config() -> GlobalConfig {
    rdm_core::root::load_global_config()
}

/// Resolves the plan repo root using the priority chain, then expands it.
///
/// 1. `RDM_ROOT` env var
/// 2. `root` field in global config
/// 3. XDG data dir (`$XDG_DATA_HOME/rdm` or `~/.local/share/rdm`)
///
/// # Errors
///
/// Returns an error if no root can be determined (e.g. `$HOME` is not set and
/// no explicit root was provided), or if the resolved path cannot be expanded.
pub fn resolve_root() -> Result<PathBuf> {
    let global = load_global_config();
    let env_root = std::env::var("RDM_ROOT").ok().map(PathBuf::from);
    let root = rdm_core::root::resolve_root(env_root, &global)?;
    Ok(rdm_core::root::expand_root(root)?)
}
