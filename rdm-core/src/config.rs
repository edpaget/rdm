/// Plan repo configuration (`rdm.toml`) and global configuration.
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::ModelTier;

/// Valid values for the `default_format` config key.
pub const VALID_FORMATS: &[&str] = &["human", "json", "table", "markdown"];

/// All known configuration keys.
pub const KNOWN_KEYS: &[&str] = &[
    "default_project",
    "default_format",
    "remote.default",
    "root",
    "auto_init",
    "default_branch",
    "hook_timeout_secs",
    "server.quick_filters",
    "plan_review",
];

/// Keys that may only be set in the global config (not in a repo `rdm.toml`).
pub const GLOBAL_ONLY_KEYS: &[&str] = &["root", "auto_init"];

/// Keys that may only be set in the repo config (not in the global config).
pub const REPO_ONLY_KEYS: &[&str] = &["server.quick_filters"];

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

/// A named tag preset rendered as a clickable chip on the HTTP server's HTML
/// list views.
///
/// Clicking a chip navigates to the same page with `?tag=<tag>` set so the
/// user filters by tag without typing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickFilter {
    /// User-facing label rendered on the chip.
    pub label: String,
    /// Tag value to filter by when this chip is clicked.
    pub tag: String,
}

/// Configuration for the `rdm serve` HTTP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    /// Quick-filter chips rendered on the roadmap, phase, and task list views.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick_filters: Vec<QuickFilter>,
}

/// Per-step model tier overrides within `[models.steps]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StepTiersConfig {
    /// Model tier for the planning step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ModelTier>,
    /// Model tier for the implementation step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implement: Option<ModelTier>,
    /// Model tier for the review-find step.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "review-find"
    )]
    pub review_find: Option<ModelTier>,
    /// Model tier for the review-verify step.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "review-verify"
    )]
    pub review_verify: Option<ModelTier>,
    /// Model tier for mechanical (non-LLM-judgment) steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical: Option<ModelTier>,
}

/// Configuration for the `[models]` table: model-tier sizing policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    /// Model id bound to the small tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small: Option<String>,
    /// Model id bound to the medium tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    /// Model id bound to the large tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large: Option<String>,
    /// Minimum tier that review steps may run on, regardless of a lower
    /// per-step override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_floor: Option<ModelTier>,
    /// Per-step tier overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<StepTiersConfig>,
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

    /// Git remote configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,

    /// The default branch name for post-commit hook filtering (e.g. `"main"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,

    /// When `true`, the MCP server auto-initializes the plan repo if uninitialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_init: Option<bool>,

    /// Wall-clock deadline (in seconds) for `rdm hook post-merge` /
    /// `rdm hook post-commit` execution. Defaults to a conservative built-in
    /// constant when unset. See [`Config::hook_timeout_secs`] for the
    /// repo-level counterpart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_timeout_secs: Option<u64>,

    /// Model-tier sizing policy (`[models]` table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsConfig>,

    /// When `true`, `roadmap create`/`phase create`/`task create` stamp the
    /// reserved `needs-plan-review` tag on new items so an agent-driven plan
    /// review can find and clear them later. See [`Config::plan_review`] for
    /// the repo-level counterpart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_review: Option<bool>,
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

    /// Git remote configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,

    /// The default branch name for post-commit hook filtering (e.g. `"main"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,

    /// HTTP server configuration (`[server]` table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerConfig>,

    /// Wall-clock deadline (in seconds) for `rdm hook post-merge` /
    /// `rdm hook post-commit` execution on this repo.
    ///
    /// Bounds how long the hook may run before it gives up, logs a
    /// `"timeout"` event, and exits successfully rather than risking an
    /// indefinite hang that blocks the invoking `git commit`/`git merge`.
    /// Falls back to a conservative built-in default when unset (and when a
    /// value of `0` is configured, since an unbounded timeout would defeat
    /// the purpose of this guard).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_timeout_secs: Option<u64>,

    /// Model-tier sizing policy (`[models]` table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsConfig>,

    /// When `true`, `roadmap create`/`phase create`/`task create` stamp the
    /// reserved `needs-plan-review` tag on new items (in addition to any
    /// user-supplied `--tags`) so an agent-driven plan review can find and
    /// clear them later via `rdm search --tag needs-plan-review`. Defaults
    /// to `false`. See [`crate::tags`] for the tag-manipulation primitives
    /// this flag gates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_review: Option<bool>,
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
            remote: self.remote.clone().or_else(|| global.remote.clone()),
            default_branch: self
                .default_branch
                .clone()
                .or_else(|| global.default_branch.clone()),
            server: self.server.clone(),
            hook_timeout_secs: self.hook_timeout_secs.or(global.hook_timeout_secs),
            models: self.models.clone().or_else(|| global.models.clone()),
            plan_review: self.plan_review.or(global.plan_review),
        }
    }
}

