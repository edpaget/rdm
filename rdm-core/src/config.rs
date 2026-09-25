/// Plan repo configuration (`rdm.toml`) and global configuration.
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Effort, ModelTier};
use crate::model_policy::Host;

/// Valid values for the `default_format` config key.
pub const VALID_FORMATS: &[&str] = &["human", "json", "table", "markdown"];

/// All known configuration keys.
pub const KNOWN_KEYS: &[&str] = &[
    "default_project",
    "default_format",
    "remote.default",
    "root",
    "default_branch",
    "hook_timeout_secs",
    "server.quick_filters",
    "plan_review",
    "dispatch.verify",
    "gates.reviewed",
];

/// Keys that may only be set in the global config (not in a repo `rdm.toml`).
pub const GLOBAL_ONLY_KEYS: &[&str] = &["root"];

/// Keys that may only be set in the repo config (not in the global config).
pub const REPO_ONLY_KEYS: &[&str] = &["server.quick_filters", "dispatch.verify", "gates.reviewed"];

/// Keys that may be overridden per project in a `[projects.<name>]` table of
/// the repo `rdm.toml`.
///
/// This is the only allowlist: a key not listed here is refused by
/// [`resolve_scoped_value`] with [`Error::KeyNotProjectScopable`], so no key
/// becomes project-scopable by accident.
pub const PROJECT_SCOPABLE_KEYS: &[&str] = &[
    "dispatch.verify",
    "gates.reviewed",
    "plan_review",
    "default_branch",
];

/// Where a configuration value was resolved from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// Provided via a CLI flag.
    Flag,
    /// Provided via an environment variable.
    Env,
    /// Read from a `[projects.<name>]` override table in the repo-level
    /// `rdm.toml`.
    Project,
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
            ConfigSource::Project => write!(f, "project config"),
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

/// Configuration for the autonomous dispatch lane (`[dispatch]` table).
///
/// Repo-only: a verify command is a property of a project, never of a user, so
/// this table has no counterpart on [`GlobalConfig`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DispatchConfig {
    /// The single command the dispatch pipeline runs once per implementation
    /// attempt, whose exit code gates the phase.
    ///
    /// rdm never decomposes, reorders, or partially runs it — timeouts,
    /// ordering, parallelism and output formatting belong to whatever task
    /// runner the command invokes. See `docs/verify-gate.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<String>,
}

/// Configuration for rdm's write-time gates (`[gates]` table).
///
/// Repo-only: whether a project enforces the `reviewed` transition gate is a
/// property of the project's process, never of a user, so this table has no
/// counterpart on [`GlobalConfig`] — the same treatment [`DispatchConfig`]
/// gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GatesConfig {
    /// When `true`, `phase update --status reviewed` and `task update --status
    /// reviewed` refuse unless an approved implementation plan, an approving
    /// `change/` review naming it, and a clean worktree all exist.
    ///
    /// Defaults to `false`: the gate is opt-in, so enabling core-enforced
    /// gates is always a deliberate act and no existing plan repo changes
    /// behavior on upgrade. See `docs/core-enforced-gates.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed: Option<bool>,
}

/// Per-project overrides of the project-scopable keys (`[projects.<name>]`).
///
/// Has the same TOML shape as the matching [`Config`] fields, so
/// `[projects.a.dispatch] verify = "..."` mirrors `[dispatch] verify = "..."`.
/// Only the keys in [`PROJECT_SCOPABLE_KEYS`] have a field here. Like
/// [`Config`], unknown keys are ignored rather than rejected: a repo config
/// that fails to parse falls back to the default config, so a strict mode
/// would turn one typo into silently dropping the whole file.
///
/// Repo-only: the global config has no project layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectOverrides {
    /// Per-project override of [`Config::default_branch`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// Per-project override of [`Config::plan_review`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_review: Option<bool>,
    /// Per-project override of [`Config::dispatch`] (`[projects.<name>.dispatch]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<DispatchConfig>,
    /// Per-project override of [`Config::gates`] (`[projects.<name>.gates]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gates: Option<GatesConfig>,
}

impl ProjectOverrides {
    /// Returns this project's override for a project-scopable `key`, in the
    /// string form `rdm config get` prints (booleans as `true`/`false`).
    ///
    /// Returns `None` when the key is not overridden, or is not one of
    /// [`PROJECT_SCOPABLE_KEYS`].
    #[must_use]
    pub fn value(&self, key: &str) -> Option<String> {
        scopable_field(
            key,
            &self.default_branch,
            self.plan_review,
            &self.dispatch,
            &self.gates,
        )
    }
}

/// Reads one project-scopable key out of the four fields [`Config`] and
/// [`ProjectOverrides`] share, so both accessors print identical forms.
fn scopable_field(
    key: &str,
    default_branch: &Option<String>,
    plan_review: Option<bool>,
    dispatch: &Option<DispatchConfig>,
    gates: &Option<GatesConfig>,
) -> Option<String> {
    match key {
        "default_branch" => default_branch.clone(),
        "plan_review" => plan_review.map(|b| b.to_string()),
        "dispatch.verify" => dispatch.as_ref().and_then(|d| d.verify.clone()),
        "gates.reviewed" => gates
            .as_ref()
            .and_then(|g| g.reviewed)
            .map(|b| b.to_string()),
        _ => None,
    }
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
    /// Per-host, per-tier model + effort profiles
    /// (`[models.profiles.<host>.<tier>]`).
    ///
    /// A profile's `model` takes precedence over the legacy
    /// `small`/`medium`/`large` keys (which only ever set the `claude`
    /// host's model); an unset field falls back to the built-in table in
    /// [`crate::model_policy`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profiles: Option<ProfilesConfig>,
}

