#[path = "support/codex_bridge.rs"]
mod codex_bridge;
#[path = "support/command.rs"]
mod command;

use serde_json::{Value, json};
use std::process::Command;

struct Fixture {
    root: tempfile::TempDir,
    phases: Value,
    calls: Vec<Value>,
    journal: Vec<String>,
    fault: &'static str,
}

impl Fixture {
    fn new(fault: &'static str) -> Self {
        let mut f = Self {
            root: tempfile::tempdir().unwrap(),
            phases: json!({"phase-1-a": {"stem":"phase-1-a","body":"Preserve `$body`.\n", "tags":["keep"], "difficulty":null,"model":null,"estimate_snapshot":"initial"},
                "phase-2-b": {"stem":"phase-2-b","body":"Second body.\n", "tags":[], "difficulty":null,"model":null,"estimate_snapshot":"initial"}}),
            calls: vec![],
            journal: vec![],
            fault,
        };
        f.git(&["init", "-q"]);
        f.git(&["config", "user.name", "Fixture"]);
        f.git(&["config", "user.email", "fixture@example.invalid"]);
        if fault == "missing_snapshot" {
            for p in f.phases.as_object_mut().unwrap().values_mut() {
                p.as_object_mut().unwrap().remove("estimate_snapshot");
            }
        }
        f.persist();
        f
    }
    fn git(&self, args: &[&str]) -> String {
        let output = command::bounded_output(
            Command::new("git")
                .args(args)
                .current_dir(self.root.path())
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", self.root.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1"),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    fn persist(&self) {
        std::fs::write(
            self.root.path().join("phases.json"),
            self.phases.to_string(),
        )
        .unwrap();
        self.git(&["add", "phases.json"]);
        self.git(&["commit", "-qm", "test: persist"]);
    }
    fn run(&mut self, apply: bool) -> Result<Value, String> {
        let args = json!([{"roadmap":"example","apply":apply,"agent":{"$callback":"agent"},
            "ctx":{"identity":{"planRoot":self.root.path(),"project":"fixture","rdmBin":"/fixture path/rdm"},
                "session":"owned","rdm":{"$callback":"rdm"},"record":{"$callback":"record"}}}]);
        codex_bridge::invoke(
            "scripts/lib/codex-runtime-estimate.mjs",
            "runEstimate",
            args,
            |name, args| self.call(name, args),
        )
    }
    fn call(&mut self, name: &str, args: Value) -> Result<Value, String> {
        if name == "record" {
            return Ok(Value::Null);
        }
        if name == "agent" {
            let prompt = args[0].as_str().unwrap();
            let stem = prompt
                .split("Phase stem: ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap();
            if self.fault == "agent_failure" {
                return Err("agent failed".into());
            }
            if self.fault == "drift" {
                self.phases["phase-1-a"]["tags"] = json!(["concurrent"]);
            }
            return Ok(
                json!({"stem":if self.fault=="wrong_target" {"wrong"} else {stem},
                "difficulty":"moderate","justification":if self.fault=="invalid_batch" {"two\nlines"} else {"Bounded change."}}),
            );
        }
        assert_eq!(name, "rdm");
        let a: Vec<String> = serde_json::from_value(args[0].clone()).unwrap();
        self.calls.push(args.clone());
        let flag = |key: &str| a.iter().position(|x| x == key).map(|i| a[i + 1].clone());
        match (a[0].as_str(), a.get(1).map(String::as_str)) {
            ("commit", _) => {
                if self.fault == "skip_commit" {
                    return Ok(json!("skipped"));
                }
                self.persist();
                self.journal.clear();
                if self.fault == "unsettled" {
                    self.journal.push("unsettled".into());
                }
                if self.fault == "interrupt_commit" {
                    return Err("interrupted".into());
                }
                Ok(json!("committed"))
            }
            ("session", Some("journal")) => Ok(json!({"id":"owned","paths":self.journal})),
            ("phase", Some("list")) => Ok(Value::Array(
                self.phases.as_object().unwrap().values().cloned().collect(),
            )),
            ("phase", Some("show")) => {
                if let Some(at) = flag("--at") {
                    let committed: Value =
                        serde_json::from_str(&self.git(&["show", &format!("{at}:phases.json")]))
                            .unwrap();
                    return Ok(committed[&a[2]].clone());
                }
                Ok(self.phases[&a[2]].clone())
            }
            ("phase", Some("update")) => {
                let p = &mut self.phases[&a[2]];
                if self.fault == "atomic_race" {
                    p["body"] = json!("Concurrent writer wins");
                    p["estimate_snapshot"] = json!("changed");
                }
                assert!(flag("--expected-estimate-snapshot").is_some());
                if flag("--expected-estimate-snapshot").as_deref()
                    != p["estimate_snapshot"].as_str()
                {
                    return Err("estimate snapshot changed".into());
                }
                assert!(flag("--body").is_none(), "estimate must never write a body");
                assert!(flag("--model").is_none());
                p["difficulty"] = json!(flag("--difficulty").unwrap());
                p["model"] = json!("sonnet");
                self.journal.push(a[2].clone());
                if self.fault == "readback" {
                    p["body"] = json!("Concurrent edit after write");
                }
                if self.fault == "interrupt_update" {
                    return Err("interrupted".into());
                }
                Ok(Value::Null)
            }
            _ => panic!("unexpected command {a:?}"),
        }
    }
    fn writes(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| c[1]["mutating"] == true)
            .count()
    }
}

#[test]
fn estimate_preview_validates_proposals_without_writes() {
    let mut f = Fixture::new("");
    let result = f.run(false).unwrap();
    assert_eq!(result["proposed"].as_array().unwrap().len(), 2);
    assert_eq!(f.writes(), 0);
}
#[test]
fn estimate_apply_preserves_body_tags_and_is_idempotent() {
    let mut f = Fixture::new("");
    let before = f.phases.clone();
    let result = f.run(true).unwrap();
    assert_eq!(result["estimated"].as_array().unwrap().len(), 2);
    assert_eq!(result["estimated"][0]["tier"], "sonnet");
    assert!(
        result["estimated"][0]["writebackScript"]
            .as_str()
            .unwrap()
            .trim_start()
            .starts_with("'/fixture path/rdm' phase update ")
    );
    for stem in ["phase-1-a", "phase-2-b"] {
        assert_eq!(f.phases[stem]["body"], before[stem]["body"]);
        assert_eq!(f.phases[stem]["tags"], before[stem]["tags"]);
    }
    let writes = f.writes();
    assert_eq!(f.run(true).unwrap()["estimated"], json!([]));
    assert_eq!(f.writes(), writes);
    assert!(
        f.calls
            .iter()
            .all(|c| !c[0].as_array().unwrap().contains(&json!("--all")))
    );
}
fn rejects_before_write(fault: &'static str, message: &str) {
    let mut f = Fixture::new(fault);
    assert!(f.run(true).unwrap_err().contains(message));
    assert_eq!(f.writes(), 0);
}
#[test]
fn estimate_invalid_batch_is_rejected() {
    rejects_before_write("invalid_batch", "invalid estimate");
}
#[test]
fn estimate_wrong_target_is_rejected() {
    rejects_before_write("wrong_target", "invalid estimate");
}
#[test]
fn estimate_agent_failure_rejects_batch() {
    rejects_before_write("agent_failure", "agent failed");
}
#[test]
fn estimate_drift_is_detected_before_writes() {
    rejects_before_write("drift", "changed");
}
#[test]
fn estimate_requires_conditional_snapshot() {
    rejects_before_write("missing_snapshot", "snapshot missing");
}
fn uncertain(fault: &'static str, message: &str) -> Fixture {
    let mut f = Fixture::new(fault);
    let error = f.run(true).unwrap_err();
    assert!(
        error.contains("uncertain") && error.contains(message),
        "{error}"
    );
    f
}
#[test]
fn estimate_readback_failure_stops_writes() {
    assert_eq!(uncertain("readback", "readback").writes(), 1);
}
#[test]
fn estimate_atomic_precondition_preserves_concurrent_edit() {
    let f = uncertain("atomic_race", "snapshot changed");
    assert_eq!(f.phases["phase-1-a"]["body"], "Concurrent writer wins");
    assert!(f.phases["phase-1-a"]["difficulty"].is_null());
    assert_eq!(f.writes(), 1);
}
#[test]
fn estimate_skipped_commit_is_not_success() {
    uncertain("skip_commit", "committed estimate");
}
#[test]
fn estimate_owned_journal_must_settle() {
    uncertain("unsettled", "journal");
}
#[test]
fn estimate_interrupted_update_is_not_retried() {
    assert_eq!(uncertain("interrupt_update", "interrupted").writes(), 1);
}
#[test]
fn estimate_interrupted_commit_is_not_retried() {
    assert_eq!(uncertain("interrupt_commit", "interrupted").writes(), 3);
}

struct RealRepo {
    root: tempfile::TempDir,
}
impl RealRepo {
    fn command(
        &self,
        bin: &std::ffi::OsStr,
        args: &[&str],
        session: &str,
    ) -> Result<String, String> {
        let out = command::bounded_output(
            Command::new(bin)
                .args(args)
                .current_dir(self.root.path())
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", self.root.path())
                .env("XDG_CONFIG_HOME", self.root.path())
                .env("RDM_ROOT", self.root.path())
                .env("RDM_PROJECT", "fixture")
                .env("RDM_SESSION", session)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Fixture")
                .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
                .env("GIT_COMMITTER_NAME", "Fixture")
                .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
        )
        .unwrap();
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(String::from_utf8(out.stdout).unwrap())
    }
    fn rdm(&self, args: &[&str], session: &str) -> String {
        self.command(
            assert_cmd::cargo::cargo_bin!("rdm").as_os_str(),
            args,
            session,
        )
        .unwrap()
    }
    fn run(&self, apply: bool) -> Value {
        let args = json!([{"roadmap":"example","apply":apply,"agent":{"$callback":"agent"},"ctx":{
            "identity":{"planRoot":self.root.path().canonicalize().unwrap(),"project":"fixture","rdmBin":assert_cmd::cargo::cargo_bin!("rdm")},
            "session":"estimate","rdm":{"$callback":"rdm"},"record":{"$callback":"record"}}}]);
        codex_bridge::invoke("scripts/lib/codex-runtime-estimate.mjs", "runEstimate", args, |name, args| {
            match name {
                "agent" => Ok(json!({"stem":"phase-1-target","difficulty":"moderate","justification":"Bounded change."})),
                "record" => Ok(Value::Null),
                "rdm" => {
                    let argv: Vec<&str> = args[0].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
                    let out = self.command(assert_cmd::cargo::cargo_bin!("rdm").as_os_str(), &argv, "estimate")?;
                    Ok(if args[1]["json"] == true { serde_json::from_str(&out).unwrap() } else { json!(out) })
                }
                _ => panic!("unexpected callback"),
            }
        }).unwrap()
    }
    fn head(&self) -> String {
        self.command(std::ffi::OsStr::new("git"), &["rev-parse", "HEAD"], "seed")
            .unwrap()
    }
}

#[test]
fn estimate_real_repository_preview_scoped_apply_and_repeat_noop() {
    let repo = RealRepo {
        root: tempfile::tempdir().unwrap(),
    };
    repo.rdm(&["init", "--default-project", "fixture"], "seed");
    repo.rdm(
        &[
            "roadmap",
            "create",
            "example",
            "--title",
            "Example",
            "--body",
            "Fixture",
            "--no-edit",
        ],
        "seed",
    );
    for (number, slug) in [("1", "target"), ("2", "other")] {
        repo.rdm(
            &[
                "phase",
                "create",
                slug,
                "--number",
                number,
                "--title",
                slug,
                "--body",
                "Preserve body.\n",
                "--roadmap",
                "example",
                "--no-edit",
            ],
            "seed",
        );
    }
    repo.rdm(
        &[
            "phase",
            "update",
            "phase-1-target",
            "--tags",
            "keep",
            "--roadmap",
            "example",
            "--no-edit",
        ],
        "seed",
    );
    repo.rdm(
        &[
            "phase",
            "update",
            "phase-2-other",
            "--difficulty",
            "hard",
            "--roadmap",
            "example",
            "--no-edit",
        ],
        "seed",
    );
    repo.rdm(&["commit", "-m", "test: seed"], "seed");
    repo.rdm(
        &[
            "phase",
            "update",
            "phase-2-other",
            "--body",
            "Unrelated pending change.",
            "--roadmap",
            "example",
            "--no-edit",
        ],
        "unrelated",
    );
    let seed = repo.head();
    assert_eq!(repo.run(false)["proposed"].as_array().unwrap().len(), 1);
    assert_eq!(repo.head(), seed);
    let before: Value = serde_json::from_str(&repo.rdm(
        &[
            "phase",
            "show",
            "phase-1-target",
            "--roadmap",
            "example",
            "--format",
            "json",
        ],
        "estimate",
    ))
    .unwrap();
    assert_eq!(repo.run(true)["estimated"].as_array().unwrap().len(), 1);
    let after: Value = serde_json::from_str(&repo.rdm(
        &[
            "phase",
            "show",
            "phase-1-target",
            "--roadmap",
            "example",
            "--format",
            "json",
        ],
        "estimate",
    ))
    .unwrap();
    assert_eq!(after["body"], before["body"]);
    assert_eq!(after["tags"], before["tags"]);
    assert_eq!(after["difficulty"], "moderate");
    let committed = repo
        .command(
            std::ffi::OsStr::new("git"),
            &[
                "show",
                "HEAD:projects/fixture/roadmaps/example/phase-2-other.md",
            ],
            "seed",
        )
        .unwrap();
    assert!(committed.contains("Preserve body."));
    assert!(!committed.contains("Unrelated pending"));
    assert!(
        repo.rdm(
            &["phase", "show", "phase-2-other", "--roadmap", "example"],
            "unrelated"
        )
        .contains("Unrelated pending change.")
    );
    let head = repo.head();
    assert_ne!(head, seed);
    assert_eq!(repo.run(true)["estimated"], json!([]));
    assert_eq!(repo.head(), head);
}

#[test]
fn estimate_spike_current_contract_and_body_preservation() {
    let root = tempfile::tempdir().unwrap();
    let result = codex_bridge::invoke(
        "scripts/lib/codex-spike-estimate.mjs",
        "runEstimateExperiment",
        json!([{
            "rdmBin":assert_cmd::cargo::cargo_bin!("rdm"), "sourceDir":codex_bridge::repository(),
            "evidenceDir":root.path(), "agent":{"$callback":"agent"}
        }]),
        |name, args| {
            assert_eq!(name, "agent");
            let stem = args[0]
                .as_str()
                .unwrap()
                .split("Phase stem: ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap();
            Ok(json!({"stem":stem,"difficulty":"easy","justification":"Bounded change."}))
        },
    )
    .unwrap();
    assert_eq!(result["first"]["estimated"].as_array().unwrap().len(), 2);
    assert_eq!(result["first"]["skipped"], json!(["phase-2-b"]));
    assert_eq!(
        result["counters"]["first"],
        json!({"agentCalls":2,"writes":2})
    );
    assert!(
        result["first"]["estimated"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["tier"] == "small")
    );
    assert_eq!(result["second"]["estimated"], json!([]));
    assert_eq!(
        result["counters"]["second"],
        json!({"agentCalls":0,"writes":0})
    );
    for (before, after) in result["before"]
        .as_array()
        .unwrap()
        .iter()
        .zip(result["after"].as_array().unwrap())
    {
        assert_eq!(before["body"], after["body"]);
        assert_eq!(before["tags"], after["tags"]);
    }
}

fn spike_rejects_malformed_rating(fault: &str) {
    let root = tempfile::tempdir().unwrap();
    let result = codex_bridge::invoke_request(json!({
        "module":codex_bridge::repository().join("scripts/lib/codex-spike-estimate.mjs"),
        "export":"runEstimateExperiment", "captureError":true, "args":[{
            "rdmBin":assert_cmd::cargo::cargo_bin!("rdm"), "sourceDir":codex_bridge::repository(),
            "evidenceDir":root.path(), "agent":{"$callback":"agent"}
        }]
    }), |name, args| {
        assert_eq!(name, "agent");
        let stem = args[0].as_str().unwrap().split("Phase stem: ").nth(1).unwrap().split_whitespace().next().unwrap();
        let mut rating = json!({"stem":stem,"difficulty":"moderate","justification":"Bounded fixture; literal `code` and $text."});
        if stem == "phase-3-c" {
            match fault {
                "missing_target" => { rating.as_object_mut().unwrap().remove("stem"); }
                "unknown_target" => rating["stem"] = json!("phase-999-foreign"),
                "wrong_target" => rating["stem"] = json!("phase-2-b"),
                "missing_result" => rating = Value::Null,
                "difficulty" => rating["difficulty"] = json!("extreme"),
                "empty" => rating["justification"] = json!("  "),
                "multiline" => rating["justification"] = json!("first\nsecond"),
                _ => panic!("unknown fault"),
            }
        }
        Ok(rating)
    }).unwrap();
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid estimate")
    );
    let identity = &result["error"]["fixtureIdentity"];
    let session = identity["session"].as_str().unwrap();
    let run = |args: &[&str]| {
        let out = command::bounded_output(
            Command::new(assert_cmd::cargo::cargo_bin!("rdm"))
                .args(args)
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", root.path())
                .env("XDG_CONFIG_HOME", root.path())
                .env("RDM_ROOT", identity["root"].as_str().unwrap())
                .env("RDM_SESSION", session)
                .env("RDM_PROJECT", "fixture")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1"),
        )
        .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let show = |stem: &str| -> Value {
        serde_json::from_str(&run(&[
            "phase",
            "show",
            stem,
            "--roadmap",
            "estimate-fixture",
            "--format",
            "json",
        ]))
        .unwrap()
    };
    for stem in ["phase-1-a", "phase-3-c"] {
        let phase = show(stem);
        assert!(phase["difficulty"].is_null());
        assert!(phase["model"].is_null());
        assert!(!phase["body"].as_str().unwrap().contains("## Estimate"));
    }
    if fault == "missing_target" {
        run(&[
            "phase",
            "update",
            "phase-1-a",
            "--difficulty",
            "moderate",
            "--no-edit",
            "--roadmap",
            "estimate-fixture",
        ]);
        assert_eq!(
            show("phase-1-a")["difficulty"],
            "moderate",
            "overlay-aware audit detects an uncommitted partial write"
        );
    }
}
#[test]
fn estimate_spike_rejects_missing_target() {
    spike_rejects_malformed_rating("missing_target");
}
#[test]
fn estimate_spike_rejects_unknown_target() {
    spike_rejects_malformed_rating("unknown_target");
}
#[test]
fn estimate_spike_rejects_wrong_known_target() {
    spike_rejects_malformed_rating("wrong_target");
}
#[test]
fn estimate_spike_rejects_missing_result() {
    spike_rejects_malformed_rating("missing_result");
}
#[test]
fn estimate_spike_rejects_invalid_difficulty() {
    spike_rejects_malformed_rating("difficulty");
}
#[test]
fn estimate_spike_rejects_empty_justification() {
    spike_rejects_malformed_rating("empty");
}
#[test]
fn estimate_spike_rejects_multiline_justification() {
    spike_rejects_malformed_rating("multiline");
}
