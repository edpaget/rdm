//! Model-tier sizing policy resolution for dispatch steps.
//!
//! This is the single source of truth for turning a dispatch step (plus an
//! optional caller tier hint) into a concrete
//! [`Profile`](crate::model_policy::Profile) — a model id and a reasoning
//! [`Effort`](crate::model::Effort) — for a given
//! [`Host`](crate::model_policy::Host), applying the `[models]` policy from
//! [`crate::config`] with the built-in
//! [`default_profile`](crate::model_policy::default_profile) table layered
//! underneath. CLI and skill consumers should call
//! [`ModelPolicy::resolve`](crate::model_policy::ModelPolicy::resolve) rather
//! than re-implementing this table.
//!
//! Resolution runs in two stages:
//! [`ModelPolicy::resolve_tier`](crate::model_policy::ModelPolicy::resolve_tier)
//! picks a [`ModelTier`](crate::model::ModelTier) (caller hint →
//! `[models.steps]` → step default, then a per-step floor), and
//! [`ModelPolicy::profile`](crate::model_policy::ModelPolicy::profile) maps
//! that tier to the host's configured-or-default profile. No built-in default
//! ever resolves to [`ModelTier::Frontier`](crate::model::ModelTier::Frontier).
//!
//! A policy may also be built for a named **preset**
//! ([`ModelPolicy::for_preset`](crate::model_policy::ModelPolicy::for_preset)):
//! a sparse overlay that changes what a tier or a role *runs on*, never which
//! tier is picked. Each field (model, effort) of the profile a step resolves
//! to is layered independently: the preset's role (step) override, else the
//! preset's override for the resolved tier, else the configured-or-default
//! profile. Presets come from config (`[models.presets.<name>]`) or are built
//! in ([`BUILTIN_PRESETS`](crate::model_policy::BUILTIN_PRESETS)); a
//! configured preset replaces a built-in one of the same name.
//! [`select_preset`](crate::model_policy::select_preset) picks the preset for
//! one run: the `--preset` / `--no-preset` flag, else `RDM_MODEL_PRESET`, else
//! the `[models] preset` key, else none.

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

use crate::config::{
    Config, PresetConfig, PresetHostConfig, ProfileConfig, StepProfilesConfig, StepTiersConfig,
};
use crate::error::Error;
use crate::model::{Effort, ModelTier, ParseError};

/// Built-in minimum tier review steps may run on when `[models]` does not
/// override it.
pub const DEFAULT_REVIEW_FLOOR: ModelTier = ModelTier::Medium;

/// Built-in, non-configurable minimum tier the [`DispatchStep::Plan`] step
/// runs on, so the planner is never sized below the medium tier whatever the
/// item's tier.
pub const PLAN_FLOOR: ModelTier = ModelTier::Medium;

/// Every tier, smallest first.
const ALL_TIERS: [ModelTier; 4] = [
    ModelTier::Small,
    ModelTier::Medium,
    ModelTier::Large,
    ModelTier::Frontier,
];

/// An agent host whose runtime runs a dispatched model.
///
/// Each host has its own model ids, its own accepted effort levels
/// ([`Host::valid_efforts`]), and its own four-tier profile table
/// ([`default_profile`]). Tiers are not aligned across hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Host {
    /// Claude Code (`claude --model <id> --effort <effort>`). The default.
    #[default]
    Claude,
    /// The Codex CLI (`codex -m <id> -c model_reasoning_effort=<effort>`).
    Codex,
}

impl Host {
    /// Every host, in canonical order.
    pub const ALL: [Host; 2] = [Host::Claude, Host::Codex];

    /// Returns the effort levels this host's runtime accepts.
    ///
    /// `claude` accepts all five [`Effort`] levels. `codex` accepts up to
    /// `xhigh`: the shipped Codex runtime's process guard refuses `max`, and
    /// rdm must never emit an effort its runtime will not run.
    #[must_use]
    pub fn valid_efforts(self) -> &'static [Effort] {
        match self {
            Host::Claude => &[
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::Xhigh,
                Effort::Max,
            ],
            Host::Codex => &[Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh],
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Claude => write!(f, "claude"),
            Host::Codex => write!(f, "codex"),
        }
    }
}

impl FromStr for Host {
    type Err = ParseError;

    /// Parses a lowercase host name.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] if `s` is not `claude` or `codex`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" => Ok(Host::Claude),
            "codex" => Ok(Host::Codex),
            other => Err(ParseError::new("host", other, "claude or codex")),
        }
    }
}

/// A resolved model id plus the reasoning effort to run it at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Profile {
    /// Concrete model id passed to the host runtime.
    pub model: String,
    /// Reasoning effort, always one of the host's [`Host::valid_efforts`].
    pub effort: Effort,
}

/// Returns the built-in `(model, effort)` profile for `host` at `tier`.
///
/// | Tier | claude | codex |
/// |---|---|---|
/// | small | opus @ low | gpt-6-sol @ medium |
/// | medium | opus @ medium | gpt-6-sol @ high |
/// | large | opus @ high | gpt-6-astra @ medium |
/// | frontier | opus @ xhigh | gpt-6-astra @ xhigh |
///
/// See `docs/model-profiles.md` for the rationale and the Codex id
/// confirmation.
#[must_use]
pub fn default_profile(host: Host, tier: ModelTier) -> (&'static str, Effort) {
    match (host, tier) {
        (Host::Claude, ModelTier::Small) => ("opus", Effort::Low),
        (Host::Claude, ModelTier::Medium) => ("opus", Effort::Medium),
        (Host::Claude, ModelTier::Large) => ("opus", Effort::High),
        (Host::Claude, ModelTier::Frontier) => ("opus", Effort::Xhigh),
        (Host::Codex, ModelTier::Small) => ("gpt-6-sol", Effort::Medium),
        (Host::Codex, ModelTier::Medium) => ("gpt-6-sol", Effort::High),
        (Host::Codex, ModelTier::Large) => ("gpt-6-astra", Effort::Medium),
        (Host::Codex, ModelTier::Frontier) => ("gpt-6-astra", Effort::Xhigh),
    }
}

/// A step in the agentic dispatch pipeline that requires a sized model.
///
/// Each variant carries its own built-in default tier (see
/// [`DispatchStep::default_tier`]) and floor (see [`DispatchStep::floor`]);
/// [`ModelPolicy`] resolves the two alongside `[models]` config and an
/// optional caller hint into a concrete [`Profile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchStep {
    /// Planning a phase or task before implementation begins.
    Plan,
    /// Implementing the planned work.
    Implement,
    /// Finding issues during code review (reasoning-heavy).
    ReviewFind,
    /// Verifying found issues are real before reporting them (reasoning-heavy).
    ReviewVerify,
    /// Clustering duplicate review findings before refutation (reasoning-heavy:
    /// a false merge drops a real defect, so precision outranks cost).
    ReviewConsolidate,
    /// Mechanical, non-judgment work (e.g. formatting, rote transforms).
    Mechanical,
}

impl DispatchStep {
    /// Every dispatch step, in pipeline order.
    pub const ALL: [DispatchStep; 6] = [
        DispatchStep::Plan,
        DispatchStep::Implement,
        DispatchStep::ReviewFind,
        DispatchStep::ReviewVerify,
        DispatchStep::ReviewConsolidate,
        DispatchStep::Mechanical,
    ];

