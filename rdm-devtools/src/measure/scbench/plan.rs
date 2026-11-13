//! Pure construction of a run plan from a [`Manifest`]: every command, path
//! and generated config, with no I/O.
//!
//! Every pinned value reaches SCBench through the generated run and agent
//! configs; no CLI flag repeats one, so no SCBench default and no second copy
//! can decide anything.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use super::lane;
use super::manifest::{Agent, ArmGroup, ArmImage, Limits, Manifest};
use super::rdm_image;

/// The directories a plan is built against (all absolute).
#[derive(Debug, Clone)]
pub struct Paths {
    /// The manifest's directory (inside the rdm checkout under test).
    pub manifest_dir: PathBuf,
    /// Where the SCBench and problem checkouts live.
    pub cache_dir: PathBuf,
    /// The run's output directory.
    pub out_dir: PathBuf,
}

/// The external executables the plan invokes.
#[derive(Debug, Clone)]
pub struct Tools {
    /// The `uv` executable.
    pub uv: String,
    /// The `docker` executable.
    pub docker: String,
    /// The host `claude` executable.
    pub claude: String,
}

/// One command to spawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    /// The executable.
    pub program: String,
    /// The arguments after the program.
    pub args: Vec<String>,
    /// The working directory (`None` inherits the runner's).
    pub cwd: Option<PathBuf>,
    /// Extra environment variables.
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|&a| a.to_owned()).collect(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    /// The program followed by its arguments.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect()
    }
}

/// How one pinned checkout is provisioned.
#[derive(Debug, Clone, Serialize)]
pub struct Checkout {
    /// The checkout's directory.
    pub dir: PathBuf,
    /// The pinned SHA.
    pub sha: String,
    /// Run only when `dir` does not exist yet.
    pub clone: CommandSpec,
    /// Run only when `dir` already exists: points the cached checkout's
    /// `origin` at the manifest's `repo`, so a changed `repo` is fetched from.
    pub set_url: CommandSpec,
    /// Always run.
    pub fetch: CommandSpec,
    /// Always run; detaches at the pin.
    pub checkout: CommandSpec,
}

/// The three version probes, run before any provisioning or arm.
#[derive(Debug, Clone, Serialize)]
pub struct Probes {
    /// `uv --version`.
    pub uv: CommandSpec,
    /// `docker version --format {{.Server.Version}}`.
    pub docker: CommandSpec,
    /// `claude --version`.
    pub claude: CommandSpec,
}

/// The model-file copy into the SCBench checkout.
#[derive(Debug, Clone, Serialize)]
pub struct ModelCopy {
    /// The manifest's model file.
    pub source: PathBuf,
    /// Its destination under `<scbench>/configs/models/`.
    pub installed: PathBuf,
}

/// One unit of a run: one arm at one seed, with its generated configs and
/// commands. The agent config, agent template and prompt are the arm's,
/// shared by its seeds; the run config, run directory and plan repos are the
/// seed's own.
#[derive(Debug, Clone, Serialize)]
pub struct ArmPlan {
    /// The arm name.
    pub name: String,
    /// The SCBench seed (`slop-code --seed <n>`) this unit runs with.
    pub seed: u64,
    /// The agent image the arm runs in.
    pub image: ArmImage,
    /// The arm's treatment group ([`super::manifest::Arm::group`]).
    pub group: ArmGroup,
    /// The verify command the arm runs under
    /// ([`super::manifest::Arm::verify`]); `None` for an upstream stock
    /// control.
    pub verify: Option<String>,
    /// `<out>/<arm>/agent.yaml`.
    pub agent_config_path: PathBuf,
    /// The agent config written there.
    pub agent_config: Value,
    /// `<out>/<arm>/seed-<n>/run.yaml`.
    pub run_config_path: PathBuf,
    /// The run config written there.
    pub run_config: Value,
    /// `<out>/<arm>/docker.j2`, for an rdm arm.
    pub docker_template_path: Option<PathBuf>,
    /// The rendered agent template written there, for an rdm arm.
    pub docker_template: Option<String>,
    /// `<out>/<arm>/prompt.jinja`, for an arm whose prompt is rdm's own
    /// (`prompt_from = "manifest"`); the run config's `prompt`.
    pub prompt_path: Option<PathBuf>,
    /// The prompt written there, its `@RDM_...@` placeholders filled
    /// ([`lane::render_prompt`]); SCBench's Jinja renders the rest.
    pub prompt: Option<String>,
    /// `<out>/<arm>/seed-<n>/plan-repos`, for an rdm arm: where the runner
    /// copies each checkpoint's plan repo (`lane::collect`).
    pub plan_repos_dir: Option<PathBuf>,
    /// SCBench's own output directory for the unit
    /// (`<out>/<arm>/seed-<n>/output`).
    pub run_dir: PathBuf,
    /// `uv run slop-code --seed <n> run ...`.
    pub run: CommandSpec,
    /// `uv run slop-code --seed <n> eval ...`.
    pub eval: CommandSpec,
}

