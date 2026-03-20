use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rdm_core::config::GlobalConfig;

/// Returns the path to the global config file.
///
/// Resolution: `$XDG_CONFIG_HOME/rdm/config.toml` or `~/.config/rdm/config.toml`.
/// Returns `None` if `$HOME` is not set.
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
/// Returns `None` if `$HOME` is not set.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        Some(PathBuf::from(xdg).join("rdm"))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".local").join("share").join("rdm"))
    }
}

/// Loads the global config from disk, returning `Default` if the file is missing.
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
/// 1. `--root` CLI flag / `RDM_ROOT` env var (passed as `cli_root`)
/// 2. `root` field in global config
/// 3. XDG data dir (`$XDG_DATA_HOME/rdm` or `~/.local/share/rdm`)
///
/// # Errors
///
/// Returns an error if no root can be determined (e.g. `$HOME` is not set
/// and no explicit root was provided).
pub fn resolve_root(cli_root: Option<PathBuf>, global: &GlobalConfig) -> Result<PathBuf> {
    if let Some(root) = cli_root {
        return Ok(root);
    }
    if let Some(root) = &global.root {
        return Ok(root.clone());
    }
    if let Some(data_dir) = default_data_dir() {
        return Ok(data_dir);
    }
    bail!(
        "cannot determine plan repo location — set --root, RDM_ROOT, \
         or add root to ~/.config/rdm/config.toml"
    )
}

/// Resolves whether staging mode is active.
///
/// The `config` should already have global defaults merged via
/// [`Config::with_global_defaults`]. Priority: CLI flag/env → config.
pub fn resolve_staging(flag: bool, config: &rdm_core::config::Config) -> bool {
    if flag {
        return true;
    }
    config.stage == Some(true)
}

/// Resolves the default project.
///
/// The `config` should already have global defaults merged via
/// [`Config::with_global_defaults`]. Priority: flag → env → config.
///
/// # Errors
///
/// Returns an error if no project could be determined.
pub fn resolve_project(flag: Option<String>, config: &rdm_core::config::Config) -> Result<String> {
    resolve_project_inner(flag, std::env::var("RDM_PROJECT").ok(), config)
}

fn resolve_project_inner(
    flag: Option<String>,
    env_project: Option<String>,
    config: &rdm_core::config::Config,
) -> Result<String> {
    if let Some(p) = flag {
        return Ok(p);
    }
    if let Some(p) = env_project {
        return Ok(p);
    }
    if let Some(p) = &config.default_project {
        return Ok(p.clone());
    }
    bail!(
        "no project specified — use --project, set RDM_PROJECT, \
         or set default_project in rdm.toml or ~/.config/rdm/config.toml"
    )
}

/// Resolves a remote name from an explicit argument or config.
///
/// The `config` should already have global defaults merged via
/// [`Config::with_global_defaults`].
///
/// # Errors
///
/// Returns an error if no remote name could be determined.
pub fn resolve_remote_name(
    name: Option<String>,
    config: &rdm_core::config::Config,
) -> Result<String> {
    if let Some(n) = name {
        return Ok(n);
    }
    if let Some(ref remote) = config.remote
        && let Some(ref d) = remote.default
    {
        return Ok(d.clone());
    }
    bail!("no remote specified — pass a remote name or set remote.default in rdm.toml")
}

/// Loads the repo config from `<root>/rdm.toml`, returning `Default` if missing.
pub fn load_repo_config(root: &Path) -> rdm_core::config::Config {
    let config_path = root.join("rdm.toml");
    std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| rdm_core::config::Config::from_toml(&s).ok())
        .unwrap_or_default()
}

/// Expands `~` and resolves `.`/`..` in a path.
///
/// # Errors
///
/// Returns an error if `~` is used but `$HOME` is not set, or if the path
/// cannot be made absolute.
pub fn expand_root(path: PathBuf) -> Result<PathBuf> {
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

    #[test]
    fn resolve_root_flag_wins() {
        let global = GlobalConfig {
            root: Some(PathBuf::from("/global/root")),
            ..Default::default()
        };
        let result = resolve_root(Some(PathBuf::from("/flag/root")), &global).unwrap();
        assert_eq!(result, PathBuf::from("/flag/root"));
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
        let global = GlobalConfig::default();
        // As long as HOME is set, we get the XDG data dir fallback
        let result = resolve_root(None, &global).unwrap();
        assert!(result.to_string_lossy().ends_with("/rdm"));
    }

    #[test]
    fn resolve_staging_flag_wins() {
        let config = rdm_core::config::Config {
            stage: Some(false),
            ..Default::default()
        };
        assert!(resolve_staging(true, &config));
    }

    #[test]
    fn resolve_staging_config_true() {
        let config = rdm_core::config::Config {
            stage: Some(true),
            ..Default::default()
        };
        assert!(resolve_staging(false, &config));
    }

    #[test]
    fn resolve_staging_default_false() {
        let config = rdm_core::config::Config::default();
        assert!(!resolve_staging(false, &config));
    }

    #[test]
    fn resolve_project_flag_wins() {
        let config = rdm_core::config::Config {
            default_project: Some("config".to_string()),
            ..Default::default()
        };
        let result =
            resolve_project_inner(Some("flag".to_string()), Some("env".to_string()), &config)
                .unwrap();
        assert_eq!(result, "flag");
    }

    #[test]
    fn resolve_project_env_wins_over_config() {
        let config = rdm_core::config::Config {
            default_project: Some("config".to_string()),
            ..Default::default()
        };
        let result = resolve_project_inner(None, Some("env".to_string()), &config).unwrap();
        assert_eq!(result, "env");
    }

    #[test]
    fn resolve_project_config_fallback() {
        let config = rdm_core::config::Config {
            default_project: Some("config".to_string()),
            ..Default::default()
        };
        let result = resolve_project_inner(None, None, &config).unwrap();
        assert_eq!(result, "config");
    }

    #[test]
    fn resolve_project_error_when_nothing() {
        let config = rdm_core::config::Config::default();
        let result = resolve_project_inner(None, None, &config);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_remote_name_flag_wins() {
        let config = rdm_core::config::Config::default();
        let result = resolve_remote_name(Some("origin".to_string()), &config).unwrap();
        assert_eq!(result, "origin");
    }

    #[test]
    fn resolve_remote_name_config_fallback() {
        let config = rdm_core::config::Config {
            remote: Some(rdm_core::config::RemoteConfig {
                default: Some("my-remote".to_string()),
            }),
            ..Default::default()
        };
        let result = resolve_remote_name(None, &config).unwrap();
        assert_eq!(result, "my-remote");
    }

    #[test]
    fn expand_root_tilde_expands_to_home() {
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
