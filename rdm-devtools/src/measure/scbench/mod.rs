//! `rdm-measure scbench run`: drives SlopCodeBench (SCBench) reproducibly
//! from the pinned manifest in `bench/scbench/manifest.toml`; and
//! `rdm-measure scbench image build|check`: builds and smoke-checks the
//! rdm-equipped agent image an `image = "rdm"` arm runs in ([`image`]).
//!
//! COST WARNING: `scbench run` without `--dry-run` runs paid Claude Code
//! agents in Docker (arms x seeds x problems x checkpoints). `--dry-run`
//! prints the plan and its projected run count ([`plan::projection`]) and
//! spawns nothing.
//!
//! A run, in order:
//!
//! 1. loads and validates the manifest ([`manifest`]) and keeps the arms
//!    `--arms` selects ([`Manifest::select_arms`]);
//! 2. refuses an `--out` that exists and is non-empty;
//! 3. with `--dry-run`, prints the plan ([`plan`]) as JSON and stops;
//! 4. resolves the rdm SHA (`git -C <manifest dir> rev-parse HEAD`);
//! 5. probes the `uv`, Docker server and host `claude` versions;
//! 6. provisions the SCBench and problem checkouts at their pins under
//!    `--cache-dir` and copies the manifest's model file into
//!    `<scbench>/configs/models/` (rdm's only addition to the checkout);
//! 7. for rdm arms, reads the SCBench environment's graded directory
//!    ([`environment`]) and refuses a `[rdm] plan_root` inside it;
//! 8. for each seed, then each arm (seed-major, so the finished seeds of an
//!    interrupted run cover every arm): writes the arm's agent config once
//!    (for an rdm arm, its rendered agent template, [`rdm_image`]; for a
//!    `prompt_from = "manifest"` arm, its rendered `prompt.jinja`,
//!    [`lane::render_prompt`]) and the unit's run config under
//!    `<out>/<arm>/seed-<n>/`, then runs
//!    `uv run slop-code --seed <n> run --no-evaluate`; for an rdm arm, copies
//!    each checkpoint's plan repo to `<out>/<arm>/seed-<n>/plan-repos/` and
//!    classifies the checkpoint ([`lane`]); then runs
//!    `uv run slop-code --seed <n> eval`. After each `slop-code` run and
//!    eval, whatever its exit status, the unit's `seed-<n>` directory is
//!    scrubbed of the provider's credential ([`redact`]).
//!
//! The run record `<out>/rdm-scbench-run.json` ([`exec::RunRecord`]) is
//! rewritten after every command, so a failed run still records the command
//! that failed.
//!
//! `scbench ladder` ([`ladder`]) is the operator's one trigger: a preflight
//! that fails before any spend (credential, `uv`, the Docker daemon and, for
//! rdm arms, `scbench image check`), then `scbench run` over every manifest
//! arm with the given seeds, then `scbench report`. The operator's guide is
//! `docs/scbench-benchmark.md`.
//!
//! `scbench report` ([`report`]) aggregates a run's SCBench outputs
//! ([`outputs`]) per arm with seed-bootstrap confidence intervals
//! ([`stats`]), sets every rdm arm against every control arm, and saves or
//! compares a baseline ([`baseline`]). It makes no call at all.
//!
//! `scbench image build` (slow: a release build of rdm plus SCBench's base
//! image) builds the payload image from the rdm checkout under test, then has
//! SCBench build the agent image; `scbench image check` runs the smoke check
//! in it. Neither runs an agent or makes a paid call.

pub mod baseline;
pub mod environment;
pub mod exec;
pub mod image;
pub mod ladder;
pub mod lane;
pub mod manifest;
pub mod outputs;
pub mod plan;
pub mod rdm_image;
pub mod redact;
pub mod report;
pub mod stats;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use serde_json::json;

use manifest::{ArmImage, Manifest};
use plan::{Paths, Tools, build_plan};

/// Options for [`run`].
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The manifest file.
    pub manifest: PathBuf,
    /// Where the pinned checkouts are kept.
    pub cache_dir: PathBuf,
    /// The run's output directory (must be missing or empty).
    pub out: PathBuf,
    /// The executables to invoke.
    pub tools: Tools,
    /// The arms to run (`--arms`); `None` runs every manifest arm.
    pub arms: Option<Vec<String>>,
    /// How many seeds each arm runs with (`1..=seeds`).
    pub seeds: u64,
    /// Print the plan; spawn nothing and create nothing.
    pub dry_run: bool,
}

/// Refuses an `--out` that exists and is not empty.
///
/// # Errors
///
/// When `out` holds anything, naming it.
fn refuse_used_out(out: &std::path::Path) -> Result<(), String> {
    if out.exists() {
        let used = std::fs::read_dir(out).map_or(true, |mut entries| entries.next().is_some());
        if used {
            return Err(format!(
                "--out {} already exists and is not empty; choose a fresh directory",
                out.display()
            ));
        }
    }
    Ok(())
}

/// Runs `scbench run`, returning the process exit code.
///
/// # Errors
///
/// Any failure that stops the run; the binary prints it and exits 1.
pub fn run(
    opts: &RunOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<u8, String> {
    let manifest_path = std::path::absolute(&opts.manifest).unwrap_or(opts.manifest.clone());
    let manifest = Manifest::load(&manifest_path)?
        .select_arms(opts.arms.as_deref())
        .map_err(|e| format!("{e} ({})", manifest_path.display()))?;
    let seeds: Vec<u64> = (1..=opts.seeds).collect();
    let out = std::path::absolute(&opts.out).unwrap_or(opts.out.clone());
    refuse_used_out(&out)?;
    let paths = Paths {
        manifest_dir: manifest.dir.clone(),
        cache_dir: std::path::absolute(&opts.cache_dir).unwrap_or(opts.cache_dir.clone()),
        out_dir: out,
    };
    let template = if manifest.arms.iter().any(|a| a.image == ArmImage::Rdm) {
        let path = manifest.dir.join(rdm_image::TEMPLATE);
        Some(
            std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read agent template {}: {e}", path.display()))?,
        )
    } else {
        None
    };
    let mut prompts = BTreeMap::new();
    for arm in &manifest.arms {
        if let Some(path) = manifest.manifest_prompt_source(arm) {
            let text = std::fs::read_to_string(&path).map_err(|e| {
                format!(
                    "arm {}: cannot read prompt {}: {e}",
                    arm.name,
                    path.display()
                )
            })?;
            prompts.insert(arm.name.clone(), text);
        }
    }
    if opts.dry_run {
        let plan = build_plan(
            &manifest,
            &paths,
            &opts.tools,
            &seeds,
            None,
            template.as_deref(),
            &prompts,
        );
        let selected: Vec<&str> = manifest.arms.iter().map(|a| a.name.as_str()).collect();
        let body = serde_json::to_string_pretty(&json!({
            "manifest": manifest,
            "manifest_path": manifest_path,
            "rdm": { "sha": null },
            "seeds": seeds,
            "selected_arms": selected,
            "projection": plan::projection(&manifest, opts.seeds),
            "plan": plan,
        }))
        .map_err(|e| e.to_string())?;
        let _ = writeln!(stdout, "{body}");
        return Ok(0);
    }
    let plan = exec::execute(
        &manifest,
        &manifest_path,
        &paths,
        &opts.tools,
        &seeds,
        template.as_deref(),
        &prompts,
        stderr,
    )?;
    let _ = writeln!(
        stderr,
        "scbench run: {} arm run(s) finished; run record {}",
        plan.arms.len(),
        plan.record_path.display()
    );
    Ok(0)
}