/// Parses the `RDM_SERVER_QUICK_FILTERS` env var into a list of [`QuickFilter`].
///
/// Format: `Label1:tag1,Label2:tag2`. Whitespace around items and around the
/// `:` separator is trimmed. Returns `Ok(vec![])` if `value` is empty.
///
/// # Errors
///
/// Returns [`Error::InvalidConfigValue`] if any item does not contain a `:`
/// separator, or if either side of the `:` is empty after trimming.
pub fn parse_quick_filters_env(value: &str) -> Result<Vec<QuickFilter>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    trimmed
        .split(',')
        .map(|item| {
            let (label, tag) = item
                .split_once(':')
                .ok_or_else(|| Error::InvalidConfigValue {
                    key: "RDM_SERVER_QUICK_FILTERS".to_string(),
                    value: item.to_string(),
                    valid: "Label:tag (comma-separated for multiple)".to_string(),
                })?;
            let label = label.trim();
            let tag = tag.trim();
            if label.is_empty() || tag.is_empty() {
                return Err(Error::InvalidConfigValue {
                    key: "RDM_SERVER_QUICK_FILTERS".to_string(),
                    value: item.to_string(),
                    valid: "Label:tag with non-empty label and tag".to_string(),
                });
            }
            Ok(QuickFilter {
                label: label.to_string(),
                tag: tag.to_string(),
            })
        })
        .collect()
}

/// Formats a list of [`QuickFilter`]s back into the `Label:tag,...` form
/// accepted by [`parse_quick_filters_env`].
///
/// Returns an empty string for an empty slice.
pub fn format_quick_filters(filters: &[QuickFilter]) -> String {
    filters
        .iter()
        .map(|f| format!("{}:{}", f.label, f.tag))
        .collect::<Vec<_>>()
        .join(",")
}

