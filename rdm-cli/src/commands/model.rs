//! `rdm model` — thin CLI porcelain over `rdm_core::model_policy::ModelPolicy`.
use anyhow::{Result, bail};
use rdm_core::config::Config;
use rdm_core::model::ModelTier;
use rdm_core::model_policy::{DispatchStep, ModelPolicy};
use serde::Serialize;

use crate::{ModelCommand, OutputFormat};

/// Every dispatch step, in canonical display order for `rdm model show`.
const ALL_STEPS: [DispatchStep; 5] = [
    DispatchStep::Plan,
    DispatchStep::Implement,
    DispatchStep::ReviewFind,
    DispatchStep::ReviewVerify,
    DispatchStep::Mechanical,
];

/// Runs the `rdm model` command family: resolving a dispatch step to a
/// concrete model id, or showing the full resolved policy.
///
/// # Errors
///
/// Returns an error if `step` or `--tier` fail to parse, or if
/// `--format table` is requested for `model show` (unsupported).
pub fn run(command: ModelCommand, repo_config: &Config, format: OutputFormat) -> Result<()> {
    let policy = ModelPolicy::from_config(repo_config);
    match command {
        ModelCommand::Resolve { step, tier } => run_resolve(&policy, step, tier),
        ModelCommand::Show => run_show(&policy, format),
    }
}

/// Note: `step` is parsed before `tier`, so if both are invalid the step error is the one surfaced.
fn run_resolve(policy: &ModelPolicy, step: String, tier: Option<String>) -> Result<()> {
    let step: DispatchStep = step.parse()?; // ParseError -> anyhow, verbatim, no .context
    let hint: Option<ModelTier> = match tier {
        Some(t) => Some(t.parse::<ModelTier>()?),
        None => None,
    };
    println!("{}", policy.resolve(step, hint));
    Ok(())
}

#[derive(Serialize)]
struct ModelPolicyView {
    small: String,
    medium: String,
    large: String,
    review_floor: String,
    steps: Vec<StepView>,
}

#[derive(Serialize)]
struct StepView {
    step: String,
    model: String,
}

fn build_view(policy: &ModelPolicy) -> ModelPolicyView {
    ModelPolicyView {
        small: policy.model_for_tier(ModelTier::Small).to_string(),
        medium: policy.model_for_tier(ModelTier::Medium).to_string(),
        large: policy.model_for_tier(ModelTier::Large).to_string(),
        review_floor: policy.review_floor().to_string(),
        steps: ALL_STEPS
            .iter()
            .map(|s| StepView {
                step: s.to_string(),
                model: policy.resolve(*s, None).to_string(),
            })
            .collect(),
    }
}

fn run_show(policy: &ModelPolicy, format: OutputFormat) -> Result<()> {
    let view = build_view(policy);
    match format {
        // Markdown renders identically to Human — settled precedent (next.rs:38, phase.rs, roadmap.rs).
        OutputFormat::Human | OutputFormat::Markdown => {
            println!("small: {}", view.small);
            println!("medium: {}", view.medium);
            println!("large: {}", view.large);
            println!("review_floor: {}", view.review_floor);
            println!();
            for s in &view.steps {
                println!("{}: {}", s.step, s.model);
            }
        }
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&view)?),
        OutputFormat::Table => bail!(
            "--format table is not supported for 'model show'; use --format human, --format json, --format markdown, or omit --format"
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_view_reflects_default_bindings_and_all_five_steps() {
        let policy = ModelPolicy::from_config(&Config::default());
        let view = build_view(&policy);
        assert_eq!(view.small, "haiku");
        assert_eq!(view.medium, "sonnet");
        assert_eq!(view.large, "opus");
        assert_eq!(view.review_floor, "medium");
        assert_eq!(view.steps.len(), 5);
        assert_eq!(view.steps[0].step, "plan");
        assert_eq!(view.steps[1].step, "implement");
        assert_eq!(view.steps[2].step, "review-find");
        assert_eq!(view.steps[2].model, "sonnet");
        assert_eq!(view.steps[3].step, "review-verify");
        assert_eq!(view.steps[3].model, "opus");
        assert_eq!(view.steps[4].step, "mechanical");
        assert_eq!(view.steps[4].model, "haiku");
    }
}
