//! `rdm-measure scbench run` and `scbench image build|check` against fake
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
        cmd.output().expect("rdm-measure runs")
    }

    /// Runs `scbench image <sub>` with the fakes.
    fn image(&self, sub: &str, extra: &[&str]) -> Output {
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
        let run_yaml = out.join(rung.name).join("run.yaml");
        assert_eq!(
            strs(&arm["run"]["args"]),
            [
                "run",
                "slop-code",
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
    let out = &b.out;
    let expected: Vec<Vec<String>> = vec![
        vec!["--version".into()],
        vec![
            "run".into(),
            "slop-code".into(),
            "run".into(),
            "--config".into(),
            p(&out.join("just-solve/run.yaml")),
            "--no-evaluate".into(),
            "--no-live-progress".into(),
        ],
        vec![
            "run".into(),
            "slop-code".into(),
            "eval".into(),
            p(&out.join("just-solve/output")),
            "--pass-policy".into(),
            "all-cases".into(),
            "--no-live-progress".into(),
        ],
        vec![
            "run".into(),
            "slop-code".into(),
            "run".into(),
            "--config".into(),
            p(&out.join("anti_slop/run.yaml")),
            "--no-evaluate".into(),
            "--no-live-progress".into(),
        ],
        vec![
            "run".into(),
            "slop-code".into(),
            "eval".into(),
            p(&out.join("anti_slop/output")),
            "--pass-policy".into(),
            "all-cases".into(),
            "--no-live-progress".into(),
        ],
    ];
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

#[test]
fn failing_step_stops_and_names_arm() {
    let b = Bench::new(&Spec::default());
    b.scenario(
        "uv",
        &json!({ "default": { "exit": 0 } }),
        &[
            (&["--version"], json!({ "stdout": "uv 0.0.0-fake\n" })),
            (&["anti_slop/run.yaml"], json!({ "exit": 3 })),
        ],
    );
    let err = fails(&b.run(&[]));
    assert!(err.contains("anti_slop"), "stderr: {err}");
    let argvs = b.argvs("uv");
    assert_eq!(argvs.len(), 4, "{argvs:?}");
    assert!(argvs[1].iter().any(|a| a.ends_with("just-solve/run.yaml")));
    assert!(argvs[2].iter().any(|a| a.ends_with("just-solve/output")));
    assert!(argvs[3].iter().any(|a| a.ends_with("anti_slop/run.yaml")));
    assert!(
        !argvs
            .iter()
            .any(|a| a.iter().any(|x| x.ends_with("anti_slop/output")))
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
    let run_yaml = arm_dir.join("run.yaml");
    let want: Vec<String> = [
        "run",
        "slop-code",
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
    let run_yaml = arm_dir.join("run.yaml");
    let want: Vec<String> = [
        "run",
        "slop-code",
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
    assert_eq!(
        agent["cost_limits"],
        json!({ "step_limit": 4321, "cost_limit": 12.5, "net_cost_limit": 25.0 })
    );

    let stock = read_json(&b.out.join("just-solve/agent.yaml"));
    assert_eq!(stock["cost_limits"]["step_limit"], 7);
    let stock_run = read_json(&b.out.join("just-solve/run.yaml"));
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
    fn snapshot(&self, tar: &Path, marker: &str) -> PathBuf {
        git(&self.home, &self.dir, &["add", "-A"]);
        git(
            &self.home,
            &self.dir,
            &["commit", "-q", "--allow-empty", "-m", "snapshot"],
        );
        let marker_path = self.dir.join(marker);
        write(&marker_path, "marker\n");
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
/// `<out>/full-lane/output/<problem>/<checkpoint>/checkpoint.yaml` and
/// `.../agent/workspace/projects/-workspace/rdm-plan/<n>.tar`.
fn full_lane_bench(b: &Bench, checkpoints: &[Checkpoint]) {
    let staged = b.root.join("staged");
    let output = b.out.join("full-lane/output");
    let mut place = Vec::new();
    for c in checkpoints {
        let yaml = staged.join(format!("{}-{}.yaml", c.problem, c.name));
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
    b.scenario(
        "uv",
        &json!({ "default": { "exit": 0 } }),
        &[
            (&["--version"], json!({ "stdout": "uv 0.0.0-fake\n" })),
            (&["full-lane/run.yaml"], json!({ "place": place })),
        ],
    );
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
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "MARKER-000001");
    mv.add(2, InProgress, None);
    let mv2 = mv.snapshot(&tars.join("mv-2.tar"), "MARKER-000002");
    mv.set(2, Done, None);
    let mv3 = mv.snapshot(&tars.join("mv-3.tar"), "MARKER-000003");
    let mut xjq = PlanRepo::new(&home, &b.root.join("fixture/xjq"));
    xjq.add(1, Done, None);
    let x1 = xjq.snapshot(&tars.join("x-1.tar"), "MARKER-XJQ-1");
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
    let repos = b.out.join("full-lane/plan-repos");
    let cp2 = repos.join("mvvault/checkpoint_2");
    assert!(cp2.join("MARKER-000003").is_file());
    assert!(!cp2.join("MARKER-000002").exists());
    assert!(cp2.join(".git").is_dir());
    assert!(repos.join("mvvault/checkpoint_1/MARKER-000001").is_file());
    assert!(repos.join("xjq/checkpoint_1/MARKER-XJQ-1").is_file());
    assert!(!has_path_named(
        &b.out.join("full-lane/output"),
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
    let run = at(&|a| a.iter().any(|x| x.ends_with("full-lane/run.yaml")));
    let eval = at(&|a| a.iter().any(|x| x.ends_with("full-lane/output")));
    let tars: Vec<usize> = argvs
        .iter()
        .enumerate()
        .filter(|(_, a)| a[0] == "tar" && a[1] == "-xf")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(tars.len(), 3, "{argvs:?}");
    assert!(tars.iter().all(|&i| run < i && i < eval), "{argvs:?}");
    let output = b.out.join("full-lane/output");
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
    assert!(!b.out.join("just-solve/plan-repos").exists());
}

#[test]
fn full_lane_classifies_each_checkpoint() {
    use rdm_core::model::PhaseStatus::{Blocked, Done, InProgress, Reviewed};
    let b = Bench::new(&full_lane_spec());
    let home = b.home();
    let tars = b.root.join("tars");
    let mut mv = PlanRepo::new(&home, &b.root.join("fixture/mvvault"));
    mv.add(1, Done, None);
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "M1");
    mv.add(2, Blocked, Some("[code] rework budget exhausted"));
    let mv2 = mv.snapshot(&tars.join("mv-2.tar"), "M2");
    mv.add(3, Reviewed, None);
    let mv3 = mv.snapshot(&tars.join("mv-3.tar"), "M3");
    let mv4 = mv.snapshot(&tars.join("mv-4.tar"), "M4");
    let mut xjq = PlanRepo::new(&home, &b.root.join("fixture/xjq"));
    xjq.add(1, InProgress, None);
    let x2 = xjq.snapshot(&tars.join("x-2.tar"), "X2");
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
        stderr(&o).contains("scbench run: full-lane: 1/6 checkpoints landed"),
        "{}",
        stderr(&o)
    );

    let repos = b.out.join("full-lane/plan-repos");
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
        .join("full-lane/output/mvvault/checkpoint_1")
        .join("agent/workspace/projects/-workspace/rdm-plan/000001.tar");
    // The exit code is tar's own (bsdtar 1, GNU tar 2).
    let want = format!("arm full-lane: extracting {} exited ", p(&placed));
    assert!(err.contains(&want), "stderr: {err}");
    assert!(
        b.argvs("uv")
            .iter()
            .any(|a| a.iter().any(|x| x.ends_with("full-lane/run.yaml"))),
        "{:?}",
        b.argvs("uv")
    );
    assert!(
        !b.argvs("uv")
            .iter()
            .any(|a| a.iter().any(|x| x.ends_with("full-lane/output"))),
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
    let mv1 = mv.snapshot(&tars.join("mv-1.tar"), "M1");
    // A phase file rdm-core cannot parse makes the whole roadmap unreadable.
    let corrupt = mv
        .dir
        .join("projects/bench/roadmaps/solution/phase-2-corrupt.md");
    write(&corrupt, "---\nphase: [not a number\n---\n");
    let mv2 = mv.snapshot(&tars.join("mv-2.tar"), "M2");
    std::fs::remove_file(&corrupt).expect("remove corrupt phase");
    // Back to checkpoint 1's phases: with `prev` carried over the unreadable
    // checkpoint, nothing is new.
    let mv3 = mv.snapshot(&tars.join("mv-3.tar"), "M3");
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
    let repos = b.out.join("full-lane/plan-repos/mvvault");
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
