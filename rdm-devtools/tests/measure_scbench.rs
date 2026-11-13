//! `rdm-measure scbench run`, `scbench ladder` and `scbench image build|check`
//! against fake
//! `uv`/`docker`/`claude` executables
//! (the `rdm-devtools-fixture` binary symlinked under each name) and local
//! stand-in git repositories. Nothing here runs SCBench, Docker or a paid
//! agent, and nothing touches the network: every remote is a local path.
//!
//! Every git call, the test's and the binary's alike, runs with
//! `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` at `/dev/null`, a fixed identity
//! and a temp `HOME`, so a hostile user or system git config cannot change a
//! result.

mod common;
mod measure_support;
mod scbench_support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use measure_support::{fails, json, measure_bin, ok, repo_root, stderr};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The `full-lane` stand-in arm's limits, distinct from every `[agent]` value.
const FULL_LANE_LIMITS: &str = "step_limit = 4321\ncost_limit = 12.5\nnet_cost_limit = 25\n";

/// Values a stand-in manifest is written with.
struct Spec {
    provider: &'static str,
    model: &'static str,
    thinking: &'static str,
    claude_code_version: &'static str,
    step_limit: u64,
    pass_policy: &'static str,
    /// `None` writes a stand-in model file into the rdm repo and points the
    /// manifest at it relatively.
    model_file: Option<PathBuf>,
    /// Adds a third arm, `just-solve-rdm`, with `image = "rdm"`.
    rdm_arm: bool,
    /// An explicit `group` written on the `just-solve-rdm` arm; `None`
    /// leaves the key out so the group follows the image.
    rdm_arm_group: Option<&'static str>,
    /// Adds a `full-lane` arm (the committed template, `prompt_from =
    /// "manifest"`, `image = "rdm"`) with limits [`FULL_LANE_LIMITS`].
    full_lane: bool,
    /// The `[rdm] plan_root` every stand-in manifest carries.
    plan_root: &'static str,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            provider: "claude_code_oauth",
            model: "opus-5.5",
            thinking: "xhigh",
            claude_code_version: "2.1.322",
            step_limit: 100,
            pass_policy: "all-cases",
            model_file: None,
            rdm_arm: false,
            rdm_arm_group: None,
            full_lane: false,
            plan_root: "/rdm-plan",
        }
    }
}

struct Bench {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    scbench_sha: String,
    problems_sha: String,
    rdm_sha: String,
    manifest: PathBuf,
    cache: PathBuf,
    out: PathBuf,
}

const GIT_ENV: &[(&str, &str)] = &[
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_SYSTEM", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "Bench Test"),
    ("GIT_AUTHOR_EMAIL", "bench@example.invalid"),
    ("GIT_COMMITTER_NAME", "Bench Test"),
    ("GIT_COMMITTER_EMAIL", "bench@example.invalid"),
    ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
    ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z"),
];

const GIT_UNSET: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_PREFIX",
    "DEVTOOLS_FAKE_SCENARIO",
];

fn hermetic(cmd: &mut Command, home: &Path) {
    for k in GIT_UNSET {
        cmd.env_remove(k);
    }
    cmd.envs(GIT_ENV.iter().copied())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"));
}

/// Sets each `(name, Some(value))` and removes each `(name, None)`.
fn apply_env(cmd: &mut Command, env: &[(&str, Option<&str>)]) {
    for (k, v) in env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
}

