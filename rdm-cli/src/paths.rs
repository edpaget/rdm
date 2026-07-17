use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rdm_core::config::{
    ConfigSource, GLOBAL_ONLY_KEYS, GlobalConfig, KNOWN_KEYS, REPO_ONLY_KEYS, ResolvedValue,
    format_quick_filters, parse_plan_review_env, parse_quick_filters_env,
};

/// Returns the path to the global config file.
///
/// Delegates to [`rdm_core::root::global_config_path`].
pub fn global_config_path() -> Option<PathBuf> {
    rdm_core::root::global_config_path()
}

/// Returns the default data directory for plan repos.
///
/// Delegates to [`rdm_core::root::default_data_dir`].
#[cfg(feature = "git")]
pub fn default_data_dir() -> Option<PathBuf> {
    rdm_core::root::default_data_dir()
}

/// Loads the global config from disk, returning `Default` if the file is missing.
///
/// Emits a warning to stderr if the config file contains invalid TOML.
pub fn load_global_config() -> GlobalConfig {
    let Some(path) = rdm_core::root::global_config_path() else {
        return GlobalConfig::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return GlobalConfig::default();
    };
    match GlobalConfig::from_toml(&contents) {
        Ok(config) => config,
        Err(e) => {
            eprintln!(
                "warning: ignoring malformed config at {}: {}",
                path.display(),
                e
            );
            GlobalConfig::default()
        }
    }
}

