//! `rdm model` — thin CLI porcelain over `rdm_core::model_policy::ModelPolicy`.
use anyhow::{Result, bail};
use rdm_core::config::Config;
use rdm_core::model::ModelTier;
use rdm_core::model_policy::{DispatchStep, Host, ModelPolicy, Profile};
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
/// concrete model id (or, with `--format json`, a model + effort profile),
/// or showing the full resolved policy for a host.
///
/// # Errors
///
/// Returns an error if `step`, `--tier`, or `--host` fail to parse, or if
/// `--format table` is requested for `model show` (unsupported), or if
/// the resolved policy cannot be serialized as JSON.
pub fn run(command: ModelCommand, repo_config: &Config, format: OutputFormat) -> Result<()> {
    let policy = ModelPolicy::from_config(repo_config);
    match command {
        ModelCommand::Resolve { step, tier, host } => {
            run_resolve(&policy, step, tier, host, format)
        }
        ModelCommand::Show { host } => run_show(&policy, parse_host(host)?, format),
    }
}

/// Parses `--host`, defaulting to [`Host::Claude`]; a bad value surfaces the
/// core `ParseError` verbatim.
fn parse_host(host: Option<String>) -> Result<Host> {
    Ok(match host {
        Some(h) => h.parse::<Host>()?,
        None => Host::default(),
    })
}

/// Note: `step` is parsed before `tier`, and `tier` before `host`, so if several are invalid the earliest error is the one surfaced.
fn run_resolve(
    policy: &ModelPolicy,
    step: String,
    tier: Option<String>,
    host: Option<String>,
    format: OutputFormat,
) -> Result<()> {
    let step: DispatchStep = step.parse()?; // ParseError -> anyhow, verbatim, no .context
    let hint: Option<ModelTier> = match tier {
        Some(t) => Some(t.parse::<ModelTier>()?),
        None => None,
    };
    let host = parse_host(host)?;
    let tier = policy.resolve_tier(step, hint);
    let profile = policy.profile(host, tier);
    match format {
        OutputFormat::Json => {
            let view = ResolvedModelView {
                step: step.to_string(),
                host: host.to_string(),
                tier: tier.to_string(),
                model: profile.model.clone(),
                effort: profile.effort.to_string(),
            };
            println!("{}", serde_json::to_string_pretty(&view)?);
        }
        // Preserve the existing bare-model-id output for all non-JSON formats:
        // skills read it verbatim via `model=$(rdm model resolve …)`.
        OutputFormat::Human | OutputFormat::Markdown | OutputFormat::Table => {
            println!("{}", profile.model);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ResolvedModelView {
    step: String,
    host: String,
    tier: String,
    model: String,
    effort: String,
}

#[derive(Serialize)]
struct ModelPolicyView {
    host: String,
    small: String,
    medium: String,
    large: String,
    frontier: String,
    review_floor: String,
    profiles: ProfilesView,
    steps: Vec<StepView>,
}

#[derive(Serialize)]
struct ProfilesView {
    small: Profile,
    medium: Profile,
    large: Profile,
    frontier: Profile,
}

#[derive(Serialize)]
struct StepView {
    step: String,
    tier: String,
    model: String,
    effort: String,
}

fn build_view(policy: &ModelPolicy, host: Host) -> ModelPolicyView {
    let profile = |tier| policy.profile(host, tier).clone();
    ModelPolicyView {
        host: host.to_string(),
        small: profile(ModelTier::Small).model,
        medium: profile(ModelTier::Medium).model,
        large: profile(ModelTier::Large).model,
        frontier: profile(ModelTier::Frontier).model,
        review_floor: policy.review_floor().to_string(),
        profiles: ProfilesView {
            small: profile(ModelTier::Small),
            medium: profile(ModelTier::Medium),
            large: profile(ModelTier::Large),
            frontier: profile(ModelTier::Frontier),
        },
        steps: ALL_STEPS
            .iter()
            .map(|s| {
                let tier = policy.resolve_tier(*s, None);
                let p = policy.profile(host, tier);
                StepView {
                    step: s.to_string(),
                    tier: tier.to_string(),
                    model: p.model.clone(),
                    effort: p.effort.to_string(),
                }
            })
            .collect(),
    }
}

fn run_show(policy: &ModelPolicy, host: Host, format: OutputFormat) -> Result<()> {
    let view = build_view(policy, host);
    match format {
        // Markdown renders identically to Human — settled precedent (next.rs:38, phase.rs, roadmap.rs).
        OutputFormat::Human | OutputFormat::Markdown => {
            println!("host: {}", view.host);
            let p = &view.profiles;
            for (name, profile) in [
                ("small", &p.small),
                ("medium", &p.medium),
                ("large", &p.large),
                ("frontier", &p.frontier),
            ] {
                println!("{name}: {} @ {}", profile.model, profile.effort);
            }
            println!("review_floor: {}", view.review_floor);
            println!();
            for s in &view.steps {
                println!("{}: {} @ {}", s.step, s.model, s.effort);
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
        let view = build_view(&policy, Host::Claude);
        assert_eq!(view.host, "claude");
        assert_eq!(view.small, "opus");
        assert_eq!(view.medium, "opus");
        assert_eq!(view.large, "opus");
        assert_eq!(view.frontier, "opus");
        assert_eq!(view.profiles.frontier.effort.to_string(), "xhigh");
        assert_eq!(view.review_floor, "medium");
        assert_eq!(view.steps.len(), 5);
        assert_eq!(view.steps[0].step, "plan");
        assert_eq!(view.steps[0].effort, "medium");
        assert_eq!(view.steps[1].step, "implement");
        assert_eq!(view.steps[2].step, "review-find");
        assert_eq!(view.steps[2].model, "opus");
        assert_eq!(view.steps[2].effort, "medium");
        assert_eq!(view.steps[3].step, "review-verify");
        assert_eq!(view.steps[3].model, "opus");
        assert_eq!(view.steps[3].effort, "high");
        assert_eq!(view.steps[4].step, "mechanical");
        assert_eq!(view.steps[4].model, "opus");
        assert_eq!(view.steps[4].effort, "low");
    }

    #[test]
    fn build_view_codex_host() {
        let policy = ModelPolicy::from_config(&Config::default());
        let view = build_view(&policy, Host::Codex);
        assert_eq!(view.host, "codex");
        assert_eq!(view.small, "gpt-6-sol");
        assert_eq!(view.large, "gpt-6-astra");
        assert_eq!(view.steps[3].model, "gpt-6-astra");
        assert_eq!(view.steps[3].effort, "medium");
    }
}