fn git(home: &Path, dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    hermetic(&mut cmd, home);
    let o = cmd.output().expect("git runs");
    assert!(
        o.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

/// A git repo at `dir` holding `files`, committed; returns its HEAD.
fn repo(home: &Path, dir: &Path, files: &[(&str, &str)]) -> String {
    std::fs::create_dir_all(dir).expect("mkdir repo");
    git(home, dir, &["init", "-q", "-b", "main"]);
    for (rel, text) in files {
        write(&dir.join(rel), text);
    }
    git(home, dir, &["add", "-A"]);
    git(home, dir, &["commit", "-q", "-m", "stand-in"]);
    git(home, dir, &["rev-parse", "HEAD"])
}

/// The committed Opus 5.5 model file.
fn committed_model_file() -> PathBuf {
    repo_root().join("bench/scbench/models/opus-5.5.yaml")
}

impl Bench {
    fn new(spec: &Spec) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Canonical, so cwd paths the fakes report compare equal on macOS.
        let root = std::fs::canonicalize(tmp.path()).expect("canonical tmp");
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("home");
        let scbench_sha = repo(
            &home,
            &root.join("src/slop-code-bench"),
            &[
                (
                    "configs/models/opus-5.yaml",
                    "internal_name: claude-opus-5\n",
                ),
                ("configs/prompts/just-solve.jinja", "just solve\n"),
                ("configs/prompts/anti_slop.jinja", "anti slop\n"),
                (
                    "configs/environments/docker-python3.12-uv.yaml",
                    "type: docker\nname: python3.12\ndocker:\n  workdir: /workspace\n",
                ),
            ],
        );
        let problems_sha = repo(
            &home,
            &root.join("src/scb-problems"),
            &[
                ("mvvault/config.yaml", "name: mvvault\n"),
                ("xjq/config.yaml", "name: xjq\n"),
            ],
        );
        let model_file = spec.model_file.as_ref().map_or_else(
            || "models/opus-5.5.yaml".to_owned(),
            |p| p.display().to_string(),
        );
        let manifest_text = format!(
            r#"[scbench]
repo = "{scbench_repo}"
sha = "{scbench_sha}"

[problems]
repo = "{problems_repo}"
sha = "{problems_sha}"
subset = ["mvvault", "xjq"]
checkpoints = {{ mvvault = 3, xjq = 2 }}

[agent]
provider = "{provider}"
model = "{model}"
model_file = "{model_file}"
thinking = "{thinking}"
claude_code_version = "{version}"
pass_policy = "{pass_policy}"
environment = "docker-python3.12-uv"
step_limit = {step_limit}
cost_limit = 0
net_cost_limit = 0

[[arm]]
name = "just-solve"
prompt = "configs/prompts/just-solve.jinja"

[[arm]]
name = "anti_slop"
prompt = "configs/prompts/anti_slop.jinja"
{rdm_arm}{full_lane}
[rdm]
plan_root = "{plan_root}"
project = "bench"
"#,
            scbench_repo = root.join("src/slop-code-bench").display(),
            problems_repo = root.join("src/scb-problems").display(),
            provider = spec.provider,
            model = spec.model,
            thinking = spec.thinking,
            version = spec.claude_code_version,
            pass_policy = spec.pass_policy,
            step_limit = spec.step_limit,
            rdm_arm = if spec.rdm_arm {
                format!(
                    "\n[[arm]]\nname = \"just-solve-rdm\"\nprompt = \"configs/prompts/just-solve.jinja\"\nimage = \"rdm\"\n{}",
                    spec.rdm_arm_group
                        .map_or_else(String::new, |g| format!("group = \"{g}\"\n"))
                )
            } else {
                String::new()
            },
            full_lane = if spec.full_lane {
                format!(
                    "\n[[arm]]\nname = \"full-lane\"\nprompt = \"prompts/full-lane.jinja\"\n\
                     prompt_from = \"manifest\"\nimage = \"rdm\"\n{FULL_LANE_LIMITS}"
                )
            } else {
                String::new()
            },
            plan_root = spec.plan_root,
        );
        let template = std::fs::read_to_string(repo_root().join("bench/scbench/agent/docker.j2"))
            .expect("committed agent template");
        let full_lane =
            std::fs::read_to_string(repo_root().join("bench/scbench/prompts/full-lane.jinja"))
                .expect("committed full-lane template");
        let rdm = root.join("rdm");
        let rdm_sha = repo(
            &home,
            &rdm,
            &[
                ("bench/scbench/manifest.toml", &manifest_text),
                (
                    "bench/scbench/models/opus-5.5.yaml",
                    "internal_name: stand-in\n",
                ),
                ("bench/scbench/agent/Dockerfile", "FROM scratch\n"),
                ("bench/scbench/agent/docker.j2", &template),
                ("bench/scbench/prompts/full-lane.jinja", &full_lane),
                ("plugins/rdm/skills/do/SKILL.md", "# do\n"),
                ("plugins/rdm/skills/land/SKILL.md", "# land\n"),
            ],
        );
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("bin");
        std::fs::create_dir_all(root.join("rec")).expect("record dir");
        for role in ["uv", "docker", "claude"] {
            std::os::unix::fs::symlink(common::fixture(), bin.join(role)).expect("symlink");
        }
        let bench = Self {
            scbench_sha,
            problems_sha,
            rdm_sha,
            manifest: rdm.join("bench/scbench/manifest.toml"),
            cache: root.join("cache"),
            out: root.join("out"),
            root,
            _tmp: tmp,
        };
        bench.scenario(
            "uv",
            &json!({ "default": { "exit": 0 } }),
            &[(&["--version"], json!({ "stdout": "uv 0.0.0-fake\n" }))],
        );
        bench.scenario(
            "docker",
            &json!({ "default": { "stdout": "99.0.0-fake\n" } }),
            &[],
        );
        bench.scenario(
            "claude",
            &json!({ "default": { "stdout": "0.0.1 (Claude Code)\n" } }),
            &[],
        );
        bench
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn bin(&self, role: &str) -> PathBuf {
        self.root.join("bin").join(role)
    }

    fn record_path(&self, role: &str) -> PathBuf {
        self.root.join(format!("rec/{role}.jsonl"))
    }

    /// Writes `<role>.scenario.json` beside the role's symlink. `responses`
    /// are tried first, in order.
    fn scenario(&self, role: &str, base: &Value, responses: &[(&[&str], Value)]) {
        let mut s = base.clone();
        s["record"] = json!(self.record_path(role));
        s["responses"] = responses
            .iter()
            .map(|(needles, r)| {
                let mut r = r.clone();
                r["argvContains"] = json!(needles);
                r
            })
            .collect();
        write(
            &self.root.join(format!("bin/{role}.scenario.json")),
            &serde_json::to_string(&s).expect("scenario json"),
        );
    }

    /// The fake's recorded calls, or `None` when it was never invoked.
    fn calls(&self, role: &str) -> Option<Vec<Value>> {
        let text = std::fs::read_to_string(self.record_path(role)).ok()?;
        Some(text.lines().map(json).collect())
    }

    fn argvs(&self, role: &str) -> Vec<Vec<String>> {
        self.calls(role)
            .unwrap_or_default()
            .iter()
            .map(|c| {
                c["argv"]
                    .as_array()
                    .expect("argv")
                    .iter()
                    .map(|a| a.as_str().expect("str").to_owned())
                    .collect()
            })
            .collect()
    }

    fn run(&self, extra: &[&str]) -> Output {
        self.run_into(&self.out, extra)
    }

    fn run_into(&self, out: &Path, extra: &[&str]) -> Output {
        self.run_command(out, extra)
            .output()
            .expect("rdm-measure runs")
    }

    /// Runs `scbench run` into `--out` with `env` applied over the hermetic
    /// environment (`None` removes a variable).
    fn run_env(&self, extra: &[&str], env: &[(&str, Option<&str>)]) -> Output {
        let mut cmd = self.run_command(&self.out, extra);
        apply_env(&mut cmd, env);
        cmd.output().expect("rdm-measure runs")
    }

    fn run_command(&self, out: &Path, extra: &[&str]) -> Command {
        let mut cmd = Command::new(measure_bin());
        cmd.args(["scbench", "run", "--manifest"])
            .arg(&self.manifest)
            .arg("--cache-dir")
            .arg(&self.cache)
            .arg("--out")
            .arg(out)
            .arg("--uv-bin")
            .arg(self.bin("uv"))
            .arg("--docker-bin")
            .arg(self.bin("docker"))
            .arg("--claude-bin")
            .arg(self.bin("claude"))
            .args(extra)
            .current_dir(&self.root);
        hermetic(&mut cmd, &self.home());
        cmd
    }

    /// Runs `scbench ladder` into `out` with the fakes. The credential
    /// environment is always explicit: `CLAUDE_CODE_OAUTH_TOKEN` is a fake
    /// token and `ANTHROPIC_API_KEY` is removed, then `env` applies
    /// (`None` removes a variable).
    fn ladder(&self, out: &Path, extra: &[&str], env: &[(&str, Option<&str>)]) -> Output {
        self.ladder_command(out, extra, env)
            .output()
            .expect("rdm-measure runs")
    }

    /// The [`Bench::ladder`] command, not yet run.
    fn ladder_command(&self, out: &Path, extra: &[&str], env: &[(&str, Option<&str>)]) -> Command {
        let mut cmd = Command::new(measure_bin());
        cmd.args(["scbench", "ladder", "--manifest"])
            .arg(&self.manifest)
            .arg("--cache-dir")
            .arg(&self.cache)
            .arg("--out")
            .arg(out)
            .arg("--uv-bin")
            .arg(self.bin("uv"))
            .arg("--docker-bin")
            .arg(self.bin("docker"))
            .arg("--claude-bin")
            .arg(self.bin("claude"))
            .args(extra)
            .current_dir(&self.root);
        hermetic(&mut cmd, &self.home());
        cmd.env("CLAUDE_CODE_OAUTH_TOKEN", "fake-token")
            .env_remove("ANTHROPIC_API_KEY");
        apply_env(&mut cmd, env);
        cmd
    }

    /// Runs `scbench report --run <run>` with `extra`.
    fn report(&self, run: &Path, extra: &[&str]) -> Output {
        let mut cmd = Command::new(measure_bin());
        cmd.args(["scbench", "report", "--run"])
            .arg(run)
            .args(extra)
            .current_dir(&self.root);
        hermetic(&mut cmd, &self.home());
        cmd.output().expect("rdm-measure runs")
    }

    /// Runs `scbench image <sub>` with the fakes.
    fn image(&self, sub: &str, extra: &[&str]) -> Output {
        self.image_env(sub, extra, &[])
    }

    /// [`Bench::image`] with `env` applied over the hermetic environment
    /// (`None` removes a variable).
    fn image_env(&self, sub: &str, extra: &[&str], env: &[(&str, Option<&str>)]) -> Output {
        let mut cmd = Command::new(measure_bin());
        cmd.args(["scbench", "image", sub, "--manifest"])
            .arg(&self.manifest)
            .arg("--cache-dir")
            .arg(&self.cache)
            .arg("--docker-bin")
            .arg(self.bin("docker"));
        if sub == "build" {
            cmd.arg("--uv-bin").arg(self.bin("uv"));
        }
        cmd.args(extra).current_dir(&self.root);
        hermetic(&mut cmd, &self.home());
        apply_env(&mut cmd, env);
        cmd.output().expect("rdm-measure runs")
    }

    fn rdm_dir(&self) -> PathBuf {
        self.root.join("rdm")
    }

    fn record(&self) -> Value {
        self.record_at(&self.out)
    }

    fn record_at(&self, out: &Path) -> Value {
        json(&std::fs::read_to_string(out.join("rdm-scbench-run.json")).expect("run record"))
    }

    fn scbench_dir(&self) -> PathBuf {
        self.cache.join("slop-code-bench")
    }

    fn problems_dir(&self) -> PathBuf {
        self.cache.join("scb-problems")
    }
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|s| s.as_str().expect("string").to_owned())
        .collect()
}

fn p(path: &Path) -> String {
    path.display().to_string()
}

/// The fake `uv`'s argv for one unit: `arm` at `seed`, run then eval.
fn unit_argvs(out: &Path, arm: &str, seed: u64, policy: &str) -> [Vec<String>; 2] {
    let dir = out.join(arm).join(format!("seed-{seed}"));
    let seed = seed.to_string();
    let v = |x: &[&str]| x.iter().map(|&a| a.to_owned()).collect::<Vec<_>>();
    [
        v(&[
            "run",
            "slop-code",
            "--seed",
            &seed,
            "run",
            "--config",
            &p(&dir.join("run.yaml")),
            "--no-evaluate",
            "--no-live-progress",
        ]),
        v(&[
            "run",
            "slop-code",
            "--seed",
            &seed,
            "eval",
            &p(&dir.join("output")),
            "--pass-policy",
            policy,
            "--no-live-progress",
        ]),
    ]
}

/// The committed manifest's `--dry-run` plan, built against `cache` and `out`.
fn committed_plan(cache: &Path, out: &Path) -> Value {
    let o = measure_support::run(&[
        "scbench",
        "run",
        "--dry-run",
        "--cache-dir",
        &p(cache),
        "--out",
        &p(out),
    ]);
    json(&ok(&o))
}

/// Where an arm's prompt template comes from in the committed ladder.
enum Template {
    /// Upstream's `configs/prompts/<file>` in the SCBench checkout.
    Upstream(&'static str),
    /// rdm's own template, rendered to `<out>/<arm>/prompt.jinja`.
    Rdm,
}

/// One committed ladder rung: name, group, image, template and limits
/// (`step_limit`, `cost_limit`, `net_cost_limit`).
struct Rung {
    name: &'static str,
    group: &'static str,
    image: &'static str,
    template: Template,
    limits: (u64, f64, f64),
}

const CONTROL_LIMITS: (u64, f64, f64) = (100, 0.0, 0.0);
const RDM_LIMITS: (u64, f64, f64) = (1000, 0.0, 0.0);

const LADDER: &[Rung] = &[
    Rung {
        name: "just-solve",
        group: "control",
        image: "stock",
        template: Template::Upstream("just-solve.jinja"),
        limits: CONTROL_LIMITS,
    },
    Rung {
        name: "anti_slop",
        group: "control",
        image: "stock",
        template: Template::Upstream("anti_slop.jinja"),
        limits: CONTROL_LIMITS,
    },
    Rung {
        name: "plan_first",
        group: "control",
        image: "stock",
        template: Template::Upstream("plan_first.jinja"),
        limits: CONTROL_LIMITS,
    },
    Rung {
        name: "plan-and-test",
        group: "control",
        image: "stock",
        template: Template::Upstream("plan-and-test.jinja"),
        limits: CONTROL_LIMITS,
    },
    Rung {
        name: "tracked",
        group: "rdm",
        image: "rdm",
        template: Template::Rdm,
        limits: RDM_LIMITS,
    },
    Rung {
        name: "planned",
        group: "rdm",
        image: "rdm",
        template: Template::Rdm,
        limits: RDM_LIMITS,
    },
    Rung {
        name: "full-lane",
        group: "rdm",
        image: "rdm",
        template: Template::Rdm,
        limits: RDM_LIMITS,
    },
];

#[test]
fn committed_ladder_arms_construct_their_invocations() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache = tmp.path().join("cache");
    let out = tmp.path().join("out");
    let plan = committed_plan(&cache, &out);
    let arms = plan["plan"]["arms"].as_array().expect("arms");
    let names: Vec<_> = arms
        .iter()
        .map(|a| a["name"].as_str().expect("name"))
        .collect();
    let want: Vec<_> = LADDER.iter().map(|r| r.name).collect();
    assert_eq!(names, want);

    for (arm, rung) in arms.iter().zip(LADDER) {
        let ctx = rung.name;
        assert_eq!(arm["group"], rung.group, "{ctx}");
        assert_eq!(arm["image"], rung.image, "{ctx}");
        let run_yaml = out.join(rung.name).join("seed-1/run.yaml");
        assert_eq!(
            strs(&arm["run"]["args"]),
            [
                "run",
                "slop-code",
                "--seed",
                "1",
                "run",
                "--config",
                &p(&run_yaml),
                "--no-evaluate",
                "--no-live-progress",
            ],
            "{ctx}"
        );
        match rung.template {
            Template::Upstream(file) => {
                assert_eq!(
                    arm["run_config"]["prompt"],
                    p(&cache.join("slop-code-bench/configs/prompts").join(file)),
                    "{ctx}"
                );
                assert_eq!(arm["prompt"], Value::Null, "{ctx}");
            }
            Template::Rdm => {
                assert_eq!(
                    arm["run_config"]["prompt"],
                    p(&out.join(rung.name).join("prompt.jinja")),
                    "{ctx}"
                );
                let prompt = arm["prompt"].as_str().expect("rendered prompt");
                assert!(!prompt.trim().is_empty(), "{ctx}");
                assert!(!prompt.contains("@RDM_"), "{ctx}: {prompt}");
            }
        }
        let agent = &arm["agent_config"];
        if rung.image == "rdm" {
            assert_eq!(agent["binary"], "/opt/rdm/bin/claude-rdm", "{ctx}");
            assert_eq!(
                agent["extra_args"],
                json!(["--plugin-dir", "/opt/rdm/plugin"]),
                "{ctx}"
            );
            assert!(arm["plan_repos_dir"].is_string(), "{ctx}");
        } else {
            assert_eq!(agent["binary"], "claude", "{ctx}");
            assert_eq!(arm["plan_repos_dir"], Value::Null, "{ctx}");
        }
        let (step_limit, cost_limit, net_cost_limit) = rung.limits;
        assert_eq!(
            agent["cost_limits"],
            json!({
                "step_limit": step_limit,
                "cost_limit": cost_limit,
                "net_cost_limit": net_cost_limit,
            }),
            "{ctx}"
        );
    }
    assert!(!cache.exists() && !out.exists());
}

#[test]
fn run_record_carries_each_arms_group() {
    let b = Bench::new(&Spec {
        rdm_arm: true,
        ..Spec::default()
    });
    ok(&b.run(&[]));
    let record = b.record();
    for (name, group) in [
        ("just-solve", "control"),
        ("anti_slop", "control"),
        ("just-solve-rdm", "rdm"),
    ] {
        assert_eq!(arm_record(&record, name)["group"], group, "{name}");
    }
}

#[test]
fn explicit_arm_group_overrides_the_image_default() {
    let b = Bench::new(&Spec {
        rdm_arm: true,
        rdm_arm_group: Some("control"),
        ..Spec::default()
    });
    let plan = json(&ok(&b.run(&["--dry-run"])));
    let planned = plan["plan"]["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .find(|a| a["name"] == "just-solve-rdm")
        .cloned()
        .expect("planned just-solve-rdm");
    assert_eq!(planned["group"], "control");
    assert_eq!(planned["image"], "rdm");

    ok(&b.run(&[]));
    let recorded = arm_record(&b.record(), "just-solve-rdm");
    assert_eq!(recorded["group"], "control");
    assert_eq!(recorded["image"], "rdm");
}

#[test]
fn model_file_is_installed_in_pinned_checkout() {
    let b = Bench::new(&Spec {
        model_file: Some(committed_model_file()),
        ..Spec::default()
    });
    ok(&b.run(&[]));
    let committed = std::fs::read(committed_model_file()).expect("committed model file");
    let installed =
        std::fs::read(b.scbench_dir().join("configs/models/opus-5.5.yaml")).expect("installed");
    assert_eq!(installed, committed);
    assert_eq!(
        git(&b.home(), &b.scbench_dir(), &["rev-parse", "HEAD"]),
        b.scbench_sha
    );
    let sha: String = Sha256::digest(&committed)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect();
    assert_eq!(b.record()["model_file"]["sha256"], sha);
}

#[test]
fn runs_each_arm_in_order() {
    let b = Bench::new(&Spec::default());
    ok(&b.run(&[]));
    let mut expected = vec![vec!["--version".to_owned()]];
    for arm in ["just-solve", "anti_slop"] {
        expected.extend(unit_argvs(&b.out, arm, 1, "all-cases"));
    }
    assert_eq!(b.argvs("uv"), expected);
    for call in &b.calls("uv").expect("uv called")[1..] {
        assert_eq!(call["cwd"], p(&b.scbench_dir()));
        assert_eq!(
            call["scbench"],
            json!({
                "SCBENCH_PROBLEMS_PATH": p(&b.problems_dir()),
                "SCBENCH_HOME": p(&b.cache.join("scbench-home")),
            })
        );
    }
}

/// The record's units as `(arm, seed)` pairs, in record order.
fn units(record: &Value) -> Vec<(String, u64)> {
    record["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .map(|a| {
            (
                a["name"].as_str().expect("name").to_owned(),
                a["seed"].as_u64().expect("seed"),
            )
        })
        .collect()
}

#[test]
fn run_seeds_run_each_selected_arm_per_seed() {
    let b = Bench::new(&Spec::default());
    ok(&b.run(&["--arms", "just-solve,anti_slop", "--seeds", "3"]));
    let mut expected = vec![vec!["--version".to_owned()]];
    for seed in 1..=3 {
        for arm in ["just-solve", "anti_slop"] {
            expected.extend(unit_argvs(&b.out, arm, seed, "all-cases"));
            let run_yaml = read_json(&b.out.join(arm).join(format!("seed-{seed}/run.yaml")));
            assert_eq!(
                run_yaml["save_dir"],
                p(&b.out.join(arm).join(format!("seed-{seed}")))
            );
            assert_eq!(run_yaml["problems"], json!(["mvvault", "xjq"]));
        }
    }
    assert_eq!(b.argvs("uv"), expected);

    let record = b.record();
    assert_eq!(record["schema"], 2);
    assert_eq!(record["seeds"], json!([1, 2, 3]));
    assert_eq!(record["selected_arms"], json!(["just-solve", "anti_slop"]));
    let arms = record["arms"].as_array().expect("arms");
    assert_eq!(arms.len(), 6);
    for (seed, arm) in (1..=3).flat_map(|s| ["just-solve", "anti_slop"].map(|a| (s, a))) {
        let unit = arms
            .iter()
            .find(|a| a["name"] == arm && a["seed"] == seed)
            .unwrap_or_else(|| panic!("no unit {arm} seed {seed}"));
        assert_eq!(
            unit["run_dir"],
            p(&b.out.join(arm).join(format!("seed-{seed}/output")))
        );
        assert!(unit["finished"].is_string(), "{unit}");
    }
}

#[test]
fn run_arms_selects_a_subset_in_manifest_order() {
    let b = Bench::new(&Spec::default());
    ok(&b.run(&["--arms", "anti_slop,just-solve", "--seeds", "3"]));
    let order: Vec<(String, u64)> = (1..=3)
        .flat_map(|s| ["just-solve", "anti_slop"].map(|a| (a.to_owned(), s)))
        .collect();
    assert_eq!(units(&b.record()), order);

    let second = b.root.join("out2");
    ok(&b.run_into(&second, &["--arms", "anti_slop"]));
    let record = b.record_at(&second);
    assert_eq!(units(&record), [("anti_slop".to_owned(), 1)]);
    assert_eq!(record["selected_arms"], json!(["anti_slop"]));
    assert!(!second.join("just-solve").exists());
    let spawned_just_solve = b.argvs("uv").iter().any(|a| {
        a.iter()
            .any(|x| x.starts_with(&p(&second.join("just-solve"))))
    });
    assert!(!spawned_just_solve, "{:?}", b.argvs("uv"));
}

#[test]
fn run_refuses_an_unknown_arm_before_spawning() {
    let b = Bench::new(&Spec::default());
    let err = fails(&b.run(&["--arms", "just-solve,nope"]));
    assert!(
        err.contains("--arms") && err.contains("\"nope\"") && err.contains("just-solve, anti_slop"),
        "stderr: {err}"
    );
    for role in ["uv", "docker", "claude"] {
        assert!(b.calls(role).is_none(), "{role} was spawned");
    }
    assert!(!b.out.exists() && !b.cache.exists());
}

#[test]
fn run_refuses_zero_seeds() {
    let b = Bench::new(&Spec::default());
    let err = fails(&b.run(&["--seeds", "0"]));
    assert!(err.contains("--seeds"), "stderr: {err}");
    assert!(b.calls("uv").is_none());
    assert!(!b.out.exists());
}

#[test]
fn failing_step_stops_and_names_arm() {
    let b = Bench::new(&Spec::default());
    b.scenario(
        "uv",
        &json!({ "default": { "exit": 0 } }),
        &[
            (&["--version"], json!({ "stdout": "uv 0.0.0-fake\n" })),
            (&["anti_slop/seed-1/run.yaml"], json!({ "exit": 3 })),
        ],
    );
    let err = fails(&b.run(&[]));
    assert!(
        err.contains("arm anti_slop seed 1: run exited 3"),
        "stderr: {err}"
    );
    let argvs = b.argvs("uv");
    assert_eq!(argvs.len(), 4, "{argvs:?}");
    assert!(
        argvs[1]
            .iter()
            .any(|a| a.ends_with("just-solve/seed-1/run.yaml"))
    );
    assert!(
        argvs[2]
            .iter()
            .any(|a| a.ends_with("just-solve/seed-1/output"))
    );
    assert!(
        argvs[3]
            .iter()
            .any(|a| a.ends_with("anti_slop/seed-1/run.yaml"))
    );
    assert!(
        !argvs
            .iter()
            .any(|a| a.iter().any(|x| x.ends_with("anti_slop/seed-1/output")))
    );
    assert_eq!(
        b.record()["arms"]
            .as_array()
            .expect("arms")
            .iter()
            .map(|a| (a["name"].clone(), a["finished"].is_string()))
            .collect::<Vec<_>>(),
        [(json!("just-solve"), true), (json!("anti_slop"), false)]
    );
    let record = b.record();
    let last = record["commands"]
        .as_array()
        .expect("commands")
        .last()
        .expect("a command");
    assert_eq!(last["exit"], 3);
}

#[test]
fn dry_run_spawns_nothing() {
    let b = Bench::new(&Spec::default());
    let plan = json(&ok(&b.run(&["--dry-run"])));
    assert_eq!(plan["rdm"]["sha"], Value::Null);
    for role in ["uv", "docker", "claude"] {
        assert!(!b.record_path(role).exists(), "{role} was spawned");
    }
    assert!(!b.cache.exists(), "--cache-dir was created");
    assert!(!b.out.exists(), "--out was created");
}

#[test]
fn dry_run_rdm_arm_shows_sha_placeholder() {
    let b = Bench::new(&Spec {
        rdm_arm: true,
        ..Spec::default()
    });
    let plan = json(&ok(&b.run(&["--dry-run"])));
    assert_eq!(plan["rdm"]["sha"], Value::Null);
    let arms = plan["plan"]["arms"].as_array().expect("arms");
    let arm = |name: &str| {
        arms.iter()
            .find(|a| a["name"] == name)
            .unwrap_or_else(|| panic!("no arm {name}: {arms:?}"))
    };
    let rdm = arm("just-solve-rdm");
    assert_eq!(rdm["agent_config"]["version"], "2.1.322-rdm.<rdm-sha>");
    let template = rdm["docker_template"].as_str().expect("docker_template");
    assert!(
        template.contains("COPY --from=rdm-scbench-payload:<rdm-sha>"),
        "{template}"
    );
    let stock = arm("just-solve");
    assert_eq!(stock["agent_config"]["version"], "2.1.322");
    assert_eq!(stock["docker_template"], Value::Null);
    // `git` is the real binary, not a recording fake: no `git` record and an
    // untouched cache (no clone) plus a null `rdm.sha` (no rev-parse) show it
    // was not spawned.
    for role in ["uv", "docker", "claude", "git"] {
        assert!(!b.record_path(role).exists(), "{role} was spawned");
    }
    assert!(!b.cache.exists(), "--cache-dir was created");
    assert!(!b.out.exists(), "--out was created");
}

#[test]
fn probe_failure_stops_before_any_arm() {
    let b = Bench::new(&Spec::default());
    b.scenario(
        "docker",
        &json!({ "default": { "exit": 1, "stderr": "daemon down\n" } }),
        &[],
    );
    let err = fails(&b.run(&[]));
    assert!(err.contains("docker"), "stderr: {err}");
    assert_eq!(b.argvs("uv"), vec![vec!["--version".to_owned()]]);
    assert!(!b.scbench_dir().exists());
}

#[test]
fn refuses_non_empty_out() {
    let b = Bench::new(&Spec::default());
    write(&b.out.join("previous.txt"), "an earlier run\n");
    let err = fails(&b.run(&[]));
    assert!(err.contains("--out"), "stderr: {err}");
    for role in ["uv", "docker", "claude"] {
        assert!(!b.record_path(role).exists(), "{role} was spawned");
    }
    assert!(!b.cache.exists());
}

#[test]
fn invocation_built_from_manifest() {
    let b = Bench::new(&Spec {
        provider: "claude_code_oauth",
        model: "opus-5.5",
        thinking: "low",
        claude_code_version: "9.9.9",
        step_limit: 7,
        pass_policy: "core-cases",
        model_file: None,
        ..Spec::default()
    });
    ok(&b.run(&[]));
    let record = b.record();
    let argvs: Vec<String> = record["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|c| strs(&c["argv"]).join(" "))
        .collect();
    for (dir, sha) in [
        (b.scbench_dir(), &b.scbench_sha),
        (b.problems_dir(), &b.problems_sha),
    ] {
        let want = format!("checkout --quiet --detach {sha}");
        assert!(argvs.iter().any(|a| a.ends_with(&want)), "{argvs:?}");
        assert_eq!(&git(&b.home(), &dir, &["rev-parse", "HEAD"]), sha);
    }

    let uv = b.argvs("uv");
    let run = &uv[1];
    let config_at = run.iter().position(|a| a == "--config").expect("--config");
    let run_config = json(&std::fs::read_to_string(&run[config_at + 1]).expect("run.yaml"));
    assert_eq!(
        run_config["model"],
        json!({ "provider": "claude_code_oauth", "name": "opus-5.5" })
    );
    assert_eq!(run_config["thinking"], "low");
    assert_eq!(run_config["pass_policy"], "core-cases");
    assert_eq!(run_config["problems"], json!(["mvvault", "xjq"]));
    assert_eq!(
        run_config["prompt"],
        p(&b.scbench_dir().join("configs/prompts/just-solve.jinja"))
    );
    let agent = json(
        &std::fs::read_to_string(run_config["agent"].as_str().expect("agent path"))
            .expect("agent.yaml"),
    );
    assert_eq!(agent["version"], "9.9.9");
    assert_eq!(agent["cost_limits"]["step_limit"], 7);

    let eval = &uv[2];
    let at = eval
        .iter()
        .position(|a| a == "--pass-policy")
        .expect("--pass-policy");
    assert_eq!(eval[at + 1], "core-cases");
}

#[test]
fn run_record_holds_pins_versions_and_commands() {
    let b = Bench::new(&Spec::default());
    ok(&b.run(&[]));
    let record = b.record();
    assert_eq!(
        record["observed"],
        json!({ "uv": "uv 0.0.0-fake", "docker": "99.0.0-fake", "claude": "0.0.1 (Claude Code)" })
    );
    let m = &record["manifest"];
    assert_eq!(m["scbench"]["sha"], b.scbench_sha);
    assert_eq!(m["problems"]["sha"], b.problems_sha);
    assert_eq!(m["problems"]["subset"], json!(["mvvault", "xjq"]));
    assert_eq!(m["agent"]["provider"], "claude_code_oauth");
    assert_eq!(m["agent"]["model"], "opus-5.5");
    assert_eq!(m["agent"]["thinking"], "xhigh");
    assert_eq!(m["agent"]["claude_code_version"], "2.1.322");
    let arms: Vec<_> = m["arm"]
        .as_array()
        .expect("arms")
        .iter()
        .map(|a| a["name"].clone())
        .collect();
    assert_eq!(arms, [json!("just-solve"), json!("anti_slop")]);
    assert_eq!(record["rdm"]["sha"], b.rdm_sha);

    let commands = record["commands"].as_array().expect("commands");
    let argvs: Vec<Vec<String>> = commands.iter().map(|c| strs(&c["argv"])).collect();
    let uv = p(&b.bin("uv"));
    let sc = p(&b.scbench_dir());
    let pr = p(&b.problems_dir());
    let s = |x: &[&str]| x.iter().map(|&a| a.to_owned()).collect::<Vec<_>>();
    let manifest_dir = p(b.manifest.parent().expect("manifest dir"));
    let mut expected = vec![
        s(&["git", "-C", &manifest_dir, "rev-parse", "HEAD"]),
        s(&[&uv, "--version"]),
        s(&[
            &p(&b.bin("docker")),
            "version",
            "--format",
            "{{.Server.Version}}",
        ]),
        s(&[&p(&b.bin("claude")), "--version"]),
    ];
    for (repo, dir, sha) in [
        (b.root.join("src/slop-code-bench"), &sc, &b.scbench_sha),
        (b.root.join("src/scb-problems"), &pr, &b.problems_sha),
    ] {
        expected.push(s(&["git", "clone", "--no-checkout", &p(&repo), dir]));
        expected.push(s(&["git", "-C", dir, "fetch", "--quiet", "origin"]));
        expected.push(s(&[
            "git", "-C", dir, "checkout", "--quiet", "--detach", sha,
        ]));
    }
    // The fake's own record of its arm calls, one-for-one by argv.
    for call in &b.argvs("uv")[1..] {
        let mut argv = vec![uv.clone()];
        argv.extend(call.iter().cloned());
        expected.push(argv);
    }
    assert_eq!(expected.len(), 14);
    assert_eq!(argvs, expected);
    for c in commands {
        assert_eq!(c["exit"], 0, "{c}");
        assert!(c["started"].is_string() && c["finished"].is_string());
    }
    for c in &commands[10..] {
        assert_eq!(c["cwd"], sc);
        assert_eq!(c["env"]["SCBENCH_PROBLEMS_PATH"], pr);
    }

    assert!(b.out.join("rdm-scbench-run.json").is_file());
    for arm in ["just-solve", "anti_slop"] {
        let dir = b.out.join(arm).join("output");
        assert!(!dir.join("rdm-scbench-run.json").exists());
    }
}

#[test]
fn second_run_reuses_cached_checkouts() {
    let b = Bench::new(&Spec::default());
    ok(&b.run(&[]));
    let second = b.root.join("out2");
    let o = b.run_into(&second, &[]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    let argvs: Vec<String> = b.record_at(&second)["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|c| strs(&c["argv"]).join(" "))
        .collect();
    assert!(!argvs.iter().any(|a| a.contains(" clone ")), "{argvs:?}");
    for (dir, repo) in [
        (b.scbench_dir(), b.root.join("src/slop-code-bench")),
        (b.problems_dir(), b.root.join("src/scb-problems")),
    ] {
        let set_url = format!("git -C {} remote set-url origin {}", p(&dir), p(&repo));
        let fetch = format!("git -C {} fetch --quiet origin", p(&dir));
        let at = argvs.iter().position(|a| *a == set_url);
        assert!(at.is_some(), "{argvs:?}");
        assert_eq!(argvs.get(at.map_or(0, |i| i + 1)), Some(&fetch));
    }
    assert_eq!(argvs.iter().filter(|a| a.contains(" fetch ")).count(), 2);
    assert_eq!(argvs.iter().filter(|a| a.contains(" checkout ")).count(), 2);
}

#[test]
fn cached_checkout_follows_a_changed_repo() {
    let b = Bench::new(&Spec::default());
    ok(&b.run(&[]));
    // A fork of SCBench whose pinned commit exists only on the fork.
    let home = b.home();
    let upstream = b.root.join("src/slop-code-bench");
    let fork = b.root.join("src/slop-code-bench-fork");
    git(&home, &b.root, &["clone", "-q", &p(&upstream), &p(&fork)]);
    write(&fork.join("FORK"), "fork only\n");
    git(&home, &fork, &["add", "-A"]);
    git(&home, &fork, &["commit", "-q", "-m", "fork only"]);
    let fork_sha = git(&home, &fork, &["rev-parse", "HEAD"]);
    let text = std::fs::read_to_string(&b.manifest).expect("manifest");
    let text = text
        .replacen(
            &format!("repo = \"{}\"", p(&upstream)),
            &format!("repo = \"{}\"", p(&fork)),
            1,
        )
        .replacen(&b.scbench_sha, &fork_sha, 1);
    write(&b.manifest, &text);

    let second = b.root.join("out2");
    let o = b.run_into(&second, &[]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    assert_eq!(
        git(&home, &b.scbench_dir(), &["rev-parse", "HEAD"]),
        fork_sha
    );
    assert_eq!(
        git(&home, &b.scbench_dir(), &["remote", "get-url", "origin"]),
        p(&fork)
    );
    assert_eq!(
        b.record_at(&second)["manifest"]["scbench"]["repo"],
        p(&fork)
    );
}

fn read_json(path: &Path) -> Value {
    json(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

#[test]
fn rdm_arm_invocation_uses_rdm_agent_config() {
    let b = Bench::new(&Spec {
        rdm_arm: true,
        ..Spec::default()
    });
    ok(&b.run(&[]));
    let arm_dir = b.out.join("just-solve-rdm");
    let run_yaml = arm_dir.join("seed-1/run.yaml");
    let want: Vec<String> = [
        "run",
        "slop-code",
        "--seed",
        "1",
        "run",
        "--config",
        &p(&run_yaml),
        "--no-evaluate",
        "--no-live-progress",
    ]
    .map(str::to_owned)
    .into();
    assert!(b.argvs("uv").contains(&want), "{:?}", b.argvs("uv"));

    let run_config = read_json(&run_yaml);
    assert_eq!(run_config["agent"], p(&arm_dir.join("agent.yaml")));
    let agent = read_json(&arm_dir.join("agent.yaml"));
    assert_eq!(agent["binary"], "/opt/rdm/bin/claude-rdm");
    assert_eq!(agent["version"], format!("2.1.322-rdm.{}", b.rdm_sha));
    assert_eq!(agent["docker_template"], p(&arm_dir.join("docker.j2")));
    assert!(arm_dir.join("docker.j2").is_file());
    assert_eq!(
        agent["extra_args"],
        json!(["--plugin-dir", "/opt/rdm/plugin"])
    );

    let stock = read_json(&b.out.join("just-solve/agent.yaml"));
    for key in ["docker_template", "extra_args", "env"] {
        assert!(stock.get(key).is_none(), "stock agent config has {key}");
    }
    assert_eq!(stock["version"], "2.1.322");
}

#[test]
fn rdm_arm_plan_repo_is_outside_graded_dir() {
    let b = Bench::new(&Spec {
        rdm_arm: true,
        ..Spec::default()
    });
    ok(&b.run(&[]));
    let arm_dir = b.out.join("just-solve-rdm");
    let agent = read_json(&arm_dir.join("agent.yaml"));
    let plan_root = agent["env"]["RDM_ROOT"].as_str().expect("RDM_ROOT");
    let record = b.record();
    let arm = record["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .find(|a| a["name"] == "just-solve-rdm")
        .expect("rdm arm record");
    let graded = arm["graded_dir"].as_str().expect("graded_dir");
    assert_eq!(graded, "/workspace");
    assert_eq!(plan_root, "/rdm-plan");
    assert!(!Path::new(plan_root).starts_with(graded));
    let template = std::fs::read_to_string(arm_dir.join("docker.j2")).expect("docker.j2");
    assert!(template.contains("mkdir -p /rdm-plan"), "{template}");
}

#[test]
fn plan_root_inside_graded_dir_is_refused() {
    let b = Bench::new(&Spec {
        rdm_arm: true,
        plan_root: "/workspace/.rdm",
        ..Spec::default()
    });
    let err = fails(&b.run(&[]));
    assert!(
        err.contains("plan_root") && err.contains("/workspace"),
        "stderr: {err}"
    );
    assert_eq!(b.argvs("uv"), vec![vec!["--version".to_owned()]]);
}

#[test]
fn image_build_uses_checkout_context_and_labels_head() {
    let b = Bench::new(&Spec::default());
    ok(&b.image("build", &[]));
    let head = git(&b.home(), &b.rdm_dir(), &["rev-parse", "HEAD"]);
    let docker = b.argvs("docker");
    assert_eq!(docker.len(), 1, "{docker:?}");
    let build = &docker[0];
    assert_eq!(build[0], "build");
    let has = |pair: [&str; 2]| build.windows(2).any(|w| w == pair);
    let dockerfile = p(&b.rdm_dir().join("bench/scbench/agent/Dockerfile"));
    assert!(has(["-f", &dockerfile]), "{build:?}");
    assert!(
        has(["--build-arg", &format!("RDM_SHA={head}")]),
        "{build:?}"
    );
    assert!(
        has(["--label", &format!("org.rdm.scbench.rdm-sha={head}")]),
        "{build:?}"
    );
    assert!(
        has(["-t", &format!("rdm-scbench-payload:{head}")]),
        "{build:?}"
    );
    assert_eq!(build.last(), Some(&p(&b.rdm_dir())));

    let image_dir = b.cache.join("rdm-image").join(&head);
    let agent_yaml = image_dir.join("agent.yaml");
    let env_yaml = b
        .scbench_dir()
        .join("configs/environments/docker-python3.12-uv.yaml");
    let uv = b.calls("uv").expect("uv called");
    assert_eq!(uv.len(), 1, "{uv:?}");
    assert_eq!(
        strs(&uv[0]["argv"]),
        [
            "run",
            "slop-code",
            "docker",
            "build-agent",
            &p(&agent_yaml),
            &p(&env_yaml),
            "--force-build"
        ]
    );
    assert_eq!(uv[0]["cwd"], p(&b.scbench_dir()));
    let agent = read_json(&agent_yaml);
    let template_path = agent["docker_template"].as_str().expect("docker_template");
    let template = std::fs::read_to_string(template_path).expect("rendered template");
    assert!(
        template.contains(&format!("COPY --from=rdm-scbench-payload:{head}")),
        "{template}"
    );
}

#[test]
fn image_build_refuses_dirty_checkout() {
    let b = Bench::new(&Spec::default());
    write(&b.rdm_dir().join("stray.txt"), "untracked\n");
    let err = fails(&b.image("build", &[]));
    assert!(err.contains(&p(&b.rdm_dir())), "stderr: {err}");
    assert!(b.calls("docker").is_none(), "docker was spawned");
}

#[test]
fn image_build_stops_when_docker_build_fails() {
    let b = Bench::new(&Spec::default());
    b.scenario("docker", &json!({ "default": { "exit": 1 } }), &[]);
    let err = fails(&b.image("build", &[]));
    assert!(err.contains("docker build"), "stderr: {err}");
    let uv = b.argvs("uv");
    assert!(
        !uv.iter().any(|a| a.iter().any(|x| x == "build-agent")),
        "{uv:?}"
    );
}

const LISTING_OK: &str = "rdm@inline\n  Source: rdm@inline\n  Skills (2)  do, land\n";

/// A built bench whose fake `docker` answers the three check probes.
fn checked(sha: Option<&str>, rdm: Value, listing: Value) -> (Bench, Output) {
    let b = Bench::new(&Spec::default());
    ok(&b.image("build", &[]));
    let sha = sha.map_or_else(|| b.rdm_sha.clone(), str::to_owned);
    b.scenario(
        "docker",
        &json!({ "default": { "exit": 0 } }),
        &[
            (
                &["/opt/rdm/RDM_SHA"],
                json!({ "stdout": format!("{sha}\n") }),
            ),
            (&["/opt/rdm/bin/rdm"], rdm),
            (&["plugin details"], listing),
        ],
    );
    std::fs::remove_file(b.record_path("docker")).expect("reset docker record");
    let o = b.image("check", &[]);
    (b, o)
}

fn rdm_ok() -> Value {
    json!({ "stdout": "rdm 0.24.1\n" })
}

#[test]
fn image_check_passes_on_matching_image() {
    let (b, o) = checked(None, rdm_ok(), json!({ "stdout": LISTING_OK }));
    ok(&o);
    let image = format!("slop-code:claude_code-2.1.322-rdm.{}-python3.12", b.rdm_sha);
    let docker = b.argvs("docker");
    assert_eq!(docker.len(), 3, "{docker:?}");
    for argv in &docker {
        assert_eq!(&argv[..2], ["run", "--rm"]);
        assert!(argv.contains(&image), "{argv:?}");
    }
}

#[test]
fn image_check_names_sha_mismatch() {
    let other = "0123456789abcdef0123456789abcdef01234567";
    let (b, o) = checked(Some(other), rdm_ok(), json!({ "stdout": LISTING_OK }));
    let err = fails(&o);
    assert!(err.contains(other) && err.contains(&b.rdm_sha), "{err}");
}

#[test]
fn image_check_names_failing_rdm_version() {
    let (_b, o) = checked(
        None,
        json!({ "exit": 127 }),
        json!({ "stdout": LISTING_OK }),
    );
    let err = fails(&o);
    assert!(err.contains("rdm --version"), "{err}");
}

#[test]
fn image_check_names_missing_rdm_skills() {
    let (_b, o) = checked(
        None,
        rdm_ok(),
        json!({ "stdout": "rdm@inline\n  Skills (1)  do\n" }),
    );
    let err = fails(&o);
    assert!(err.contains("land"), "{err}");

    let (_b, o) = checked(None, rdm_ok(), json!({ "stdout": "rdm@inline\n" }));
    let err = fails(&o);
    assert!(err.contains("shows no rdm skills"), "{err}");
}

#[test]
fn image_check_reports_every_failure() {
    let other = "0123456789abcdef0123456789abcdef01234567";
    let (_b, o) = checked(Some(other), json!({ "exit": 1 }), json!({ "exit": 1 }));
    let err = fails(&o);
    for want in [other, "rdm --version", "shows no rdm skills"] {
        assert!(err.contains(want), "missing {want:?} in {err}");
    }
}

// --- The full rdm lane arm -------------------------------------------------

/// The full-lane arm of a dry-run plan of the committed manifest.
/// The committed arm `name`'s prompt as the dry-run plan renders it
/// (placeholders filled, Jinja left for SCBench).
fn committed_arm_prompt(name: &str) -> String {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plan = committed_plan(&tmp.path().join("cache"), &tmp.path().join("out"));
    let arm = plan["plan"]["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no committed arm {name}"));
    assert_eq!(arm["image"], "rdm");
    arm["prompt"].as_str().expect("rendered prompt").to_owned()
}

/// Renders `template` as SCBench's `render_prompt` does, with Jinja.
fn jinja(template: &str, is_continuation: bool) -> String {
    let mut env = minijinja::Environment::new();
    env.add_template("prompt", template)
        .expect("template parses");
    env.get_template("prompt")
        .expect("template")
        .render(minijinja::context! {
            spec => "SPEC-SENTINEL\n",
            is_continuation => is_continuation,
            agent_type => "claude_code",
            agent_version => "2.1.322",
            model_name => "opus-5.5",
        })
        .expect("template renders")
}

fn has(text: &str, want: &str) {
    assert!(text.contains(want), "missing {want:?} in:\n{text}");
}

fn lacks(text: &str, not: &str) {
    assert!(!text.contains(not), "has {not:?} in:\n{text}");
}

/// The first-checkpoint and continuation setup every rdm rung shares: one
/// roadmap, created once, and one new phase per checkpoint.
fn assert_checkpoint_setup(first: &str, cont: &str) {
    for want in [
        "git init -b main",
        "rdm config set dispatch.verify",
        "rdm roadmap create solution",
        "rdm phase create checkpoint-1 ",
        "--number 1 ",
    ] {
        has(first, want);
    }
    lacks(first, "rdm phase list");

    has(
        cont,
        "rdm phase list --roadmap solution --project bench --format json",
    );
    has(cont, "rdm phase create checkpoint-N ");
    lacks(cont, "rdm roadmap create");
    lacks(cont, "git init");
}

#[test]
fn full_lane_prompt_renders_first_checkpoint_and_continuation() {
    let template = committed_arm_prompt("full-lane");
    let first = jinja(&template, false);
    let cont = jinja(&template, true);
    assert_checkpoint_setup(&first, &cont);

    for text in [&first, &cont] {
        has(
            text,
            "`rdm:dispatch-phase` with arguments `solution N --project bench`",
        );
        has(text, "`rdm:land` with arguments `solution/");
        has(text, "SPEC-SENTINEL");
        lacks(text, "@RDM_");
    }
}

#[test]
fn ladder_rung_prompts_render_first_checkpoint_and_continuation() {
    for rung in ["tracked", "planned"] {
        let template = committed_arm_prompt(rung);
        let first = jinja(&template, false);
        let cont = jinja(&template, true);
        assert_checkpoint_setup(&first, &cont);
        for text in [&first, &cont] {
            has(text, "rdm phase update N --status done");
            has(text, "--roadmap solution --project bench");
            has(text, "SPEC-SENTINEL");
            lacks(text, "@RDM_");
            lacks(text, "rdm:land");
            if rung == "tracked" {
                lacks(text, "rdm:dispatch-phase");
                lacks(text, "rdm plan");
            } else {
                has(
                    text,
                    "`rdm:dispatch-phase` with arguments `solution N --plan-only --project bench`",
                );
                has(
                    text,
                    "rdm plan list --implements phase/solution/N --status approved --project bench",
                );
                has(
                    text,
                    "rdm worktree remove solution --delete-branch --project bench",
                );
            }
        }
    }
}

#[test]
fn full_lane_invocation_uses_rdm_image_template_and_arm_limits() {
    let b = Bench::new(&Spec {
        step_limit: 7,
        full_lane: true,
        ..Spec::default()
    });
    ok(&b.run(&[]));
    let arm_dir = b.out.join("full-lane");
    let run_yaml = arm_dir.join("seed-1/run.yaml");
    let want: Vec<String> = [
        "run",
        "slop-code",
        "--seed",
        "1",
        "run",
        "--config",
        &p(&run_yaml),
        "--no-evaluate",
        "--no-live-progress",
    ]
    .map(str::to_owned)
    .into();
    assert!(b.argvs("uv").contains(&want), "{:?}", b.argvs("uv"));

    let run_config = read_json(&run_yaml);
    let prompt = arm_dir.join("prompt.jinja");
    assert_eq!(run_config["prompt"], p(&prompt));
    let text = std::fs::read_to_string(&prompt).expect("prompt.jinja");
    assert!(!text.contains("@RDM_"), "{text}");
    assert!(text.contains("{{ spec | trim }}"), "{text}");

    assert_eq!(run_config["agent"], p(&arm_dir.join("agent.yaml")));
    let agent = read_json(&arm_dir.join("agent.yaml"));
    assert_eq!(agent["binary"], "/opt/rdm/bin/claude-rdm");
    assert_eq!(agent["version"], format!("2.1.322-rdm.{}", b.rdm_sha));
    assert_eq!(agent["docker_template"], p(&arm_dir.join("docker.j2")));
    assert_eq!(
        agent["extra_args"],
        json!(["--plugin-dir", "/opt/rdm/plugin"])
    );
    assert!(
        agent["env"]["RDM_PLAN_EXPORT_SUBDIR"].is_string(),
        "{agent}"
    );
    assert_eq!(agent["env"]["RDM_ROADMAP"], "solution", "{agent}");
    assert_eq!(
        agent["cost_limits"],
        json!({ "step_limit": 4321, "cost_limit": 12.5, "net_cost_limit": 25.0 })
    );

    let stock = read_json(&b.out.join("just-solve/agent.yaml"));
    assert_eq!(stock["cost_limits"]["step_limit"], 7);
    let stock_run = read_json(&b.out.join("just-solve/seed-1/run.yaml"));
    assert_eq!(
        stock_run["prompt"],
        p(&b.scbench_dir().join("configs/prompts/just-solve.jinja"))
    );
}

/// One evolving fixture plan repo, written by rdm-core itself and committed
/// to git, snapshotted into tars as the agent wrapper does.
struct PlanRepo {
    home: PathBuf,
    dir: PathBuf,
    store: rdm_store_fs::FsStore,
}

impl PlanRepo {
    fn new(home: &Path, dir: &Path) -> Self {
        use rdm_core::store::Store;
        std::fs::create_dir_all(dir).expect("mkdir plan repo");
        git(home, dir, &["init", "-q", "-b", "main"]);
        let mut store = rdm_store_fs::FsStore::new(dir);
        rdm_core::ops::init::init(&mut store).expect("init");
        rdm_core::ops::project::create_project(&mut store, "bench", "bench").expect("project");
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "bench",
                slug: "solution",
                title: "SlopCodeBench solution",
                ..Default::default()
            },
        )
        .expect("roadmap");
        store.commit().expect("flush");
        git(home, dir, &["add", "-A"]);
        git(home, dir, &["commit", "-q", "-m", "init"]);
        Self {
            home: home.to_path_buf(),
            dir: dir.to_path_buf(),
            store,
        }
    }

    /// Creates phase `n` (`phase-<n>-checkpoint-<n>`) at `status`.
    fn add(&mut self, n: u32, status: rdm_core::model::PhaseStatus, reason: Option<&str>) {
        let (slug, title) = (format!("checkpoint-{n}"), format!("Checkpoint {n}"));
        rdm_core::ops::phase::create_phase(
            &mut self.store,
            rdm_core::ops::phase::CreatePhase {
                project: "bench",
                roadmap: "solution",
                slug: &slug,
                title: &title,
                number: Some(n),
                ..Default::default()
            },
        )
        .expect("phase");
        self.set(n, status, reason);
    }

    /// Moves phase `n` to `status`, recording `reason` when given.
    fn set(&mut self, n: u32, status: rdm_core::model::PhaseStatus, reason: Option<&str>) {
        use rdm_core::ops::update::{BodyUpdate, ReasonUpdate, TagsUpdate, TitleUpdate};
        use rdm_core::store::Store;
        let stem = format!("phase-{n}-checkpoint-{n}");
        rdm_core::ops::phase::update_phase(
            &mut self.store,
            "bench",
            "solution",
            &stem,
            Some(status),
            TagsUpdate::Keep,
            BodyUpdate::Keep,
            None,
            None,
            None,
            None,
            TitleUpdate::Keep,
        )
        .expect("update phase");
        if let Some(reason) = reason {
            rdm_core::ops::phase::set_phase_blocked_reason(
                &mut self.store,
                "bench",
                "solution",
                &stem,
                ReasonUpdate::Set(reason.to_owned()),
            )
            .expect("blocked reason");
        }
        self.store.commit().expect("flush");
    }

    /// Commits the plan repo and archives it (with `.git` and a `marker`
    /// file) to `tar`, as the wrapper's `tar -C "$RDM_ROOT" -cf <tar> .`.
    /// Each `extra` `(name, text)` is written next to the marker after the
    /// commit, so it is a plain tar member only.
    fn snapshot(&self, tar: &Path, marker: &str, extra: &[(&str, &str)]) -> PathBuf {
        git(&self.home, &self.dir, &["add", "-A"]);
        git(
            &self.home,
            &self.dir,
            &["commit", "-q", "--allow-empty", "-m", "snapshot"],
        );
        let marker_path = self.dir.join(marker);
        write(&marker_path, "marker\n");
        for (name, text) in extra {
            write(&self.dir.join(name), text);
        }
        std::fs::create_dir_all(tar.parent().expect("tar parent")).expect("mkdir tars");
        let o = Command::new("tar")
            .arg("-C")
            .arg(&self.dir)
            .arg("-cf")
            .arg(tar)
            .arg(".")
            .output()
            .expect("tar runs");
        assert!(
            o.status.success(),
            "tar: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        std::fs::remove_file(&marker_path).expect("remove marker");
        for (name, _) in extra {
            std::fs::remove_file(self.dir.join(name)).expect("remove extra");
        }
        tar.to_path_buf()
    }
}

/// One checkpoint SCBench writes: its `order` and the export tars the
/// wrapper wrote during it (file name, source tar).
struct Checkpoint {
    problem: &'static str,
    name: &'static str,
    order: u32,
    tars: Vec<(&'static str, PathBuf)>,
}

/// A full-lane bench whose fake `uv` run of the full-lane arm leaves
/// `checkpoints` in SCBench's output layout, written out literally:
/// `<out>/full-lane/seed-1/output/<problem>/<checkpoint>/checkpoint.yaml`
/// and `.../agent/workspace/projects/-workspace/rdm-plan/<n>.tar`.
fn full_lane_bench(b: &Bench, checkpoints: &[Checkpoint]) {
    full_lane_bench_seeds(b, &[(1, checkpoints)]);
}

/// [`full_lane_bench`] for several seeds: each seed's run of the full-lane
/// arm (matched by its own `seed-<n>/run.yaml`) leaves its own checkpoints
/// under `<out>/full-lane/seed-<n>/output/`.
fn full_lane_bench_seeds(b: &Bench, seeds: &[(u64, &[Checkpoint])]) {
    let staged = b.root.join("staged");
    let mut responses: Vec<(Vec<String>, Value)> = vec![(
        vec!["--version".to_owned()],
        json!({ "stdout": "uv 0.0.0-fake\n" }),
    )];
    for (seed, checkpoints) in seeds {
        let output = b.out.join(format!("full-lane/seed-{seed}/output"));
        let mut place = Vec::new();
        for c in *checkpoints {
            let yaml = staged.join(format!("{seed}-{}-{}.yaml", c.problem, c.name));
            write(
                &yaml,
                &format!(
                    "name: {}\norder: {}\nstate: Core Tests\nspecification: spec.md\n",
                    c.name, c.order
                ),
            );
            let dir = output.join(c.problem).join(c.name);
            place.push(json!({ "from": yaml, "to": dir.join("checkpoint.yaml") }));
            for (file, tar) in &c.tars {
                let to = dir
                    .join("agent/workspace/projects/-workspace/rdm-plan")
                    .join(file);
                place.push(json!({ "from": tar, "to": to }));
            }
        }
        responses.push((
            vec![format!("full-lane/seed-{seed}/run.yaml")],
            json!({ "place": place }),
        ));
    }
    let needles: Vec<Vec<&str>> = responses
        .iter()
        .map(|(n, _)| n.iter().map(String::as_str).collect())
        .collect();
    let responses: Vec<(&[&str], Value)> = needles
        .iter()
        .zip(&responses)
        .map(|(n, (_, r))| (n.as_slice(), r.clone()))
        .collect();
    b.scenario("uv", &json!({ "default": { "exit": 0 } }), &responses);
}

fn full_lane_spec() -> Spec {
    Spec {
        full_lane: true,
        ..Spec::default()
    }
}

/// The run of `full_lane_run_copies_plan_repos_next_to_output`'s fixture.
fn copy_out_run() -> Bench {
    use rdm_core::model::PhaseStatus::{Done, InProgress};
    let b = Bench::new(&full_lane_spec());
    let home = b.home();
    let tars = b.root.join("tars");
    let mut mv = PlanRepo::new(&home, &b.root.join("fixture/mvvault"));
    mv.add(1, Done, None);
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "MARKER-000001", &[]);
    mv.add(2, InProgress, None);
    let mv2 = mv.snapshot(&tars.join("mv-2.tar"), "MARKER-000002", &[]);
    mv.set(2, Done, None);
    let mv3 = mv.snapshot(&tars.join("mv-3.tar"), "MARKER-000003", &[]);
    let mut xjq = PlanRepo::new(&home, &b.root.join("fixture/xjq"));
    xjq.add(1, Done, None);
    let x1 = xjq.snapshot(&tars.join("x-1.tar"), "MARKER-XJQ-1", &[]);
    full_lane_bench(
        &b,
        &[
            Checkpoint {
                problem: "mvvault",
                name: "checkpoint_1",
                order: 1,
                tars: vec![("000001.tar", mv1)],
            },
            Checkpoint {
                problem: "mvvault",
                name: "checkpoint_2",
                order: 2,
                tars: vec![("000002.tar", mv2), ("000003.tar", mv3)],
            },
            Checkpoint {
                problem: "xjq",
                name: "checkpoint_1",
                order: 1,
                tars: vec![("000001.tar", x1)],
            },
        ],
    );
    ok(&b.run(&[]));
    b
}

fn arm_record(record: &Value, name: &str) -> Value {
    record["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .find(|a| a["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no arm {name}"))
}

fn has_path_named(dir: &Path, name: &str) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let path = e.path();
            e.file_name() == name || (path.is_dir() && has_path_named(&path, name))
        })
}

#[test]
fn full_lane_run_copies_plan_repos_next_to_output() {
    let b = copy_out_run();
    let repos = b.out.join("full-lane/seed-1/plan-repos");
    let cp2 = repos.join("mvvault/checkpoint_2");
    assert!(cp2.join("MARKER-000003").is_file());
    assert!(!cp2.join("MARKER-000002").exists());
    assert!(cp2.join(".git").is_dir());
    assert!(repos.join("mvvault/checkpoint_1/MARKER-000001").is_file());
    assert!(repos.join("xjq/checkpoint_1/MARKER-XJQ-1").is_file());
    assert!(!has_path_named(
        &b.out.join("full-lane/seed-1/output"),
        "plan-repos"
    ));

    let record = b.record();
    let lane = &arm_record(&record, "full-lane")["lane"];
    let plan_repos: Vec<_> = lane["problems"]
        .as_array()
        .expect("problems")
        .iter()
        .map(|p| (p["problem"].clone(), p["plan_repo"].clone()))
        .collect();
    assert_eq!(
        plan_repos,
        [
            (json!("mvvault"), json!(p(&cp2))),
            (json!("xjq"), json!(p(&repos.join("xjq/checkpoint_1")))),
        ]
    );

    let argvs: Vec<Vec<String>> = record["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|c| strs(&c["argv"]))
        .collect();
    let at = |pred: &dyn Fn(&[String]) -> bool| {
        argvs.iter().position(|a| pred(a)).expect("command present")
    };
    let run = at(&|a| a.iter().any(|x| x.ends_with("full-lane/seed-1/run.yaml")));
    let eval = at(&|a| a.iter().any(|x| x.ends_with("full-lane/seed-1/output")));
    let tars: Vec<usize> = argvs
        .iter()
        .enumerate()
        .filter(|(_, a)| a[0] == "tar" && a[1] == "-xf")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(tars.len(), 3, "{argvs:?}");
    assert!(tars.iter().all(|&i| run < i && i < eval), "{argvs:?}");
    let output = b.out.join("full-lane/seed-1/output");
    let export = "agent/workspace/projects/-workspace/rdm-plan";
    assert_eq!(
        argvs[tars[1]],
        [
            "tar".to_owned(),
            "-xf".to_owned(),
            p(&output
                .join("mvvault/checkpoint_2")
                .join(export)
                .join("000003.tar")),
            "-C".to_owned(),
            p(&cp2),
        ]
    );
}

#[test]
fn stock_arm_has_no_lane_record() {
    let b = copy_out_run();
    let record = b.record();
    assert_eq!(arm_record(&record, "just-solve")["lane"], Value::Null);
    assert!(!b.out.join("just-solve/seed-1/plan-repos").exists());
}

#[test]
fn full_lane_classifies_each_checkpoint() {
    use rdm_core::model::PhaseStatus::{Blocked, Done, InProgress, Reviewed};
    let b = Bench::new(&full_lane_spec());
    let home = b.home();
    let tars = b.root.join("tars");
    let mut mv = PlanRepo::new(&home, &b.root.join("fixture/mvvault"));
    mv.add(1, Done, None);
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "M1", &[]);
    mv.add(2, Blocked, Some("[code] rework budget exhausted"));
    let mv2 = mv.snapshot(&tars.join("mv-2.tar"), "M2", &[]);
    mv.add(3, Reviewed, None);
    let mv3 = mv.snapshot(&tars.join("mv-3.tar"), "M3", &[]);
    let mv4 = mv.snapshot(&tars.join("mv-4.tar"), "M4", &[]);
    let mut xjq = PlanRepo::new(&home, &b.root.join("fixture/xjq"));
    xjq.add(1, InProgress, None);
    let x2 = xjq.snapshot(&tars.join("x-2.tar"), "X2", &[]);
    let cp = |problem, name, order, tars| Checkpoint {
        problem,
        name,
        order,
        tars,
    };
    full_lane_bench(
        &b,
        &[
            cp("mvvault", "checkpoint_1", 1, vec![("000001.tar", mv1)]),
            cp("mvvault", "checkpoint_2", 2, vec![("000002.tar", mv2)]),
            cp("mvvault", "checkpoint_3", 3, vec![("000003.tar", mv3)]),
            cp("mvvault", "checkpoint_4", 4, vec![("000004.tar", mv4)]),
            cp("xjq", "checkpoint_1", 1, vec![]),
            cp("xjq", "checkpoint_2", 2, vec![("000001.tar", x2)]),
        ],
    );
    let o = b.run(&[]);
    ok(&o);
    assert!(
        stderr(&o).contains("scbench run: full-lane seed 1: 1/6 checkpoints landed"),
        "{}",
        stderr(&o)
    );

    let repos = b.out.join("full-lane/seed-1/plan-repos");
    let at = |problem: &str, name: &str| json!(p(&repos.join(problem).join(name)));
    let row = |problem: &str,
               name: &str,
               order: u32,
               repo: bool,
               phase: Value,
               status: Value,
               outcome: &str,
               reason: Value,
               detail: Value| {
        json!({
            "checkpoint": name,
            "order": order,
            "plan_repo": if repo { at(problem, name) } else { Value::Null },
            "phase": phase,
            "status": status,
            "outcome": outcome,
            "reason": reason,
            "detail": detail,
        })
    };
    let n = Value::Null;
    let expected = json!({ "problems": [
        {
            "problem": "mvvault",
            "plan_repo": at("mvvault", "checkpoint_4"),
            "checkpoints": [
                row("mvvault", "checkpoint_1", 1, true, json!("phase-1-checkpoint-1"),
                    json!("done"), "landed", n.clone(), n.clone()),
                row("mvvault", "checkpoint_2", 2, true, json!("phase-2-checkpoint-2"),
                    json!("blocked"), "lane-failure", json!("parked"),
                    json!("[code] rework budget exhausted")),
                row("mvvault", "checkpoint_3", 3, true, json!("phase-3-checkpoint-3"),
                    json!("reviewed"), "lane-failure", json!("escalated"),
                    json!("reviewed but not landed: rdm:land escalated or did not run")),
                row("mvvault", "checkpoint_4", 4, true, n.clone(), n.clone(),
                    "lane-failure", json!("phase-missing"), n.clone()),
            ],
        },
        {
            "problem": "xjq",
            "plan_repo": at("xjq", "checkpoint_2"),
            "checkpoints": [
                row("xjq", "checkpoint_1", 1, false, n.clone(), n.clone(),
                    "lane-failure", json!("plan-repo-missing"), n.clone()),
                row("xjq", "checkpoint_2", 2, true, json!("phase-1-checkpoint-1"),
                    json!("in-progress"), "lane-failure", json!("unfinished"),
                    json!("in-progress")),
            ],
        },
    ]});
    assert_eq!(arm_record(&b.record(), "full-lane")["lane"], expected);
}

#[test]
fn failed_extraction_stops_before_eval() {
    let b = Bench::new(&full_lane_spec());
    let junk = b.root.join("tars/junk.tar");
    write(&junk, "not a tar archive\n");
    full_lane_bench(
        &b,
        &[Checkpoint {
            problem: "mvvault",
            name: "checkpoint_1",
            order: 1,
            tars: vec![("000001.tar", junk)],
        }],
    );
    let err = fails(&b.run(&[]));
    let placed = b
        .out
        .join("full-lane/seed-1/output/mvvault/checkpoint_1")
        .join("agent/workspace/projects/-workspace/rdm-plan/000001.tar");
    // The exit code is tar's own (bsdtar 1, GNU tar 2).
    let want = format!("arm full-lane: extracting {} exited ", p(&placed));
    assert!(err.contains(&want), "stderr: {err}");
    assert!(
        b.argvs("uv")
            .iter()
            .any(|a| a.iter().any(|x| x.ends_with("full-lane/seed-1/run.yaml"))),
        "{:?}",
        b.argvs("uv")
    );
    assert!(
        !b.argvs("uv")
            .iter()
            .any(|a| a.iter().any(|x| x.ends_with("full-lane/seed-1/output"))),
        "eval ran: {:?}",
        b.argvs("uv")
    );
    let record = b.record();
    let last = record["commands"]
        .as_array()
        .expect("commands")
        .last()
        .expect("a command");
    assert_eq!(strs(&last["argv"])[..2], ["tar", "-xf"]);
    assert_ne!(last["exit"], 0);
}

#[test]
fn unreadable_plan_repo_keeps_previous_phases() {
    use rdm_core::model::PhaseStatus::Done;
    let b = Bench::new(&full_lane_spec());
    let home = b.home();
    let tars = b.root.join("tars");
    let mut mv = PlanRepo::new(&home, &b.root.join("fixture/mvvault"));
    mv.add(1, Done, None);
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "M1", &[]);
    // A phase file rdm-core cannot parse makes the whole roadmap unreadable.
    let corrupt = mv
        .dir
        .join("projects/bench/roadmaps/solution/phase-2-corrupt.md");
    write(&corrupt, "---\nphase: [not a number\n---\n");
    let mv2 = mv.snapshot(&tars.join("mv-2.tar"), "M2", &[]);
    std::fs::remove_file(&corrupt).expect("remove corrupt phase");
    // Back to checkpoint 1's phases: with `prev` carried over the unreadable
    // checkpoint, nothing is new.
    let mv3 = mv.snapshot(&tars.join("mv-3.tar"), "M3", &[]);
    let cp = |name, order, tars| Checkpoint {
        problem: "mvvault",
        name,
        order,
        tars,
    };
    full_lane_bench(
        &b,
        &[
            cp("checkpoint_1", 1, vec![("000001.tar", mv1)]),
            cp("checkpoint_2", 2, vec![("000001.tar", mv2)]),
            cp("checkpoint_3", 3, vec![("000001.tar", mv3)]),
        ],
    );
    ok(&b.run(&[]));
    let lane = &arm_record(&b.record(), "full-lane")["lane"];
    let rows = lane["problems"][0]["checkpoints"]
        .as_array()
        .expect("checkpoints");
    let summary: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r["checkpoint"].clone(),
                r["phase"].clone(),
                r["outcome"].clone(),
                r["reason"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                json!("checkpoint_1"),
                json!("phase-1-checkpoint-1"),
                json!("landed"),
                Value::Null
            ),
            (
                json!("checkpoint_2"),
                Value::Null,
                json!("lane-failure"),
                json!("plan-repo-unreadable")
            ),
            (
                json!("checkpoint_3"),
                Value::Null,
                json!("lane-failure"),
                json!("phase-missing")
            ),
        ]
    );
    let detail = rows[1]["detail"].as_str().expect("unreadable detail");
    assert!(detail.contains("phase-2-corrupt"), "{}", rows[1]);
    assert_eq!(rows[1]["status"], Value::Null);
    let repos = b.out.join("full-lane/seed-1/plan-repos/mvvault");
    assert_eq!(rows[1]["plan_repo"], json!(p(&repos.join("checkpoint_2"))));
    assert_eq!(
        lane["problems"][0]["plan_repo"],
        json!(p(&repos.join("checkpoint_3")))
    );
}

#[test]
fn missing_manifest_prompt_fails_before_any_spawn() {
    let b = Bench::new(&full_lane_spec());
    let prompt = b.rdm_dir().join("bench/scbench/prompts/full-lane.jinja");
    std::fs::remove_file(&prompt).expect("remove full-lane prompt");
    let want = format!("arm full-lane: cannot read prompt {}", p(&prompt));
    for extra in [&[][..], &["--dry-run"][..]] {
        let o = b.run(extra);
        let err = fails(&o);
        assert!(err.contains(&want), "{extra:?} stderr: {err}");
        assert!(measure_support::stdout(&o).is_empty(), "{extra:?}");
        for role in ["uv", "docker", "claude"] {
            assert!(
                !b.record_path(role).exists(),
                "{extra:?}: {role} was spawned"
            );
        }
        assert!(!b.cache.exists(), "{extra:?}: --cache-dir was created");
        assert!(!b.out.exists(), "{extra:?}: --out was created");
    }
}

#[test]
fn rdm_arm_collects_plan_repos_per_seed() {
    use rdm_core::model::PhaseStatus::{Blocked, Done};
    let b = Bench::new(&full_lane_spec());
    let home = b.home();
    let tars = b.root.join("tars");
    let mut one = PlanRepo::new(&home, &b.root.join("fixture/seed-1"));
    one.add(1, Done, None);
    let t1 = one.snapshot(&tars.join("s1.tar"), "SEED-1", &[]);
    let mut two = PlanRepo::new(&home, &b.root.join("fixture/seed-2"));
    two.add(1, Blocked, Some("[plan] escalated"));
    let t2 = two.snapshot(&tars.join("s2.tar"), "SEED-2", &[]);
    let cp = |tar: &PathBuf| {
        vec![Checkpoint {
            problem: "mvvault",
            name: "checkpoint_1",
            order: 1,
            tars: vec![("000001.tar", tar.clone())],
        }]
    };
    let (c1, c2) = (cp(&t1), cp(&t2));
    full_lane_bench_seeds(&b, &[(1, &c1), (2, &c2)]);
    let o = b.run(&["--arms", "full-lane", "--seeds", "2"]);
    ok(&o);
    for seed in [1, 2] {
        let line = format!(
            "scbench run: full-lane seed {seed}: {}/1 checkpoints landed",
            2 - seed
        );
        assert!(stderr(&o).contains(&line), "{}", stderr(&o));
        let repo = b.out.join(format!(
            "full-lane/seed-{seed}/plan-repos/mvvault/checkpoint_1"
        ));
        assert!(repo.join(format!("SEED-{seed}")).is_file(), "seed {seed}");
    }
    let record = b.record();
    let lane = |seed: u64| {
        record["arms"]
            .as_array()
            .expect("arms")
            .iter()
            .find(|a| a["name"] == "full-lane" && a["seed"] == seed)
            .expect("unit")["lane"]["problems"][0]["checkpoints"][0]
            .clone()
    };
    assert_eq!(lane(1)["outcome"], "landed");
    assert_eq!(lane(2)["reason"], "parked");
    assert_eq!(
        lane(2)["plan_repo"],
        p(&b.out
            .join("full-lane/seed-2/plan-repos/mvvault/checkpoint_1"))
    );
}

// --- The ladder ------------------------------------------------------------

/// Has the fake `uv` answer `--version` and, on each unit's run (matched by
/// its own `<out>/<arm>/seed-<n>/run.yaml`), leave a one-checkpoint SCBench
/// tree per problem in that unit's `output/`, for every arm and seed under
/// each of `outs`.
fn ladder_outputs(b: &Bench, outs: &[&Path], arms: &[&str], seeds: u64) {
    ladder_outputs_with(b, outs, arms, seeds, &|_, response| response);
}

/// [`ladder_outputs`], with each unit's run response passed through
/// `tweak(unit_dir, response)` first.
fn ladder_outputs_with(
    b: &Bench,
    outs: &[&Path],
    arms: &[&str],
    seeds: u64,
    tweak: &dyn Fn(&Path, Value) -> Value,
) {
    let staged = b.root.join("staged-scbench");
    let mut responses: Vec<(String, Value)> = Vec::new();
    let mut n = 0;
    for out in outs {
        for arm in arms {
            for seed in 1..=seeds {
                let unit = out.join(arm).join(format!("seed-{seed}"));
                let run_dir = unit.join("output");
                let mut place = Vec::new();
                for problem in ["mvvault", "xjq"] {
                    let cp = scbench_support::simple(problem, "checkpoint_1", 1);
                    for (rel, text) in scbench_support::files(&run_dir, &cp) {
                        n += 1;
                        let from = staged.join(format!("{n}"));
                        write(&from, &text);
                        place.push(json!({ "from": from, "to": run_dir.join(rel) }));
                    }
                }
                let response = tweak(&unit, json!({ "place": place }));
                responses.push((p(&unit.join("run.yaml")), response));
            }
        }
    }
    let version = ["--version"];
    let needles: Vec<[&str; 1]> = responses.iter().map(|(n, _)| [n.as_str()]).collect();
    let mut all: Vec<(&[&str], Value)> = vec![(&version, json!({ "stdout": "uv 0.0.0-fake\n" }))];
    all.extend(
        needles
            .iter()
            .zip(&responses)
            .map(|(n, (_, r))| (&n[..], r.clone())),
    );
    b.scenario("uv", &json!({ "default": { "exit": 0 } }), &all);
}

fn docker_version() -> Vec<String> {
    ["version", "--format", "{{.Server.Version}}"]
        .map(str::to_owned)
        .into()
}

/// A bench with an rdm arm whose `image build` ran, and whose fake `docker`
/// answers the three image-check probes with `sha` (default: HEAD) and the
/// version probe; the build's uv and docker records are reset.
fn built_rdm_bench(sha: Option<&str>) -> Bench {
    let b = Bench::new(&Spec {
        rdm_arm: true,
        ..Spec::default()
    });
    ok(&b.image("build", &[]));
    let sha = sha.map_or_else(|| b.rdm_sha.clone(), str::to_owned);
    b.scenario(
        "docker",
        &json!({ "default": { "stdout": "99.0.0-fake\n" } }),
        &[
            (
                &["/opt/rdm/RDM_SHA"],
                json!({ "stdout": format!("{sha}\n") }),
            ),
            (&["/opt/rdm/bin/rdm"], rdm_ok()),
            (&["plugin details"], json!({ "stdout": LISTING_OK })),
        ],
    );
    for role in ["uv", "docker"] {
        std::fs::remove_file(b.record_path(role)).expect("reset record");
    }
    b
}

fn spawned_slop_code(b: &Bench) -> bool {
    b.argvs("uv")
        .iter()
        .any(|a| a.iter().any(|x| x == "slop-code"))
}

#[test]
fn ladder_runs_preflight_every_arm_then_the_report() {
    let b = Bench::new(&Spec::default());
    let arms = ["just-solve", "anti_slop"];
    ladder_outputs(&b, &[&b.out], &arms, 3);
    let o = b.ladder(&b.out, &["--seeds", "3"], &[]);
    let stdout = ok(&o);

    // The preflight's `uv --version`, then the run's own version probe (a
    // run always records the versions it observed), then every arm x seed,
    // seed-major.
    let mut expected = vec![vec!["--version".to_owned()], vec!["--version".to_owned()]];
    for seed in 1..=3 {
        for arm in arms {
            expected.extend(unit_argvs(&b.out, arm, seed, "all-cases"));
        }
    }
    assert_eq!(b.argvs("uv"), expected);
    assert_eq!(b.argvs("docker"), [docker_version(), docker_version()]);

    let report = read_json(&b.out.join("rdm-scbench-report.json"));
    let got: Vec<(Value, Value)> = report["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .map(|a| (a["name"].clone(), a["seeds"].clone()))
        .collect();
    assert_eq!(got, arms.map(|a| (json!(a), json!([1, 2, 3]))).to_vec());
    assert_eq!(
        report["arms"][0]["summary"]["pass_rate"]["n"], 3,
        "{}",
        report["arms"][0]
    );
    let md = std::fs::read_to_string(b.out.join("rdm-scbench-report.md")).expect("md report");
    assert_eq!(stdout, md);
}

#[test]
fn ladder_with_an_rdm_arm_runs_the_image_check_first() {
    let b = built_rdm_bench(None);
    ok(&b.ladder(&b.out, &["--seeds", "1"], &[]));
    let docker = b.argvs("docker");
    assert_eq!(docker.len(), 5, "{docker:?}");
    assert_eq!(docker[0], docker_version());
    for argv in &docker[1..4] {
        assert_eq!(&argv[..2], ["run", "--rm"], "{docker:?}");
    }
    assert_eq!(docker[4], docker_version());
    let mut expected = vec![vec!["--version".to_owned()], vec!["--version".to_owned()]];
    for arm in ["just-solve", "anti_slop", "just-solve-rdm"] {
        expected.extend(unit_argvs(&b.out, arm, 1, "all-cases"));
    }
    assert_eq!(b.argvs("uv"), expected);
    assert!(b.out.join("rdm-scbench-report.json").is_file());
}

#[test]
fn ladder_preflight_failures_spawn_no_slop_code() {
    struct Case {
        name: &'static str,
        bench: Bench,
        extra: Vec<String>,
        env: Vec<(&'static str, Option<&'static str>)>,
        names: &'static str,
        check: fn(&Bench),
    }
    let no_uv = Bench::new(&Spec::default());
    let missing_uv = p(&no_uv.root.join("bin/no-such-uv"));
    let daemon_down = Bench::new(&Spec::default());
    daemon_down.scenario(
        "docker",
        &json!({ "default": { "exit": 1, "stderr": "Cannot connect to the Docker daemon\n" } }),
        &[],
    );
    let cases = vec![
        Case {
            name: "(a) unset OAuth token",
            bench: Bench::new(&Spec::default()),
            extra: Vec::new(),
            env: vec![("CLAUDE_CODE_OAUTH_TOKEN", None)],
            names: "CLAUDE_CODE_OAUTH_TOKEN",
            check: |b| {
                for role in ["uv", "docker", "claude"] {
                    assert!(b.calls(role).is_none(), "{role} was invoked");
                }
            },
        },
        Case {
            name: "(a') unset API key",
            bench: Bench::new(&Spec {
                provider: "anthropic",
                ..Spec::default()
            }),
            extra: Vec::new(),
            env: vec![("ANTHROPIC_API_KEY", None)],
            names: "ANTHROPIC_API_KEY",
            check: |b| assert!(b.calls("uv").is_none()),
        },
        Case {
            name: "(b) no uv",
            bench: no_uv,
            extra: vec!["--uv-bin".to_owned(), missing_uv],
            env: Vec::new(),
            names: "uv not found",
            check: |b| assert!(b.calls("docker").is_none(), "docker was invoked"),
        },
        Case {
            name: "(c) Docker daemon down",
            bench: daemon_down,
            extra: Vec::new(),
            env: Vec::new(),
            names: "Docker daemon is not reachable",
            check: |b| assert_eq!(b.argvs("uv"), [vec!["--version".to_owned()]]),
        },
        Case {
            name: "(d) image check fails",
            bench: built_rdm_bench(Some("0123456789abcdef0123456789abcdef01234567")),
            extra: Vec::new(),
            env: Vec::new(),
            names: "rdm image check failed",
            check: |b| assert_eq!(b.argvs("uv"), [vec!["--version".to_owned()]]),
        },
    ];
    for c in cases {
        let extra: Vec<&str> = std::iter::once("--seeds")
            .chain(std::iter::once("2"))
            .chain(c.extra.iter().map(String::as_str))
            .collect();
        let err = fails(&c.bench.ladder(&c.bench.out, &extra, &c.env));
        assert!(
            err.contains("error: ladder preflight: ") && err.contains(c.names),
            "{}: stderr: {err}",
            c.name
        );
        assert!(!err.contains("fake-token"), "{}: {err}", c.name);
        assert!(
            !spawned_slop_code(&c.bench),
            "{}: {:?}",
            c.name,
            c.bench.argvs("uv")
        );
        assert!(!c.bench.out.exists(), "{}: --out was created", c.name);
        (c.check)(&c.bench);
    }
}

#[test]
fn ladder_dry_run_prints_the_projection_and_spawns_nothing() {
    let b = Bench::new(&Spec::default());
    let o = b.ladder(&b.out, &["--seeds", "3", "--dry-run"], &[]);
    let body = json(&ok(&o));
    assert_eq!(
        body["projection"],
        json!({
            "arms": 2,
            "seeds": 3,
            "problems": { "mvvault": 3, "xjq": 2 },
            "checkpoints": 5,
            "sessions": 12,
            "checkpoint_runs": 30,
        })
    );
    assert_eq!(
        body["preflight"]["credential"],
        json!({ "variable": "CLAUDE_CODE_OAUTH_TOKEN", "set": true })
    );
    assert_eq!(body["preflight"]["image_check"], false);
    assert_eq!(
        strs(&body["preflight"]["docker"]),
        [vec![p(&b.bin("docker"))], docker_version()].concat()
    );
    assert_eq!(body["plan"]["seeds"], json!([1, 2, 3]));
    assert_eq!(
        body["plan"]["plan"]["arms"].as_array().map(Vec::len),
        Some(6)
    );
    assert!(
        stderr(&o).contains(
            "scbench ladder: projected 30 checkpoint runs (2 arms × 3 seeds × 5 checkpoints over 2 problems), 12 problem sessions"
        ),
        "{}",
        stderr(&o)
    );
    for role in ["uv", "docker", "claude"] {
        assert!(b.calls(role).is_none(), "{role} was spawned");
    }
    assert!(!b.out.exists() && !b.cache.exists());

    let unset = b.ladder(
        &b.out,
        &["--seeds", "3", "--dry-run"],
        &[("CLAUDE_CODE_OAUTH_TOKEN", None)],
    );
    assert_eq!(json(&ok(&unset))["preflight"]["credential"]["set"], false);
}

#[test]
fn ladder_passes_the_baseline_through() {
    let b = Bench::new(&Spec::default());
    let (out1, out2) = (b.root.join("out1"), b.root.join("out2"));
    let arms = ["just-solve", "anti_slop"];
    ladder_outputs(&b, &[&out1, &out2], &arms, 2);
    ok(&b.ladder(&out1, &["--seeds", "2"], &[]));
    ok(&b.report(&out1, &["--save-baseline", "first"]));
    assert!(
        b.rdm_dir()
            .join("bench/scbench/baselines/first.json")
            .is_file()
    );
    ok(&b.ladder(&out2, &["--seeds", "2", "--baseline", "first"], &[]));
    let report = read_json(&out2.join("rdm-scbench-report.json"));
    assert_eq!(report["baseline"]["name"], "first");
    assert_eq!(report["baseline"]["comparable"], true);
}

#[test]
fn ladder_preflight_loads_the_baseline_before_any_spawn() {
    let b = Bench::new(&Spec::default());
    ladder_outputs(&b, &[&b.out], &["just-solve", "anti_slop"], 2);
    let err = fails(&b.ladder(&b.out, &["--seeds", "2", "--baseline", "missing"], &[]));
    let path = b.rdm_dir().join("bench/scbench/baselines/missing.json");
    assert!(
        err.contains("error: ladder preflight: --baseline: cannot read ")
            && err.contains(&p(&path)),
        "stderr: {err}"
    );
    for role in ["uv", "docker", "claude"] {
        assert!(b.calls(role).is_none(), "{role} was invoked");
    }
    assert!(!b.out.exists(), "--out was created");
}

#[test]
fn ladder_run_failure_hints_at_report_and_skips_the_report() {
    let b = Bench::new(&Spec::default());
    let arms = ["just-solve", "anti_slop"];
    let failing = b.out.join("anti_slop/seed-2");
    ladder_outputs_with(&b, &[&b.out], &arms, 2, &|unit, response| {
        if unit == failing {
            json!({ "exit": 3, "stderr": "agent crashed\n" })
        } else {
            response
        }
    });
    let o = b.ladder(&b.out, &["--seeds", "2"], &[]);
    let err = fails(&o);
    assert!(
        err.contains("arm anti_slop seed 2: run exited 3")
            && err.contains(&format!(
                "rdm-measure scbench report --run {} reports them",
                p(&b.out)
            )),
        "stderr: {err}"
    );
    assert!(!b.out.join("rdm-scbench-report.json").exists());
    assert!(!b.out.join("rdm-scbench-report.md").exists());

    // The hinted command reports the finished units and skips the failed one.
    let r = json(&ok(&b.report(&b.out, &["--format", "json"])));
    let seeds: Vec<(Value, Value)> = r["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .map(|a| (a["name"].clone(), a["seeds"].clone()))
        .collect();
    assert_eq!(
        seeds,
        [
            (json!("just-solve"), json!([1, 2])),
            (json!("anti_slop"), json!([1])),
        ]
    );
    assert_eq!(
        r["skipped_runs"],
        json!([{ "arm": "anti_slop", "seed": 2, "reason": "not finished" }])
    );
}

#[test]
fn ladder_report_failure_hints_that_the_run_finished() {
    let b = Bench::new(&Spec::default());
    let arms = ["just-solve", "anti_slop"];
    let corrupt = b.out.join("just-solve/seed-1");
    let bad = corrupt.join("output/mvvault/checkpoint_1/evaluation.json");
    let garbage = b.root.join("staged-garbage");
    write(&garbage, "{not json");
    ladder_outputs_with(&b, &[&b.out], &arms, 1, &|unit, mut response| {
        if unit == corrupt {
            response["place"]
                .as_array_mut()
                .expect("place")
                .push(json!({ "from": garbage, "to": bad }));
        }
        response
    });
    let err = fails(&b.ladder(&b.out, &["--seeds", "1"], &[]));
    assert!(
        err.contains(&p(&bad))
            && err.contains(&format!(
                "the run finished and is on disk; rdm-measure scbench report --run {} reports it",
                p(&b.out)
            )),
        "stderr: {err}"
    );
    assert!(b.out.join("rdm-scbench-run.json").is_file());
    assert!(!b.out.join("rdm-scbench-report.json").exists());
}

/// A fake credential value; never a real one.
const TOKEN: &str = "sk-ant-oat01-fake-SCRUBME-0123456789";

/// The credential environment the scrub tests run with.
const WITH_TOKEN: &[(&str, Option<&str>)] = &[
    ("CLAUDE_CODE_OAUTH_TOKEN", Some(TOKEN)),
    ("ANTHROPIC_API_KEY", None),
];

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(bytes).expect("gzip");
    enc.finish().expect("gzip finish")
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .read_to_end(&mut out)
        .expect("gunzip");
    out
}

fn holds(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|w| w == needle)
}

/// Asserts that no regular file under `dir` (gzip decoded) holds [`TOKEN`].
fn assert_no_token(dir: &Path) {
    for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).expect("metadata");
        if meta.is_dir() {
            assert_no_token(&path);
        } else if meta.is_file() {
            let mut bytes = std::fs::read(&path).expect("read");
            if bytes.starts_with(&[0x1f, 0x8b]) {
                bytes = gunzip(&bytes);
            }
            assert!(
                !holds(&bytes, TOKEN.as_bytes()),
                "{} holds the token",
                path.display()
            );
        }
    }
}

