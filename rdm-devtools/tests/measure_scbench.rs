//! `rdm-measure scbench run` against fake `uv`/`docker`/`claude` executables
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
"#,
            scbench_repo = root.join("src/slop-code-bench").display(),
            problems_repo = root.join("src/scb-problems").display(),
            provider = spec.provider,
            model = spec.model,
            thinking = spec.thinking,
            version = spec.claude_code_version,
            pass_policy = spec.pass_policy,
            step_limit = spec.step_limit,
        );
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

#[test]
fn committed_manifest_plans_both_control_arms() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache = tmp.path().join("cache");
    let out = tmp.path().join("out");
    let o = measure_support::run(&[
        "scbench",
        "run",
        "--dry-run",
        "--cache-dir",
        &p(&cache),
        "--out",
        &p(&out),
    ]);
    let plan = json(&ok(&o));
    let names: Vec<_> = plan["plan"]["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .map(|a| a["name"].as_str().expect("name").to_owned())
        .collect();
    assert_eq!(names, ["just-solve", "anti_slop"]);
    assert!(!cache.exists() && !out.exists());
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
    assert_eq!(argvs.iter().filter(|a| a.contains(" fetch ")).count(), 2);
    assert_eq!(argvs.iter().filter(|a| a.contains(" checkout ")).count(), 2);
}