/// Everything a run spawns and writes, in order.
#[derive(Debug, Clone, Serialize)]
pub struct RunPlan {
    /// `git -C <manifest dir> rev-parse HEAD`.
    pub rdm_sha: CommandSpec,
    /// The version probes.
    pub probes: Probes,
    /// The SCBench checkout.
    pub scbench: Checkout,
    /// The problem checkout.
    pub problems: Checkout,
    /// The model-file copy.
    pub model_file: ModelCopy,
    /// The units in seed-major order: every arm (in manifest order) for the
    /// first seed, then every arm for the next.
    pub arms: Vec<ArmPlan>,
    /// The run record's path (`<out>/rdm-scbench-run.json`).
    pub record_path: PathBuf,
}

/// The run record's file name, beside the arm directories.
pub const RECORD_FILE: &str = "rdm-scbench-run.json";

/// How many SCBench runs a manifest's arms and a seed count make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Projection {
    /// The arms run.
    pub arms: u64,
    /// The seeds each arm runs with.
    pub seeds: u64,
    /// Each problem's checkpoint count (`[problems] checkpoints`).
    pub problems: BTreeMap<String, u64>,
    /// The checkpoints over the subset (the sum of `problems`).
    pub checkpoints: u64,
    /// The `slop-code` problem sessions: arms x seeds x problems.
    pub sessions: u64,
    /// The agent checkpoint runs: arms x seeds x checkpoints.
    pub checkpoint_runs: u64,
}

/// The projected run count of `manifest`'s arms over `seeds` seeds.
pub fn projection(manifest: &Manifest, seeds: u64) -> Projection {
    let arms = manifest.arms.len() as u64;
    let problems: BTreeMap<String, u64> = manifest
        .problems
        .subset
        .iter()
        .map(|p| {
            (
                p.clone(),
                manifest.problems.checkpoints.get(p).copied().unwrap_or(0),
            )
        })
        .collect();
    let checkpoints = problems.values().sum();
    Projection {
        arms,
        seeds,
        sessions: arms * seeds * problems.len() as u64,
        checkpoint_runs: arms * seeds * checkpoints,
        checkpoints,
        problems,
    }
}

/// The version probes a run (and the ladder's preflight) spawns.
pub fn probes(tools: &Tools) -> Probes {
    Probes {
        uv: CommandSpec::new(&tools.uv, &["--version"]),
        docker: CommandSpec::new(
            &tools.docker,
            &["version", "--format", "{{.Server.Version}}"],
        ),
        claude: CommandSpec::new(&tools.claude, &["--version"]),
    }
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

/// `git -C <manifest dir> rev-parse HEAD`: the rdm SHA under test.
pub fn rdm_sha_command(paths: &Paths) -> CommandSpec {
    CommandSpec::new("git", &["-C", &s(&paths.manifest_dir), "rev-parse", "HEAD"])
}

/// The SCBench checkout's directory under `cache_dir`.
pub fn scbench_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("slop-code-bench")
}

/// How the SCBench checkout is provisioned under `cache_dir`.
pub fn scbench_checkout(manifest: &Manifest, cache_dir: &Path) -> Checkout {
    checkout(
        &manifest.scbench.repo,
        &manifest.scbench.sha,
        scbench_dir(cache_dir),
    )
}