/// One file the fake `slop-code run` leaves: its path under the unit's
/// `output/` and its staged bytes.
struct Leak {
    rel: &'static str,
    bytes: Vec<u8>,
}

fn leaks() -> Vec<Leak> {
    vec![
        Leak {
            rel: "p1/infer.log",
            bytes: format!(
                "exec: docker exec --env CLAUDE_CODE_OAUTH_TOKEN={TOKEN} --env X=1 box\nnext line\n"
            )
            .into_bytes(),
        },
        Leak {
            rel: "p1/checkpoint_1/deep/nested/trace.json",
            bytes: format!("{{\"t\":\"{TOKEN}\",\"again\":\"{TOKEN}\"}}\n").into_bytes(),
        },
        Leak {
            rel: "p1/checkpoint_1/blob.jsonl.gz",
            bytes: gzip(format!("{{\"line\":\"pre {TOKEN} post\"}}\n").as_bytes()),
        },
    ]
}

/// Stages `files` and has the fake `uv`'s `just-solve/seed-1` run place them
/// under that unit's `output/`, then exit `exit`. Returns each placed path.
fn leaky_run(b: &Bench, files: &[Leak], exit: u64) -> Vec<PathBuf> {
    let staged = b.root.join("staged-leaks");
    let output = b.out.join("just-solve/seed-1/output");
    let mut place = Vec::new();
    let mut placed = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let from = staged.join(i.to_string());
        std::fs::create_dir_all(&staged).expect("mkdir staged");
        std::fs::write(&from, &f.bytes).expect("stage");
        let to = output.join(f.rel);
        place.push(json!({ "from": from, "to": to }));
        placed.push(to);
    }
    b.scenario(
        "uv",
        &json!({ "default": { "exit": 0 } }),
        &[
            (&["--version"], json!({ "stdout": "uv 0.0.0-fake\n" })),
            (
                &["just-solve/seed-1/run.yaml"],
                json!({ "place": place, "exit": exit }),
            ),
        ],
    );
    placed
}