    /// Returns this step's built-in default tier, used when neither a caller
    /// hint nor a `[models.steps]` override is present.
    #[must_use]
    pub fn default_tier(&self) -> ModelTier {
        match self {
            DispatchStep::Plan => ModelTier::Medium,
            DispatchStep::Implement => ModelTier::Medium,
            DispatchStep::ReviewFind => ModelTier::Medium,
            DispatchStep::ReviewVerify => ModelTier::Large,
            DispatchStep::ReviewConsolidate => ModelTier::Large,
            DispatchStep::Mechanical => ModelTier::Small,
        }
    }

    /// Returns the minimum tier this step's resolved tier is clamped up to,
    /// if any.
    ///
    /// `Plan` is floored at [`PLAN_FLOOR`] (planner ≥ implementer);
    /// `ReviewFind`, `ReviewVerify` and `ReviewConsolidate` at the configured
    /// `review_floor`;
    /// `Implement` follows the item tier and `Mechanical` is exempt so rote
    /// work can still run cheap.
    #[must_use]
    pub fn floor(&self, review_floor: ModelTier) -> Option<ModelTier> {
        match self {
            DispatchStep::Plan => Some(PLAN_FLOOR),
            DispatchStep::ReviewFind
            | DispatchStep::ReviewVerify
            | DispatchStep::ReviewConsolidate => Some(review_floor),
            DispatchStep::Implement | DispatchStep::Mechanical => None,
        }
    }
}

impl fmt::Display for DispatchStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DispatchStep::Plan => write!(f, "plan"),
            DispatchStep::Implement => write!(f, "implement"),
            DispatchStep::ReviewFind => write!(f, "review-find"),
            DispatchStep::ReviewVerify => write!(f, "review-verify"),
            DispatchStep::ReviewConsolidate => write!(f, "review-consolidate"),
            DispatchStep::Mechanical => write!(f, "mechanical"),
        }
    }
}

impl FromStr for DispatchStep {
    type Err = ParseError;

    /// Parses a kebab-case dispatch step name.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] if `s` is not one of `plan`, `implement`,
    /// `review-find`, `review-verify`, `review-consolidate`, or `mechanical`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "plan" => Ok(DispatchStep::Plan),
            "implement" => Ok(DispatchStep::Implement),
            "review-find" => Ok(DispatchStep::ReviewFind),
            "review-verify" => Ok(DispatchStep::ReviewVerify),
            "review-consolidate" => Ok(DispatchStep::ReviewConsolidate),
            "mechanical" => Ok(DispatchStep::Mechanical),
            other => Err(ParseError::new(
                "dispatch step",
                other,
                "plan, implement, review-find, review-verify, review-consolidate, or mechanical",
            )),
        }
    }
}

/// The Claude model id the built-in presets put roles on.
const FABLE_MODEL: &str = "fable";

/// The names of the presets built into core, sorted.
///
/// - `fable`: on the `claude` host, `plan`, `implement`, `review-find`,
///   `review-verify` and `review-consolidate` run on Fable at the effort of
///   the tier they resolve to; `mechanical` is left on the profile table.
/// - `fable-build`: on the `claude` host, `plan` and `implement` run on
///   Fable; the review roles are left on the profile table.
///
/// Neither touches the `codex` host.
pub const BUILTIN_PRESETS: [&str; 2] = ["fable", "fable-build"];

/// Returns the built-in preset named `name`, or `None` if core ships no
/// preset of that name (see [`BUILTIN_PRESETS`]).
///
/// This ignores config: a configured `[models.presets.<name>]` table of the
/// same name takes its place in [`ModelPolicy::for_preset`].
#[must_use]
pub fn builtin_preset(name: &str) -> Option<PresetConfig> {
    let fable = || {
        Some(ProfileConfig {
            model: Some(FABLE_MODEL.to_string()),
            effort: None,
        })
    };
    let steps = match name {
        "fable" => StepProfilesConfig {
            plan: fable(),
            implement: fable(),
            review_find: fable(),
            review_verify: fable(),
            review_consolidate: fable(),
            mechanical: None,
        },
        "fable-build" => StepProfilesConfig {
            plan: fable(),
            implement: fable(),
            ..Default::default()
        },
        _ => return None,
    };
    Some(PresetConfig {
        claude: Some(PresetHostConfig {
            tiers: None,
            steps: Some(steps),
        }),
        codex: None,
    })
}

/// Returns every preset name available under `config`: the built-in presets
/// ([`BUILTIN_PRESETS`]) and the configured `[models.presets.<name>]` tables,
/// sorted and deduplicated.
#[must_use]
pub fn available_presets(config: &Config) -> Vec<String> {
    let mut names: Vec<String> = BUILTIN_PRESETS.iter().map(ToString::to_string).collect();
    if let Some(models) = config.models.as_ref() {
        names.extend(models.presets.keys().cloned());
    }
    names.sort();
    names.dedup();
    names
}

/// The environment variable that selects a model preset for one run when no
/// `--preset` / `--no-preset` flag is given (see [`select_preset`]).
pub const MODEL_PRESET_ENV: &str = "RDM_MODEL_PRESET";

/// Where a selected model preset name came from (see [`select_preset`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetSource {
    /// The `--preset <name>` command-line flag.
    Flag,
    /// The [`MODEL_PRESET_ENV`] (`RDM_MODEL_PRESET`) environment variable.
    Env,
    /// The `preset` key of the `[models]` config table.
    Config,
}

impl PresetSource {
    /// Returns a one-word label for this source (`flag`, `env` or
    /// `config`), for compact text output such as `rdm model show`'s
    /// `preset:` line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PresetSource::Flag => "flag",
            PresetSource::Env => "env",
            PresetSource::Config => "config",
        }
    }
}

impl fmt::Display for PresetSource {
    /// Names the exact origin: `--preset flag`, `RDM_MODEL_PRESET
    /// environment variable` or `[models] preset config key`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresetSource::Flag => write!(f, "--preset flag"),
            PresetSource::Env => write!(f, "{MODEL_PRESET_ENV} environment variable"),
            PresetSource::Config => write!(f, "[models] preset config key"),
        }
    }
}

/// The command-line layer of a preset selection: what the `--preset` /
/// `--no-preset` flags asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetRequest<'a> {
    /// Neither flag was given: fall through to the env var, then config.
    Unspecified,
    /// `--preset <name>`: use this preset, whatever the env var or config say.
    Named(&'a str),
    /// `--no-preset`: use no preset, whatever the env var or config say.
    NoPreset,
}

/// A preset chosen by [`select_preset`], with the layer that chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPreset {
    /// The preset name, verbatim as its source gave it.
    pub name: String,
    /// The layer the name came from.
    pub source: PresetSource,
}