/// The `[models.profiles]` table: model + effort profiles keyed by host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProfilesConfig {
    /// Profiles for the Claude Code host (`[models.profiles.claude]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<HostProfilesConfig>,
    /// Profiles for the Codex host (`[models.profiles.codex]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<HostProfilesConfig>,
}

impl ProfilesConfig {
    /// Returns the profiles configured for `host`, if any.
    #[must_use]
    pub fn for_host(&self, host: Host) -> Option<&HostProfilesConfig> {
        match host {
            Host::Claude => self.claude.as_ref(),
            Host::Codex => self.codex.as_ref(),
        }
    }
}

/// One host's profiles, keyed by model tier (`[models.profiles.<host>]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HostProfilesConfig {
    /// Profile for the small tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small: Option<ProfileConfig>,
    /// Profile for the medium tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<ProfileConfig>,
    /// Profile for the large tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub large: Option<ProfileConfig>,
    /// Profile for the opt-in frontier tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier: Option<ProfileConfig>,
}

impl HostProfilesConfig {
    /// Returns the profile configured for `tier`, if any.
    #[must_use]
    pub fn for_tier(&self, tier: ModelTier) -> Option<&ProfileConfig> {
        match tier {
            ModelTier::Small => self.small.as_ref(),
            ModelTier::Medium => self.medium.as_ref(),
            ModelTier::Large => self.large.as_ref(),
            ModelTier::Frontier => self.frontier.as_ref(),
        }
    }
}

/// A configured model + effort profile for one host and tier. Either field
/// may be omitted to keep its fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProfileConfig {
    /// Model id this tier runs on for the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reasoning effort this tier runs at for the host. Must be one of the
    /// host's [`Host::valid_efforts`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
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
    /// unrecognized value, or if a `[models.profiles.<host>.<tier>]` effort
    /// is not one the host accepts (see [`Host::valid_efforts`]).
    pub fn validate(&self) -> Result<()> {
        validate_format(&self.default_format)?;
        validate_models(&self.models)
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

    /// Autonomous dispatch-lane configuration (`[dispatch]` table).
    ///
    /// Repo-only — deliberately absent from [`GlobalConfig`], and carried
    /// through [`Config::with_global_defaults`] unchanged with no global
    /// fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<DispatchConfig>,

    /// Write-time gate configuration (`[gates]` table).
    ///
    /// Repo-only — deliberately absent from [`GlobalConfig`], and carried
    /// through [`Config::with_global_defaults`] unchanged with no global
    /// fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gates: Option<GatesConfig>,

    /// Per-project overrides of the project-scopable keys, keyed by project
    /// name (`[projects.<name>]` tables).
    ///
    /// Repo-only, and carried through [`Config::with_global_defaults`]
    /// unchanged. Resolve a value through [`resolve_scoped_value`] rather than
    /// reading this map directly, so every consumer applies the same
    /// precedence.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, ProjectOverrides>,
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
    /// unrecognized value, or if a `[models.profiles.<host>.<tier>]` effort
    /// is not one the host accepts (see [`Host::valid_efforts`]).
    pub fn validate(&self) -> Result<()> {
        validate_format(&self.default_format)?;
        validate_models(&self.models)
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
            // Repo-only: no global fallback exists to fall back TO.
            dispatch: self.dispatch.clone(),
            gates: self.gates.clone(),
            // The global config has no project layer.
            projects: self.projects.clone(),
        }
    }

    /// Returns the plan-repo-wide value of a project-scopable `key`, in the
    /// string form `rdm config get` prints (booleans as `true`/`false`).
    ///
    /// Returns `None` when the key is unset, or is not one of
    /// [`PROJECT_SCOPABLE_KEYS`].
    #[must_use]
    pub fn scopable_value(&self, key: &str) -> Option<String> {
        scopable_field(
            key,
            &self.default_branch,
            self.plan_review,
            &self.dispatch,
            &self.gates,
        )
    }
}

