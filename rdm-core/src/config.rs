/// Plan repo configuration (`rdm.toml`).
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Configuration stored in `rdm.toml` at the plan repo root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    /// The default project to use when `--project` is not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project: Option<String>,
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
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn empty_config_round_trip() {
        let config = Config::default();
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }
}