/// `bytes` with every [`TOKEN`] replaced by the plain placeholder.
fn redacted(bytes: &[u8]) -> Vec<u8> {
    String::from_utf8(bytes.to_vec())
        .expect("utf-8")
        .replace(TOKEN, "***redacted***")
        .into_bytes()
}

#[test]
fn scrub_removes_credential_from_log_nested_file_and_gzip_member() {
    let b = Bench::new(&Spec::default());
    let files = leaks();
    let placed = leaky_run(&b, &files, 0);
    let o = b.run_env(&[], WITH_TOKEN);
    ok(&o);
    assert_no_token(&b.out);
    for (f, path) in files.iter().zip(&placed) {
        let now = std::fs::read(path).expect("placed file");
        if f.rel.ends_with(".gz") {
            assert_eq!(gunzip(&now), redacted(&gunzip(&f.bytes)), "{}", f.rel);
        } else {
            assert_eq!(now, redacted(&f.bytes), "{}", f.rel);
        }
    }
    let err = stderr(&o);
    assert!(!err.contains(TOKEN));
    assert!(
        err.contains("scbench run: scrubbed the credential from 3 file(s)"),
        "{err}"
    );
}

#[test]
fn scrub_runs_when_slop_code_run_exits_nonzero() {
    let b = Bench::new(&Spec::default());
    leaky_run(&b, &leaks(), 3);
    let o = b.run_env(&[], WITH_TOKEN);
    let err = fails(&o);
    assert!(
        err.contains("arm just-solve seed 1: run exited 3"),
        "stderr: {err}"
    );
    assert!(!err.contains(TOKEN));
    assert_no_token(&b.out);
}

