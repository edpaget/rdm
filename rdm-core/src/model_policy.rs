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

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

use crate::config::{Config, StepTiersConfig};
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
    /// Mechanical, non-judgment work (e.g. formatting, rote transforms).
    Mechanical,
}

impl DispatchStep {
    /// Returns this step's built-in default tier, used when neither a caller
    /// hint nor a `[models.steps]` override is present.
    #[must_use]
    pub fn default_tier(&self) -> ModelTier {
        match self {
            DispatchStep::Plan => ModelTier::Medium,
            DispatchStep::Implement => ModelTier::Medium,
            DispatchStep::ReviewFind => ModelTier::Medium,
            DispatchStep::ReviewVerify => ModelTier::Large,
            DispatchStep::Mechanical => ModelTier::Small,
        }
    }

    /// Returns the minimum tier this step's resolved tier is clamped up to,
    /// if any.
    ///
    /// `Plan` is floored at [`PLAN_FLOOR`] (planner ≥ implementer);
    /// `ReviewFind` and `ReviewVerify` at the configured `review_floor`;
    /// `Implement` follows the item tier and `Mechanical` is exempt so rote
    /// work can still run cheap.
    #[must_use]
    pub fn floor(&self, review_floor: ModelTier) -> Option<ModelTier> {
        match self {
            DispatchStep::Plan => Some(PLAN_FLOOR),
            DispatchStep::ReviewFind | DispatchStep::ReviewVerify => Some(review_floor),
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
    /// `review-find`, `review-verify`, or `mechanical`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "plan" => Ok(DispatchStep::Plan),
            "implement" => Ok(DispatchStep::Implement),
            "review-find" => Ok(DispatchStep::ReviewFind),
            "review-verify" => Ok(DispatchStep::ReviewVerify),
            "mechanical" => Ok(DispatchStep::Mechanical),
            other => Err(ParseError::new(
                "dispatch step",
                other,
                "plan, implement, review-find, review-verify, or mechanical",
            )),
        }
    }
}

/// Resolves a [`DispatchStep`] (plus an optional caller tier hint) to a
/// concrete [`Profile`] for a [`Host`].
///
/// Built from the `[models]` table of a [`Config`] via
/// [`ModelPolicy::from_config`], filling every unset field with a built-in
/// default so callers never need to special-case an absent `[models]` table.
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
    /// Resolved profiles, indexed `[host][tier]` in [`Host::ALL`] /
    /// [`ALL_TIERS`] order.
    profiles: [[Profile; 4]; 2],
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
        let profiles = Host::ALL.map(|host| ALL_TIERS.map(|tier| build(host, tier)));
        let review_floor = models
            .and_then(|m| m.review_floor)
            .unwrap_or(DEFAULT_REVIEW_FLOOR);
        let steps = models.and_then(|m| m.steps.clone()).unwrap_or_default();
        ModelPolicy {
            profiles,
            review_floor,
            steps,
        }
    }

    /// Returns the configured (or default) profile for `host` at `tier`.
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
    /// profile for `host`. Equivalent to
    /// `self.profile(host, self.resolve_tier(step, hint))`.
    #[must_use]
    pub fn resolve(&self, step: DispatchStep, hint: Option<ModelTier>, host: Host) -> &Profile {
        self.profile(host, self.resolve_tier(step, hint))
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{
        Config, HostProfilesConfig, ModelsConfig, ProfileConfig, ProfilesConfig, StepTiersConfig,
    };
    use crate::model::{Effort, ModelTier};
    use crate::model_policy::{DispatchStep, Host, ModelPolicy, PLAN_FLOOR, default_profile};

    const ALL_STEPS: [DispatchStep; 5] = [
        DispatchStep::Plan,
        DispatchStep::Implement,
        DispatchStep::ReviewFind,
        DispatchStep::ReviewVerify,
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
            (DispatchStep::Mechanical, "mechanical"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            let parsed: DispatchStep = expected.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn dispatch_step_fromstr_rejects_unknown_value() {
        let err = "bogus".parse::<DispatchStep>().unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }
}
