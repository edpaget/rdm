//! `rdm-measure scbench run`: drives SlopCodeBench (SCBench) reproducibly
//! from the pinned manifest in `bench/scbench/manifest.toml`.
//!
//! COST WARNING: `scbench run` without `--dry-run` runs paid Claude Code
//! agents in Docker (arms x problems x checkpoints). `--dry-run` prints the
//! plan and spawns nothing.
//!
//! A run, in order:
//!
//! 1. loads and validates the manifest ([`manifest`]);
//! 2. refuses an `--out` that exists and is non-empty;
//! 3. with `--dry-run`, prints the plan ([`plan`]) as JSON and stops;
//! 4. resolves the rdm SHA (`git -C <manifest dir> rev-parse HEAD`);
//! 5. probes the `uv`, Docker server and host `claude` versions;
//! 6. provisions the SCBench and problem checkouts at their pins under
//!    `--cache-dir` and copies the manifest's model file into
//!    `<scbench>/configs/models/` (rdm's only addition to the checkout);
//! 7. for each arm, writes its agent and run configs, then runs
//!    `uv run slop-code run --no-evaluate` and `uv run slop-code eval`.
//!
//! The run record `<out>/rdm-scbench-run.json` ([`exec::RunRecord`]) is
//! rewritten after every command, so a failed run still records the command
//! that failed.

pub mod exec;
pub mod manifest;
pub mod plan;

use std::io::Write;
use std::path::PathBuf;

use serde_json::json;

use manifest::Manifest;
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
    /// Print the plan; spawn nothing and create nothing.
    pub dry_run: bool,
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
    let manifest = Manifest::load(&manifest_path)?;
    let out = std::path::absolute(&opts.out).unwrap_or(opts.out.clone());
    if out.exists() {
        let used = std::fs::read_dir(&out).map_or(true, |mut entries| entries.next().is_some());
        if used {
            return Err(format!(
                "--out {} already exists and is not empty; choose a fresh directory",
                out.display()
            ));
        }
    }
    let paths = Paths {
        manifest_dir: manifest.dir.clone(),
        cache_dir: std::path::absolute(&opts.cache_dir).unwrap_or(opts.cache_dir.clone()),
        out_dir: out,
    };
    let plan = build_plan(&manifest, &paths, &opts.tools);
    if opts.dry_run {
        let body = serde_json::to_string_pretty(&json!({
            "manifest": manifest,
            "manifest_path": manifest_path,
            "rdm": { "sha": null },
            "plan": plan,
        }))
        .map_err(|e| e.to_string())?;
        let _ = writeln!(stdout, "{body}");
        return Ok(0);
    }
    exec::execute(&manifest, &manifest_path, &plan)?;
    let _ = writeln!(
        stderr,
        "scbench run: {} arm(s) finished; run record {}",
        plan.arms.len(),
        plan.record_path.display()
    );
    Ok(0)
}