#[test]
fn scrub_without_credential_changes_no_file() {
    for value in [None, Some("")] {
        let b = Bench::new(&Spec::default());
        let files = leaks();
        let placed = leaky_run(&b, &files, 0);
        let o = b.run_env(
            &[],
            &[
                ("CLAUDE_CODE_OAUTH_TOKEN", value),
                ("ANTHROPIC_API_KEY", None),
            ],
        );
        ok(&o);
        for (f, path) in files.iter().zip(&placed) {
            assert_eq!(
                std::fs::read(path).expect("placed"),
                f.bytes,
                "{value:?} {}",
                f.rel
            );
        }
        assert!(!stderr(&o).contains("scrubbed"), "{}", stderr(&o));
    }
}

#[test]
fn scrub_names_unwritable_file_and_keeps_exit_status() {
    use std::os::unix::fs::PermissionsExt;
    // Root ignores mode bits, so a read-only file is writable there.
    if rustix::process::geteuid().is_root() {
        return;
    }
    let b = Bench::new(&Spec::default());
    let files = vec![
        Leak {
            rel: "p1/locked.log",
            bytes: format!("token {TOKEN}\n").into_bytes(),
        },
        Leak {
            rel: "p1/open.log",
            bytes: format!("token {TOKEN}\n").into_bytes(),
        },
    ];
    let placed = leaky_run(&b, &files, 0);
    let staged_locked = b.root.join("staged-leaks/0");
    std::fs::set_permissions(&staged_locked, std::fs::Permissions::from_mode(0o444))
        .expect("chmod");
    let o = b.run_env(&[], WITH_TOKEN);
    ok(&o);
    let err = stderr(&o);
    assert!(
        err.contains(&format!(
            "cannot scrub the credential from {}",
            p(&placed[0])
        )),
        "{err}"
    );
    assert!(!err.contains(TOKEN));
    assert_eq!(
        std::fs::read(&placed[1]).expect("open.log"),
        redacted(&files[1].bytes)
    );
}