/// The agent config a stock arm runs with, under `limits`.
pub fn stock_agent_config(a: &Agent, limits: &Limits) -> Value {
    json!({
        "type": "claude_code",
        "binary": "claude",
        "permission_mode": "bypassPermissions",
        "version": a.claude_code_version,
        "cost_limits": {
            "cost_limit": limits.cost_limit,
            "step_limit": limits.step_limit,
            "net_cost_limit": limits.net_cost_limit,
        },
    })
}

fn checkout(repo: &str, sha: &str, dir: PathBuf) -> Checkout {
    let d = s(&dir);
    Checkout {
        clone: CommandSpec::new("git", &["clone", "--no-checkout", repo, &d]),
        set_url: CommandSpec::new("git", &["-C", &d, "remote", "set-url", "origin", repo]),
        fetch: CommandSpec::new("git", &["-C", &d, "fetch", "--quiet", "origin"]),
        checkout: CommandSpec::new("git", &["-C", &d, "checkout", "--quiet", "--detach", sha]),
        sha: sha.to_owned(),
        dir,
    }
}

/// Builds the plan for `manifest` against `paths`, invoking `tools`, with one
/// unit per arm and seed in `seeds`, seed-major.
///
/// `rdm_sha` is the rdm checkout's HEAD (`None` before it is resolved, as in
/// `--dry-run`, where rdm arms show [`rdm_image::SHA_PLACEHOLDER`]).
/// `template` is the unrendered [`rdm_image::TEMPLATE`]; the caller reads it
/// when an arm selects the rdm image. `prompts` maps each
/// `prompt_from = "manifest"` arm's name to its unrendered template, which
/// the caller reads.
pub fn build_plan(
    manifest: &Manifest,
    paths: &Paths,
    tools: &Tools,
    seeds: &[u64],
    rdm_sha: Option<&str>,
    template: Option<&str>,
    prompts: &BTreeMap<String, String>,
) -> RunPlan {
    let scbench_dir = scbench_dir(&paths.cache_dir);
    let problems_dir = paths.cache_dir.join("scb-problems");
    let source = manifest.model_file_source();
    let file_name = source
        .file_name()
        .map_or_else(|| "model.yaml".into(), std::ffi::OsStr::to_os_string);
    let installed = scbench_dir.join("configs/models").join(file_name);

    let mut env = BTreeMap::new();
    env.insert("SCBENCH_PROBLEMS_PATH".to_owned(), s(&problems_dir));
    env.insert(
        "SCBENCH_HOME".to_owned(),
        s(&paths.cache_dir.join("scbench-home")),
    );
    let a = &manifest.agent;
    let units = seeds
        .iter()
        .flat_map(|&seed| manifest.arms.iter().map(move |arm| (seed, arm)));
    let arms = units
        .map(|(seed, arm)| {
            let dir = paths.out_dir.join(&arm.name);
            let seed_dir = dir.join(format!("seed-{seed}"));
            let seed_arg = seed.to_string();
            let agent_config_path = dir.join("agent.yaml");
            let run_config_path = seed_dir.join("run.yaml");
            let run_dir = seed_dir.join("output");
            let limits = arm.limits(a);
            let (prompt_path, prompt) = match (prompts.get(&arm.name), &manifest.rdm) {
                (Some(raw), Some(rdm)) if manifest.manifest_prompt_source(arm).is_some() => (
                    Some(dir.join("prompt.jinja")),
                    Some(lane::render_prompt(raw, rdm)),
                ),
                _ => (None, None),
            };
            let (agent_config, docker_template_path, docker_template) =
                match (arm.image, &manifest.rdm) {
                    (ArmImage::Rdm, Some(rdm)) => {
                        let sha = rdm_sha.unwrap_or(rdm_image::SHA_PLACEHOLDER);
                        let path = dir.join("docker.j2");
                        let rendered = rdm_image::render_template(
                            template.unwrap_or_default(),
                            &a.claude_code_version,
                            sha,
                            &rdm.plan_root,
                        );
                        (
                            rdm_image::agent_config(manifest, rdm, sha, &path, &limits),
                            Some(path),
                            Some(rendered),
                        )
                    }
                    _ => (stock_agent_config(a, &limits), None, None),
                };
            let prompt_config = prompt_path
                .as_deref()
                .map_or_else(|| s(&scbench_dir.join(&arm.prompt)), s);
            let run_config = json!({
                "agent": s(&agent_config_path),
                "environment": a.environment,
                "prompt": prompt_config,
                "model": { "provider": a.provider, "name": a.model },
                "thinking": a.thinking,
                "pass_policy": a.pass_policy,
                "problems": manifest.problems.subset,
                "save_dir": s(&seed_dir),
                "save_template": "output",
            });
            let uv = |args: &[&str]| CommandSpec {
                cwd: Some(scbench_dir.clone()),
                env: env.clone(),
                ..CommandSpec::new(&tools.uv, args)
            };
            ArmPlan {
                name: arm.name.clone(),
                seed,
                image: arm.image,
                group: arm.group(),
                verify: arm.verify(manifest.rdm.as_ref()).map(str::to_owned),
                docker_template_path,
                docker_template,
                prompt_path,
                prompt,
                plan_repos_dir: (arm.image == ArmImage::Rdm).then(|| seed_dir.join("plan-repos")),
                run: uv(&[
                    "run",
                    "slop-code",
                    "--seed",
                    &seed_arg,
                    "run",
                    "--config",
                    &s(&run_config_path),
                    "--no-evaluate",
                    "--no-live-progress",
                ]),
                eval: uv(&[
                    "run",
                    "slop-code",
                    "--seed",
                    &seed_arg,
                    "eval",
                    &s(&run_dir),
                    "--pass-policy",
                    &a.pass_policy,
                    "--no-live-progress",
                ]),
                agent_config_path,
                agent_config,
                run_config_path,
                run_config,
                run_dir,
            }
        })
        .collect();
    RunPlan {
        rdm_sha: rdm_sha_command(paths),
        probes: probes(tools),
        scbench: scbench_checkout(manifest, &paths.cache_dir),
        problems: checkout(
            &manifest.problems.repo,
            &manifest.problems.sha,
            problems_dir,
        ),
        model_file: ModelCopy { source, installed },
        arms,
        record_path: paths.out_dir.join(RECORD_FILE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        let text = r#"
[scbench]
repo = "R1"
sha = "1111111111111111111111111111111111111111"
[problems]
repo = "R2"
sha = "2222222222222222222222222222222222222222"
subset = ["p1", "p2"]
checkpoints = { p1 = 3, p2 = 2 }
[agent]
provider = "prov"
model = "mod"
model_file = "models/m.yaml"
thinking = "low"
claude_code_version = "9.9.9"
pass_policy = "core-cases"
environment = "env"
step_limit = 7
cost_limit = 0
net_cost_limit = 0
[[arm]]
name = "a1"
prompt = "configs/prompts/a1.jinja"
[[arm]]
name = "a2"
prompt = "configs/prompts/a2.jinja"
"#;
        Manifest::parse(text, Path::new("/rdm/bench")).expect("valid")
    }

    fn plan() -> RunPlan {
        plan_with_seeds(&[1])
    }

    fn plan_with_seeds(seeds: &[u64]) -> RunPlan {
        build_plan(
            &manifest(),
            &Paths {
                manifest_dir: "/rdm/bench".into(),
                cache_dir: "/c".into(),
                out_dir: "/o".into(),
            },
            &Tools {
                uv: "UV".into(),
                docker: "DOCKER".into(),
                claude: "CLAUDE".into(),
            },
            seeds,
            None,
            None,
            &BTreeMap::new(),
        )
    }

    #[test]
    fn provisioning_pins_both_checkouts() {
        let p = plan();
        assert_eq!(
            p.scbench.checkout.argv(),
            [
                "git",
                "-C",
                "/c/slop-code-bench",
                "checkout",
                "--quiet",
                "--detach",
                "1111111111111111111111111111111111111111"
            ]
        );
        assert_eq!(
            p.problems.clone.argv(),
            ["git", "clone", "--no-checkout", "R2", "/c/scb-problems"]
        );
        assert_eq!(
            p.problems.set_url.argv(),
            [
                "git",
                "-C",
                "/c/scb-problems",
                "remote",
                "set-url",
                "origin",
                "R2"
            ]
        );
        assert_eq!(
            p.rdm_sha.argv(),
            ["git", "-C", "/rdm/bench", "rev-parse", "HEAD"]
        );
        assert_eq!(
            p.probes.docker.argv(),
            ["DOCKER", "version", "--format", "{{.Server.Version}}"]
        );
    }

    #[test]
    fn model_file_lands_in_checkout_configs() {
        let p = plan();
        assert_eq!(
            p.model_file.source,
            PathBuf::from("/rdm/bench/models/m.yaml")
        );
        assert_eq!(
            p.model_file.installed,
            PathBuf::from("/c/slop-code-bench/configs/models/m.yaml")
        );
    }

    #[test]
    fn arms_carry_manifest_values() {
        let p = plan();
        assert_eq!(p.arms.len(), 2);
        let a = &p.arms[0];
        assert_eq!(
            a.run_config["model"],
            json!({"provider": "prov", "name": "mod"})
        );
        assert_eq!(a.run_config["thinking"], "low");
        assert_eq!(a.run_config["problems"], json!(["p1", "p2"]));
        assert_eq!(
            a.run_config["prompt"],
            "/c/slop-code-bench/configs/prompts/a1.jinja"
        );
        assert_eq!(a.run_config["agent"], "/o/a1/agent.yaml");
        assert_eq!(a.run_config["save_dir"], "/o/a1/seed-1");
        assert_eq!(a.run_dir, PathBuf::from("/o/a1/seed-1/output"));
        assert_eq!(a.agent_config["version"], "9.9.9");
        assert_eq!(a.agent_config["cost_limits"]["step_limit"], 7);
        assert_eq!(
            a.run.argv(),
            [
                "UV",
                "run",
                "slop-code",
                "--seed",
                "1",
                "run",
                "--config",
                "/o/a1/seed-1/run.yaml",
                "--no-evaluate",
                "--no-live-progress"
            ]
        );
        assert_eq!(
            a.eval.argv(),
            [
                "UV",
                "run",
                "slop-code",
                "--seed",
                "1",
                "eval",
                "/o/a1/seed-1/output",
                "--pass-policy",
                "core-cases",
                "--no-live-progress"
            ]
        );
        assert_eq!(a.run.cwd.as_deref(), Some(Path::new("/c/slop-code-bench")));
        assert_eq!(a.run.env["SCBENCH_PROBLEMS_PATH"], "/c/scb-problems");
        assert_eq!(a.eval.env["SCBENCH_HOME"], "/c/scbench-home");
        assert_eq!(p.record_path, PathBuf::from("/o/rdm-scbench-run.json"));
    }

    #[test]
    fn units_are_seed_major_with_per_seed_paths() {
        let p = plan_with_seeds(&[1, 2, 3]);
        let units: Vec<(u64, &str)> = p.arms.iter().map(|a| (a.seed, a.name.as_str())).collect();
        assert_eq!(
            units,
            [
                (1, "a1"),
                (1, "a2"),
                (2, "a1"),
                (2, "a2"),
                (3, "a1"),
                (3, "a2")
            ]
        );
        let a = &p.arms[3];
        assert_eq!(a.agent_config_path, PathBuf::from("/o/a2/agent.yaml"));
        assert_eq!(a.run_config_path, PathBuf::from("/o/a2/seed-2/run.yaml"));
        assert_eq!(a.run_config["save_dir"], "/o/a2/seed-2");
        assert_eq!(a.run_dir, PathBuf::from("/o/a2/seed-2/output"));
        assert_eq!(&a.eval.args[..4], ["run", "slop-code", "--seed", "2"]);
    }

    #[test]
    fn projection_multiplies_arms_seeds_and_checkpoints() {
        let got = projection(&manifest(), 3);
        assert_eq!(
            got,
            Projection {
                arms: 2,
                seeds: 3,
                problems: BTreeMap::from([("p1".to_owned(), 3), ("p2".to_owned(), 2)]),
                checkpoints: 5,
                sessions: 12,
                checkpoint_runs: 30,
            }
        );
    }
}