/// Selects the model preset for one run.
///
/// Precedence: [`PresetRequest::NoPreset`] selects no preset without reading
/// the env var or config; else a [`PresetRequest::Named`] flag; else a
/// non-blank [`MODEL_PRESET_ENV`] value, read through `env` (a set but
/// whitespace-only value counts as unset); else the `[models] preset` config
/// key; else no preset. Only the winning layer is checked, so a bad lower
/// layer never fails a run a higher layer decides.
///
/// # Errors
///
/// Returns [`Error::UnknownModelPreset`], carrying the winning layer as its
/// `source` and listing [`available_presets`], if the selected name is
/// neither a built-in nor a configured preset.
///
/// # Examples
///
/// ```
/// use rdm_core::config::Config;
/// use rdm_core::model_policy::{PresetRequest, PresetSource, select_preset};
///
/// let env = |k: &str| (k == "RDM_MODEL_PRESET").then(|| "fable".to_string());
/// let flag = select_preset(PresetRequest::Named("fable-build"), &Config::default(), env)
///     .unwrap()
///     .unwrap();
/// assert_eq!((flag.name.as_str(), flag.source), ("fable-build", PresetSource::Flag));
///
/// let from_env = select_preset(PresetRequest::Unspecified, &Config::default(), env)
///     .unwrap()
///     .unwrap();
/// assert_eq!((from_env.name.as_str(), from_env.source), ("fable", PresetSource::Env));
/// ```
pub fn select_preset(
    request: PresetRequest<'_>,
    config: &Config,
    env: impl Fn(&str) -> Option<String>,
) -> crate::error::Result<Option<SelectedPreset>> {
    let picked = match request {
        PresetRequest::NoPreset => return Ok(None),
        PresetRequest::Named(name) => Some((name.to_string(), PresetSource::Flag)),
        PresetRequest::Unspecified => env(MODEL_PRESET_ENV)
            .filter(|v| !v.trim().is_empty())
            .map(|v| (v, PresetSource::Env))
            .or_else(|| {
                config
                    .models
                    .as_ref()
                    .and_then(|m| m.preset.clone())
                    .map(|v| (v, PresetSource::Config))
            }),
    };
    let Some((name, source)) = picked else {
        return Ok(None);
    };
    let available = available_presets(config);
    if !available.contains(&name) {
        return Err(Error::UnknownModelPreset {
            name,
            source: Some(source),
            available,
        });
    }
    Ok(Some(SelectedPreset { name, source }))
}

/// Resolves a [`DispatchStep`] (plus an optional caller tier hint) to a
/// concrete [`Profile`] for a [`Host`].
///
/// Built from the `[models]` table of a [`Config`] via
/// [`ModelPolicy::from_config`], filling every unset field with a built-in
/// default so callers never need to special-case an absent `[models]` table,
/// or via [`ModelPolicy::for_preset`] with a named preset layered on top.
///
/// # Examples
///
/// ```
/// use rdm_core::config::Config;
/// use rdm_core::model::{Effort, ModelTier};
/// use rdm_core::model_policy::{DispatchStep, Host, ModelPolicy};
///
/// let policy = ModelPolicy::from_config(&Config::default());
/// let p = policy.resolve(DispatchStep::Implement, Some(ModelTier::Small), Host::Claude);
/// assert_eq!((p.model.as_str(), p.effort), ("opus", Effort::Low));
/// // The plan step is floored up to at least the medium tier.
/// let p = policy.resolve(DispatchStep::Plan, Some(ModelTier::Small), Host::Codex);
/// assert_eq!((p.model.as_str(), p.effort), ("gpt-6-sol", Effort::High));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPolicy {
    /// Tier-layer profiles (configured or default, with the preset's tier
    /// overrides), indexed `[host][tier]` in [`Host::ALL`] / [`ALL_TIERS`]
    /// order.
    profiles: [[Profile; 4]; 2],
    /// Fully layered profiles (the preset's role overrides over
    /// `profiles`), indexed `[host][step][tier]` in [`Host::ALL`] /
    /// [`DispatchStep::ALL`] / [`ALL_TIERS`] order.
    resolved: [[[Profile; 4]; 6]; 2],
    review_floor: ModelTier,
    steps: StepTiersConfig,
}

fn host_index(host: Host) -> usize {
    match host {
        Host::Claude => 0,
        Host::Codex => 1,
    }
}

fn tier_index(tier: ModelTier) -> usize {
    match tier {
        ModelTier::Small => 0,
        ModelTier::Medium => 1,
        ModelTier::Large => 2,
        ModelTier::Frontier => 3,
    }
}

fn step_index(step: DispatchStep) -> usize {
    match step {
        DispatchStep::Plan => 0,
        DispatchStep::Implement => 1,
        DispatchStep::ReviewFind => 2,
        DispatchStep::ReviewVerify => 3,
        DispatchStep::ReviewConsolidate => 4,
        DispatchStep::Mechanical => 5,
    }
}

/// Applies `overlay`'s set fields over `base`, field by field.
fn layer(base: &Profile, overlay: Option<&ProfileConfig>) -> Profile {
    Profile {
        model: overlay
            .and_then(|o| o.model.clone())
            .unwrap_or_else(|| base.model.clone()),
        effort: overlay.and_then(|o| o.effort).unwrap_or(base.effort),
    }
}