#[test]
fn scrub_runs_after_failing_build_agent() {
    let b = Bench::new(&Spec::default());
    let staged = b.root.join("staged-leaks/home.log");
    write(
        &staged,
        &format!("docker --env CLAUDE_CODE_OAUTH_TOKEN={TOKEN}\n"),
    );
    let leaked = b.cache.join("scbench-home/logs/build.log");
    b.scenario(
        "uv",
        &json!({ "default": { "exit": 0 } }),
        &[(
            &["build-agent"],
            json!({ "place": [{ "from": staged, "to": leaked }], "exit": 2 }),
        )],
    );
    let o = b.image_env("build", &[], WITH_TOKEN);
    let err = fails(&o);
    assert!(
        err.contains("slop-code docker build-agent") && err.contains("exited 2"),
        "stderr: {err}"
    );
    assert!(!err.contains(TOKEN));
    assert_eq!(
        std::fs::read_to_string(&leaked).expect("leaked log"),
        "docker --env CLAUDE_CODE_OAUTH_TOKEN=***redacted***\n"
    );
}

#[test]
fn scrub_masks_plan_repo_tar_and_lane_still_extracts() {
    use rdm_core::model::PhaseStatus::Done;
    let b = Bench::new(&full_lane_spec());
    let tars = b.root.join("tars");
    let mut mv = PlanRepo::new(&b.home(), &b.root.join("fixture/mvvault"));
    mv.add(1, Done, None);
    let leak = format!("token={TOKEN}\n");
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "M1", &[("leak.txt", &leak)]);
    let original_len = std::fs::metadata(&mv1).expect("tar").len();
    full_lane_bench(
        &b,
        &[
            Checkpoint {
                problem: "mvvault",
                name: "checkpoint_1",
                order: 1,
                tars: vec![("000001.tar", mv1)],
            },
            Checkpoint {
                problem: "xjq",
                name: "checkpoint_1",
                order: 1,
                tars: vec![],
            },
        ],
    );
    let o = b.run_env(&[], WITH_TOKEN);
    ok(&o);
    assert!(!stderr(&o).contains(TOKEN));
    let placed = b.out.join(
        "full-lane/seed-1/output/mvvault/checkpoint_1/agent/workspace/projects/-workspace/rdm-plan/000001.tar",
    );
    let tar = std::fs::read(&placed).expect("placed tar");
    assert_eq!(tar.len() as u64, original_len);
    assert!(!holds(&tar, TOKEN.as_bytes()));
    let repos = b.out.join("full-lane/seed-1/plan-repos");
    let extracted = repos.join("mvvault/checkpoint_1/leak.txt");
    assert_eq!(
        std::fs::read_to_string(&extracted)
            .expect("extracted leak")
            .len(),
        leak.len()
    );
    assert_no_token(&b.out);
    let lane = &arm_record(&b.record(), "full-lane")["lane"];
    assert_eq!(
        lane["problems"][0]["checkpoints"][0]["outcome"], "landed",
        "{lane}"
    );
}

