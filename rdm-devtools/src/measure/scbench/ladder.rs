//! `rdm-measure scbench ladder`: the operator's one trigger. A preflight that
//! fails before any spend, then `scbench run` over every manifest arm with
//! the given seeds, then `scbench report` over the run.
//!
//! COST WARNING: without `--dry-run` this runs paid Claude Code agents in
//! Docker (arms x seeds x problems x checkpoints). `--dry-run` prints the
//! projected run count ([`plan::projection`]), the preflight it would run and
//! the run plan, and spawns and creates nothing.
//!
//! The preflight runs cheapest first and stops at the first failure, before
//! any `slop-code` command and before `--out` is created:
//!
//! 1. with `--baseline`, the baseline loads from the manifest's baselines
//!    directory ([`baseline::load`]), as the report step will load it;
//! 2. the provider's credential variable is set ([`credential_variable`];
//!    only whether it is set is read, never its value);
//! 3. `uv --version` exits 0;
//! 4. `docker version --format {{.Server.Version}}` exits 0 (the daemon is
//!    reachable);
//! 5. when an arm runs in the rdm image, `scbench image check` passes
//!    ([`image::check`]).
//!
//! The run then probes `uv`, Docker and `claude` again itself, as every
//! `scbench run` does, so its record carries the observed versions.
//!
//! SIGINT, SIGTERM and SIGHUP are intercepted from before the preflight: a
//! signal during it stops the ladder before any `slop-code` command, and one
//! during the run stops it as `scbench run` does ([`super::exec::execute`]).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::exec::{Io, signal_name, spawn_command};
use super::image::{self, ImageOptions};
use super::manifest::{ArmImage, Manifest};
use super::plan::{self, CommandSpec, Probes, Tools};
use super::report::{self, Format, ReportOptions};
use super::{RunOptions, baseline, refuse_used_out};

/// Options for [`run`].
#[derive(Debug, Clone)]
pub struct LadderOptions {
    /// The manifest file.
    pub manifest: PathBuf,
    /// Where the pinned checkouts are kept.
    pub cache_dir: PathBuf,
    /// The run's output directory (must be missing or empty).
    pub out: PathBuf,
    /// The executables to invoke.
    pub tools: Tools,
    /// How many seeds every arm runs with.
    pub seeds: u64,
    /// The problems to run (`--problems`, [`RunOptions::problems`]); `None`
    /// runs every subset problem.
    pub problems: Option<Vec<String>>,
    /// A baseline the report renders the run beside.
    pub baseline: Option<String>,
    /// Print the projection, preflight and plan; spawn nothing and create
    /// nothing.
    pub dry_run: bool,
    /// How long a stopped run waits for `slop-code` to exit
    /// ([`RunOptions::stop_grace`]).
    pub stop_grace: Duration,
}

/// The environment variable SCBench's provider `provider` reads its
/// credential from; `None` for a provider rdm does not know.
pub fn credential_variable(provider: &str) -> Option<&'static str> {
    match provider {
        "claude_code_oauth" => Some("CLAUDE_CODE_OAUTH_TOKEN"),
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        _ => None,
    }
}

fn is_set(variable: &str) -> bool {
    std::env::var_os(variable).is_some_and(|v| !v.is_empty())
}

fn probe(spec: &CommandSpec, missing: &str, failing: &str) -> Result<(), String> {
    let (ran, _) = spawn_command(spec, Io::Capture);
    if ran.ok() {
        return Ok(());
    }
    let what = if ran.spawn_failed() { missing } else { failing };
    Err(format!(
        "{what} (`{}` {})",
        spec.argv().join(" "),
        ran.describe()
    ))
}

fn preflight(
    manifest: &Manifest,
    manifest_path: &Path,
    opts: &LadderOptions,
    probes: &Probes,
    image_check: bool,
    stderr: &mut dyn Write,
) -> Result<(), String> {
    if let Some(name) = &opts.baseline {
        baseline::load(&baseline::dir(Some(manifest_path))?, name)?;
    }
    let provider = &manifest.agent.provider;
    let Some(variable) = credential_variable(provider) else {
        return Err(format!(
            "provider {provider} has no known credential variable; the known pairs are \
             claude_code_oauth with CLAUDE_CODE_OAUTH_TOKEN and anthropic with ANTHROPIC_API_KEY"
        ));
    };
    if !is_set(variable) {
        return Err(format!(
            "{variable} is not set; provider {provider} needs it \
             (see docs/scbench-benchmark.md § Prerequisites)"
        ));
    }
    probe(&probes.uv, "uv not found", "uv is not usable")?;
    probe(
        &probes.docker,
        "the Docker daemon is not reachable (docker not found)",
        "the Docker daemon is not reachable",
    )?;
    if image_check {
        let check = image::check(
            &ImageOptions {
                manifest: opts.manifest.clone(),
                cache_dir: opts.cache_dir.clone(),
                docker: opts.tools.docker.clone(),
                uv: opts.tools.uv.clone(),
            },
            stderr,
        )?;
        if check != 0 {
            return Err("rdm image check failed (see above)".to_owned());
        }
    }
    Ok(())
}

