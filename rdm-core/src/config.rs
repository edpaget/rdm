/// Plan repo configuration (`rdm.toml`) and global configuration.
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Valid values for the `default_format` config key.
pub const VALID_FORMATS: &[&str] = &["human", "json", "table", "markdown"];

/// All known configuration keys.
pub const KNOWN_KEYS: &[&str] = &[
    "default_project",
    "default_format",
    "stage",
    "remote.default",
    "root",
];

/// Keys that may only be set in the global config (not in a repo `rdm.toml`).
pub const GLOBAL_ONLY_KEYS: &[&str] = &["root"];

/// Where a configuration value was resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// Provided via a CLI flag.
    Flag,
    /// Provided via an environment variable.
    Env,
    /// Read from the repo-level `rdm.toml`.
    Repo,
    /// Read from the global config file.
    Global,
    /// A built-in default.
    Default,
}

impl fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigSource::Flag => write!(f, "CLI flag"),
            ConfigSource::Env => write!(f, "environment variable"),
            ConfigSource::Repo => write!(f, "repo config"),
            ConfigSource::Global => write!(f, "global config"),
            ConfigSource::Default => write!(f, "default"),
        }
    }
}

/// A resolved configuration value together with its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedValue<T> {
    /// The resolved value.
    pub value: T,
    /// Where the value came from.
    pub source: ConfigSource,
}

/// Configuration for the default git remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RemoteConfig {
    /// The default remote name used by `rdm push` and `rdm pull`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Global configuration stored at `~/.config/rdm/config.toml`.
///
/// Fields here act as fallback defaults for repo-level config and CLI flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    /// Default plan repo root path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,

    /// The default project to use when `--project` is not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project: Option<String>,

    /// Default output format (human, json, table, markdown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_format: Option<String>,

    /// When `true`, defers git commits until an explicit `rdm commit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<bool>,

    /// Git remote configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,

    /// The default branch name for post-commit hook filtering (e.g. `"main"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
}

impl GlobalConfig {
    /// Parses a `GlobalConfig` from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigParse`] if the string is not valid TOML or does
    /// not match the expected config schema. Returns [`Error::InvalidConfigValue`]
    /// if a field value fails validation.
    pub fn from_toml(s: &str) -> Result<Self> {
        let config: Self = toml::from_str(s)?;
        config.validate()?;
        Ok(config)
    }

    /// Serializes the global config to a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigSerialize`] if serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Validates the global config values.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidConfigValue`] if `default_format` is set to an
    /// unrecognized value.
    pub fn validate(&self) -> Result<()> {
        validate_format(&self.default_format)
    }
}

/// Configuration stored in `rdm.toml` at the plan repo root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    /// The default project to use when `--project` is not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project: Option<String>,

    /// Default output format (human, json, table, markdown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_format: Option<String>,

    /// When `true`, defers git commits until an explicit `rdm commit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<bool>,

    /// Git remote configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,

    /// The default branch name for post-commit hook filtering (e.g. `"main"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
}

impl Config {
    /// Parses a `Config` from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigParse`] if the string is not valid TOML or does
    /// not match the expected config schema. Returns [`Error::InvalidConfigValue`]
    /// if a field value fails validation.
    pub fn from_toml(s: &str) -> Result<Self> {
        let config: Self = toml::from_str(s)?;
        config.validate()?;
        Ok(config)
    }

    /// Serializes the config to a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigSerialize`] if serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Validates the config values.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidConfigValue`] if `default_format` is set to an
    /// unrecognized value.
    pub fn validate(&self) -> Result<()> {
        validate_format(&self.default_format)
    }

    /// Returns a new `Config` where `None` fields are filled from the
    /// given [`GlobalConfig`] defaults.
    ///
    /// Fields that are already `Some` in `self` are preserved.
    pub fn with_global_defaults(&self, global: &GlobalConfig) -> Config {
        Config {
            default_project: self
                .default_project
                .clone()
                .or_else(|| global.default_project.clone()),
            default_format: self
                .default_format
                .clone()
                .or_else(|| global.default_format.clone()),
            stage: self.stage.or(global.stage),
            remote: self.remote.clone().or_else(|| global.remote.clone()),
            default_branch: self
                .default_branch
                .clone()
                .or_else(|| global.default_branch.clone()),
        }
    }
}