#[test]
fn scrub_runs_after_slop_code_eval_whatever_its_exit() {
    for exit in [0u64, 4] {
        let b = Bench::new(&Spec::default());
        let staged = b.root.join("staged-leaks/eval.log");
        write(
            &staged,
            &format!("eval --env CLAUDE_CODE_OAUTH_TOKEN={TOKEN}\n"),
        );
        let leaked = b.out.join("just-solve/seed-1/output/p1/eval.log");
        b.scenario(
            "uv",
            &json!({ "default": { "exit": 0 } }),
            &[
                (&["--version"], json!({ "stdout": "uv 0.0.0-fake\n" })),
                (
                    &["just-solve/seed-1/output"],
                    json!({ "place": [{ "from": staged, "to": leaked }], "exit": exit }),
                ),
            ],
        );
        let o = b.run_env(&[], WITH_TOKEN);
        if exit == 0 {
            ok(&o);
        } else {
            let err = fails(&o);
            assert!(
                err.contains("arm just-solve seed 1: eval exited 4"),
                "stderr: {err}"
            );
        }
        assert!(!stderr(&o).contains(TOKEN));
        assert_no_token(&b.out);
        assert_eq!(
            std::fs::read_to_string(&leaked).expect("eval log"),
            "eval --env CLAUDE_CODE_OAUTH_TOKEN=***redacted***\n",
            "exit {exit}"
        );
    }
}

// --- Interrupt teardown ------------------------------------------------------
//
// The runner is spawned in its own process group with its output in files
// (never pipes, so a leaked descendant cannot block the test), and the only
// signal the test sends goes to the runner's pid. Every pid the fake reports
// is tracked as soon as it is read, and the runner's guard kills and reaps
// its group on any failure.

impl Bench {
    /// Spawns `cmd` (an `rdm-measure` command) in its own process group,
    /// stdout and stderr to files under the bench root. Returns the guard and
    /// the two log paths.
    fn spawn_logged(&self, mut cmd: Command) -> (common::ChildGuard, PathBuf, PathBuf) {
        use std::os::unix::process::CommandExt;
        let out_log = self.root.join("runner.stdout.log");
        let err_log = self.root.join("runner.stderr.log");
        cmd.process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&out_log).expect("stdout log"))
            .stderr(std::fs::File::create(&err_log).expect("stderr log"));
        let child = cmd.spawn().expect("rdm-measure spawns");
        (common::ChildGuard::new(child), out_log, err_log)
    }
}

/// Waits (30 s, or until the runner exits) for the fake's readiness file;
/// otherwise panics with the runner's stderr (the guards then clean up).
fn wait_ready(path: &Path, err_log: &Path, runner: &mut std::process::Child) -> Value {
    use std::time::{Duration, Instant};
    let end = Instant::now() + Duration::from_secs(30);
    while !path.exists() && Instant::now() < end && matches!(runner.try_wait(), Ok(None)) {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !path.exists() {
        panic!(
            "the fake never became ready; runner stderr:\n{}",
            std::fs::read_to_string(err_log).unwrap_or_default()
        );
    }
    json(&std::fs::read_to_string(path).expect("ready file"))
}

/// Has the fake `uv --version` probe write its readiness file, then take 2 s
/// to answer, so a test can signal the runner while it waits on the probe.
/// Returns the readiness file's path.
fn slow_uv_probe(b: &Bench) -> PathBuf {
    let ready = b.root.join("ready.json");
    b.scenario(
        "uv",
        &json!({ "default": { "exit": 0 } }),
        &[(
            &["--version"],
            json!({ "ready": ready, "sleepMs": 2000, "stdout": "uv 0.0.0-fake\n" }),
        )],
    );
    ready
}

/// [`hanging_on`] for the unit's `slop-code run --no-evaluate`.
fn hanging_run(b: &Bench, behaviour: Value) -> PathBuf {
    hanging_on(b, "--no-evaluate", behaviour)
}

/// Has the fake `uv` call whose argv contains `needle` leave a
/// credential-laden log under the unit's output, write its readiness file,
/// then apply the `behaviour` keys (`hang`, `grandchild`, `ignore_sigterm`);
/// every other `uv` call exits 0. The fake `docker` lists no
/// container. Returns the readiness file's path.
fn hanging_on(b: &Bench, needle: &str, behaviour: Value) -> PathBuf {
    let ready = b.root.join("ready.json");
    let staged = b.root.join("staged-leaks/infer.log");
    write(
        &staged,
        &format!("exec: docker exec --env CLAUDE_CODE_OAUTH_TOKEN={TOKEN} box\n"),
    );
    let leaked = b.out.join("just-solve/seed-1/output/p1/infer.log");
    let mut response = json!({
        "place": [{ "from": staged, "to": leaked }],
        "ready": ready,
    });
    if let (Some(r), Value::Object(more)) = (response.as_object_mut(), behaviour) {
        r.extend(more);
    }
    b.scenario(
        "uv",
        &json!({ "default": { "exit": 0 } }),
        &[
            (&["--version"], json!({ "stdout": "uv 0.0.0-fake\n" })),
            (&[needle], response),
        ],
    );
    b.scenario(
        "docker",
        &json!({ "default": { "stdout": "99.0.0-fake\n" } }),
        &[(&["ps"], json!({ "stdout": "" }))],
    );
    ready
}

/// How an interrupted run ended.
struct Stopped {
    status: std::process::ExitStatus,
    stderr: String,
    ready: Value,
    since_signal: std::time::Duration,
}

/// Spawns `scbench run --arms just-solve` with `extra` and the fake token,
/// then [`interrupt_command`].
fn interrupt_run(
    b: &Bench,
    ready_path: &Path,
    extra: &[&str],
    signal: rustix::process::Signal,
    before_signal: impl FnOnce(&Value),
) -> Stopped {
    let mut args = vec!["--arms", "just-solve"];
    args.extend_from_slice(extra);
    let mut cmd = b.run_command(&b.out, &args);
    apply_env(&mut cmd, WITH_TOKEN);
    interrupt_command(b, cmd, ready_path, signal, before_signal)
}

/// Spawns `cmd` ([`Bench::spawn_logged`]), waits for the fake at
/// `ready_path`, runs `before_signal`, sends `signal` to the runner only,
/// and waits (30 s) for it to exit. Asserts the fake and any grandchild are
/// gone within 3 s.
fn interrupt_command(
    b: &Bench,
    cmd: Command,
    ready_path: &Path,
    signal: rustix::process::Signal,
    before_signal: impl FnOnce(&Value),
) -> Stopped {
    use std::time::{Duration, Instant};
    let (mut runner, _out_log, err_log) = b.spawn_logged(cmd);
    let mut reap = common::ReapGuard::default();
    let ready = wait_ready(ready_path, &err_log, runner.child());
    let fake = u32::try_from(ready["pid"].as_u64().expect("fake pid")).expect("pid fits");
    reap.track(fake);
    reap.track_group(fake);
    let grandchild = ready["grandchild"]
        .as_u64()
        .map(|g| u32::try_from(g).expect("pid fits"));
    if let Some(g) = grandchild {
        reap.track(g);
    }
    before_signal(&ready);
    let pid = i32::try_from(runner.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .expect("runner pid");
    rustix::process::kill_process(pid, signal).expect("signal the runner");
    let at = Instant::now();
    let status = common::exit_within(runner.child(), Duration::from_secs(30));
    let since_signal = at.elapsed();
    let stderr = std::fs::read_to_string(&err_log).unwrap_or_default();
    let status = status.unwrap_or_else(|| panic!("the runner did not exit; stderr:\n{stderr}"));
    assert!(
        common::gone_within(fake, Duration::from_secs(3)),
        "the fake slop-code {fake} survived; stderr:\n{stderr}"
    );
    if let Some(g) = grandchild {
        assert!(
            common::gone_within(g, Duration::from_secs(3)),
            "the grandchild {g} survived; stderr:\n{stderr}"
        );
    }
    reap.clear();
    Stopped {
        status,
        stderr,
        ready,
        since_signal,
    }
}

/// The record's `slop-code run --no-evaluate` command. It is not the last
/// command: the docker teardown calls are recorded after it.
fn slop_code_run_command(record: &Value) -> Value {
    record["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .rev()
        .find(|c| {
            let argv = strs(&c["argv"]);
            argv.iter().any(|a| a == "slop-code") && argv.iter().any(|a| a == "--no-evaluate")
        })
        .cloned()
        .expect("a slop-code run command")
}

#[test]
fn sigterm_stops_slop_code_and_its_grandchild() {
    let b = Bench::new(&Spec::default());
    let ready = hanging_run(&b, json!({ "grandchild": true, "hang": true }));
    let s = interrupt_run(
        &b,
        &ready,
        &["--stop-grace-secs", "5"],
        rustix::process::Signal::TERM,
        |_| {},
    );
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    let record = b.record();
    let cmd = slop_code_run_command(&record);
    assert_eq!(cmd["interrupted"], "SIGTERM", "{cmd}");
    assert!(cmd.get("killed").is_none(), "{cmd}");
    let tmpdir = s.ready["tmpdir"].as_str().expect("the fake saw TMPDIR");
    assert_eq!(cmd["tmpdir"], tmpdir);
    let name = Path::new(tmpdir)
        .file_name()
        .expect("scratch name")
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("scb"), "{tmpdir}");
    assert!(!Path::new(tmpdir).exists(), "scratch {tmpdir} left behind");
    assert!(!s.stderr.contains(TOKEN));
    assert_no_token(&b.out);

    let r = json(&ok(&b.report(&b.out, &["--format", "json"])));
    assert_eq!(
        r["skipped_runs"],
        json!([{
            "arm": "just-solve",
            "seed": 1,
            "reason": "not finished: interrupted by SIGTERM during run",
        }])
    );
}

#[test]
fn sigint_is_forwarded_as_sigint() {
    let b = Bench::new(&Spec::default());
    let ready = hanging_run(&b, json!({ "grandchild": true, "hang": true }));
    let s = interrupt_run(
        &b,
        &ready,
        &["--stop-grace-secs", "5"],
        rustix::process::Signal::INT,
        |_| {},
    );
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr
            .contains("arm just-solve seed 1: interrupted by SIGINT during run"),
        "{}",
        s.stderr
    );
    let record = b.record();
    assert_eq!(slop_code_run_command(&record)["interrupted"], "SIGINT");
    assert_eq!(record["arms"][0]["interrupted"]["signal"], "SIGINT");
}

#[test]
fn slop_code_ignoring_sigterm_is_killed_after_the_grace() {
    let b = Bench::new(&Spec::default());
    let ready = hanging_run(&b, json!({ "ignore_sigterm": true, "hang": true }));
    let s = interrupt_run(
        &b,
        &ready,
        &["--stop-grace-secs", "1"],
        rustix::process::Signal::TERM,
        |_| {},
    );
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr.contains(
            "arm just-solve seed 1: interrupted by SIGTERM during run (problems mvvault, xjq)"
        ),
        "{}",
        s.stderr
    );
    assert!(
        s.stderr
            .contains("did not exit within the 1s grace and was killed"),
        "{}",
        s.stderr
    );
    assert!(
        s.since_signal >= std::time::Duration::from_secs(1),
        "exited {:?} after the signal, inside the grace",
        s.since_signal
    );
    let record = b.record();
    assert_eq!(slop_code_run_command(&record)["killed"], true);
    let arm = &record["arms"][0];
    assert_eq!(arm["interrupted"]["signal"], "SIGTERM");
    assert_eq!(arm["interrupted"]["step"], "run");
    assert_eq!(arm["interrupted"]["killed"], true);
    assert!(arm["finished"].is_null(), "{arm}");
    assert_no_token(&b.out);
}

