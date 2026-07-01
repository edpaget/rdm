//! Plan repo root resolution: locating the plan repo directory (env-derived
//! override, global config, or XDG data dir) and expanding path shorthand
//! (`~`, `.`, `..`).
//!
//! These helpers are shared by every interface (CLI, TUI, server) so the plan
//! repo is located identically no matter how rdm is invoked. Binary crates
//! cannot import each other, so this logic lives in the core library.

use std::path::PathBuf;

use crate::config::GlobalConfig;
use crate::error::{Error, Result};

/// Returns the path to the global config file.
///
/// Resolution: `$XDG_CONFIG_HOME/rdm/config.toml` or `~/.config/rdm/config.toml`.
/// Returns `None` if neither `$XDG_CONFIG_HOME` nor `$HOME` is set.
pub fn global_config_path() -> Option<PathBuf> {
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
/// Returns `None` if neither `$XDG_DATA_HOME` nor `$HOME` is set.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        Some(PathBuf::from(xdg).join("rdm"))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".local").join("share").join("rdm"))
    }
}

/// Loads the global config from disk, returning [`GlobalConfig::default`] if
/// the file is missing, unreadable, or fails to parse.
pub fn load_global_config() -> GlobalConfig {
    let Some(path) = global_config_path() else {
        return GlobalConfig::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return GlobalConfig::default();
    };
    GlobalConfig::from_toml(&contents).unwrap_or_default()
}

/// Resolves the plan repo root using the priority chain:
///
/// 1. `explicit` — a caller-supplied override (e.g. an already-merged `--root`
///    flag or `RDM_ROOT` env var)
/// 2. `root` field in the global config
/// 3. XDG data dir (`$XDG_DATA_HOME/rdm` or `~/.local/share/rdm`)
///
/// The returned path is *not* expanded; pass it through [`expand_root`] to
/// resolve `~` and normalize `.`/`..` segments.
///
/// # Errors
///
/// Returns [`Error::RootNotDetermined`] if none of the priority sources yields
/// a root (e.g. `$HOME` is not set and no explicit or global root was
/// provided).
pub fn resolve_root(explicit: Option<PathBuf>, global: &GlobalConfig) -> Result<PathBuf> {
    if let Some(root) = explicit {
        return Ok(root);
    }
    if let Some(root) = &global.root {
        return Ok(root.clone());
    }
    if let Some(data_dir) = default_data_dir() {
        return Ok(data_dir);
    }
    Err(Error::RootNotDetermined)
}

/// Expands `~` and resolves `.`/`..` in a path.
///
/// # Errors
///
/// Returns [`Error::HomeNotSet`] if `~` is used but `$HOME` is not set.
/// Returns [`Error::PathResolutionFailed`] if the path cannot be made
/// absolute.
pub fn expand_root(path: PathBuf) -> Result<PathBuf> {
    let path = if let Ok(rest) = path.strip_prefix("~") {
        let home = std::env::var("HOME").map_err(|source| Error::HomeNotSet { source })?;
        PathBuf::from(home).join(rest)
    } else {
        path
    };
    let abs = std::path::absolute(&path).map_err(|source| Error::PathResolutionFailed {
        path: path.clone(),
        source,
    })?;
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
    /// `XDG_DATA_HOME`), which race under the threaded test runner.
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
    fn resolve_root_explicit_wins() {
        let global = GlobalConfig {
            root: Some(PathBuf::from("/global/root")),
            ..Default::default()
        };
        let result = resolve_root(Some(PathBuf::from("/explicit/root")), &global).unwrap();
        assert_eq!(result, PathBuf::from("/explicit/root"));
    }

    #[test]
    fn resolve_root_global_config_wins() {
        let global = GlobalConfig {
            root: Some(PathBuf::from("/global/root")),
            ..Default::default()
        };
        let result = resolve_root(None, &global).unwrap();
        assert_eq!(result, PathBuf::from("/global/root"));
    }

    #[test]
    fn resolve_root_xdg_fallback() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let global = GlobalConfig::default();
        // As long as HOME is set, we get the XDG data dir fallback.
        let result = resolve_root(None, &global).unwrap();
        assert!(result.to_string_lossy().ends_with("/rdm"));
    }

    #[test]
    fn resolve_root_error_when_nothing() {
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
        let result = resolve_root(None, &global);
        assert!(matches!(result, Err(Error::RootNotDetermined)));
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

    #[test]
    fn expand_root_tilde_without_home_preserves_source() {
        use std::error::Error as _;
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Unset HOME so the `~` expansion fails. The EnvGuard restores it on
        // drop, even if an assertion panics.
        let _restore = EnvGuard::capture(&["HOME"]);
        // SAFETY: ENV_LOCK is held above, so no other thread reads or writes
        // HOME while it is unset.
        unsafe {
            std::env::remove_var("HOME");
        }
        let err = expand_root(PathBuf::from("~/plans")).unwrap_err();
        assert!(matches!(err, Error::HomeNotSet { .. }));
        // The underlying VarError must be preserved as the error source so the
        // anyhow chain (`{:#}`) still prints the ": environment variable not
        // found" tail, matching pre-refactor behavior.
        assert!(err.source().is_some());
    }
}