/// Parses the `RDM_PLAN_REVIEW` env var override.
///
/// Accepts only the literal, case-sensitive strings `"true"` and `"false"` —
/// this is a loud override, not a fuzzy boolean parse, so a typo (`"1"`,
/// `"yes"`, `"True"`, an empty string) surfaces as an error instead of
/// silently resolving to `false`.
///
/// # Errors
///
/// Returns [`Error::InvalidConfigValue`] if `value` is anything other than
/// `"true"` or `"false"`.
pub fn parse_plan_review_env(value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(Error::InvalidConfigValue {
            key: "RDM_PLAN_REVIEW".to_string(),
            value: other.to_string(),
            valid: "true or false".to_string(),
        }),
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
    fn config_with_quick_filters_round_trip() {
        let toml_str = r#"
[server]
quick_filters = [
    { label = "Bugs", tag = "bug" },
    { label = "UI", tag = "ui" },
]
"#;
        let config = Config::from_toml(toml_str).unwrap();
        let server = config.server.as_ref().expect("server section parsed");
        assert_eq!(server.quick_filters.len(), 2);
        assert_eq!(server.quick_filters[0].label, "Bugs");
        assert_eq!(server.quick_filters[0].tag, "bug");
        assert_eq!(server.quick_filters[1].label, "UI");
        assert_eq!(server.quick_filters[1].tag, "ui");

        let serialized = config.to_toml().unwrap();
        let reparsed = Config::from_toml(&serialized).unwrap();
        assert_eq!(reparsed, config);
    }

    #[test]
    fn parse_quick_filters_env_basic() {
        let parsed = parse_quick_filters_env("Bugs:bug,UI:ui").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].label, "Bugs");
        assert_eq!(parsed[0].tag, "bug");
        assert_eq!(parsed[1].label, "UI");
        assert_eq!(parsed[1].tag, "ui");
    }

    #[test]
    fn parse_quick_filters_env_trims_whitespace() {
        let parsed = parse_quick_filters_env("  Bugs : bug , UI : ui  ").unwrap();
        assert_eq!(parsed[0].label, "Bugs");
        assert_eq!(parsed[0].tag, "bug");
        assert_eq!(parsed[1].label, "UI");
    }

    #[test]
    fn parse_quick_filters_env_empty() {
        let parsed = parse_quick_filters_env("").unwrap();
        assert!(parsed.is_empty());
        let parsed = parse_quick_filters_env("   ").unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_quick_filters_env_missing_separator_rejected() {
        let err = parse_quick_filters_env("Bugs").unwrap_err();
        match err {
            Error::InvalidConfigValue { key, .. } => {
                assert_eq!(key, "RDM_SERVER_QUICK_FILTERS");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parse_quick_filters_env_empty_side_rejected() {
        assert!(parse_quick_filters_env("Bugs:").is_err());
        assert!(parse_quick_filters_env(":bug").is_err());
    }

    #[test]
    fn format_quick_filters_basic() {
        let filters = vec![
            QuickFilter {
                label: "Bugs".to_string(),
                tag: "bug".to_string(),
            },
            QuickFilter {
                label: "UI".to_string(),
                tag: "ui".to_string(),
            },
        ];
        assert_eq!(format_quick_filters(&filters), "Bugs:bug,UI:ui");
    }

    #[test]
    fn format_quick_filters_empty() {
        assert_eq!(format_quick_filters(&[]), "");
    }

    #[test]
    fn quick_filters_format_parse_round_trip() {
        let filters = parse_quick_filters_env("Bugs:bug,Refactor:refactor").unwrap();
        let formatted = format_quick_filters(&filters);
        let reparsed = parse_quick_filters_env(&formatted).unwrap();
        assert_eq!(reparsed, filters);
    }

    #[test]
    fn parse_global_config_with_root() {
        let toml_str = r#"
root = "/some/path"
default_project = "myproj"

[remote]
default = "upstream"
"#;
        let config = GlobalConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.root, Some(PathBuf::from("/some/path")));
        assert_eq!(config.default_project, Some("myproj".to_string()));
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
        assert_eq!(config.remote, None);
        assert_eq!(config.default_branch, None);
        assert_eq!(config.models, None);
        assert_eq!(config.plan_review, None);
    }

    #[test]
    fn global_config_round_trip() {
        let config = GlobalConfig {
            root: Some(PathBuf::from("/plans")),
            default_project: Some("proj".to_string()),
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
            remote: Some(RemoteConfig {
                default: Some("upstream".to_string()),
            }),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        // repo config wins for default_project
        assert_eq!(merged.default_project, Some("repo-proj".to_string()));
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

    #[test]
    fn with_global_defaults_includes_default_branch() {
        let repo_config = Config::default();
        let global = GlobalConfig {
            default_branch: Some("trunk".to_string()),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.default_branch, Some("trunk".to_string()));
    }

    #[test]
    fn with_global_defaults_repo_default_branch_wins() {
        let repo_config = Config {
            default_branch: Some("develop".to_string()),
            ..Default::default()
        };
        let global = GlobalConfig {
            default_branch: Some("trunk".to_string()),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.default_branch, Some("develop".to_string()));
    }

    #[test]
    fn config_with_default_branch_round_trip() {
        let config = Config {
            default_branch: Some("develop".to_string()),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
        assert_eq!(parsed.default_branch, Some("develop".to_string()));
    }

    // --- hook_timeout_secs tests ---

    #[test]
    fn config_with_hook_timeout_round_trip() {
        let config = Config {
            hook_timeout_secs: Some(45),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
        assert_eq!(parsed.hook_timeout_secs, Some(45));
    }

    #[test]
    fn with_global_defaults_includes_hook_timeout() {
        let repo_config = Config::default();
        let global = GlobalConfig {
            hook_timeout_secs: Some(60),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.hook_timeout_secs, Some(60));
    }

    #[test]
    fn with_global_defaults_repo_hook_timeout_wins() {
        let repo_config = Config {
            hook_timeout_secs: Some(10),
            ..Default::default()
        };
        let global = GlobalConfig {
            hook_timeout_secs: Some(60),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.hook_timeout_secs, Some(10));
    }

    #[test]
    fn parse_global_config_with_hook_timeout() {
        let toml_str = "hook_timeout_secs = 20";
        let config = GlobalConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.hook_timeout_secs, Some(20));
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

    // --- [models] config tests ---

    #[test]
    fn parse_config_with_full_models_table() {
        let toml_str = r#"
[models]
small = "haiku"
medium = "sonnet"
large = "opus"
review_floor = "medium"

[models.steps]
plan = "medium"
implement = "large"
review-find = "medium"
review-verify = "large"
mechanical = "small"
"#;
        let config = Config::from_toml(toml_str).unwrap();
        let models = config.models.expect("models section parsed");
        assert_eq!(models.small, Some("haiku".to_string()));
        assert_eq!(models.medium, Some("sonnet".to_string()));
        assert_eq!(models.large, Some("opus".to_string()));
        assert_eq!(models.review_floor, Some(ModelTier::Medium));

        let steps = models.steps.expect("steps section parsed");
        assert_eq!(steps.plan, Some(ModelTier::Medium));
        assert_eq!(steps.implement, Some(ModelTier::Large));
        assert_eq!(steps.review_find, Some(ModelTier::Medium));
        assert_eq!(steps.review_verify, Some(ModelTier::Large));
        assert_eq!(steps.mechanical, Some(ModelTier::Small));
    }

    #[test]
    fn config_with_models_round_trip() {
        let config = Config {
            models: Some(ModelsConfig {
                small: Some("haiku".to_string()),
                medium: Some("sonnet".to_string()),
                large: Some("opus".to_string()),
                review_floor: Some(ModelTier::Medium),
                steps: Some(StepTiersConfig {
                    plan: Some(ModelTier::Medium),
                    implement: Some(ModelTier::Large),
                    review_find: Some(ModelTier::Medium),
                    review_verify: Some(ModelTier::Large),
                    mechanical: Some(ModelTier::Small),
                }),
            }),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        assert!(toml_str.contains("review-find"));
        assert!(toml_str.contains("review-verify"));
        assert!(!toml_str.contains("review_find"));
        assert!(!toml_str.contains("review_verify"));

        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn models_omitted_when_none() {
        let config = Config {
            default_project: Some("fbm".to_string()),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        assert!(!toml_str.contains("[models]"));
    }

    #[test]
    fn with_global_defaults_fills_models_from_global() {
        let repo_config = Config::default();
        let global_models = ModelsConfig {
            small: Some("haiku".to_string()),
            medium: Some("sonnet".to_string()),
            large: Some("opus".to_string()),
            review_floor: Some(ModelTier::Medium),
            steps: None,
        };
        let global = GlobalConfig {
            models: Some(global_models.clone()),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.models, Some(global_models));
    }

    #[test]
    fn with_global_defaults_repo_models_wins() {
        let repo_models = ModelsConfig {
            small: Some("haiku".to_string()),
            medium: None,
            large: None,
            review_floor: None,
            steps: None,
        };
        let global_models = ModelsConfig {
            small: Some("haiku".to_string()),
            medium: Some("sonnet".to_string()),
            large: Some("opus".to_string()),
            review_floor: Some(ModelTier::Large),
            steps: None,
        };
        let repo_config = Config {
            models: Some(repo_models.clone()),
            ..Default::default()
        };
        let global = GlobalConfig {
            models: Some(global_models),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        // Wholesale override: repo's models table wins entirely, no deep merge
        // of individual fields against the global table.
        assert_eq!(merged.models, Some(repo_models));
    }

    #[test]
    fn config_without_models_parses_to_none() {
        let toml_str = r#"default_project = "fbm""#;
        let config = Config::from_toml(toml_str).unwrap();
        assert!(config.models.is_none());
    }

    #[test]
    fn validate_config_invalid_model_tier_rejected() {
        let toml_str = "[models]\nreview_floor = \"extra-large\"";
        let err = Config::from_toml(toml_str).unwrap_err();
        assert!(
            matches!(err, Error::ConfigParse(_)),
            "expected ConfigParse, got {err:?}"
        );
    }

    #[test]
    fn validate_config_invalid_step_tier_rejected() {
        let toml_str = "[models.steps]\nplan = \"extra-large\"";
        let err = Config::from_toml(toml_str).unwrap_err();
        assert!(
            matches!(err, Error::ConfigParse(_)),
            "expected ConfigParse, got {err:?}"
        );
    }

    #[test]
    fn empty_models_steps_table_parses_to_all_none() {
        let toml_str = "[models]\n[models.steps]\n";
        let config = Config::from_toml(toml_str).unwrap();
        let models = config.models.expect("models section parsed");
        let steps = models.steps.expect("steps section parsed");
        assert_eq!(steps, StepTiersConfig::default());
    }

    // --- plan_review tests ---

    #[test]
    fn parse_plan_review_env_true_and_false() {
        assert!(parse_plan_review_env("true").unwrap());
        assert!(!parse_plan_review_env("false").unwrap());
    }

    #[test]
    fn parse_plan_review_env_invalid_rejected() {
        for bad in ["1", "yes", "True", "FALSE", ""] {
            let err = parse_plan_review_env(bad).unwrap_err();
            match err {
                Error::InvalidConfigValue { key, value, .. } => {
                    assert_eq!(key, "RDM_PLAN_REVIEW");
                    assert_eq!(value, bad);
                }
                other => panic!("unexpected error for '{bad}': {other:?}"),
            }
        }
    }

    #[test]
    fn config_with_plan_review_round_trip() {
        let config = Config {
            plan_review: Some(true),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        let parsed = Config::from_toml(&toml_str).unwrap();
        assert_eq!(parsed, config);
        assert_eq!(parsed.plan_review, Some(true));
    }

    #[test]
    fn with_global_defaults_includes_plan_review() {
        let repo_config = Config::default();
        let global = GlobalConfig {
            plan_review: Some(true),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.plan_review, Some(true));
    }

    #[test]
    fn with_global_defaults_repo_plan_review_wins() {
        let repo_config = Config {
            plan_review: Some(false),
            ..Default::default()
        };
        let global = GlobalConfig {
            plan_review: Some(true),
            ..Default::default()
        };
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(merged.plan_review, Some(false));
    }

    #[test]
    fn partial_models_table_parses() {
        let toml_str = "[models]\nsmall = \"haiku\"\n";
        let config = Config::from_toml(toml_str).unwrap();
        let models = config.models.expect("models section parsed");
        assert_eq!(models.small, Some("haiku".to_string()));
        assert_eq!(models.medium, None);
        assert_eq!(models.large, None);
        assert_eq!(models.review_floor, None);
        assert_eq!(models.steps, None);
    }
}