impl ModelPolicy {
    /// Builds a `ModelPolicy` from a repo [`Config`], filling every field the
    /// `[models]` table leaves unset with its built-in default.
    ///
    /// Per host and tier, the model is the profile `model`, else (`claude`
    /// only) the legacy `[models] small/medium/large` key, else the built-in
    /// table; the effort is the profile `effort`, else the built-in table.
    /// Efforts are validated when the config is loaded
    /// ([`Config::validate`]), so this never fails.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self::build(config, None)
    }

    /// Builds a `ModelPolicy` from a repo [`Config`] with the named preset
    /// layered over the profile table, or with no preset when `preset` is
    /// `None` (identical to [`ModelPolicy::from_config`]).
    ///
    /// A configured `[models.presets.<name>]` table is used when present —
    /// replacing, not merging with, a built-in preset of the same name —
    /// else the built-in preset ([`builtin_preset`]). Tier resolution
    /// ([`ModelPolicy::resolve_tier`]) is unaffected by a preset.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownModelPreset`], listing
    /// [`available_presets`], if `preset` names neither a configured nor a
    /// built-in preset.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_core::config::Config;
    /// use rdm_core::model_policy::{DispatchStep, Host, ModelPolicy};
    ///
    /// let policy = ModelPolicy::for_preset(&Config::default(), Some("fable")).unwrap();
    /// let p = policy.resolve(DispatchStep::Implement, None, Host::Claude);
    /// assert_eq!(p.model, "fable");
    /// ```
    pub fn for_preset(config: &Config, preset: Option<&str>) -> crate::error::Result<Self> {
        let Some(name) = preset else {
            return Ok(Self::build(config, None));
        };
        let configured = config
            .models
            .as_ref()
            .and_then(|m| m.presets.get(name))
            .cloned();
        let Some(resolved) = configured.or_else(|| builtin_preset(name)) else {
            return Err(Error::UnknownModelPreset {
                name: name.to_string(),
                source: None,
                available: available_presets(config),
            });
        };
        Ok(Self::build(config, Some(&resolved)))
    }

    /// The one builder behind [`ModelPolicy::from_config`] and
    /// [`ModelPolicy::for_preset`].
    fn build(config: &Config, preset: Option<&PresetConfig>) -> Self {
        let models = config.models.as_ref();
        let build = |host: Host, tier: ModelTier| -> Profile {
            let (default_model, default_effort) = default_profile(host, tier);
            let configured = models
                .and_then(|m| m.profiles.as_ref())
                .and_then(|p| p.for_host(host))
                .and_then(|h| h.for_tier(tier));
            let legacy = match (host, tier) {
                (Host::Claude, ModelTier::Small) => models.and_then(|m| m.small.clone()),
                (Host::Claude, ModelTier::Medium) => models.and_then(|m| m.medium.clone()),
                (Host::Claude, ModelTier::Large) => models.and_then(|m| m.large.clone()),
                _ => None,
            };
            Profile {
                model: configured
                    .and_then(|c| c.model.clone())
                    .or(legacy)
                    .unwrap_or_else(|| default_model.to_string()),
                effort: configured.and_then(|c| c.effort).unwrap_or(default_effort),
            }
        };
        let preset_host = |host: Host| preset.and_then(|p| p.for_host(host));
        let tier_override = |host: Host, tier: ModelTier| {
            preset_host(host)
                .and_then(|h| h.tiers.as_ref())
                .and_then(|t| t.for_tier(tier))
        };
        let role_override = |host: Host, step: DispatchStep| {
            preset_host(host)
                .and_then(|h| h.steps.as_ref())
                .and_then(|s| s.for_step(step))
        };
        let profiles = Host::ALL
            .map(|host| ALL_TIERS.map(|tier| layer(&build(host, tier), tier_override(host, tier))));
        let resolved = Host::ALL.map(|host| {
            DispatchStep::ALL.map(|step| {
                ALL_TIERS.map(|tier| {
                    layer(
                        &profiles[host_index(host)][tier_index(tier)],
                        role_override(host, step),
                    )
                })
            })
        });
        let review_floor = models
            .and_then(|m| m.review_floor)
            .unwrap_or(DEFAULT_REVIEW_FLOOR);
        let steps = models.and_then(|m| m.steps.clone()).unwrap_or_default();
        ModelPolicy {
            profiles,
            resolved,
            review_floor,
            steps,
        }
    }

    /// Returns the tier-layer profile for `host` at `tier`: the configured
    /// (or default) profile with the preset's tier override, if any, applied.
    ///
    /// A preset's role (step) overrides are not visible here — they apply
    /// only through [`ModelPolicy::resolve`]. With no preset, this is the
    /// configured (or default) profile.
    #[must_use]
    pub fn profile(&self, host: Host, tier: ModelTier) -> &Profile {
        &self.profiles[host_index(host)][tier_index(tier)]
    }

    /// Returns the minimum tier that review-reasoning steps may run on.
    #[must_use]
    pub fn review_floor(&self) -> ModelTier {
        self.review_floor
    }

    /// Returns the `[models.steps]` override for `step`, if configured.
    fn step_override(&self, step: DispatchStep) -> Option<ModelTier> {
        match step {
            DispatchStep::Plan => self.steps.plan,
            DispatchStep::Implement => self.steps.implement,
            DispatchStep::ReviewFind => self.steps.review_find,
            DispatchStep::ReviewVerify => self.steps.review_verify,
            DispatchStep::ReviewConsolidate => self.steps.review_consolidate,
            DispatchStep::Mechanical => self.steps.mechanical,
        }
    }

    /// Resolves the [`ModelTier`] for `step` given an optional caller `hint`.
    ///
    /// Precedence: the caller `hint`, else the `[models.steps]` override for
    /// `step`, else the step's built-in default tier
    /// ([`DispatchStep::default_tier`]). The result is then clamped up (never
    /// down) to the step's [`floor`](DispatchStep::floor): [`PLAN_FLOOR`]
    /// for `plan`, [`ModelPolicy::review_floor`] for the review steps.
    #[must_use]
    pub fn resolve_tier(&self, step: DispatchStep, hint: Option<ModelTier>) -> ModelTier {
        let base = hint
            .or_else(|| self.step_override(step))
            .unwrap_or_else(|| step.default_tier());
        match step.floor(self.review_floor) {
            Some(floor) => base.max(floor),
            None => base,
        }
    }

    /// Resolves `step` (plus an optional caller `hint`) to the concrete
    /// profile for `host`: the tier-layer profile
    /// (`self.profile(host, self.resolve_tier(step, hint))`) with the
    /// preset's role override for `step`, if any, layered field by field on
    /// top. With no preset, the two are equal.
    #[must_use]
    pub fn resolve(&self, step: DispatchStep, hint: Option<ModelTier>, host: Host) -> &Profile {
        let tier = self.resolve_tier(step, hint);
        &self.resolved[host_index(host)][step_index(step)][tier_index(tier)]
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{
        Config, HostProfilesConfig, ModelsConfig, PresetConfig, PresetHostConfig, ProfileConfig,
        ProfilesConfig, StepProfilesConfig, StepTiersConfig,
    };
    use crate::error::Error;
    use crate::model::{Effort, ModelTier};
    use crate::model_policy::{
        BUILTIN_PRESETS, DispatchStep, Host, MODEL_PRESET_ENV, ModelPolicy, PLAN_FLOOR,
        PresetRequest, PresetSource, SelectedPreset, available_presets, builtin_preset,
        default_profile, select_preset,
    };

    const ALL_STEPS: [DispatchStep; 6] = [
        DispatchStep::Plan,
        DispatchStep::Implement,
        DispatchStep::ReviewFind,
        DispatchStep::ReviewVerify,
        DispatchStep::ReviewConsolidate,
        DispatchStep::Mechanical,
    ];
    const ALL_TIERS: [ModelTier; 4] = [
        ModelTier::Small,
        ModelTier::Medium,
        ModelTier::Large,
        ModelTier::Frontier,
    ];

    fn default_policy() -> ModelPolicy {
        ModelPolicy::from_config(&Config::default())
    }

    fn pair(policy: &ModelPolicy, host: Host, tier: ModelTier) -> (String, Effort) {
        let p = policy.profile(host, tier);
        (p.model.clone(), p.effort)
    }

    fn resolved(
        policy: &ModelPolicy,
        step: DispatchStep,
        hint: Option<ModelTier>,
        host: Host,
    ) -> (String, Effort) {
        let p = policy.resolve(step, hint, host);
        (p.model.clone(), p.effort)
    }

    fn o(model: &str, effort: Effort) -> (String, Effort) {
        (model.to_string(), effort)
    }

    fn models(m: ModelsConfig) -> Config {
        Config {
            models: Some(m),
            ..Default::default()
        }
    }

    #[test]
    fn default_table_claude() {
        let policy = default_policy();
        let expected = [
            (ModelTier::Small, o("opus", Effort::Low)),
            (ModelTier::Medium, o("opus", Effort::Medium)),
            (ModelTier::Large, o("opus", Effort::High)),
            (ModelTier::Frontier, o("opus", Effort::Xhigh)),
        ];
        for (tier, want) in expected {
            assert_eq!(pair(&policy, Host::Claude, tier), want, "claude {tier}");
            let (m, e) = default_profile(Host::Claude, tier);
            assert_eq!((m.to_string(), e), want);
        }
        assert_eq!(policy.review_floor(), ModelTier::Medium);
    }

    #[test]
    fn default_table_codex() {
        let policy = default_policy();
        let expected = [
            (ModelTier::Small, o("gpt-6-sol", Effort::Medium)),
            (ModelTier::Medium, o("gpt-6-sol", Effort::High)),
            (ModelTier::Large, o("gpt-6-astra", Effort::Medium)),
            (ModelTier::Frontier, o("gpt-6-astra", Effort::Xhigh)),
        ];
        for (tier, want) in expected {
            assert_eq!(pair(&policy, Host::Codex, tier), want, "codex {tier}");
            let (m, e) = default_profile(Host::Codex, tier);
            assert_eq!((m.to_string(), e), want);
        }
    }

    #[test]
    fn default_table_efforts_are_valid_for_their_host() {
        for host in [Host::Claude, Host::Codex] {
            for tier in ALL_TIERS {
                let (_, effort) = default_profile(host, tier);
                assert!(
                    host.valid_efforts().contains(&effort),
                    "{host} {tier} default effort {effort} invalid for host"
                );
            }
        }
    }

    #[test]
    fn plan_is_floored_to_medium() {
        assert_eq!(PLAN_FLOOR, ModelTier::Medium);
        let policy = default_policy();
        assert_eq!(
            policy.resolve_tier(DispatchStep::Plan, Some(ModelTier::Small)),
            ModelTier::Medium
        );
        assert_eq!(
            policy.resolve_tier(DispatchStep::Plan, Some(ModelTier::Large)),
            ModelTier::Large
        );
        assert_eq!(
            policy.resolve_tier(DispatchStep::Plan, Some(ModelTier::Frontier)),
            ModelTier::Frontier
        );
        let configured = ModelPolicy::from_config(&models(ModelsConfig {
            steps: Some(StepTiersConfig {
                plan: Some(ModelTier::Small),
                ..Default::default()
            }),
            ..Default::default()
        }));
        assert_eq!(
            configured.resolve_tier(DispatchStep::Plan, None),
            ModelTier::Medium
        );
        assert_eq!(
            resolved(
                &policy,
                DispatchStep::Plan,
                Some(ModelTier::Small),
                Host::Claude
            ),
            o("opus", Effort::Medium)
        );
    }

    #[test]
    fn implement_follows_item_tier() {
        let policy = default_policy();
        assert_eq!(
            policy.resolve_tier(DispatchStep::Implement, Some(ModelTier::Small)),
            ModelTier::Small
        );
        assert_eq!(
            resolved(
                &policy,
                DispatchStep::Implement,
                Some(ModelTier::Small),
                Host::Claude
            ),
            o("opus", Effort::Low)
        );
        assert_eq!(
            resolved(
                &policy,
                DispatchStep::Implement,
                Some(ModelTier::Small),
                Host::Codex
            ),
            o("gpt-6-sol", Effort::Medium)
        );
    }

    #[test]
    fn no_default_resolves_to_frontier() {
        let policy = default_policy();
        let hints = [
            None,
            Some(ModelTier::Small),
            Some(ModelTier::Medium),
            Some(ModelTier::Large),
        ];
        for step in ALL_STEPS {
            for hint in hints {
                let tier = policy.resolve_tier(step, hint);
                assert_ne!(tier, ModelTier::Frontier, "{step} with hint {hint:?}");
                for host in [Host::Claude, Host::Codex] {
                    assert_ne!(
                        policy.resolve(step, hint, host),
                        policy.profile(host, ModelTier::Frontier),
                        "{step} {hint:?} on {host} resolved to the frontier profile"
                    );
                }
            }
        }
    }

    #[test]
    fn explicit_frontier_hint_resolves_frontier_profile() {
        let policy = default_policy();
        for step in ALL_STEPS {
            assert_eq!(
                policy.resolve_tier(step, Some(ModelTier::Frontier)),
                ModelTier::Frontier
            );
            assert_eq!(
                resolved(&policy, step, Some(ModelTier::Frontier), Host::Claude),
                o("opus", Effort::Xhigh)
            );
            assert_eq!(
                resolved(&policy, step, Some(ModelTier::Frontier), Host::Codex),
                o("gpt-6-astra", Effort::Xhigh)
            );
        }
    }

    #[test]
    fn no_default_profile_is_excluded_model() {
        let excluded = ["haiku", "sonnet", "fable", "gpt-6-luna"];
        let policy = default_policy();
        for host in [Host::Claude, Host::Codex] {
            for tier in ALL_TIERS {
                let model = &policy.profile(host, tier).model;
                assert!(
                    !excluded.contains(&model.as_str()),
                    "{host} {tier} defaults to excluded model {model}"
                );
            }
            for step in ALL_STEPS {
                for hint in [None, Some(ModelTier::Small)] {
                    let model = &policy.resolve(step, hint, host).model;
                    assert!(!excluded.contains(&model.as_str()));
                }
            }
        }
    }

    #[test]
    fn review_find_hint_small_is_floored_to_medium() {
        let policy = default_policy();
        assert_eq!(
            resolved(
                &policy,
                DispatchStep::ReviewFind,
                Some(ModelTier::Small),
                Host::Claude
            ),
            o("opus", Effort::Medium)
        );
    }

    #[test]
    fn review_find_hint_large_escalates_above_floor() {
        let policy = default_policy();
        assert_eq!(
            resolved(
                &policy,
                DispatchStep::ReviewFind,
                Some(ModelTier::Large),
                Host::Claude
            ),
            o("opus", Effort::High)
        );
    }

    #[test]
    fn mechanical_hint_small_is_exempt_from_floor() {
        let policy = default_policy();
        assert_eq!(
            resolved(
                &policy,
                DispatchStep::Mechanical,
                Some(ModelTier::Small),
                Host::Claude
            ),
            o("opus", Effort::Low)
        );
    }

    #[test]
    fn review_verify_default_tier_is_large() {
        let policy = default_policy();
        assert_eq!(
            resolved(&policy, DispatchStep::ReviewVerify, None, Host::Claude),
            o("opus", Effort::High)
        );
        assert_eq!(
            resolved(&policy, DispatchStep::ReviewVerify, None, Host::Codex),
            o("gpt-6-astra", Effort::Medium)
        );
    }

    #[test]
    fn review_consolidate_default_tier_is_large() {
        let policy = default_policy();
        assert_eq!(
            DispatchStep::ReviewConsolidate.default_tier(),
            ModelTier::Large
        );
        assert_eq!(
            resolved(&policy, DispatchStep::ReviewConsolidate, None, Host::Claude),
            o("opus", Effort::High)
        );
    }

    #[test]
    fn review_consolidate_is_lifted_by_the_review_floor() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            review_floor: Some(ModelTier::Frontier),
            ..Default::default()
        }));
        assert_eq!(
            policy.resolve_tier(DispatchStep::ReviewConsolidate, None),
            ModelTier::Frontier
        );
    }

    #[test]
    fn review_consolidate_step_override_is_floored_to_review_floor() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            steps: Some(StepTiersConfig {
                review_consolidate: Some(ModelTier::Small),
                ..Default::default()
            }),
            ..Default::default()
        }));
        // Default review floor is Medium; a `small` override is lifted to it.
        assert_eq!(
            policy.resolve_tier(DispatchStep::ReviewConsolidate, None),
            ModelTier::Medium
        );
    }

    #[test]
    fn review_consolidate_step_override_is_honoured() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            steps: Some(StepTiersConfig {
                review_consolidate: Some(ModelTier::Frontier),
                ..Default::default()
            }),
            ..Default::default()
        }));
        assert_eq!(
            policy.resolve_tier(DispatchStep::ReviewConsolidate, None),
            ModelTier::Frontier
        );
    }

    #[test]
    fn caller_hint_overrides_configured_step_tier_upward() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            steps: Some(StepTiersConfig {
                implement: Some(ModelTier::Small),
                ..Default::default()
            }),
            ..Default::default()
        }));
        assert_eq!(
            policy.resolve_tier(DispatchStep::Implement, Some(ModelTier::Large)),
            ModelTier::Large
        );
    }

    #[test]
    fn per_step_config_used_when_no_hint() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            steps: Some(StepTiersConfig {
                implement: Some(ModelTier::Small),
                ..Default::default()
            }),
            ..Default::default()
        }));
        assert_eq!(
            policy.resolve_tier(DispatchStep::Implement, None),
            ModelTier::Small
        );
    }

    #[test]
    fn custom_tier_bindings_and_raised_floor_honored_end_to_end() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            small: Some("custom-small".to_string()),
            medium: Some("custom-medium".to_string()),
            large: Some("custom-large".to_string()),
            review_floor: Some(ModelTier::Large),
            ..Default::default()
        }));
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Small),
            o("custom-small", Effort::Low)
        );
        // Default review-find tier is Medium; floored up to Large.
        assert_eq!(
            resolved(&policy, DispatchStep::ReviewFind, None, Host::Claude),
            o("custom-large", Effort::High)
        );
        // Mechanical is exempt from the floor, so its default Small tier stands.
        assert_eq!(
            resolved(&policy, DispatchStep::Mechanical, None, Host::Claude),
            o("custom-small", Effort::Low)
        );
    }

    #[test]
    fn legacy_tier_keys_set_claude_model_keep_default_effort() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            small: Some("custom".to_string()),
            ..Default::default()
        }));
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Small),
            o("custom", Effort::Low)
        );
        assert_eq!(
            pair(&policy, Host::Codex, ModelTier::Small),
            o("gpt-6-sol", Effort::Medium)
        );
    }

    #[test]
    fn profile_overrides_per_host_and_tier() {
        let policy = ModelPolicy::from_config(&models(ModelsConfig {
            small: Some("legacy-small".to_string()),
            profiles: Some(ProfilesConfig {
                claude: Some(HostProfilesConfig {
                    small: Some(ProfileConfig {
                        model: Some("profile-small".to_string()),
                        effort: None,
                    }),
                    large: Some(ProfileConfig {
                        model: None,
                        effort: Some(Effort::Max),
                    }),
                    ..Default::default()
                }),
                codex: Some(HostProfilesConfig {
                    frontier: Some(ProfileConfig {
                        model: Some("gpt-6-sol".to_string()),
                        effort: Some(Effort::High),
                    }),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }));
        // A profile `model` beats the legacy key; effort keeps its default.
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Small),
            o("profile-small", Effort::Low)
        );
        // An effort-only override keeps the default model.
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Large),
            o("opus", Effort::Max)
        );
        assert_eq!(
            pair(&policy, Host::Codex, ModelTier::Frontier),
            o("gpt-6-sol", Effort::High)
        );
        // Untouched entries keep the built-in table.
        assert_eq!(
            pair(&policy, Host::Codex, ModelTier::Large),
            o("gpt-6-astra", Effort::Medium)
        );
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Frontier),
            o("opus", Effort::Xhigh)
        );
    }

    #[test]
    fn host_display_and_fromstr_round_trip() {
        for (host, name) in [(Host::Claude, "claude"), (Host::Codex, "codex")] {
            assert_eq!(host.to_string(), name);
            assert_eq!(name.parse::<Host>().unwrap(), host);
        }
        assert_eq!(Host::default(), Host::Claude);
    }

    #[test]
    fn host_fromstr_rejects_unknown_value() {
        let err = "gemini".parse::<Host>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid host: 'gemini' (expected claude or codex)"
        );
    }

    #[test]
    fn host_valid_efforts() {
        assert_eq!(
            Host::Claude.valid_efforts(),
            &[
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::Xhigh,
                Effort::Max
            ]
        );
        assert_eq!(
            Host::Codex.valid_efforts(),
            &[Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh]
        );
    }

    #[test]
    fn dispatch_step_display_and_fromstr_round_trip() {
        let variants = [
            (DispatchStep::Plan, "plan"),
            (DispatchStep::Implement, "implement"),
            (DispatchStep::ReviewFind, "review-find"),
            (DispatchStep::ReviewVerify, "review-verify"),
            (DispatchStep::ReviewConsolidate, "review-consolidate"),
            (DispatchStep::Mechanical, "mechanical"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: DispatchStep = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    // --- presets ---

    const HINTS: [Option<ModelTier>; 5] = [
        None,
        Some(ModelTier::Small),
        Some(ModelTier::Medium),
        Some(ModelTier::Large),
        Some(ModelTier::Frontier),
    ];

    const JUDGMENT_STEPS: [DispatchStep; 5] = [
        DispatchStep::Plan,
        DispatchStep::Implement,
        DispatchStep::ReviewFind,
        DispatchStep::ReviewVerify,
        DispatchStep::ReviewConsolidate,
    ];

    fn pc(model: Option<&str>, effort: Option<Effort>) -> Option<ProfileConfig> {
        Some(ProfileConfig {
            model: model.map(ToString::to_string),
            effort,
        })
    }

    /// A `[models]` table exercising profiles, legacy keys, step tiers and
    /// the review floor at once.
    fn nontrivial_models() -> ModelsConfig {
        ModelsConfig {
            small: Some("legacy-small".to_string()),
            large: Some("legacy-large".to_string()),
            review_floor: Some(ModelTier::Large),
            steps: Some(StepTiersConfig {
                implement: Some(ModelTier::Large),
                mechanical: Some(ModelTier::Medium),
                ..Default::default()
            }),
            profiles: Some(ProfilesConfig {
                claude: Some(HostProfilesConfig {
                    medium: pc(Some("profile-medium"), Some(Effort::Xhigh)),
                    large: pc(None, Some(Effort::Max)),
                    ..Default::default()
                }),
                codex: Some(HostProfilesConfig {
                    small: pc(None, Some(Effort::Low)),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }
    }

    fn with_preset(mut m: ModelsConfig, name: &str, preset: PresetConfig) -> Config {
        m.presets.insert(name.to_string(), preset);
        models(m)
    }

    fn claude_preset(host: PresetHostConfig) -> PresetConfig {
        PresetConfig {
            claude: Some(host),
            codex: None,
        }
    }

    #[test]
    fn no_preset_resolves_as_before_the_phase() {
        for cfg in [Config::default(), models(nontrivial_models())] {
            let policy = ModelPolicy::for_preset(&cfg, None).unwrap();
            for step in ALL_STEPS {
                for hint in HINTS {
                    for host in Host::ALL {
                        // The pre-phase definition of `resolve`.
                        assert_eq!(
                            policy.resolve(step, hint, host),
                            policy.profile(host, policy.resolve_tier(step, hint)),
                            "{step} {hint:?} {host}"
                        );
                    }
                }
            }
            assert_eq!(policy, ModelPolicy::from_config(&cfg));
        }
        // The configured table itself is unchanged by the preset machinery.
        let policy = ModelPolicy::for_preset(&models(nontrivial_models()), None).unwrap();
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Small),
            o("legacy-small", Effort::Low)
        );
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Medium),
            o("profile-medium", Effort::Xhigh)
        );
        assert_eq!(
            pair(&policy, Host::Claude, ModelTier::Large),
            o("legacy-large", Effort::Max)
        );
        assert_eq!(
            pair(&policy, Host::Codex, ModelTier::Small),
            o("gpt-6-sol", Effort::Low)
        );
    }

    #[test]
    fn tier_override_applies_to_every_step_at_that_tier_only() {
        let base_models = nontrivial_models();
        let base = ModelPolicy::from_config(&models(base_models.clone()));
        let cfg = with_preset(
            base_models,
            "p",
            claude_preset(PresetHostConfig {
                tiers: Some(HostProfilesConfig {
                    large: pc(Some("x"), None),
                    ..Default::default()
                }),
                steps: None,
            }),
        );
        let policy = ModelPolicy::for_preset(&cfg, Some("p")).unwrap();
        let mut hit_large = 0;
        for step in ALL_STEPS {
            for hint in HINTS {
                for host in Host::ALL {
                    let tier = base.resolve_tier(step, hint);
                    assert_eq!(policy.resolve_tier(step, hint), tier);
                    let got = resolved(&policy, step, hint, host);
                    if host == Host::Claude && tier == ModelTier::Large {
                        hit_large += 1;
                        // Large's configured effort (`max`) is kept.
                        assert_eq!(got, o("x", Effort::Max), "{step} {hint:?}");
                    } else {
                        assert_eq!(
                            got,
                            resolved(&base, step, hint, host),
                            "{step} {hint:?} {host}"
                        );
                    }
                }
            }
        }
        assert!(hit_large > 0);
    }

    #[test]
    fn role_override_applies_at_every_tier_with_the_tier_effort() {
        let base_models = nontrivial_models();
        let base = ModelPolicy::from_config(&models(base_models.clone()));
        let cfg = with_preset(
            base_models,
            "p",
            claude_preset(PresetHostConfig {
                tiers: None,
                steps: Some(StepProfilesConfig {
                    implement: pc(Some("x"), None),
                    ..Default::default()
                }),
            }),
        );
        let policy = ModelPolicy::for_preset(&cfg, Some("p")).unwrap();
        for tier in ALL_TIERS {
            let want_effort = base.profile(Host::Claude, tier).effort;
            assert_eq!(
                resolved(&policy, DispatchStep::Implement, Some(tier), Host::Claude),
                o("x", want_effort),
                "{tier}"
            );
        }
        // Medium's effort comes from the configured table, not the defaults.
        assert_eq!(
            resolved(
                &policy,
                DispatchStep::Implement,
                Some(ModelTier::Medium),
                Host::Claude
            ),
            o("x", Effort::Xhigh)
        );
        for step in ALL_STEPS {
            for hint in HINTS {
                for host in Host::ALL {
                    if step == DispatchStep::Implement && host == Host::Claude {
                        continue;
                    }
                    assert_eq!(
                        resolved(&policy, step, hint, host),
                        resolved(&base, step, hint, host),
                        "{step} {hint:?} {host}"
                    );
                }
            }
        }
        // The role layer is not visible through the tier table.
        for tier in ALL_TIERS {
            assert_eq!(
                policy.profile(Host::Claude, tier),
                base.profile(Host::Claude, tier)
            );
        }
    }

    #[test]
    fn role_override_beats_tier_override_field_by_field() {
        let cfg = with_preset(
            ModelsConfig::default(),
            "p",
            claude_preset(PresetHostConfig {
                tiers: Some(HostProfilesConfig {
                    medium: pc(Some("t"), None),
                    ..Default::default()
                }),
                steps: Some(StepProfilesConfig {
                    implement: pc(Some("r"), None),
                    ..Default::default()
                }),
            }),
        );
        let policy = ModelPolicy::for_preset(&cfg, Some("p")).unwrap();
        let medium = Some(ModelTier::Medium);
        assert_eq!(
            resolved(&policy, DispatchStep::Implement, medium, Host::Claude),
            o("r", Effort::Medium)
        );
        assert_eq!(
            resolved(&policy, DispatchStep::Plan, medium, Host::Claude),
            o("t", Effort::Medium)
        );

        // The role sets only the model; the tier override's effort still
        // applies under it.
        let cfg = with_preset(
            ModelsConfig::default(),
            "p",
            claude_preset(PresetHostConfig {
                tiers: Some(HostProfilesConfig {
                    medium: pc(Some("t"), Some(Effort::Max)),
                    ..Default::default()
                }),
                steps: Some(StepProfilesConfig {
                    implement: pc(Some("r"), None),
                    ..Default::default()
                }),
            }),
        );
        let policy = ModelPolicy::for_preset(&cfg, Some("p")).unwrap();
        assert_eq!(
            resolved(&policy, DispatchStep::Implement, medium, Host::Claude),
            o("r", Effort::Max)
        );
        assert_eq!(
            resolved(&policy, DispatchStep::Plan, medium, Host::Claude),
            o("t", Effort::Max)
        );
    }

    #[test]
    fn builtin_fable_puts_every_judgment_role_on_fable() {
        let cfg = Config::default();
        let base = ModelPolicy::from_config(&cfg);
        let policy = ModelPolicy::for_preset(&cfg, Some("fable")).unwrap();
        for step in ALL_STEPS {
            for hint in HINTS {
                let want_claude = if JUDGMENT_STEPS.contains(&step) {
                    let effort = base.resolve(step, hint, Host::Claude).effort;
                    o("fable", effort)
                } else {
                    resolved(&base, step, hint, Host::Claude)
                };
                assert_eq!(
                    resolved(&policy, step, hint, Host::Claude),
                    want_claude,
                    "{step} {hint:?}"
                );
                assert_eq!(
                    resolved(&policy, step, hint, Host::Codex),
                    resolved(&base, step, hint, Host::Codex),
                    "codex {step} {hint:?}"
                );
            }
        }
    }

    #[test]
    fn builtin_fable_build_puts_only_plan_and_implement_on_fable() {
        let cfg = Config::default();
        let base = ModelPolicy::from_config(&cfg);
        let policy = ModelPolicy::for_preset(&cfg, Some("fable-build")).unwrap();
        for hint in HINTS {
            for step in [DispatchStep::Plan, DispatchStep::Implement] {
                assert_eq!(
                    resolved(&policy, step, hint, Host::Claude).0,
                    "fable",
                    "{step} {hint:?}"
                );
            }
            for step in [
                DispatchStep::ReviewFind,
                DispatchStep::ReviewVerify,
                DispatchStep::ReviewConsolidate,
                DispatchStep::Mechanical,
            ] {
                assert_eq!(
                    resolved(&policy, step, hint, Host::Claude),
                    resolved(&base, step, hint, Host::Claude),
                    "{step} {hint:?}"
                );
            }
            for step in ALL_STEPS {
                assert_eq!(
                    resolved(&policy, step, hint, Host::Codex),
                    resolved(&base, step, hint, Host::Codex)
                );
            }
        }
    }

    #[test]
    fn configured_preset_replaces_builtin_of_the_same_name() {
        let cfg =
            Config::from_toml("[models.presets.fable.claude.steps.mechanical]\nmodel = \"y\"\n")
                .unwrap();
        let base = ModelPolicy::from_config(&cfg);
        let policy = ModelPolicy::for_preset(&cfg, Some("fable")).unwrap();
        for hint in HINTS {
            assert_eq!(
                resolved(&policy, DispatchStep::Mechanical, hint, Host::Claude).0,
                "y"
            );
            // Not merged with the built-in: every judgment role is unchanged.
            for step in JUDGMENT_STEPS {
                assert_eq!(
                    resolved(&policy, step, hint, Host::Claude),
                    resolved(&base, step, hint, Host::Claude),
                    "{step} {hint:?}"
                );
            }
        }
    }

    #[test]
    fn unknown_preset_errors_listing_available_names() {
        let cfg = with_preset(ModelsConfig::default(), "mine", PresetConfig::default());
        match ModelPolicy::for_preset(&cfg, Some("nope")) {
            Err(Error::UnknownModelPreset {
                name,
                source,
                available,
            }) => {
                assert_eq!(name, "nope");
                assert_eq!(source, None);
                assert_eq!(available, ["fable", "fable-build", "mine"]);
            }
            other => panic!("expected UnknownModelPreset, got {other:?}"),
        }
        let msg = ModelPolicy::for_preset(&cfg, Some("nope"))
            .unwrap_err()
            .to_string();
        assert_eq!(
            msg,
            "unknown model preset 'nope' — available presets: fable, fable-build, mine"
        );
        // An empty configured preset is still a known name.
        assert!(ModelPolicy::for_preset(&cfg, Some("mine")).is_ok());
    }

    #[test]
    fn available_presets_dedups_configured_builtin_names() {
        let mut m = ModelsConfig::default();
        m.presets
            .insert("zeta".to_string(), PresetConfig::default());
        m.presets
            .insert("fable".to_string(), PresetConfig::default());
        assert_eq!(
            available_presets(&models(m)),
            ["fable", "fable-build", "zeta"]
        );
        assert_eq!(
            available_presets(&Config::default()),
            ["fable", "fable-build"]
        );
        for name in BUILTIN_PRESETS {
            assert!(builtin_preset(name).is_some(), "{name}");
        }
        assert!(builtin_preset("nope").is_none());
    }

    /// An injected env holding only `RDM_MODEL_PRESET = value`, if any.
    fn preset_env(value: Option<&str>) -> impl Fn(&str) -> Option<String> {
        let value = value.map(ToString::to_string);
        move |k: &str| {
            assert_eq!(k, MODEL_PRESET_ENV, "only the preset env var is read");
            value.clone()
        }
    }

    /// Config with presets `a`, `b`, `c` and `[models] preset = config_preset`.
    fn abc_config(config_preset: Option<&str>) -> Config {
        let mut m = ModelsConfig {
            preset: config_preset.map(ToString::to_string),
            ..Default::default()
        };
        for name in ["a", "b", "c"] {
            m.presets.insert(name.to_string(), PresetConfig::default());
        }
        models(m)
    }

    fn selected(name: &str, source: PresetSource) -> Option<SelectedPreset> {
        Some(SelectedPreset {
            name: name.to_string(),
            source,
        })
    }

    #[test]
    fn select_preset_precedence_is_flag_then_env_then_config() {
        let cfg = abc_config(Some("c"));
        let flag = PresetRequest::Named("a");
        let none = PresetRequest::Unspecified;
        assert_eq!(
            select_preset(flag, &cfg, preset_env(Some("b"))).unwrap(),
            selected("a", PresetSource::Flag)
        );
        assert_eq!(
            select_preset(none, &cfg, preset_env(Some("b"))).unwrap(),
            selected("b", PresetSource::Env)
        );
        assert_eq!(
            select_preset(none, &cfg, preset_env(None)).unwrap(),
            selected("c", PresetSource::Config)
        );
        assert_eq!(
            select_preset(none, &abc_config(None), preset_env(None)).unwrap(),
            None
        );
        assert_eq!(
            select_preset(none, &Config::default(), preset_env(None)).unwrap(),
            None
        );
    }

    #[test]
    fn select_no_preset_ignores_env_and_config() {
        // A bad env var and config key are never read under `NoPreset`.
        let cfg = abc_config(Some("bad-config"));
        let env = |_: &str| -> Option<String> { panic!("env must not be read") };
        assert_eq!(
            select_preset(PresetRequest::NoPreset, &cfg, env).unwrap(),
            None
        );
    }

    #[test]
    fn select_preset_blank_env_falls_through_to_config() {
        let cfg = abc_config(Some("c"));
        for blank in ["", "  ", "\t"] {
            assert_eq!(
                select_preset(PresetRequest::Unspecified, &cfg, preset_env(Some(blank))).unwrap(),
                selected("c", PresetSource::Config),
                "{blank:?}"
            );
        }
    }

    #[test]
    fn select_preset_unknown_name_carries_its_source() {
        for (request, env, config, source) in [
            (PresetRequest::Named("nope"), None, None, PresetSource::Flag),
            (
                PresetRequest::Unspecified,
                Some("nope"),
                None,
                PresetSource::Env,
            ),
            (
                PresetRequest::Unspecified,
                None,
                Some("nope"),
                PresetSource::Config,
            ),
        ] {
            let cfg = abc_config(config);
            match select_preset(request, &cfg, preset_env(env)) {
                Err(Error::UnknownModelPreset {
                    name,
                    source: got,
                    available,
                }) => {
                    assert_eq!(name, "nope");
                    assert_eq!(got, Some(source));
                    assert_eq!(available, ["a", "b", "c", "fable", "fable-build"]);
                }
                other => panic!("{source}: expected UnknownModelPreset, got {other:?}"),
            }
        }
        // An empty flag value is an unknown name, not "no preset".
        assert!(matches!(
            select_preset(
                PresetRequest::Named(""),
                &Config::default(),
                preset_env(None)
            ),
            Err(Error::UnknownModelPreset {
                source: Some(PresetSource::Flag),
                ..
            })
        ));
    }

    #[test]
    fn select_preset_higher_layer_shadows_a_bad_lower_one() {
        let cfg = abc_config(Some("bad-config"));
        assert_eq!(
            select_preset(
                PresetRequest::Named("fable"),
                &cfg,
                preset_env(Some("bad-env"))
            )
            .unwrap(),
            selected("fable", PresetSource::Flag)
        );
        assert_eq!(
            select_preset(PresetRequest::Unspecified, &cfg, preset_env(Some("b"))).unwrap(),
            selected("b", PresetSource::Env)
        );
    }

    #[test]
    fn unknown_preset_display_with_and_without_source() {
        let available = vec!["fable".to_string(), "fable-build".to_string()];
        let err = |source| Error::UnknownModelPreset {
            name: "nope".to_string(),
            source,
            available: available.clone(),
        };
        assert_eq!(
            err(None).to_string(),
            "unknown model preset 'nope' — available presets: fable, fable-build"
        );
        for (source, spelled) in [
            (PresetSource::Flag, "--preset flag"),
            (PresetSource::Env, "RDM_MODEL_PRESET environment variable"),
            (PresetSource::Config, "[models] preset config key"),
        ] {
            assert_eq!(
                err(Some(source)).to_string(),
                format!(
                    "unknown model preset 'nope' (from the {spelled}) — available presets: fable, fable-build"
                )
            );
        }
        assert_eq!(PresetSource::Flag.label(), "flag");
        assert_eq!(PresetSource::Env.label(), "env");
        assert_eq!(PresetSource::Config.label(), "config");
    }

    #[test]
    fn dispatch_step_fromstr_rejects_unknown_value() {
        let err = "bogus".parse::<DispatchStep>().unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }
}
