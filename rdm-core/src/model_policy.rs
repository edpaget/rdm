//! Model-tier sizing policy resolution for dispatch steps.
//!
//! This is the single source of truth for turning a dispatch step (plus an
//! optional caller tier hint) into a concrete model id, applying the
//! `[models]` policy from [`crate::config`] with built-in defaults layered
//! underneath. CLI and skill consumers should call [`ModelPolicy::resolve`]
//! rather than re-implementing this table.

use std::fmt;
use std::str::FromStr;

use crate::config::{Config, StepTiersConfig};
use crate::model::{ModelTier, ParseError};

/// Built-in model id bound to [`ModelTier::Small`] when `[models]` does not
/// override it.
pub const DEFAULT_SMALL_MODEL: &str = "haiku";
/// Built-in model id bound to [`ModelTier::Medium`] when `[models]` does not
/// override it.
pub const DEFAULT_MEDIUM_MODEL: &str = "sonnet";
/// Built-in model id bound to [`ModelTier::Large`] when `[models]` does not
/// override it.
pub const DEFAULT_LARGE_MODEL: &str = "opus";
/// Built-in minimum tier review steps may run on when `[models]` does not
/// override it.
pub const DEFAULT_REVIEW_FLOOR: ModelTier = ModelTier::Medium;

/// A step in the agentic dispatch pipeline that requires a sized model.
///
/// Each variant carries its own built-in default tier (see
/// [`DispatchStep::default_tier`]) and floor exemption (see
/// [`DispatchStep::review_floored`]); [`ModelPolicy`] resolves the two
/// alongside `[models]` config and an optional caller hint into a concrete
/// model id.
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

    /// Returns `true` if this step's resolved tier is clamped up to the
    /// configured review floor (see [`ModelPolicy::review_floor`]).
    ///
    /// `ReviewFind` and `ReviewVerify` are reasoning-heavy review steps that
    /// are floored; `Mechanical` is exempt so rote work can still run cheap.
    #[must_use]
    pub fn review_floored(&self) -> bool {
        matches!(self, DispatchStep::ReviewFind | DispatchStep::ReviewVerify)
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
/// concrete model id.
///
/// Built from the `[models]` table of a [`Config`] via [`ModelPolicy::from_config`],
/// filling every unset field with a built-in default so callers never need
/// to special-case an absent `[models]` table.
///
/// # Examples
///
/// ```
/// use rdm_core::config::Config;
/// use rdm_core::model_policy::{DispatchStep, ModelPolicy};
///
/// let policy = ModelPolicy::from_config(&Config::default());
/// assert_eq!(policy.resolve(DispatchStep::Implement, None), "sonnet");
/// // Review-reasoning steps are floored up to at least the medium tier.
/// assert_eq!(policy.resolve(DispatchStep::ReviewFind, None), "sonnet");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPolicy {
    small_model: String,
    medium_model: String,
    large_model: String,
    review_floor: ModelTier,
    steps: StepTiersConfig,
}