/// Resolves the plan repo root using the priority chain:
///
/// 1. `--root` CLI flag / `RDM_ROOT` env var (passed as `cli_root`)
/// 2. `root` field in global config
/// 3. XDG data dir (`$XDG_DATA_HOME/rdm` or `~/.local/share/rdm`)
///
/// Delegates the chain to [`rdm_core::root::resolve_root`].
///
/// # Errors
///
/// Returns an error if no root can be determined (e.g. `$HOME` is not set
/// and no explicit root was provided).
pub fn resolve_root(cli_root: Option<PathBuf>, global: &GlobalConfig) -> Result<PathBuf> {
    rdm_core::root::resolve_root(cli_root, global).map_err(|_| {
        anyhow::anyhow!(
            "cannot determine plan repo location — set --root, RDM_ROOT, \
             or add root to ~/.config/rdm/config.toml"
        )
    })
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

/// Resolves the review author. Priority: `--author` flag →
/// `RDM_REVIEW_AUTHOR` env var → `$USER` / `$USERNAME`.
///
/// # Errors
///
/// Returns an error when none of the sources yield a non-empty value.
#[cfg(feature = "git")]
pub fn resolve_review_author(flag: Option<String>) -> Result<String> {
    resolve_review_author_inner(
        flag,
        std::env::var("RDM_REVIEW_AUTHOR").ok(),
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .ok(),
    )
}

#[cfg(feature = "git")]
fn resolve_review_author_inner(
    flag: Option<String>,
    env_author: Option<String>,
    os_user: Option<String>,
) -> Result<String> {
    for candidate in [flag, env_author, os_user].into_iter().flatten() {
        if !candidate.trim().is_empty() {
            return Ok(candidate);
        }
    }
    bail!("cannot determine review author — pass --author <name> or set RDM_REVIEW_AUTHOR")
}

/// Resolves a remote name from an explicit argument or config.
///
/// The `config` should already have global defaults merged via
/// [`Config::with_global_defaults`].
///
/// # Errors
///
/// Returns an error if no remote name could be determined.
#[cfg(feature = "git")]
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
///
/// Emits a warning to stderr if the config file contains invalid TOML.
pub fn load_repo_config(root: &Path) -> rdm_core::config::Config {
    let config_path = root.join("rdm.toml");
    let Ok(contents) = std::fs::read_to_string(&config_path) else {
        return rdm_core::config::Config::default();
    };
    match rdm_core::config::Config::from_toml(&contents) {
        Ok(config) => config,
        Err(e) => {
            eprintln!(
                "warning: ignoring malformed config at {}: {}",
                config_path.display(),
                e
            );
            rdm_core::config::Config::default()
        }
    }
}

/// Resolves whether plan-review tag stamping is enabled, from the
/// `RDM_PLAN_REVIEW` env var and config.
///
/// The `config` should already have global defaults merged via
/// [`rdm_core::config::Config::with_global_defaults`]. Priority: env →
/// config `plan_review` → `false`.
///
/// # Errors
///
/// Returns an error if `RDM_PLAN_REVIEW` is set to a value other than the
/// literal `"true"` or `"false"`.
pub fn resolve_plan_review(config: &rdm_core::config::Config) -> Result<bool> {
    resolve_plan_review_inner(std::env::var("RDM_PLAN_REVIEW").ok(), config)
}

fn resolve_plan_review_inner(
    env_value: Option<String>,
    config: &rdm_core::config::Config,
) -> Result<bool> {
    if let Some(v) = env_value {
        return parse_plan_review_env(&v).map_err(|e| anyhow::anyhow!("{e}"));
    }
    Ok(config.plan_review.unwrap_or(false))
}

/// Resolves the output format from the CLI flag, `RDM_FORMAT` env var, and config.
///
/// Priority: flag → env → config `default_format` → Human (as string `"human"`).
pub fn resolve_format(flag: Option<String>, config: &rdm_core::config::Config) -> String {
    resolve_format_inner(flag, std::env::var("RDM_FORMAT").ok(), config)
}

fn resolve_format_inner(
    flag: Option<String>,
    env_format: Option<String>,
    config: &rdm_core::config::Config,
) -> String {
    if let Some(f) = flag {
        return f;
    }
    if let Some(f) = env_format {
        return f;
    }
    if let Some(f) = &config.default_format {
        return f.clone();
    }
    "human".to_string()
}

/// Resolves a config value by key across repo and global config.
///
/// Returns `None` if the key is not set in either config.
pub fn resolve_config_value(
    key: &str,
    repo: &rdm_core::config::Config,
    global: &GlobalConfig,
) -> Option<ResolvedValue<String>> {
    if let Some(v) = get_config_field(repo, key) {
        return Some(ResolvedValue {
            value: v,
            source: ConfigSource::Repo,
        });
    }
    if let Some(v) = get_global_config_field(global, key) {
        return Some(ResolvedValue {
            value: v,
            source: ConfigSource::Global,
        });
    }
    None
}

/// Saves a repo config to `<root>/rdm.toml`.
///
/// # Errors
///
/// Returns an error if serialization or file I/O fails.
pub fn save_repo_config(root: &Path, config: &rdm_core::config::Config) -> Result<()> {
    let toml_str = config
        .to_toml()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to serialize repo config")?;
    std::fs::write(root.join("rdm.toml"), toml_str).context("failed to write rdm.toml")?;
    Ok(())
}

/// Saves the global config to the XDG config path.
///
/// Creates the parent directory if it does not exist.
///
/// # Errors
///
/// Returns an error if the global config path cannot be determined, or if
/// serialization or file I/O fails.
pub fn save_global_config(config: &GlobalConfig) -> Result<()> {
    let path = global_config_path()
        .ok_or_else(|| anyhow::anyhow!("cannot determine global config path — is $HOME set?"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let toml_str = config
        .to_toml()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to serialize global config")?;
    std::fs::write(&path, toml_str)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Extracts a field value from a repo config by key name.
pub fn get_config_field(config: &rdm_core::config::Config, key: &str) -> Option<String> {
    match key {
        "default_project" => config.default_project.clone(),
        "default_format" => config.default_format.clone(),
        "remote.default" => config.remote.as_ref().and_then(|r| r.default.clone()),
        "default_branch" => config.default_branch.clone(),
        "hook_timeout_secs" => config.hook_timeout_secs.map(|n| n.to_string()),
        "plan_review" => config.plan_review.map(|b| b.to_string()),
        // NOTE: a malformed RDM_SERVER_QUICK_FILTERS env value is echoed
        // back raw with "(source: environment variable)" by the generic
        // resolution chain in commands/config.rs — a pre-existing quirk of
        // how config get/list check the env var directly, shared with every
        // other key. Not fixed here; out of scope for this phase.
        "server.quick_filters" => config
            .server
            .as_ref()
            .map(|s| format_quick_filters(&s.quick_filters)),
        _ => None,
    }
}

/// Extracts a field value from a global config by key name.
pub fn get_global_config_field(config: &GlobalConfig, key: &str) -> Option<String> {
    match key {
        "root" => config.root.as_ref().map(|p| p.display().to_string()),
        "default_project" => config.default_project.clone(),
        "default_format" => config.default_format.clone(),
        "remote.default" => config.remote.as_ref().and_then(|r| r.default.clone()),
        "auto_init" => config.auto_init.map(|b| b.to_string()),
        "default_branch" => config.default_branch.clone(),
        "hook_timeout_secs" => config.hook_timeout_secs.map(|n| n.to_string()),
        "plan_review" => config.plan_review.map(|b| b.to_string()),
        _ => None,
    }
}

/// Sets a field on a repo config by key name, with validation.
///
/// # Errors
///
/// Returns an error if the key is unknown or the value is invalid.
pub fn set_config_field(
    config: &mut rdm_core::config::Config,
    key: &str,
    value: &str,
) -> Result<()> {
    match key {
        "default_project" => config.default_project = Some(value.to_string()),
        "default_format" => {
            config.default_format = Some(value.to_string());
            config.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        "remote.default" => {
            config.remote.get_or_insert_with(Default::default).default = Some(value.to_string());
        }
        "default_branch" => config.default_branch = Some(value.to_string()),
        "hook_timeout_secs" => {
            config.hook_timeout_secs = Some(parse_u64(key, value)?);
        }
        "plan_review" => {
            config.plan_review = Some(parse_bool(value)?);
        }
        "server.quick_filters" => {
            let filters = parse_quick_filters_env(value).map_err(|_| {
                anyhow::anyhow!(
                    "invalid value '{value}' for 'server.quick_filters' — use the \
                     'Label:tag,Label2:tag2' form (e.g. 'Bug:bug,Refactor:refactor'); \
                     pass an empty string to clear all chips"
                )
            })?;
            config
                .server
                .get_or_insert_with(Default::default)
                .quick_filters = filters;
        }
        "root" | "auto_init" => bail!("'{key}' can only be set in global config — use --global"),
        _ => bail!(
            "unknown config key: {key} — valid keys: {}",
            KNOWN_KEYS.join(", ")
        ),
    }
    Ok(())
}

/// Sets a field on a global config by key name, with validation.
///
/// # Errors
///
/// Returns an error if the key is unknown or the value is invalid.
pub fn set_global_config_field(config: &mut GlobalConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "root" => config.root = Some(PathBuf::from(value)),
        "default_project" => config.default_project = Some(value.to_string()),
        "default_format" => {
            config.default_format = Some(value.to_string());
            config.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        "remote.default" => {
            config.remote.get_or_insert_with(Default::default).default = Some(value.to_string());
        }
        "auto_init" => {
            config.auto_init = Some(parse_bool(value)?);
        }
        "default_branch" => config.default_branch = Some(value.to_string()),
        "hook_timeout_secs" => {
            config.hook_timeout_secs = Some(parse_u64(key, value)?);
        }
        "plan_review" => {
            config.plan_review = Some(parse_bool(value)?);
        }
        "server.quick_filters" => {
            bail!("'{key}' can only be set in repo config — omit --global")
        }
        _ => bail!(
            "unknown config key: {key} — valid keys: {}",
            KNOWN_KEYS.join(", ")
        ),
    }
    Ok(())
}

/// Validates that a key is in `KNOWN_KEYS`.
///
/// # Errors
///
/// Returns an error with a helpful message if the key is unknown.
pub fn validate_key(key: &str) -> Result<()> {
    if !KNOWN_KEYS.contains(&key) {
        bail!(
            "unknown config key: {key} — valid keys: {}",
            KNOWN_KEYS.join(", ")
        );
    }
    Ok(())
}

/// Checks if a key is global-only.
pub fn is_global_only(key: &str) -> bool {
    GLOBAL_ONLY_KEYS.contains(&key)
}

/// Checks if a key is repo-only (cannot be set with `--global`).
pub fn is_repo_only(key: &str) -> bool {
    REPO_ONLY_KEYS.contains(&key)
}

fn parse_bool(s: &str) -> Result<bool> {
    match s {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => bail!("invalid boolean value: {s} — use 'true' or 'false'"),
    }
}

fn parse_u64(key: &str, s: &str) -> Result<u64> {
    s.parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid value for {key}: {s} — use a non-negative integer"))
}

/// Expands `~` and resolves `.`/`..` in a path.
///
/// Delegates to [`rdm_core::root::expand_root`].
///
/// # Errors
///
/// Returns an error if `~` is used but `$HOME` is not set, or if the path
/// cannot be made absolute.
pub fn expand_root(path: PathBuf) -> Result<PathBuf> {
    Ok(rdm_core::root::expand_root(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn resolve_format_flag_wins() {
        let config = rdm_core::config::Config {
            default_format: Some("json".to_string()),
            ..Default::default()
        };
        let result = resolve_format_inner(
            Some("table".to_string()),
            Some("markdown".to_string()),
            &config,
        );
        assert_eq!(result, "table");
    }

    #[test]
    fn resolve_format_env_wins_over_config() {
        let config = rdm_core::config::Config {
            default_format: Some("json".to_string()),
            ..Default::default()
        };
        let result = resolve_format_inner(None, Some("markdown".to_string()), &config);
        assert_eq!(result, "markdown");
    }

    #[test]
    fn resolve_format_config_fallback() {
        let config = rdm_core::config::Config {
            default_format: Some("json".to_string()),
            ..Default::default()
        };
        let result = resolve_format_inner(None, None, &config);
        assert_eq!(result, "json");
    }

    #[test]
    fn resolve_format_default_human() {
        let config = rdm_core::config::Config::default();
        let result = resolve_format_inner(None, None, &config);
        assert_eq!(result, "human");
    }

    #[test]
    fn resolve_config_value_repo_wins() {
        let repo = rdm_core::config::Config {
            default_project: Some("repo-proj".to_string()),
            ..Default::default()
        };
        let global = GlobalConfig {
            default_project: Some("global-proj".to_string()),
            ..Default::default()
        };
        let resolved = resolve_config_value("default_project", &repo, &global).unwrap();
        assert_eq!(resolved.value, "repo-proj");
        assert_eq!(resolved.source, rdm_core::config::ConfigSource::Repo);
    }

    #[test]
    fn resolve_config_value_global_fallback() {
        let repo = rdm_core::config::Config::default();
        let global = GlobalConfig {
            default_project: Some("global-proj".to_string()),
            ..Default::default()
        };
        let resolved = resolve_config_value("default_project", &repo, &global).unwrap();
        assert_eq!(resolved.value, "global-proj");
        assert_eq!(resolved.source, rdm_core::config::ConfigSource::Global);
    }

    #[test]
    fn resolve_config_value_not_set() {
        let repo = rdm_core::config::Config::default();
        let global = GlobalConfig::default();
        let resolved = resolve_config_value("default_project", &repo, &global);
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_review_author_priority_chain() {
        // Flag wins over env and OS user.
        let author = resolve_review_author_inner(
            Some("flag".to_string()),
            Some("env".to_string()),
            Some("os".to_string()),
        )
        .unwrap();
        assert_eq!(author, "flag");
        // Env wins over OS user.
        let author =
            resolve_review_author_inner(None, Some("env".to_string()), Some("os".to_string()))
                .unwrap();
        assert_eq!(author, "env");
        // OS user is the last fallback.
        let author = resolve_review_author_inner(None, None, Some("os".to_string())).unwrap();
        assert_eq!(author, "os");
        // Empty/whitespace values are skipped, not returned.
        let author = resolve_review_author_inner(
            Some("  ".to_string()),
            Some(String::new()),
            Some("os".to_string()),
        )
        .unwrap();
        assert_eq!(author, "os");
        // Nothing available → actionable error.
        let err = resolve_review_author_inner(None, None, None).unwrap_err();
        assert!(err.to_string().contains("--author"));
    }

    #[test]
    fn resolve_plan_review_env_true_wins_over_config() {
        let config = rdm_core::config::Config {
            plan_review: Some(false),
            ..Default::default()
        };
        let result = resolve_plan_review_inner(Some("true".to_string()), &config).unwrap();
        assert!(result);
    }

    #[test]
    fn resolve_plan_review_env_false_wins_over_config() {
        let config = rdm_core::config::Config {
            plan_review: Some(true),
            ..Default::default()
        };
        let result = resolve_plan_review_inner(Some("false".to_string()), &config).unwrap();
        assert!(!result);
    }

    #[test]
    fn resolve_plan_review_config_fallback() {
        let config = rdm_core::config::Config {
            plan_review: Some(true),
            ..Default::default()
        };
        let result = resolve_plan_review_inner(None, &config).unwrap();
        assert!(result);
    }

    #[test]
    fn resolve_plan_review_default_false() {
        let config = rdm_core::config::Config::default();
        let result = resolve_plan_review_inner(None, &config).unwrap();
        assert!(!result);
    }

    #[test]
    fn resolve_plan_review_env_invalid_errors() {
        let config = rdm_core::config::Config::default();
        let err = resolve_plan_review_inner(Some("yes".to_string()), &config).unwrap_err();
        assert!(err.to_string().contains("RDM_PLAN_REVIEW"));
    }

    #[test]
    fn load_repo_config_falls_back_on_malformed_toml() {
        let temp = tempfile::TempDir::new().unwrap();
        let config_path = temp.path().join("rdm.toml");
        // Write malformed TOML
        std::fs::write(&config_path, "invalid = [").unwrap();

        let config = load_repo_config(temp.path());
        assert_eq!(config, rdm_core::config::Config::default());
    }

    #[test]
    fn load_repo_config_silent_on_missing_file() {
        let temp = tempfile::TempDir::new().unwrap();
        // Don't create rdm.toml

        let config = load_repo_config(temp.path());
        assert_eq!(config, rdm_core::config::Config::default());
    }

    #[test]
    fn load_repo_config_silent_on_valid_toml() {
        let temp = tempfile::TempDir::new().unwrap();
        let config_path = temp.path().join("rdm.toml");
        let valid_toml = "default_project = \"my-proj\"\n";
        std::fs::write(&config_path, valid_toml).unwrap();

        let config = load_repo_config(temp.path());
        assert_eq!(config.default_project, Some("my-proj".to_string()));
    }
}
