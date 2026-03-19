/// Plan repo configuration (`rdm.toml`).
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Configuration for the default git remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RemoteConfig {
    /// The default remote name used by `rdm push` and `rdm pull`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
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