#[test]
fn interrupt_removes_only_this_runs_containers() {
    let b = Bench::new(&Spec::default());
    let ready = hanging_run(&b, json!({ "hang": true }));
    let s = interrupt_run(
        &b,
        &ready,
        &["--stop-grace-secs", "5"],
        rustix::process::Signal::TERM,
        |ready| {
            let t = ready["tmpdir"].as_str().expect("the fake saw TMPDIR");
            let mounts = |source: &str| json!([{ "Type": "bind", "Source": source, "Destination": "/workspace" }]);
            let inspect = [
                format!("ours-agent {}", mounts(&format!("{t}/tmpws"))),
                format!("other-run {}", mounts("/private/tmp/scbOTHER/tmpws")),
                format!("sibling {}", mounts(&format!("{t}x/ws"))),
                "nomounts []".to_owned(),
            ]
            .join("\n");
            // `--format` contains `rm`, so `inspect` is matched first.
            b.scenario(
                "docker",
                &json!({ "default": { "stdout": "99.0.0-fake\n" } }),
                &[
                    (&["inspect"], json!({ "stdout": format!("{inspect}\n") })),
                    (
                        &["ps"],
                        json!({ "stdout": "ours-agent\nother-run\nsibling\nnomounts\n" }),
                    ),
                    (&["--force"], json!({ "exit": 0 })),
                ],
            );
        },
    );
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    let removals: Vec<Vec<String>> = b
        .argvs("docker")
        .into_iter()
        .filter(|a| a.first().is_some_and(|c| c == "rm"))
        .collect();
    assert_eq!(removals, [["rm", "--force", "ours-agent"]]);
    let inspected = b
        .argvs("docker")
        .into_iter()
        .find(|a| a.first().is_some_and(|c| c == "inspect"))
        .expect("an inspect call");
    for id in ["ours-agent", "other-run", "sibling", "nomounts"] {
        assert!(inspected.iter().any(|a| a == id), "{inspected:?}");
    }
    assert_eq!(
        b.record()["arms"][0]["interrupted"]["removed_containers"],
        json!(["ours-agent"])
    );
    assert!(
        s.stderr.contains("removed 1 container(s) this run started"),
        "{}",
        s.stderr
    );
}

#[test]
fn interrupt_with_failing_docker_still_scrubs_and_names_the_unit() {
    let b = Bench::new(&Spec::default());
    let ready = hanging_run(&b, json!({ "hang": true }));
    b.scenario(
        "docker",
        &json!({ "default": { "stdout": "99.0.0-fake\n" } }),
        &[(&["ps"], json!({ "stderr": "cannot connect\n", "exit": 1 }))],
    );
    let s = interrupt_run(
        &b,
        &ready,
        &["--stop-grace-secs", "5"],
        rustix::process::Signal::TERM,
        |_| {},
    );
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr
            .contains("scbench run: could not list containers (exited 1)"),
        "{}",
        s.stderr
    );
    assert!(
        s.stderr
            .contains("arm just-solve seed 1: interrupted by SIGTERM during run"),
        "{}",
        s.stderr
    );
    assert!(!s.stderr.contains(TOKEN));
    assert_no_token(&b.out);
    assert_eq!(
        b.record()["arms"][0]["interrupted"]["removed_containers"],
        json!([])
    );
}

/// A bind mount of `source`, as `docker inspect --format '{{json .Mounts}}'`
/// prints it, after `id`.
fn mounted(id: &str, source: &str) -> String {
    format!(
        "{id} {}",
        json!([{ "Type": "bind", "Source": source, "Destination": "/workspace" }])
    )
}

/// Hangs the fake on the unit's run, then, once it is ready, has the fake
/// `docker` answer with `responses(<the scratch TMPDIR>)` (tried in order)
/// and stops the run with SIGTERM.
fn interrupt_with_docker(
    b: &Bench,
    responses: impl FnOnce(&str) -> Vec<(&'static [&'static str], Value)>,
) -> Stopped {
    let ready = hanging_run(b, json!({ "hang": true }));
    interrupt_run(
        b,
        &ready,
        &["--stop-grace-secs", "5"],
        rustix::process::Signal::TERM,
        |ready| {
            let t = ready["tmpdir"].as_str().expect("the fake saw TMPDIR");
            b.scenario(
                "docker",
                &json!({ "default": { "stdout": "99.0.0-fake\n" } }),
                &responses(t),
            );
        },
    )
}

/// The `docker rm` calls the run made.
fn removals(b: &Bench) -> Vec<Vec<String>> {
    b.argvs("docker")
        .into_iter()
        .filter(|a| a.first().is_some_and(|c| c == "rm"))
        .collect()
}

#[test]
fn sighup_is_forwarded_as_sighup() {
    let b = Bench::new(&Spec::default());
    let ready = hanging_run(&b, json!({ "grandchild": true, "hang": true }));
    let s = interrupt_run(
        &b,
        &ready,
        &["--stop-grace-secs", "5"],
        rustix::process::Signal::HUP,
        |_| {},
    );
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    let record = b.record();
    assert_eq!(slop_code_run_command(&record)["interrupted"], "SIGHUP");
    assert_eq!(record["arms"][0]["interrupted"]["signal"], "SIGHUP");
}

#[test]
fn sigterm_during_eval_tears_down_the_eval_scratch() {
    let b = Bench::new(&Spec::default());
    let ready = hanging_on(&b, "--pass-policy", json!({ "hang": true }));
    let s = interrupt_run(
        &b,
        &ready,
        &["--stop-grace-secs", "5"],
        rustix::process::Signal::TERM,
        |ready| {
            let t = ready["tmpdir"].as_str().expect("the fake saw TMPDIR");
            let inspect = format!("{}\n", mounted("eval-box", &format!("{t}/ws")));
            b.scenario(
                "docker",
                &json!({ "default": { "stdout": "99.0.0-fake\n" } }),
                &[
                    (&["inspect"], json!({ "stdout": inspect })),
                    (&["ps"], json!({ "stdout": "eval-box\n" })),
                    (&["--force"], json!({ "exit": 0 })),
                ],
            );
        },
    );
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr
            .contains("arm just-solve seed 1: interrupted by SIGTERM during eval"),
        "{}",
        s.stderr
    );
    let eval_tmpdir = s.ready["tmpdir"].as_str().expect("the fake saw TMPDIR");
    let record = b.record();
    let commands = record["commands"].as_array().expect("commands");
    let slop = |flag: &str| {
        commands
            .iter()
            .find(|c| strs(&c["argv"]).iter().any(|a| a == flag))
            .cloned()
            .expect("a slop-code command")
    };
    let eval = slop("--pass-policy");
    assert_eq!(eval["interrupted"], "SIGTERM", "{eval}");
    assert_eq!(eval["tmpdir"], eval_tmpdir);
    let run = slop("--no-evaluate");
    assert!(run.get("interrupted").is_none(), "{run}");
    assert_ne!(
        run["tmpdir"], eval_tmpdir,
        "each spawn gets its own scratch"
    );
    let arm = &record["arms"][0];
    assert_eq!(arm["interrupted"]["step"], "eval");
    assert_eq!(
        arm["interrupted"]["removed_containers"],
        json!(["eval-box"])
    );
    assert_eq!(removals(&b), [["rm", "--force", "eval-box"]]);
    assert!(!Path::new(eval_tmpdir).exists());
    assert!(!s.stderr.contains(TOKEN));
    assert_no_token(&b.out);
    let r = json(&ok(&b.report(&b.out, &["--format", "json"])));
    assert_eq!(
        r["skipped_runs"][0]["reason"],
        "not finished: interrupted by SIGTERM during eval"
    );
}

#[test]
fn sigterm_during_lane_collection_reports_the_interrupt_not_the_tar_failure() {
    use rdm_core::model::PhaseStatus::Done;
    let b = Bench::new(&full_lane_spec());
    let tars = b.root.join("tars");
    let mut mv = PlanRepo::new(&b.home(), &b.root.join("fixture/mvvault"));
    mv.add(1, Done, None);
    let leak = format!("token={TOKEN}\n");
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "M1", &[("leak.txt", &leak)]);
    full_lane_bench(
        &b,
        &[Checkpoint {
            problem: "mvvault",
            name: "checkpoint_1",
            order: 1,
            tars: vec![("000001.tar", mv1)],
        }],
    );
    // A fake `tar`, first on the runner's PATH, that is still extracting when
    // the signal arrives and then fails.
    let tar_bin = b.root.join("tar-bin");
    std::fs::create_dir_all(&tar_bin).expect("tar bin");
    std::os::unix::fs::symlink(common::fixture(), tar_bin.join("tar")).expect("symlink");
    let ready = b.root.join("tar-ready.json");
    write(
        &tar_bin.join("tar.scenario.json"),
        &json!({
            "record": b.record_path("tar"),
            "default": { "ready": ready, "sleepMs": 2000, "stderr": "tar: boom\n", "exit": 2 },
        })
        .to_string(),
    );
    let path = format!(
        "{}:{}",
        tar_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = b.run_command(&b.out, &["--arms", "full-lane"]);
    apply_env(&mut cmd, WITH_TOKEN);
    cmd.env("PATH", path);
    let s = interrupt_command(&b, cmd, &ready, rustix::process::Signal::TERM, |_| {});
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr.contains(
            "arm full-lane seed 1: interrupted by SIGTERM during lane (problems mvvault, xjq); \
             no slop-code command was running"
        ),
        "{}",
        s.stderr
    );
    assert!(!s.stderr.contains("extracting"), "{}", s.stderr);
    let record = b.record();
    assert_eq!(
        arm_record(&record, "full-lane")["interrupted"]["step"],
        "lane"
    );
    assert!(
        !b.argvs("uv").iter().any(|a| a.iter().any(|x| x == "eval")),
        "eval ran after the interrupt"
    );
    assert!(!s.stderr.contains(TOKEN));
    assert_no_token(&b.out);
}

#[test]
fn a_signal_during_a_probe_spawns_nothing_more() {
    let b = Bench::new(&Spec::default());
    let ready = slow_uv_probe(&b);
    let s = interrupt_run(&b, &ready, &[], rustix::process::Signal::TERM, |_| {});
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr.contains(&format!(
            "interrupted by SIGTERM before `{} version --format",
            p(&b.bin("docker"))
        )),
        "{}",
        s.stderr
    );
    assert!(
        b.calls("docker").is_none(),
        "docker was spawned after the signal"
    );
    assert!(!spawned_slop_code(&b));
    assert!(b.record()["arms"][0].get("interrupted").is_none());
}

#[test]
fn ladder_signal_during_the_preflight_stops_before_any_run() {
    let b = Bench::new(&Spec::default());
    let ready = slow_uv_probe(&b);
    let cmd = b.ladder_command(&b.out, &["--seeds", "1"], &[]);
    let s = interrupt_command(&b, cmd, &ready, rustix::process::Signal::TERM, |_| {});
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr.contains(
            "ladder: interrupted by SIGTERM during the preflight; no slop-code command ran"
        ),
        "{}",
        s.stderr
    );
    assert!(!spawned_slop_code(&b));
    assert!(!b.out.exists(), "--out was created");
}

#[test]
fn interrupt_still_removes_ours_when_inspect_misses_a_vanished_container() {
    let b = Bench::new(&Spec::default());
    let s = interrupt_with_docker(&b, |t| {
        vec![
            (
                &["inspect"],
                json!({
                    "stdout": format!("{}\n", mounted("ours", &format!("{t}/ws"))),
                    "stderr": "Error: No such object: gone\n",
                    "exit": 1,
                }),
            ),
            (&["ps"], json!({ "stdout": "gone\nours\n" })),
            (&["--force"], json!({ "exit": 0 })),
        ]
    });
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr.contains(
            "could not inspect all the listed containers (exited 1; the ones it printed are \
             still attributed)"
        ),
        "{}",
        s.stderr
    );
    assert_eq!(removals(&b), [["rm", "--force", "ours"]]);
    assert_eq!(
        b.record()["arms"][0]["interrupted"]["removed_containers"],
        json!(["ours"])
    );
}

#[test]
fn interrupt_with_failing_inspect_removes_nothing() {
    let b = Bench::new(&Spec::default());
    let s = interrupt_with_docker(&b, |_| {
        vec![
            (
                &["inspect"],
                json!({ "stderr": "daemon gone\n", "exit": 1 }),
            ),
            (&["ps"], json!({ "stdout": "ours\n" })),
        ]
    });
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr
            .contains("scbench run: could not inspect containers (exited 1)"),
        "{}",
        s.stderr
    );
    assert!(removals(&b).is_empty());
    assert_eq!(
        b.record()["arms"][0]["interrupted"]["removed_containers"],
        json!([])
    );
}

#[test]
fn malformed_inspect_line_is_skipped_and_the_rest_removed() {
    let b = Bench::new(&Spec::default());
    let s = interrupt_with_docker(&b, |t| {
        vec![
            (
                &["inspect"],
                json!({
                    "stdout": format!("garbage-line\n{}\n", mounted("ours", &format!("{t}/ws"))),
                }),
            ),
            (&["ps"], json!({ "stdout": "other\nours\n" })),
            (&["--force"], json!({ "exit": 0 })),
        ]
    });
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr.contains(
            "could not inspect containers (line(s) 1 of the output were not `<id> <mounts JSON>`"
        ),
        "{}",
        s.stderr
    );
    assert!(!s.stderr.contains("garbage-line"), "{}", s.stderr);
    assert_eq!(removals(&b), [["rm", "--force", "ours"]]);
}

#[test]
fn failed_rm_records_no_removed_container() {
    let b = Bench::new(&Spec::default());
    let s = interrupt_with_docker(&b, |t| {
        vec![
            (
                &["inspect"],
                json!({ "stdout": format!("{}\n", mounted("ours", &format!("{t}/ws"))) }),
            ),
            (&["ps"], json!({ "stdout": "ours\n" })),
            (
                &["--force"],
                json!({ "stderr": "Error: boom\n", "exit": 1 }),
            ),
        ]
    });
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert!(
        s.stderr
            .contains("scbench run: could not remove containers (ours exited 1)"),
        "{}",
        s.stderr
    );
    assert!(
        s.stderr.contains("removed 0 container(s) this run started"),
        "{}",
        s.stderr
    );
    assert_eq!(
        b.record()["arms"][0]["interrupted"]["removed_containers"],
        json!([])
    );
}

#[test]
fn one_failed_rm_does_not_hide_the_other_removals() {
    let b = Bench::new(&Spec::default());
    let s = interrupt_with_docker(&b, |t| {
        let inspect = format!(
            "{}\n{}\n",
            mounted("box-a", &format!("{t}/ws")),
            mounted("box-b", &format!("{t}/home"))
        );
        vec![
            (&["inspect"], json!({ "stdout": inspect })),
            (&["ps"], json!({ "stdout": "box-a\nbox-b\n" })),
            (
                &["--force", "box-a"],
                json!({ "stderr": "Error: No such container\n", "exit": 1 }),
            ),
            (&["--force"], json!({ "exit": 0 })),
        ]
    });
    assert_eq!(s.status.code(), Some(1), "stderr:\n{}", s.stderr);
    assert_eq!(
        removals(&b),
        [["rm", "--force", "box-a"], ["rm", "--force", "box-b"]]
    );
    assert!(
        s.stderr
            .contains("could not remove containers (box-a exited 1)"),
        "{}",
        s.stderr
    );
    assert_eq!(
        b.record()["arms"][0]["interrupted"]["removed_containers"],
        json!(["box-b"])
    );
}