/// Runs `scbench ladder`, returning the process exit code.
///
/// # Errors
///
/// When the manifest cannot be loaded, `--out` is used, the `--baseline`
/// name is invalid, the signal handling cannot be installed
/// (`cannot install SIGINT/SIGTERM handling: ...`), a signal arrives during
/// the preflight (`ladder: interrupted by <SIG> during the preflight; no
/// slop-code command ran`, which takes precedence over a preflight failure),
/// a preflight check fails (`ladder preflight: ...`), or the run or the
/// report fails (either with a hint that the run's finished units are
/// reportable with `scbench report --run`). The ladder installs the signal
/// handling once, before the preflight, and the run uses the same one
/// ([`super::run_with`]), so a signal recorded at any point stops it.
pub fn run(
    opts: &LadderOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<u8, String> {
    if let Some(name) = &opts.baseline {
        baseline::check_name("--baseline", name)?;
    }
    let manifest_path = std::path::absolute(&opts.manifest).unwrap_or(opts.manifest.clone());
    let manifest = Manifest::load(&manifest_path)?
        .select_problems(opts.problems.as_deref())
        .map_err(|e| format!("{e} ({})", manifest_path.display()))?;
    let out = std::path::absolute(&opts.out).unwrap_or(opts.out.clone());
    refuse_used_out(&out)?;
    let projection = plan::projection(&manifest, opts.seeds);
    let probes = plan::probes(&opts.tools);
    let image_check = manifest.arms.iter().any(|a| a.image == ArmImage::Rdm);
    let _ = writeln!(
        stderr,
        "scbench ladder: projected {} checkpoint runs ({} arms × {} seeds × {} checkpoints over {} problems), {} problem sessions",
        projection.checkpoint_runs,
        projection.arms,
        projection.seeds,
        projection.checkpoints,
        projection.problems.len(),
        projection.sessions,
    );
    let run_options = |dry_run: bool| RunOptions {
        manifest: manifest_path.clone(),
        cache_dir: opts.cache_dir.clone(),
        out: out.clone(),
        tools: opts.tools.clone(),
        arms: None,
        problems: opts.problems.clone(),
        seeds: opts.seeds,
        dry_run,
        stop_grace: opts.stop_grace,
    };

    if opts.dry_run {
        let mut plan_json = Vec::new();
        super::run(&run_options(true), &mut plan_json, stderr)?;
        let plan: Value = serde_json::from_slice(&plan_json).map_err(|e| e.to_string())?;
        let variable = credential_variable(&manifest.agent.provider);
        let body = serde_json::to_string_pretty(&json!({
            "projection": projection,
            "preflight": {
                "credential": {
                    "variable": variable,
                    "set": variable.is_some_and(is_set),
                },
                "uv": probes.uv.argv(),
                "docker": probes.docker.argv(),
                "image_check": image_check,
            },
            "plan": plan,
        }))
        .map_err(|e| e.to_string())?;
        let _ = writeln!(stdout, "{body}");
        return Ok(0);
    }

    let interrupts = crate::process::Interrupts::install()
        .map_err(|e| format!("cannot install SIGINT/SIGTERM handling: {e}"))?;
    let preflight = preflight(
        &manifest,
        &manifest_path,
        opts,
        &probes,
        image_check,
        stderr,
    );
    if let Some(sig) = interrupts.received() {
        return Err(format!(
            "ladder: interrupted by {} during the preflight; no slop-code command ran",
            signal_name(sig)
        ));
    }
    preflight.map_err(|e| format!("ladder preflight: {e}"))?;
    super::run_with(&run_options(false), &interrupts, stdout, stderr).map_err(|e| {
        format!(
            "{e}; the finished arm runs are on disk; rdm-measure scbench report --run {} reports them",
            out.display()
        )
    })?;
    report::run(
        &ReportOptions {
            run: out.clone(),
            baseline: opts.baseline.clone(),
            save_baseline: None,
            format: Format::Markdown,
        },
        stdout,
        stderr,
    )
    .map_err(|e| {
        format!(
            "{e}; the run finished and is on disk; rdm-measure scbench report --run {} reports it",
            out.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_known_provider_names_its_credential_variable() {
        assert_eq!(
            credential_variable("claude_code_oauth"),
            Some("CLAUDE_CODE_OAUTH_TOKEN")
        );
        assert_eq!(credential_variable("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(credential_variable("openrouter"), None);
    }
}
