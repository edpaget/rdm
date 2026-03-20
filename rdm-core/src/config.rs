/// Plan repo configuration (`rdm.toml`) and global configuration.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;

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

    /// When `true`, defers git commits until an explicit `rdm commit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<bool>,

    /// Git remote configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,
}

impl GlobalConfig {
    /// Parses a `GlobalConfig` from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigParse`] if the string is not valid TOML or does
    /// not match the expected config schema.
    pub fn from_toml(s: &str) -> Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Serializes the global config to a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigSerialize`] if serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

/// Configuration stored in `rdm.toml` at the plan repo root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    /// The default project to use when `--project` is not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project: Option<String>,

    /// When `true`, defers git commits until an explicit `rdm commit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<bool>,

    /// Git remote configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,
}

impl Config {
    /// Parses a `Config` from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigParse`] if the string is not valid TOML or does
    /// not match the expected config schema.
    pub fn from_toml(s: &str) -> Result<Self> {
        Ok(toml::from_str(s)?)
    }

    /// Serializes the config to a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigSerialize`] if serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
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
            stage: self.stage.or(global.stage),
            remote: self.remote.clone().or_else(|| global.remote.clone()),
        }
    }
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
            stage: None,
            remote: None,
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
            remote: None,
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
            stage: None,
            remote: Some(RemoteConfig {
                default: Some("origin".to_string()),
            }),
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
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = GlobalConfig::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn config_with_global_defaults() {
        let repo_config = Config {
            default_project: Some("repo-proj".to_string()),
            stage: None,
            remote: None,
        };
        let global = GlobalConfig {
            root: Some(PathBuf::from("/global")),
            default_project: Some("global-proj".to_string()),
            stage: Some(true),
            remote: Some(RemoteConfig {
                default: Some("upstream".to_string()),
            }),
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
            stage: None,
            remote: None,
        };
        let toml_str = config.to_toml().unwrap();
        assert!(!toml_str.contains("[remote]"));
    }
}