/// Resolves a project-scopable `key` for `project`, returning the value and
/// where it came from.
///
/// This is the single project-aware resolution rule: `rdm config get`/`list`
/// and every consumer of a project-scopable key read through it, so what
/// `config get` reports and what a consumer does never disagree.
///
/// Precedence:
///
/// 1. The environment: for `gates.reviewed` only, `RDM_REVIEWED_GATE` — the
///    variable the `reviewed` gate itself honors, so it comes first and the
///    resolver never disagrees with [`resolve_reviewed_gate`]; then the
///    generic `RDM_<KEY>` (dots become underscores, e.g.
///    `RDM_DISPATCH_VERIFY`). Every value of a boolean key (`gates.reviewed`,
///    `plan_review`) must be the literal `"true"` or `"false"`; other keys'
///    values are returned raw.
/// 2. `repo.projects[project]`, when `project` is `Some`.
/// 3. The plan-repo-wide value in `repo`.
/// 4. `global`, only for keys not in [`REPO_ONLY_KEYS`].
/// 5. `None` — typed defaults belong to the consumer.
///
/// `dispatch.verify` is trimmed at every layer (environment, project, repo),
/// and a blank value at any layer counts as unset, so resolution falls
/// through to the next layer; blank everywhere resolves to `None`.
///
/// `repo` must be the repo config as read from `rdm.toml`, not one already
/// merged with the global config, or step 4's repo-only rule is bypassed.
/// `env` is injected so the rule stays a pure function; pass
/// `|k| std::env::var(k).ok()` for the process environment.
///
/// # Errors
///
/// Returns [`Error::KeyNotProjectScopable`] if `key` is not one of
/// [`PROJECT_SCOPABLE_KEYS`], and [`Error::InvalidConfigValue`] (naming the
/// variable) if an environment override of a boolean key is anything other
/// than the literal `"true"` or `"false"`.
pub fn resolve_scoped_value(
    key: &str,
    project: Option<&str>,
    repo: &Config,
    global: &GlobalConfig,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<ResolvedValue<String>>> {
    if !PROJECT_SCOPABLE_KEYS.contains(&key) {
        return Err(Error::KeyNotProjectScopable {
            key: key.to_string(),
        });
    }
    let found = |value: String, source: ConfigSource| Ok(Some(ResolvedValue { value, source }));

    if key == "gates.reviewed"
        && let Some(v) = env("RDM_REVIEWED_GATE")
    {
        return found(parse_reviewed_gate_env(&v)?.to_string(), ConfigSource::Env);
    }
    let generic_env = format!("RDM_{}", key.to_uppercase().replace('.', "_"));
    // Every layer's value passes through `present`: a blank `dispatch.verify`
    // at any layer is unset, so it falls through to the next layer instead of
    // letting a verification gate pass without running anything.
    let present = |v: String| match key {
        "dispatch.verify" => {
            let t = v.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        _ => Some(v),
    };
    let generic = env(&generic_env).and_then(present);
    if let Some(v) = generic {
        let v = match key {
            "gates.reviewed" | "plan_review" => parse_bool_env(&generic_env, &v)?.to_string(),
            _ => v,
        };
        return found(v, ConfigSource::Env);
    }
    if let Some(v) = project
        .and_then(|p| repo.projects.get(p))
        .and_then(|o| o.value(key))
        .and_then(present)
    {
        return found(v, ConfigSource::Project);
    }
    if let Some(v) = repo.scopable_value(key).and_then(present) {
        return found(v, ConfigSource::Repo);
    }
    if !REPO_ONLY_KEYS.contains(&key) {
        let global_value = match key {
            "plan_review" => global.plan_review.map(|b| b.to_string()),
            "default_branch" => global.default_branch.clone(),
            _ => None,
        };
        if let Some(v) = global_value {
            return found(v, ConfigSource::Global);
        }
    }
    Ok(None)
}

/// Loads `<plan_root>/rdm.toml` and resolves a project-scopable `key` for
/// `project` through [`resolve_scoped_value`], reading the process
/// environment.
///
/// The entry point for callers that hold a plan root but no already-loaded
/// [`Config`], such as the HTTP server. A missing or malformed `rdm.toml`
/// counts as the default config.
///
/// # Errors
///
/// Returns the same errors as [`resolve_scoped_value`].
pub fn resolve_scoped_value_at(
    plan_root: &Path,
    global: &GlobalConfig,
    key: &str,
    project: Option<&str>,
) -> Result<Option<ResolvedValue<String>>> {
    resolve_scoped_value_at_with_env(plan_root, global, key, project, |k| std::env::var(k).ok())
}

fn resolve_scoped_value_at_with_env(
    plan_root: &Path,
    global: &GlobalConfig,
    key: &str,
    project: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<ResolvedValue<String>>> {
    let config = std::fs::read_to_string(plan_root.join(REPO_CONFIG_FILE))
        .ok()
        .and_then(|c| Config::from_toml(&c).ok())
        .unwrap_or_default();
    resolve_scoped_value(key, project, &config, global, env)
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
    parse_bool_env("RDM_PLAN_REVIEW", value)
}

/// Parses a boolean env var override that accepts only the literal `"true"`
/// or `"false"`, naming `var` in the error.
fn parse_bool_env(var: &str, value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(Error::InvalidConfigValue {
            key: var.to_string(),
            value: other.to_string(),
            valid: "true or false".to_string(),
        }),
    }
}

/// Parses the `RDM_REVIEWED_GATE` env var into a boolean.
///
/// Like [`parse_plan_review_env`], this is a loud override rather than a fuzzy
/// boolean parse, so a typo (`"1"`, `"yes"`, `"True"`, an empty string)
/// surfaces as an error instead of silently resolving to `false` and quietly
/// disabling the gate.
///
/// # Errors
///
/// Returns [`Error::InvalidConfigValue`] if `value` is anything other than
/// `"true"` or `"false"`.
pub fn parse_reviewed_gate_env(value: &str) -> Result<bool> {
    parse_bool_env("RDM_REVIEWED_GATE", value)
}

/// The name of the repo-level config file at a plan root.
pub const REPO_CONFIG_FILE: &str = "rdm.toml";

/// Resolves whether the core-enforced `reviewed` transition gate is enforcing.
///
/// This is the **single rule** every interface shares. `rdm-cli`, `rdm-server`
/// and any future front end call it rather than re-deriving the precedence,
/// so the surfaces can never disagree about when the gate is on: an operator
/// who sets `RDM_REVIEWED_GATE=true` gets the same answer from `rdm phase
/// update` and from a `PATCH /phases/{id}`.
///
/// Precedence: `RDM_REVIEWED_GATE` (passed in as `env_value`, so the rule
/// itself stays a pure function) → the config's `gates.reviewed` → `false`.
///
/// Defaulting to `false` is load-bearing: the gate is opt-in, so no existing
/// plan repo changes behavior on upgrade.
///
/// # Errors
///
/// Returns [`Error::InvalidConfigValue`] if `env_value` is anything other than
/// the literal `"true"` or `"false"` — a typo disables nothing silently.
pub fn resolve_reviewed_gate(env_value: Option<&str>, config: Option<&Config>) -> Result<bool> {
    if let Some(v) = env_value {
        return parse_reviewed_gate_env(v);
    }
    Ok(config
        .and_then(|c| c.gates.as_ref())
        .and_then(|g| g.reviewed)
        .unwrap_or(false))
}

/// Loads `<plan_root>/rdm.toml` and resolves the `reviewed`-gate flag through
/// [`resolve_reviewed_gate`], reading `RDM_REVIEWED_GATE` from the process
/// environment.
///
/// This is the entry point for callers that hold a plan root but no
/// already-merged [`Config`] — the HTTP server, which re-reads the key on each
/// mutation so an operator toggling the gate need not restart a long-lived
/// process. A missing or malformed `rdm.toml` resolves to `false` rather than
/// erroring: the gate is opt-in, and an unreadable config must never be the
/// thing that turns it on.
///
/// # Errors
///
/// Returns [`Error::InvalidConfigValue`] if `RDM_REVIEWED_GATE` is set to
/// anything other than the literal `"true"` or `"false"`.
pub fn reviewed_gate_enabled_at(plan_root: &Path) -> Result<bool> {
    let config = std::fs::read_to_string(plan_root.join(REPO_CONFIG_FILE))
        .ok()
        .and_then(|c| Config::from_toml(&c).ok());
    resolve_reviewed_gate(
        std::env::var("RDM_REVIEWED_GATE").ok().as_deref(),
        config.as_ref(),
    )
}

/// Validates that every configured profile effort is accepted by its host.
fn validate_models(models: &Option<ModelsConfig>) -> Result<()> {
    let Some(profiles) = models.as_ref().and_then(|m| m.profiles.as_ref()) else {
        return Ok(());
    };
    for host in Host::ALL {
        let Some(host_profiles) = profiles.for_host(host) else {
            continue;
        };
        for tier in [
            ModelTier::Small,
            ModelTier::Medium,
            ModelTier::Large,
            ModelTier::Frontier,
        ] {
            if let Some(effort) = host_profiles.for_tier(tier).and_then(|p| p.effort)
                && !host.valid_efforts().contains(&effort)
            {
                return Err(Error::InvalidConfigValue {
                    key: format!("models.profiles.{host}.{tier}.effort"),
                    value: effort.to_string(),
                    valid: host
                        .valid_efforts()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }
    }
    Ok(())
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
    fn parse_global_config_tolerates_retired_auto_init_key() {
        // `auto_init` backed a now-removed command and no longer exists as a
        // field. A global config file left over from before its retirement
        // must still parse rather than erroring out and bricking every
        // command.
        let toml_str = r#"
root = "/some/path"
auto_init = true
"#;
        let config = GlobalConfig::from_toml(toml_str).unwrap();
        assert_eq!(config.root, Some(PathBuf::from("/some/path")));
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
        assert_eq!(ConfigSource::Project.to_string(), "project config");
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
                profiles: None,
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
            profiles: None,
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
            profiles: None,
        };
        let global_models = ModelsConfig {
            small: Some("haiku".to_string()),
            medium: Some("sonnet".to_string()),
            large: Some("opus".to_string()),
            review_floor: Some(ModelTier::Large),
            steps: None,
            profiles: None,
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

    // --- [models.profiles] config tests ---

    #[test]
    fn parse_config_with_model_profiles() {
        let toml_str = r#"
[models]
small = "opus"
[models.profiles.claude.large]
effort = "xhigh"
[models.profiles.claude.frontier]
model = "opus"
effort = "max"
[models.profiles.codex.small]
model = "gpt-6-sol"
[models.profiles.codex.frontier]
model = "gpt-6-astra"
effort = "xhigh"
"#;
        let config = Config::from_toml(toml_str).unwrap();
        let models = config.models.expect("models parsed");
        assert_eq!(models.small, Some("opus".to_string()));
        let profiles = models.profiles.expect("profiles parsed");
        let claude = profiles.claude.expect("claude profiles");
        assert_eq!(
            claude.large,
            Some(ProfileConfig {
                model: None,
                effort: Some(Effort::Xhigh)
            })
        );
        assert_eq!(
            claude.frontier,
            Some(ProfileConfig {
                model: Some("opus".to_string()),
                effort: Some(Effort::Max)
            })
        );
        assert_eq!(claude.small, None);
        let codex = profiles.codex.expect("codex profiles");
        assert_eq!(
            codex.small,
            Some(ProfileConfig {
                model: Some("gpt-6-sol".to_string()),
                effort: None
            })
        );
        assert_eq!(
            codex.frontier,
            Some(ProfileConfig {
                model: Some("gpt-6-astra".to_string()),
                effort: Some(Effort::Xhigh)
            })
        );
        // Round trip.
        let config = Config::from_toml(toml_str).unwrap();
        let again = Config::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(again, config);
    }

    #[test]
    fn unknown_effort_rejected_at_load_naming_valid_values() {
        let toml_str = "[models.profiles.claude.small]\neffort = \"ultra\"\n";
        for err in [
            Config::from_toml(toml_str).unwrap_err(),
            GlobalConfig::from_toml(toml_str).unwrap_err(),
        ] {
            assert!(
                matches!(err, Error::ConfigParse(_)),
                "expected ConfigParse, got {err:?}"
            );
            let msg = err.to_string();
            assert!(msg.contains("ultra"), "{msg}");
            for valid in ["low", "medium", "high", "xhigh", "max"] {
                assert!(msg.contains(&format!("`{valid}`")), "{msg}");
            }
        }
    }

    #[test]
    fn codex_max_effort_rejected_naming_key_and_valid_values() {
        let toml_str = "[models.profiles.codex.small]\neffort = \"max\"\n";
        for err in [
            Config::from_toml(toml_str).unwrap_err(),
            GlobalConfig::from_toml(toml_str).unwrap_err(),
        ] {
            match err {
                Error::InvalidConfigValue { key, value, valid } => {
                    assert_eq!(key, "models.profiles.codex.small.effort");
                    assert_eq!(value, "max");
                    assert_eq!(valid, "low, medium, high, xhigh");
                }
                other => panic!("expected InvalidConfigValue, got {other:?}"),
            }
        }
    }

    #[test]
    fn claude_max_effort_accepted() {
        let toml_str = "[models.profiles.claude.frontier]\neffort = \"max\"\n";
        assert!(Config::from_toml(toml_str).is_ok());
        assert!(GlobalConfig::from_toml(toml_str).is_ok());
    }

    #[test]
    fn frontier_accepted_as_step_tier_and_review_floor() {
        let toml_str =
            "[models]\nreview_floor = \"frontier\"\n[models.steps]\nplan = \"frontier\"\n";
        let config = Config::from_toml(toml_str).unwrap();
        let models = config.models.unwrap();
        assert_eq!(models.review_floor, Some(ModelTier::Frontier));
        assert_eq!(models.steps.unwrap().plan, Some(ModelTier::Frontier));
    }
    #[test]
    fn dispatch_verify_toml_roundtrip() {
        let toml_str = "[dispatch]\nverify = \"bash scripts/ci.sh\"\n";
        let config = Config::from_toml(toml_str).unwrap();
        let dispatch = config.dispatch.clone().expect("dispatch section parsed");
        assert_eq!(dispatch.verify, Some("bash scripts/ci.sh".to_string()));
        let round = Config::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(round, config);
    }

    #[test]
    fn dispatch_table_omitted_when_unset() {
        let config = Config::default();
        let toml_str = config.to_toml().unwrap();
        assert!(
            !toml_str.contains("[dispatch]"),
            "an untouched repo config must gain no [dispatch] table: {toml_str}"
        );
    }

    #[test]
    fn dispatch_verify_is_a_known_repo_only_key() {
        assert!(KNOWN_KEYS.contains(&"dispatch.verify"));
        assert!(REPO_ONLY_KEYS.contains(&"dispatch.verify"));
        assert!(!GLOBAL_ONLY_KEYS.contains(&"dispatch.verify"));
    }

    #[test]
    fn with_global_defaults_carries_dispatch_through() {
        let repo_config = Config {
            dispatch: Some(DispatchConfig {
                verify: Some("bash scripts/ci.sh".to_string()),
            }),
            ..Default::default()
        };
        let global = GlobalConfig::default();
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(
            merged.dispatch.and_then(|d| d.verify),
            Some("bash scripts/ci.sh".to_string())
        );
        // A global config cannot supply one: an unset repo value stays unset.
        let empty = Config::default().with_global_defaults(&global);
        assert_eq!(empty.dispatch, None);
    }

    // --- gates.reviewed tests ---

    #[test]
    fn parse_reviewed_gate_env_true_and_false() {
        assert!(parse_reviewed_gate_env("true").unwrap());
        assert!(!parse_reviewed_gate_env("false").unwrap());
    }

    #[test]
    fn parse_reviewed_gate_env_invalid_rejected() {
        for bad in ["1", "yes", "True", "", "on"] {
            let err = parse_reviewed_gate_env(bad).unwrap_err();
            assert!(
                err.to_string().contains("RDM_REVIEWED_GATE"),
                "expected the key named in: {err}"
            );
        }
    }

    #[test]
    fn gates_reviewed_toml_roundtrip() {
        let toml_str = "[gates]\nreviewed = true\n";
        let config = Config::from_toml(toml_str).unwrap();
        let gates = config.gates.clone().expect("gates section parsed");
        assert_eq!(gates.reviewed, Some(true));
        let back = config.to_toml().unwrap();
        assert!(back.contains("[gates]"), "round-tripped: {back}");
        assert!(back.contains("reviewed = true"), "round-tripped: {back}");
    }

    #[test]
    fn gates_table_omitted_when_unset() {
        let config = Config::default();
        let toml_str = config.to_toml().unwrap();
        assert!(
            !toml_str.contains("[gates]"),
            "an unset gates table must not be serialized: {toml_str}"
        );
    }

    #[test]
    fn gates_is_repo_only_and_survives_global_merge() {
        let repo_config = Config {
            gates: Some(GatesConfig {
                reviewed: Some(true),
            }),
            ..Default::default()
        };
        let global = GlobalConfig::default();
        let merged = repo_config.with_global_defaults(&global);
        assert_eq!(
            merged.gates.and_then(|g| g.reviewed),
            Some(true),
            "the repo-only gates table must survive the global merge unchanged"
        );
        assert!(REPO_ONLY_KEYS.contains(&"gates.reviewed"));
        assert!(KNOWN_KEYS.contains(&"gates.reviewed"));
    }

    // --- the shared resolution rule every interface calls ---

    fn gates_config(reviewed: Option<bool>) -> Config {
        Config {
            gates: Some(GatesConfig { reviewed }),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_reviewed_gate_env_wins_over_config_in_both_directions() {
        let on = gates_config(Some(true));
        let off = gates_config(Some(false));
        assert!(!resolve_reviewed_gate(Some("false"), Some(&on)).unwrap());
        assert!(resolve_reviewed_gate(Some("true"), Some(&off)).unwrap());
    }

    #[test]
    fn resolve_reviewed_gate_falls_back_to_config_then_false() {
        assert!(resolve_reviewed_gate(None, Some(&gates_config(Some(true)))).unwrap());
        assert!(!resolve_reviewed_gate(None, Some(&gates_config(Some(false)))).unwrap());
        assert!(!resolve_reviewed_gate(None, Some(&gates_config(None))).unwrap());
        assert!(!resolve_reviewed_gate(None, Some(&Config::default())).unwrap());
        // No config at all — the server's missing-`rdm.toml` case.
        assert!(!resolve_reviewed_gate(None, None).unwrap());
    }

    #[test]
    fn resolve_reviewed_gate_rejects_a_junk_env_value() {
        let err = resolve_reviewed_gate(Some("yes"), Some(&gates_config(Some(true)))).unwrap_err();
        assert!(
            err.to_string().contains("RDM_REVIEWED_GATE"),
            "expected the key named in: {err}"
        );
    }

    #[test]
    fn reviewed_gate_enabled_at_reads_the_repo_config() {
        let dir = tempfile::tempdir().unwrap();
        // No rdm.toml at all — opt-in means off, never an error.
        assert!(!reviewed_gate_enabled_at(dir.path()).unwrap());

        std::fs::write(
            dir.path().join(REPO_CONFIG_FILE),
            "[gates]\nreviewed = true\n",
        )
        .unwrap();
        assert!(reviewed_gate_enabled_at(dir.path()).unwrap());

        std::fs::write(
            dir.path().join(REPO_CONFIG_FILE),
            "[gates]\nreviewed = false\n",
        )
        .unwrap();
        assert!(!reviewed_gate_enabled_at(dir.path()).unwrap());
    }

    #[test]
    fn reviewed_gate_enabled_at_never_enables_on_a_malformed_config() {
        // An unreadable config must not be the thing that turns a gate on.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(REPO_CONFIG_FILE),
            "this is not = = toml [[[",
        )
        .unwrap();
        assert!(!reviewed_gate_enabled_at(dir.path()).unwrap());
    }

    // --- [projects.<name>] override layer ---

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    /// A repo config with `value` set plan-repo-wide for every scopable key,
    /// and `[projects.a]` overriding every one of them with `a-value`.
    fn layered_config() -> Config {
        Config::from_toml(
            r#"
default_branch = "repo-branch"
plan_review = false

[dispatch]
verify = "repo-verify"

[gates]
reviewed = false

[projects.a]
default_branch = "a-branch"
plan_review = true

[projects.a.dispatch]
verify = "a-verify"

[projects.a.gates]
reviewed = true
"#,
        )
        .unwrap()
    }

    fn global_with_scopables() -> GlobalConfig {
        GlobalConfig {
            default_branch: Some("global-branch".to_string()),
            plan_review: Some(true),
            ..Default::default()
        }
    }

    fn resolved(value: &str, source: ConfigSource) -> Option<ResolvedValue<String>> {
        Some(ResolvedValue {
            value: value.to_string(),
            source,
        })
    }

    #[test]
    fn project_scopable_keys_are_the_four_known_keys() {
        assert_eq!(
            PROJECT_SCOPABLE_KEYS,
            &[
                "dispatch.verify",
                "gates.reviewed",
                "plan_review",
                "default_branch"
            ]
        );
        for key in PROJECT_SCOPABLE_KEYS {
            assert!(KNOWN_KEYS.contains(key), "{key} must be a known key");
        }
    }

    #[test]
    fn projects_table_round_trips() {
        let config = layered_config();
        let a = config.projects.get("a").expect("[projects.a] parsed");
        assert_eq!(
            a.dispatch.as_ref().and_then(|d| d.verify.as_deref()),
            Some("a-verify")
        );
        let back = Config::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn config_without_projects_serializes_no_projects_table() {
        let config = Config {
            default_project: Some("fbm".to_string()),
            ..Default::default()
        };
        let toml_str = config.to_toml().unwrap();
        assert!(
            !toml_str.contains("projects"),
            "an untouched repo config must gain no [projects] table: {toml_str}"
        );
    }

    #[test]
    fn projects_survive_the_global_merge_unchanged() {
        let config = layered_config();
        let merged = config.with_global_defaults(&global_with_scopables());
        assert_eq!(merged.projects, config.projects);
    }

    #[test]
    fn scopable_value_reads_each_key_in_the_config_get_string_form() {
        let config = layered_config();
        assert_eq!(
            config.scopable_value("dispatch.verify").as_deref(),
            Some("repo-verify")
        );
        assert_eq!(
            config.scopable_value("gates.reviewed").as_deref(),
            Some("false")
        );
        assert_eq!(
            config.scopable_value("plan_review").as_deref(),
            Some("false")
        );
        assert_eq!(
            config.scopable_value("default_branch").as_deref(),
            Some("repo-branch")
        );
        assert_eq!(config.scopable_value("remote.default"), None);
        let a = &config.projects["a"];
        assert_eq!(a.value("dispatch.verify").as_deref(), Some("a-verify"));
        assert_eq!(a.value("gates.reviewed").as_deref(), Some("true"));
        assert_eq!(a.value("plan_review").as_deref(), Some("true"));
        assert_eq!(a.value("default_branch").as_deref(), Some("a-branch"));
        assert_eq!(ProjectOverrides::default().value("plan_review"), None);
    }

    #[test]
    fn resolver_env_beats_project_for_every_key() {
        let config = layered_config();
        let global = global_with_scopables();
        let env = env_of(&[
            ("RDM_DISPATCH_VERIFY", "env-verify"),
            ("RDM_GATES_REVIEWED", "false"),
            ("RDM_PLAN_REVIEW", "false"),
            ("RDM_DEFAULT_BRANCH", "env-branch"),
        ]);
        for (key, want) in [
            ("dispatch.verify", "env-verify"),
            ("gates.reviewed", "false"),
            ("plan_review", "false"),
            ("default_branch", "env-branch"),
        ] {
            assert_eq!(
                resolve_scoped_value(key, Some("a"), &config, &global, &env).unwrap(),
                resolved(want, ConfigSource::Env),
                "{key}"
            );
        }
    }

    #[test]
    fn resolver_treats_a_blank_verify_env_as_unset() {
        let config = layered_config();
        let global = global_with_scopables();
        for blank in ["", "  \t "] {
            let env = move |k: &str| (k == "RDM_DISPATCH_VERIFY").then(|| blank.to_string());
            assert_eq!(
                resolve_scoped_value("dispatch.verify", Some("a"), &config, &global, env).unwrap(),
                resolved("a-verify", ConfigSource::Project),
                "{blank:?}"
            );
            assert_eq!(
                resolve_scoped_value("dispatch.verify", None, &Config::default(), &global, env)
                    .unwrap(),
                None,
                "{blank:?}"
            );
        }
    }

    #[test]
    fn resolver_treats_a_blank_verify_as_unset_at_every_layer() {
        let global = global_with_scopables();
        let blank_env = |k: &str| (k == "RDM_DISPATCH_VERIFY").then(|| "  ".to_string());
        let toml =
            "[dispatch]\nverify = \"  echo repo  \"\n[projects.a.dispatch]\nverify = \"  \"\n";
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(
            resolve_scoped_value("dispatch.verify", Some("a"), &config, &global, blank_env)
                .unwrap(),
            resolved("echo repo", ConfigSource::Repo)
        );
        let blank_repo = Config::from_toml("[dispatch]\nverify = \" \t \"\n").unwrap();
        assert_eq!(
            resolve_scoped_value("dispatch.verify", None, &blank_repo, &global, no_env).unwrap(),
            None
        );
        let all_blank =
            Config::from_toml("[dispatch]\nverify = \"\"\n[projects.a.dispatch]\nverify = \" \"\n")
                .unwrap();
        assert_eq!(
            resolve_scoped_value("dispatch.verify", Some("a"), &all_blank, &global, blank_env)
                .unwrap(),
            None
        );
    }

    #[test]
    fn resolver_project_beats_repo_for_every_key() {
        let config = layered_config();
        let global = global_with_scopables();
        for (key, want) in [
            ("dispatch.verify", "a-verify"),
            ("gates.reviewed", "true"),
            ("plan_review", "true"),
            ("default_branch", "a-branch"),
        ] {
            assert_eq!(
                resolve_scoped_value(key, Some("a"), &config, &global, no_env).unwrap(),
                resolved(want, ConfigSource::Project),
                "{key}"
            );
        }
    }

    #[test]
    fn resolver_unknown_project_and_no_project_fall_through_to_repo() {
        let config = layered_config();
        let global = global_with_scopables();
        for project in [Some("b"), None] {
            for (key, want) in [
                ("dispatch.verify", "repo-verify"),
                ("gates.reviewed", "false"),
                ("plan_review", "false"),
                ("default_branch", "repo-branch"),
            ] {
                assert_eq!(
                    resolve_scoped_value(key, project, &config, &global, no_env).unwrap(),
                    resolved(want, ConfigSource::Repo),
                    "{key} for {project:?}"
                );
            }
        }
    }

    #[test]
    fn resolver_falls_back_to_global_only_where_the_key_allows_it() {
        let config = Config::default();
        let global = global_with_scopables();
        assert_eq!(
            resolve_scoped_value("plan_review", Some("a"), &config, &global, no_env).unwrap(),
            resolved("true", ConfigSource::Global)
        );
        assert_eq!(
            resolve_scoped_value("default_branch", Some("a"), &config, &global, no_env).unwrap(),
            resolved("global-branch", ConfigSource::Global)
        );
        for key in ["dispatch.verify", "gates.reviewed"] {
            assert_eq!(
                resolve_scoped_value(key, Some("a"), &config, &global, no_env).unwrap(),
                None,
                "{key} is repo-only and must never resolve from global config"
            );
        }
    }

    #[test]
    fn resolver_honors_rdm_reviewed_gate_for_gates_reviewed() {
        let config = layered_config();
        let global = GlobalConfig::default();
        assert_eq!(
            resolve_scoped_value(
                "gates.reviewed",
                Some("a"),
                &config,
                &global,
                env_of(&[("RDM_REVIEWED_GATE", "false")])
            )
            .unwrap(),
            resolved("false", ConfigSource::Env)
        );
        // RDM_REVIEWED_GATE — the variable the gate itself honors — wins
        // when both are set, so the resolver agrees with the gate.
        const GATE_ON: &[(&str, &str)] = &[
            ("RDM_REVIEWED_GATE", "true"),
            ("RDM_GATES_REVIEWED", "false"),
        ];
        const GATE_OFF: &[(&str, &str)] = &[
            ("RDM_REVIEWED_GATE", "false"),
            ("RDM_GATES_REVIEWED", "true"),
        ];
        for (pairs, gate) in [(GATE_ON, "true"), (GATE_OFF, "false")] {
            let both = env_of(pairs);
            let gate_decision =
                resolve_reviewed_gate(both("RDM_REVIEWED_GATE").as_deref(), Some(&config)).unwrap();
            assert_eq!(
                resolve_scoped_value("gates.reviewed", None, &config, &global, &both).unwrap(),
                resolved(&gate_decision.to_string(), ConfigSource::Env)
            );
            assert_eq!(gate_decision.to_string(), gate);
        }
        // RDM_REVIEWED_GATE applies to gates.reviewed only.
        assert_eq!(
            resolve_scoped_value(
                "plan_review",
                None,
                &config,
                &global,
                env_of(&[("RDM_REVIEWED_GATE", "true")])
            )
            .unwrap(),
            resolved("false", ConfigSource::Repo)
        );
    }

    #[test]
    fn resolver_rejects_an_invalid_rdm_reviewed_gate() {
        let err = resolve_scoped_value(
            "gates.reviewed",
            None,
            &Config::default(),
            &GlobalConfig::default(),
            env_of(&[("RDM_REVIEWED_GATE", "yes")]),
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::InvalidConfigValue { key, .. } if key == "RDM_REVIEWED_GATE"),
            "got {err:?}"
        );
    }

    #[test]
    fn resolver_rejects_an_invalid_generic_boolean_env_value() {
        for (key, var) in [
            ("gates.reviewed", "RDM_GATES_REVIEWED"),
            ("plan_review", "RDM_PLAN_REVIEW"),
        ] {
            for bad in ["yes", "True", "1", ""] {
                let err = resolve_scoped_value(
                    key,
                    None,
                    &Config::default(),
                    &GlobalConfig::default(),
                    |k: &str| (k == var).then(|| bad.to_string()),
                )
                .unwrap_err();
                assert!(
                    matches!(&err, Error::InvalidConfigValue { key, value, .. }
                        if key == var && value == bad),
                    "{var}={bad:?}: got {err:?}"
                );
            }
        }
    }

    #[test]
    fn resolver_refuses_a_non_scopable_key_naming_the_scopable_ones() {
        let err = resolve_scoped_value(
            "remote.default",
            Some("a"),
            &Config::default(),
            &GlobalConfig::default(),
            no_env,
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::KeyNotProjectScopable { key } if key == "remote.default"),
            "got {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("remote.default"), "{msg}");
        for key in PROJECT_SCOPABLE_KEYS {
            assert!(msg.contains(key), "{key} missing from: {msg}");
        }
    }

    #[test]
    fn resolve_scoped_value_at_reads_the_plan_root() {
        let dir = tempfile::tempdir().unwrap();
        let global = GlobalConfig::default();
        // A missing rdm.toml resolves like a default config.
        assert_eq!(
            resolve_scoped_value_at_with_env(
                dir.path(),
                &global,
                "dispatch.verify",
                Some("a"),
                no_env
            )
            .unwrap(),
            None
        );
        std::fs::write(
            dir.path().join(REPO_CONFIG_FILE),
            layered_config().to_toml().unwrap(),
        )
        .unwrap();
        for project in [Some("a"), Some("b"), None] {
            for key in PROJECT_SCOPABLE_KEYS {
                assert_eq!(
                    resolve_scoped_value_at_with_env(dir.path(), &global, key, project, no_env)
                        .unwrap(),
                    resolve_scoped_value(key, project, &layered_config(), &global, no_env).unwrap(),
                    "{key} for {project:?}"
                );
            }
        }
        // A malformed rdm.toml also counts as default.
        std::fs::write(dir.path().join(REPO_CONFIG_FILE), "not = = toml [[[").unwrap();
        assert_eq!(
            resolve_scoped_value_at_with_env(
                dir.path(),
                &global,
                "dispatch.verify",
                Some("a"),
                no_env
            )
            .unwrap(),
            None
        );
    }
}