/// Validates that a `default_format` value (if present) is one of the known formats.
fn validate_format(format: &Option<String>) -> Result<()> {
    if let Some(f) = format
        && !VALID_FORMATS.contains(&f.as_str())
    {
        return Err(Error::InvalidConfigValue {
            key: "default_format".to_string(),
            value: f.clone(),
            valid: VALID_FORMATS.join(", "),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml_str = r#"default_project = "fbm""#;
        let config = Config::from_toml(toml_str).unwrap();
        assert_eq!(config.default_project, Some("fbm".to_string()));
    }

    #[test]
    fn parse_empty_config() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(config.default_project, None);
    }

    #[test]
    fn config_round_trip() {
        let config = Config {
            default_project: Some("fbm".to_string()),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn config_with_stage_round_trip() {
        let config = Config {
            default_project: Some("fbm".to_string()),
            stage: Some(true),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
        assert_eq!(parsed.stage, Some(true));
    }

    #[test]
    fn empty_config_round_trip() {
        let config = Config::default();
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn config_with_remote_round_trip() {
        let config = Config {
            default_project: Some("fbm".to_string()),
            remote: Some(RemoteConfig {
                default: Some("origin".to_string()),
            }),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
        assert_eq!(parsed.remote.unwrap().default, Some("origin".to_string()));
    }

    #[test]
    fn config_without_remote_parses() {
        let toml_str = r#"default_project = "fbm""#;
        let config = Config::from_toml(toml_str).unwrap();
        assert_eq!(config.remote, None);
    }

    #[test]
    fn parse_global_config_with_root() {
        let toml_str = r#"
root = "/some/path"
default_project = "myproj"
stage = true

[remote]
default = "upstream"
"#;
        let config = GlobalConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.root, Some(PathBuf::from("/some/path")));
        assert_eq!(config.default_project, Some("myproj".to_string()));
        assert_eq!(config.stage, Some(true));
        assert_eq!(
            config.remote,
            Some(RemoteConfig {
                default: Some("upstream".to_string())
            })
        );
    }

    #[test]
    fn parse_global_config_empty() {
        let config = GlobalConfig::from_toml("").unwrap();
        assert_eq!(config.root, None);
        assert_eq!(config.default_project, None);
        assert_eq!(config.stage, None);
        assert_eq!(config.remote, None);
    }

    #[test]
    fn global_config_round_trip() {
        let config = GlobalConfig {
            root: Some(PathBuf::from("/plans")),
            default_project: Some("proj".to_string()),
            stage: Some(true),
            remote: Some(RemoteConfig {
                default: Some("origin".to_string()),
            }),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = GlobalConfig::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn config_with_global_defaults() {
        let repo_config = Config {
            default_project: Some("repo-proj".to_string()),
            ..Default::default()
        };
        let global = GlobalConfig {
            root: Some(PathBuf::from("/global")),
            default_project: Some("global-proj".to_string()),
            stage: Some(true),
            remote: Some(RemoteConfig {
                default: Some("upstream".to_string()),
            }),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        // repo config wins for default_project
        assert_eq!(merged.default_project, Some("repo-proj".to_string()));
        // global fills in stage
        assert_eq!(merged.stage, Some(true));
        // global fills in remote
        assert_eq!(
            merged.remote,
            Some(RemoteConfig {
                default: Some("upstream".to_string())
            })
        );
    }

    #[test]
    fn remote_config_omitted_when_none() {
        let config = Config {
            default_project: Some("fbm".to_string()),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        assert!(!toml_str.contains("[remote]"));
    }

    // --- default_format tests ---

    #[test]
    fn parse_config_with_default_format() {
        let toml_str = r#"default_format = "json""#;
        let config = Config::from_toml(toml_str).unwrap();
        assert_eq!(config.default_format, Some("json".to_string()));
    }

    #[test]
    fn config_default_format_round_trip() {
        let config = Config {
            default_format: Some("table".to_string()),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn global_config_with_default_format() {
        let toml_str = r#"default_format = "markdown""#;
        let config = GlobalConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.default_format, Some("markdown".to_string()));
    }

    #[test]
    fn validate_config_invalid_format() {
        let toml_str = r#"default_format = "xml""#;
        let err = Config::from_toml(toml_str).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("xml"),
            "error should mention the invalid value"
        );
        assert!(
            msg.contains("default_format"),
            "error should mention the key"
        );
    }

    #[test]
    fn validate_config_valid_formats() {
        for fmt in VALID_FORMATS {
            let toml_str = format!("default_format = \"{fmt}\"");
            Config::from_toml(&toml_str).unwrap_or_else(|e| panic!("'{fmt}' should be valid: {e}"));
        }
    }

    #[test]
    fn with_global_defaults_includes_format() {
        let repo_config = Config::default();
        let global = GlobalConfig {
            default_format: Some("json".to_string()),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.default_format, Some("json".to_string()));
    }

    #[test]
    fn with_global_defaults_repo_format_wins() {
        let repo_config = Config {
            default_format: Some("table".to_string()),
            ..Default::default()
        };
        let global = GlobalConfig {
            default_format: Some("json".to_string()),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.default_format, Some("table".to_string()));
    }

    // --- ConfigSource display tests ---

    #[test]
    fn config_source_display() {
        assert_eq!(ConfigSource::Flag.to_string(), "CLI flag");
        assert_eq!(ConfigSource::Env.to_string(), "environment variable");
        assert_eq!(ConfigSource::Repo.to_string(), "repo config");
        assert_eq!(ConfigSource::Global.to_string(), "global config");
        assert_eq!(ConfigSource::Default.to_string(), "default");
    }
}
