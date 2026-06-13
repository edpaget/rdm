//! Plan repo root resolution.
//!
//! Replicates the root-resolution chain used by `rdm-cli` so the TUI locates
//! the same plan repo the CLI would. `rdm-cli` is a binary crate and cannot be
//! imported, so this is a deliberate (third) copy of `expand_root` and the
//! resolution logic; hoisting it into `rdm-core` is tracked as a follow-up.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rdm_core::config::GlobalConfig;

/// Returns the path to the global config file.
///
/// Resolution: `$XDG_CONFIG_HOME/rdm/config.toml` or `~/.config/rdm/config.toml`.
/// Returns `None` if `$HOME` is not set.
pub(crate) fn global_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        Some(PathBuf::from(xdg).join("rdm").join("config.toml"))
    } else {
        std::env::var("HOME").ok().map(|home| {
            PathBuf::from(home)
                .join(".config")
                .join("rdm")
                .join("config.toml")
        })
    }
}

/// Returns the default data directory for plan repos.
///
/// Resolution: `$XDG_DATA_HOME/rdm` or `~/.local/share/rdm`.
/// Returns `None` if `$HOME` is not set.
pub(crate) fn default_data_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        Some(PathBuf::from(xdg).join("rdm"))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".local").join("share").join("rdm"))
    }
}

/// Loads the global config from disk, returning `Default` if the file is missing.
pub(crate) fn load_global_config() -> GlobalConfig {
    let Some(path) = global_config_path() else {
        return GlobalConfig::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return GlobalConfig::default();
    };
    GlobalConfig::from_toml(&contents).unwrap_or_default()
}

/// Resolves the plan repo root using the CLI priority chain, then expands it.
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
    let root = resolve_root_inner(env_root, &global)?;
    expand_root(root)
}

/// Pure core of [`resolve_root`], exposed for testing.
///
/// # Errors
///
/// Returns an error if none of the priority sources yields a root.
fn resolve_root_inner(env_root: Option<PathBuf>, global: &GlobalConfig) -> Result<PathBuf> {
    if let Some(root) = env_root {
        return Ok(root);
    }
    if let Some(root) = &global.root {
        return Ok(root.clone());
    }
    if let Some(data_dir) = default_data_dir() {
        return Ok(data_dir);
    }
    bail!(
        "cannot determine plan repo location — set RDM_ROOT, \
         or add root to ~/.config/rdm/config.toml"
    )
}

/// Expands `~` and resolves `.`/`..` in a path.
///
/// # Errors
///
/// Returns an error if `~` is used but `$HOME` is not set, or if the path
/// cannot be made absolute.
pub(crate) fn expand_root(path: PathBuf) -> Result<PathBuf> {
    let path = if let Ok(rest) = path.strip_prefix("~") {
        let home = std::env::var("HOME").context("~ used in path but $HOME is not set")?;
        PathBuf::from(home).join(rest)
    } else {
        path
    };
    let abs = std::path::absolute(&path)
        .with_context(|| format!("failed to resolve path: {}", path.display()))?;
    let mut normalized = PathBuf::new();
    for component in abs.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            c => normalized.push(c),
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that read or mutate process-global env vars (`HOME`,
    /// `XDG_DATA_HOME`), which race under the threaded `cargo test` runner.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Restores the captured env vars when dropped, so a panic mid-test cannot
    /// leak removed vars to the rest of the process.
    struct EnvGuard(Vec<(&'static str, Option<String>)>);

    impl EnvGuard {
        /// Captures the current values of `keys` for later restoration.
        fn capture(keys: &[&'static str]) -> Self {
            Self(keys.iter().map(|k| (*k, std::env::var(k).ok())).collect())
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                // SAFETY: the test owning this guard holds ENV_LOCK for the
                // guard's whole lifetime, so no other thread reads or writes
                // these vars while we restore them.
                unsafe {
                    match v {
                        Some(val) => std::env::set_var(k, val),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }
    }

    #[test]
    fn resolve_root_inner_env_wins() {
        let global = GlobalConfig {
            root: Some(PathBuf::from("/global/root")),
            ..Default::default()
        };
        let result = resolve_root_inner(Some(PathBuf::from("/env/root")), &global).unwrap();
        assert_eq!(result, PathBuf::from("/env/root"));
    }

    #[test]
    fn resolve_root_inner_global_config_wins() {
        let global = GlobalConfig {
            root: Some(PathBuf::from("/global/root")),
            ..Default::default()
        };
        let result = resolve_root_inner(None, &global).unwrap();
        assert_eq!(result, PathBuf::from("/global/root"));
    }

    #[test]
    fn resolve_root_inner_xdg_fallback() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let global = GlobalConfig::default();
        // As long as HOME is set, we get the XDG data dir fallback.
        let result = resolve_root_inner(None, &global).unwrap();
        assert!(result.to_string_lossy().ends_with("/rdm"));
    }

    #[test]
    fn resolve_root_inner_error_when_nothing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Simulate no HOME and no XDG so default_data_dir() returns None. The
        // EnvGuard restores both vars on drop, even if the assertion panics.
        let global = GlobalConfig::default();
        let _restore = EnvGuard::capture(&["HOME", "XDG_DATA_HOME"]);
        // SAFETY: ENV_LOCK is held above, so no other thread reads or writes
        // these vars while they are unset.
        unsafe {
            std::env::remove_var("HOME");
            std::env::remove_var("XDG_DATA_HOME");
        }
        let result = resolve_root_inner(None, &global);
        assert!(result.is_err());
    }

    #[test]
    fn expand_root_tilde_expands_to_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::var("HOME").unwrap();
        let result = expand_root(PathBuf::from("~")).unwrap();
        assert_eq!(result, PathBuf::from(&home));
    }

    #[test]
    fn expand_root_absolute_path_unchanged() {
        let result = expand_root(PathBuf::from("/tmp/plans")).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/plans"));
    }
}