impl ModelPolicy {
    /// Builds a `ModelPolicy` from a repo [`Config`], filling every field the
    /// `[models]` table leaves unset with its built-in default.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let models = config.models.as_ref();
        let small_model = models
            .and_then(|m| m.small.clone())
            .unwrap_or_else(|| DEFAULT_SMALL_MODEL.to_string());
        let medium_model = models
            .and_then(|m| m.medium.clone())
            .unwrap_or_else(|| DEFAULT_MEDIUM_MODEL.to_string());
        let large_model = models
            .and_then(|m| m.large.clone())
            .unwrap_or_else(|| DEFAULT_LARGE_MODEL.to_string());
        let review_floor = models
            .and_then(|m| m.review_floor)
            .unwrap_or(DEFAULT_REVIEW_FLOOR);
        let steps = models.and_then(|m| m.steps.clone()).unwrap_or_default();
        ModelPolicy {
            small_model,
            medium_model,
            large_model,
            review_floor,
            steps,
        }
    }

    /// Maps a [`ModelTier`] to its configured (or default) model id.
    #[must_use]
    pub fn model_for_tier(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Small => &self.small_model,
            ModelTier::Medium => &self.medium_model,
            ModelTier::Large => &self.large_model,
        }
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
    /// ([`DispatchStep::default_tier`]). If `step` is
    /// [`review_floored`](DispatchStep::review_floored), the result is then
    /// clamped up to [`ModelPolicy::review_floor`] (never down).
    #[must_use]
    pub fn resolve_tier(&self, step: DispatchStep, hint: Option<ModelTier>) -> ModelTier {
        let base = hint
            .or_else(|| self.step_override(step))
            .unwrap_or_else(|| step.default_tier());
        if step.review_floored() {
            base.max(self.review_floor)
        } else {
            base
        }
    }

    /// Resolves `step` (plus an optional caller `hint`) to a concrete model
    /// id. Equivalent to `self.model_for_tier(self.resolve_tier(step, hint))`.
    #[must_use]
    pub fn resolve(&self, step: DispatchStep, hint: Option<ModelTier>) -> &str {
        self.model_for_tier(self.resolve_tier(step, hint))
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{Config, ModelsConfig, StepTiersConfig};
    use crate::model::ModelTier;
    use crate::model_policy::{DispatchStep, ModelPolicy};

    #[test]
    fn zero_config_tier_bindings_and_floor() {
        let policy = ModelPolicy::from_config(&Config::default());
        assert_eq!(policy.model_for_tier(ModelTier::Small), "haiku");
        assert_eq!(policy.model_for_tier(ModelTier::Medium), "sonnet");
        assert_eq!(policy.model_for_tier(ModelTier::Large), "opus");
        assert_eq!(policy.review_floor(), ModelTier::Medium);
    }

    #[test]
    fn review_find_hint_small_is_floored_to_medium() {
        let policy = ModelPolicy::from_config(&Config::default());
        assert_eq!(
            policy.resolve(DispatchStep::ReviewFind, Some(ModelTier::Small)),
            "sonnet"
        );
    }

    #[test]
    fn review_find_hint_large_escalates_above_floor() {
        let policy = ModelPolicy::from_config(&Config::default());
        assert_eq!(
            policy.resolve(DispatchStep::ReviewFind, Some(ModelTier::Large)),
            "opus"
        );
    }

    #[test]
    fn mechanical_hint_small_is_exempt_from_floor() {
        let policy = ModelPolicy::from_config(&Config::default());
        assert_eq!(
            policy.resolve(DispatchStep::Mechanical, Some(ModelTier::Small)),
            "haiku"
        );
    }

    #[test]
    fn review_verify_default_tier_is_large() {
        let policy = ModelPolicy::from_config(&Config::default());
        assert_eq!(policy.resolve(DispatchStep::ReviewVerify, None), "opus");
    }

    #[test]
    fn caller_hint_overrides_configured_step_tier_upward() {
        let config = Config {
            models: Some(ModelsConfig {
                steps: Some(StepTiersConfig {
                    implement: Some(ModelTier::Small),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let policy = ModelPolicy::from_config(&config);
        assert_eq!(
            policy.resolve(DispatchStep::Implement, Some(ModelTier::Large)),
            "opus"
        );
    }

    #[test]
    fn per_step_config_used_when_no_hint() {
        let config = Config {
            models: Some(ModelsConfig {
                steps: Some(StepTiersConfig {
                    implement: Some(ModelTier::Small),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let policy = ModelPolicy::from_config(&config);
        assert_eq!(policy.resolve(DispatchStep::Implement, None), "haiku");
    }

    #[test]
    fn custom_tier_bindings_and_raised_floor_honored_end_to_end() {
        let config = Config {
            models: Some(ModelsConfig {
                small: Some("custom-small".to_string()),
                medium: Some("custom-medium".to_string()),
                large: Some("custom-large".to_string()),
                review_floor: Some(ModelTier::Large),
                steps: None,
            }),
            ..Default::default()
        };
        let policy = ModelPolicy::from_config(&config);
        assert_eq!(policy.model_for_tier(ModelTier::Small), "custom-small");
        // Default review-find tier is Medium; floored up to Large.
        assert_eq!(
            policy.resolve(DispatchStep::ReviewFind, None),
            "custom-large"
        );
        // Mechanical is exempt from the floor, so its default Small tier stands.
        assert_eq!(
            policy.resolve(DispatchStep::Mechanical, None),
            "custom-small"
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
