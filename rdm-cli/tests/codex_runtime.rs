#[path = "support/codex_bridge.rs"]
mod bridge;
#[path = "support/command.rs"]
mod command;

use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;

const RUNTIME: &str = "scripts/lib/codex-runtime.mjs";
const SPIKE: &str = "scripts/lib/codex-spike-review.mjs";

fn git(root: &Path, args: &[&str]) -> String {
    let output = command::bounded_output(
        Command::new("git")
            .args(args)
            .current_dir(root)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid"),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    base: String,
    head: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        fs::write(root.join("a.js"), "export const a = 1;\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        fs::write(root.join("a.js"), "export const a = 2;\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "head"]);
        let head = git(&root, &["rev-parse", "HEAD"]);
        Self {
            _dir: dir,
            root,
            base,
            head,
        }
    }
    fn ctx(&self) -> Value {
        json!({"identity":{"sourceDir":self.root,"project":"fixture","rdmBin":env!("CARGO_BIN_EXE_rdm")},"runDir":self.root,"record":{"$builtin":"noop"},"rdm":{"$callback":"rdm"}})
    }
    fn spec(&self) -> Value {
        json!({"base":self.base,"head":self.head,"target":"Acceptance criteria: a is 2."})
    }
    fn review(
        &self,
        spec: Value,
        callback: impl FnMut(&str, Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        bridge::invoke(
            RUNTIME,
            "reviewCode",
            json!([self.ctx(), spec, deps(), {}, "medium"]),
            callback,
        )
    }
}
fn deps() -> Value {
    json!({"agent":{"$callback":"agent"},"parallel":{"$builtin":"parallel"},"pipeline":{"$builtin":"pipeline"},"log":{"$builtin":"noop"}})
}
fn reply(name: &str, args: Value) -> Result<Value, String> {
    assert_eq!(name, "agent");
    if args[1]["label"] == "find:code:ac" {
        Ok(json!({"findings":[],"ac":[{"criterion":"a is 2","status":"PASS","evidence":"a.js:1"}]}))
    } else {
        Ok(json!({"findings":[]}))
    }
}
fn finding(index: usize) -> Value {
    json!({"id":format!("fixture-{index}"),"concern":"correctness","severity":"blocking","confidence":90,"what_fails":"Fixture failure","location":"a.js:1"})
}

#[test]
fn code_review_allows_intentionally_omitted_ac() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["reviewers"] = json!(["correctness"]);
    let result = fixture.review(spec, reply).unwrap();
    assert_eq!(result["coverage"]["complete"], true);
    assert_eq!(result["outcome"], "reviewed");
}
#[test]
fn code_review_selected_ac_requires_evidence() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["reviewers"] = json!(["ac"]);
    let error = fixture
        .review(spec, |_, _| Ok(json!({"findings":[],"ac":[]})))
        .unwrap_err();
    assert!(error.contains("incomplete"), "{error}");
}
#[test]
fn code_review_clean_range_has_ac_coverage() {
    let fixture = Fixture::new();
    let result = fixture.review(fixture.spec(), reply).unwrap();
    assert_eq!(result["outcome"], "reviewed");
    assert_eq!(result["base"], fixture.base);
    assert_eq!(result["head"], fixture.head);
}
#[test]
fn code_review_rejects_wrong_head_before_judgment() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["head"] = json!(fixture.base);
    let error = fixture
        .review(spec, |_, _| panic!("must not invoke judgment"))
        .unwrap_err();
    assert!(error.contains("HEAD"), "{error}");
}
#[test]
fn code_review_rejects_revision_options() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["base"] = json!("--all");
    let error = fixture
        .review(spec, |_, _| panic!("must not invoke judgment"))
        .unwrap_err();
    assert!(error.contains("revision"), "{error}");
}
#[test]
fn code_review_rejects_dirty_source_after_judgment() {
    let fixture = Fixture::new();
    let error = fixture
        .review(fixture.spec(), |name, args| {
            fs::write(fixture.root.join("a.js"), "changed").unwrap();
            reply(name, args)
        })
        .unwrap_err();
    assert!(
        error.contains("clean") || error.contains("changed"),
        "{error}"
    );
}
#[test]
fn code_review_rejects_clean_head_drift() {
    let fixture = Fixture::new();
    let mut moved = false;
    let error = fixture
        .review(fixture.spec(), |name, args| {
            if !moved {
                moved = true;
                git(
                    &fixture.root,
                    &["commit", "--allow-empty", "-qm", "concurrent"],
                );
            }
            reply(name, args)
        })
        .unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert!(git(&fixture.root, &["status", "--porcelain"]).is_empty());
}
#[test]
fn code_review_missing_finder_is_incomplete() {
    let fixture = Fixture::new();
    let error = fixture
        .review(fixture.spec(), |name, args| {
            if args[1]["label"] == "find:code:correctness" {
                Err("finder unavailable".into())
            } else {
                reply(name, args)
            }
        })
        .unwrap_err();
    assert!(error.contains("incomplete"), "{error}");
}
fn refutation_case(overflow: bool) {
    let fixture = Fixture::new();
    let mut refutations = 0;
    let error=fixture.review(fixture.spec(),|name,args| {
        let label=args[1]["label"].as_str().unwrap();
        if label.starts_with("refute:") {refutations+=1;return if overflow {Ok(json!({"refuted":true,"confidence":100}))}else{Err("refuter crashed".into())};}
        if label=="find:code:correctness" {return Ok(json!({"findings":(0..if overflow {6}else{1}).map(finding).collect::<Vec<_>>()}));}
        reply(name,args)
    }).unwrap_err();
    assert!(error.contains("incomplete"), "{error}");
    assert_eq!(refutations, if overflow { 5 } else { 1 });
}
#[test]
fn code_review_refuter_failure_is_incomplete() {
    refutation_case(false);
}
#[test]
fn code_review_refuter_budget_overflow_is_incomplete() {
    refutation_case(true);
}

#[test]
fn plan_review_pins_content_and_detects_drift() {
    let fixture = Fixture::new();
    let plan = fixture.root.join("plan.md");
    fs::write(&plan, "Implement a = 2 and test it.").unwrap();
    let args = json!([fixture.ctx(),{"planFile":plan},deps(),{}]);
    assert_eq!(
        bridge::invoke(RUNTIME, "reviewPlan", args.clone(), reply).unwrap()["coverage"]["complete"],
        true
    );
    let error = bridge::invoke(RUNTIME, "reviewPlan", args, |name, args| {
        fs::write(&plan, "changed").unwrap();
        reply(name, args)
    })
    .unwrap_err();
    assert!(error.contains("changed"), "{error}");
}

fn spike(blocking: bool) {
    let fixture = Fixture::new();
    let plan = fixture.root.join("plan.md");
    fs::write(&plan, "Implement addition and test add(2,3) == 5.").unwrap();
    let mut labels = Vec::new();
    let result=bridge::invoke(SPIKE,"runReviewExperiment",json!([{"agent":{"$callback":"agent"},"target":"a must be 2","model":"fixture-model","planFile":plan}]),|name,args| {
        let label=args[1]["label"].as_str().unwrap().to_owned();labels.push(label.clone());
        if label.starts_with("refute:"){return Ok(json!({"refuted":false,"confidence":95}));}
        if blocking && label=="find:code:correctness" {return Ok(json!({"findings":[finding(0)]}));}
        reply(name,args)
    }).unwrap();
    assert_eq!(
        result["codeOutcome"],
        if blocking { "rework" } else { "reviewed" }
    );
    assert_eq!(result["plan"]["outcome"], "reviewed");
    if blocking {
        let first = labels
            .iter()
            .position(|s| s.starts_with("refute:"))
            .unwrap();
        assert!(labels[..first].iter().any(|s| s == "find:code:tests"));
        assert_eq!(result["code"]["budget"]["graded"], 1);
    }
}
#[test]
fn spike_uses_plan_identifier_and_keeps_finder_refuter_barrier() {
    spike(true);
}
#[test]
fn spike_clean_review_requires_actual_plan_round() {
    spike(false);
}
#[test]
fn spike_missing_ac_cannot_report_acceptance() {
    let error = bridge::invoke(
        SPIKE,
        "runReviewExperiment",
        json!([{"agent":{"$callback":"agent"},"target":"adds","model":"fixture-model"}]),
        |_, args| {
            Ok(if args[1]["label"] == "find:code:ac" {
                Value::Null
            } else {
                json!({"findings":[]})
            })
        },
    )
    .unwrap_err();
    assert!(error.contains("incomplete"), "{error}");
}
#[test]
fn spike_rejects_budget_errors_and_empty_ac() {
    for result in [
        json!({"coverage":{"complete":true},"budget":{"passedThroughBudget":1},"acTable":[{"status":"PASS"}]}),
        json!({"coverage":{"complete":true},"budget":{"refuterErrors":1},"acTable":[{"status":"PASS"}]}),
        json!({"coverage":{"complete":true},"budget":{},"acTable":[]}),
    ] {
        let error = bridge::invoke(
            SPIKE,
            "requireCompleteReview",
            json!([result, true]),
            |_, _| panic!(),
        )
        .unwrap_err();
        assert!(error.contains("incomplete"), "{error}");
    }
}

fn host() -> Value {
    json!({"capabilities":{"gpt-6-astra":["medium","high"]}})
}
/// A core `rdm model resolve --host codex --format json` reply for `step`.
fn codex_profile(step: &Value, tier: &str, model: &str, effort: &str) -> Value {
    json!({"step":step,"host":"codex","tier":tier,"model":model,"effort":effort})
}
#[test]
fn models_take_model_and_effort_from_the_core_codex_profile() {
    let fixture = Fixture::new();
    let mut calls = Vec::new();
    let result = bridge::invoke(
        RUNTIME,
        "resolveModels",
        json!([
            fixture.ctx(),
            host(),
            ["review-find", "review-verify"],
            "small"
        ]),
        |name, args| {
            assert_eq!(name, "rdm");
            calls.push(args[0].clone());
            let effort = if args[0][2] == "review-find" {
                "medium"
            } else {
                "high"
            };
            Ok(codex_profile(&args[0][2], "large", "gpt-6-astra", effort))
        },
    )
    .unwrap();
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(call[3], "--tier");
        assert_eq!(call[4], "small");
        let argv: Vec<&str> = call
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            argv.windows(2).any(|w| w == ["--host", "codex"]),
            "resolution must ask core for the codex host: {argv:?}"
        );
    }
    assert_eq!(result["review-find"]["tier"], "large");
    assert_eq!(result["review-find"]["model"], "gpt-6-astra");
    assert_eq!(result["review-find"]["effort"], "medium");
    assert_eq!(result["review-verify"]["effort"], "high");
}
#[test]
fn models_need_no_host_configuration() {
    let fixture = Fixture::new();
    let result = bridge::invoke(
        RUNTIME,
        "resolveModels",
        json!([fixture.ctx(), null, ["plan"], "frontier"]),
        |_, args| {
            Ok(codex_profile(
                &args[0][2],
                "frontier",
                "gpt-6-astra",
                "xhigh",
            ))
        },
    )
    .unwrap();
    assert_eq!(result["plan"]["tier"], "frontier");
    assert_eq!(result["plan"]["effort"], "xhigh");
}
#[test]
fn models_refuse_host_tiers_and_steps_with_the_core_remedy() {
    let fixture = Fixture::new();
    for (key, value) in [
        (
            "tiers",
            json!({"large":{"model":"gpt-6-astra","effort":"high"}}),
        ),
        (
            "steps",
            json!({"review-find":{"tier":"large","model":"gpt-6-astra","effort":"high"}}),
        ),
    ] {
        let mut configured = host();
        configured[key] = value;
        let error = bridge::invoke(
            RUNTIME,
            "resolveModels",
            json!([fixture.ctx(), configured, ["review-find"]]),
            |_, _| panic!("a refused host config must not reach core resolution"),
        )
        .unwrap_err();
        assert!(error.contains(&format!("host.{key}")), "{error}");
        assert!(error.contains("rdm model resolve --host codex"), "{error}");
        assert!(error.contains("[models.profiles.codex."), "{error}");
    }
}
#[test]
fn models_reject_unusable_core_profiles_and_undeclared_capabilities() {
    let fixture = Fixture::new();
    for (reply, configured) in [
        // A Claude alias can never reach a Codex process.
        (
            json!({"tier":"large","model":"opus","effort":"high"}),
            Value::Null,
        ),
        // `max` is a Claude-only effort the Codex process guard refuses.
        (
            json!({"tier":"large","model":"gpt-6-astra","effort":"max"}),
            Value::Null,
        ),
        // No effort at all.
        (json!({"tier":"large","model":"gpt-6-astra"}), Value::Null),
        // A declared capability list that omits the resolved effort / model.
        (
            json!({"tier":"large","model":"gpt-6-astra","effort":"low"}),
            host(),
        ),
        (
            json!({"tier":"large","model":"gpt-6-sol","effort":"high"}),
            host(),
        ),
    ] {
        let error = bridge::invoke(
            RUNTIME,
            "resolveModels",
            json!([fixture.ctx(), configured, ["review-find"]]),
            |_, _| {
                let mut profile = reply.clone();
                profile["step"] = json!("review-find");
                profile["host"] = json!("codex");
                Ok(profile)
            },
        )
        .unwrap_err();
        assert!(error.contains("Unsupported"), "{error}");
    }
}
#[test]
fn run_codex_requires_an_explicit_effort() {
    let fixture = Fixture::new();
    let error = bridge::invoke(
        "scripts/lib/codex-process.mjs",
        "runCodex",
        json!([{"bin":"/nonexistent-codex","cwd":fixture.root,"prompt":"fixture","schema":{"type":"object","properties":{},"required":[],"additionalProperties":false},"model":"fixture-model"}]),
        |_, _| panic!(),
    )
    .unwrap_err();
    assert!(error.contains("effort"), "{error}");
}
#[test]
fn judgment_rejects_mechanical_and_unknown_roles_without_processes() {
    let fixture = Fixture::new();
    for role in ["mechanical", "unrecognized"] {
        let error=bridge::invoke_request(json!({"module":bridge::repository().join(RUNTIME),"export":"createJudgmentAgent","args":[fixture.ctx(),{},{}],"callArgs":["x",{"agentType":role}]}),|_,_|panic!()).unwrap_err();
        assert!(error.contains("role"), "{error}");
    }
    assert!(!fixture.root.join("call-1").exists());
}
#[test]
fn judgment_preaborted_signal_creates_no_child_evidence() {
    let fixture = Fixture::new();
    let mut context = fixture.ctx();
    context["record"] = json!({"$callback":"record"});
    let error=bridge::invoke_request(json!({"module":bridge::repository().join(RUNTIME),"export":"createJudgmentAgent","args":[context,{"plan":{"model":"gpt-6-astra","effort":"medium"}},{"signal":{"$builtin":"abortedSignal"}}],"callArgs":["Rate fixture",{"agentType":"estimator","label":"estimate:rate:fixture","schema":{"type":"object","additionalProperties":false,"properties":{},"required":[]}}]}),|_,_|panic!("preaborted call must not record agent-started")).unwrap_err();
    assert!(
        error.contains("cancel") || error.contains("abort"),
        "{error}"
    );
    assert!(!fixture.root.join("call-1").exists());
}

fn phase_spec(fixture: &Fixture) -> Value {
    let mut spec = fixture.spec();
    spec["item"] = json!({"type":"phase","roadmap":"example","phase":"phase-3-runtime"});
    spec
}
fn phase_item() -> Value {
    json!({"roadmap":"example","body":"Acceptance criteria: a is 2.","model":"medium","tags":[]})
}
#[test]
fn phase_review_scopes_reads_and_checks_checkout_identity() {
    let fixture = Fixture::new();
    let branch = git(&fixture.root, &["symbolic-ref", "--short", "HEAD"]);
    for identity_case in ["valid", "wrong-branch", "wrong-path"] {
        let valid = identity_case == "valid";
        let mut agent_calls = 0;
        let result=fixture.review(phase_spec(&fixture),|name,args|{
            if name=="rdm" {
                if args[0][0]=="phase" {let argv=args[0].as_array().unwrap();let index=argv.iter().position(|x|x=="--project").unwrap();assert_eq!(argv[index+1],"fixture");return Ok(phase_item());}
                return Ok(json!([{"item":"example","path":if identity_case == "wrong-path" {fixture.root.parent().unwrap()} else {&fixture.root},"branch":if identity_case == "wrong-branch" {"wrong-branch"}else{&branch}}]));
            }
            agent_calls+=1;reply(name,args)
        });
        if valid {
            assert_eq!(result.unwrap()["outcome"], "reviewed");
            assert!(agent_calls > 0);
        } else {
            assert!(result.unwrap_err().contains("worktree identity"));
            assert_eq!(agent_calls, 0);
        }
    }
}
#[test]
fn phase_review_rejects_body_drift() {
    let fixture = Fixture::new();
    let mut item = phase_item();
    let branch = git(&fixture.root, &["symbolic-ref", "--short", "HEAD"]);
    let error = fixture
        .review(phase_spec(&fixture), |name, args| {
            if name == "rdm" {
                return Ok(if args[0][0] == "phase" {
                    item.clone()
                } else {
                    json!([{"item":"example","path":fixture.root,"branch":branch}])
                });
            }
            item["body"] = json!("Changed acceptance criteria");
            reply(name, args)
        })
        .unwrap_err();
    assert!(error.contains("changed"), "{error}");
}
#[test]
fn phase_review_rejects_policy_drift_before_judgment() {
    let fixture = Fixture::new();
    let mut initial = phase_item();
    initial["model"] = json!("small");
    let branch = git(&fixture.root, &["symbolic-ref", "--short", "HEAD"]);
    let error = bridge::invoke(
        RUNTIME,
        "reviewCode",
        json!([
            fixture.ctx(),
            phase_spec(&fixture),
            deps(),
            {},
            "small",
            initial
        ]),
        |name, args| {
            assert_eq!(name, "rdm", "policy drift must reject before judgment");
            Ok(if args[0][0] == "phase" {
                phase_item()
            } else {
                json!([{"item":"example","path":fixture.root,"branch":branch}])
            })
        },
    )
    .unwrap_err();
    assert!(error.contains("changed during model resolution"), "{error}");
}
#[test]
fn plan_review_forwards_selected_reviewers_and_parent_intent() {
    let fixture = Fixture::new();
    let plan = fixture.root.join("plan.md");
    fs::write(&plan, "Implement a = 2.").unwrap();
    let mut labels = Vec::new();
    let result=bridge::invoke(RUNTIME,"reviewPlan",json!([fixture.ctx(),{"planFile":plan,"roadmap":"example","reviewers":["intent-alignment"]},deps(),{}]),|name,args|{
        labels.push(args[1]["label"].as_str().unwrap().to_owned());let prompt=args[0].as_str().unwrap();assert!(prompt.contains("roadmap show example"),"{prompt}");assert!(prompt.contains(env!("CARGO_BIN_EXE_rdm")),"{prompt}");assert!(prompt.contains("fixture"),"{prompt}");reply(name,args)
    }).unwrap();
    assert_eq!(result["coverage"]["complete"], true);
    assert_eq!(labels, vec!["find:plan:intent-alignment"]);
}
#[test]
fn code_review_empty_reviewer_selection_cannot_approve() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["reviewers"] = json!([]);
    let error = fixture
        .review(spec, |_, _| {
            panic!("empty selection must not invoke agents")
        })
        .unwrap_err();
    assert!(error.contains("empty") || error.contains("ZERO"), "{error}");
}

#[test]
fn code_review_supplies_approved_plan_scope_to_independent_refuter() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["planSlug"] = json!("approved-slice");
    let mut read = false;
    let mut refuted = false;
    let result=fixture.review(spec,|name,args|{
        if name=="rdm" {assert_eq!(args[0][0],"plan");assert_eq!(args[0][1],"show");assert_eq!(args[0][2],"approved-slice");read=true;return Ok(json!({"project":"fixture","slug":"approved-slice","status":"approved","body":"Implement a = 2.","implements":"rdm:phase/example/phase-3-runtime"}));}
        let label=args[1]["label"].as_str().unwrap();
        if label.starts_with("refute:") {refuted=true;let prompt=args[0].as_str().unwrap();assert!(prompt.contains("plan show 'approved-slice'"),"{prompt}");assert!(prompt.contains("Grade whether this finding is IN SCOPE"),"{prompt}");assert!(prompt.contains(env!("CARGO_BIN_EXE_rdm")),"{prompt}");assert!(prompt.contains("fixture"),"{prompt}");return Ok(json!({"refuted":true,"confidence":95}));}
        if label=="find:code:correctness" {return Ok(json!({"findings":[finding(0)]}));}
        reply(name,args)
    }).unwrap();
    assert!(read);
    assert!(refuted);
    assert_eq!(result["outcome"], "reviewed");
}

#[test]
#[cfg(unix)]
fn judgment_process_receives_selected_plan_and_session_identity() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let binary = fixture.root.join("codex");
    fs::write(
        &binary,
        "#!/bin/sh\ncat >/dev/null\nenv > captured-env\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let mut ctx = fixture.ctx();
    ctx["identity"]["planRoot"] = json!(fixture.root.join("plan"));
    ctx["session"] = json!("owned-session");
    let error=bridge::invoke_request(json!({"module":bridge::repository().join(RUNTIME),"export":"createJudgmentAgent","args":[ctx,{"plan":{"model":"gpt-6-astra","effort":"medium"}},{}],"callArgs":["Rate fixture",{"agentType":"estimator","label":"estimate:rate:fixture","schema":{"type":"object","additionalProperties":false,"properties":{},"required":[]}}],"env":{"PATH":format!("{}:{}",fixture.root.display(),std::env::var("PATH").unwrap()),"RDM_ROOT":"/wrong-root","RDM_PROJECT":"wrong-project","RDM_SESSION":"wrong-session","RDM_BIN":"wrong-binary","RDM_EXTRA":"must-remove"}}),|_,_|panic!()).unwrap_err();
    assert!(error.contains("exit 99"), "{error}");
    let env = fs::read_to_string(fixture.root.join("captured-env")).unwrap();
    assert!(
        env.lines()
            .any(|line| line == format!("RDM_ROOT={}", fixture.root.join("plan").display())),
        "{env}"
    );
    assert!(
        env.lines().any(|line| line == "RDM_PROJECT=fixture"),
        "{env}"
    );
    assert!(
        env.lines().any(|line| line == "RDM_SESSION=owned-session"),
        "{env}"
    );
    assert!(
        env.lines()
            .any(|line| line == format!("RDM_BIN={}", env!("CARGO_BIN_EXE_rdm"))),
        "{env}"
    );
    assert!(!env.contains("RDM_EXTRA="), "{env}");
}

#[test]
fn session_records_owned_identity_and_refuses_reusing_evidence() {
    let source = Fixture::new();
    let plan = Fixture::new();
    let run = plan.root.join("run");
    let args = json!([{"sourceDir":source.root,"planRoot":plan.root,"rdmBin":env!("CARGO_BIN_EXE_rdm"),"project":"fixture","session":"parent-session","runDir":run,"operation":"estimate"}]);
    let result=bridge::invoke_request(json!({"module":bridge::repository().join("scripts/lib/codex-runtime-state.mjs"),"export":"createRun","args":args,"steps":[{"method":"finish","args":[{"approved":false}]},{"method":"rdm","args":[["status"]]}]}),|_,_|panic!()).unwrap();
    assert!(result[1]["error"].as_str().unwrap().contains("closed"));
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(run.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["status"], "completed");
    assert_eq!(manifest["identity"]["parentSession"], "parent-session");
    assert!(
        manifest["identity"]["session"]
            .as_str()
            .unwrap()
            .starts_with("codex-")
    );
    let error = bridge::invoke(
        "scripts/lib/codex-runtime-state.mjs",
        "createRun",
        args,
        |_, _| panic!(),
    )
    .unwrap_err();
    assert!(error.to_lowercase().contains("exist"), "{error}");
}
#[test]
fn session_uncertain_write_prevents_success_and_records_recovery_evidence() {
    let source = Fixture::new();
    let plan = Fixture::new();
    let run = plan.root.join("run");
    let result=bridge::invoke_request(json!({"module":bridge::repository().join("scripts/lib/codex-runtime-state.mjs"),"export":"createRun","args":[{"sourceDir":source.root,"planRoot":plan.root,"rdmBin":env!("CARGO_BIN_EXE_rdm"),"project":"fixture","session":"parent-session","runDir":run,"operation":"estimate"}],"steps":[{"method":"rdm","args":[["nonexistent-command"],{"mutating":true}]},{"method":"finish","args":[{}]},{"method":"fail","args":[{"message":"interrupted"}]}]}),|_,_|panic!()).unwrap();
    assert!(result[0]["error"].is_string());
    assert!(result[1]["error"].as_str().unwrap().contains("uncertain"));
    let journal = fs::read_to_string(run.join("journal.jsonl")).unwrap();
    assert!(journal.contains("write-intent"));
    assert!(journal.contains("write-uncertain"));
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(run.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["status"], "failed");
}
#[test]
fn session_forbids_all_session_commit_and_identity_overrides() {
    let source = Fixture::new();
    let plan = Fixture::new();
    let run = plan.root.join("run");
    let result=bridge::invoke_request(json!({"module":bridge::repository().join("scripts/lib/codex-runtime-state.mjs"),"export":"createRun","args":[{"sourceDir":source.root,"planRoot":plan.root,"rdmBin":env!("CARGO_BIN_EXE_rdm"),"project":"fixture","session":"parent-session","runDir":run,"operation":"estimate"}],"steps":[{"method":"rdm","args":[["commit","--all"],{"mutating":true}]},{"method":"rdm","args":[["--root","/other","status"]]},{"method":"fail","args":[{"message":"test complete"}]}]}),|_,_|panic!()).unwrap();
    assert!(result[0]["error"].as_str().unwrap().contains("--all"));
    assert!(result[1]["error"].as_str().unwrap().contains("override"));
}
#[test]
fn runtime_invalid_specs_fail_before_creating_evidence() {
    let fixture = Fixture::new();
    let run = fixture.root.join("evidence");
    for spec in [
        json!({"operation":"unknown","runDir":run}),
        json!({"operation":"estimate","concurrency":0,"runDir":run}),
    ] {
        assert!(bridge::invoke(RUNTIME, "runRuntime", json!([spec]), |_, _| panic!()).is_err());
        assert!(!run.exists());
    }
}

#[test]
fn runtime_cli_invalid_specs_have_no_success_output_or_run_evidence() {
    let fixture = Fixture::new();
    let run = fixture.root.join("evidence");
    let file = fixture.root.join("spec.json");
    for content in [
        "{bad json".to_owned(),
        json!({"operation":"unknown","runDir":run}).to_string(),
        json!({"operation":"estimate","concurrency":0,"runDir":run}).to_string(),
    ] {
        fs::write(&file, content).unwrap();
        let mut command = assert_cmd::Command::new("node");
        command
            .arg(bridge::repository().join("scripts/rdm-codex.mjs"))
            .arg(&file)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &fixture.root)
            .env("CODEX_HOME", fixture.root.join("codex-home"))
            .timeout(std::time::Duration::from_secs(5));
        command
            .assert()
            .code(1)
            .stdout("")
            .stderr(predicates::str::contains("Codex runtime failed:"));
        assert!(!run.exists());
    }
}

#[test]
#[cfg(unix)]
fn runtime_model_resolution_process_cannot_change_phase_policy_silently() {
    use std::os::unix::fs::PermissionsExt;
    let source = Fixture::new();
    let plan = Fixture::new();
    let run = plan.root.join("run");
    let binary = plan.root.join("fake-rdm");
    let mut initial = phase_item();
    initial["model"] = json!("small");
    let mut changed = phase_item();
    changed["model"] = json!("large");
    let worktrees = json!([{"item":"example","path":source.root,"branch":git(&source.root,&["symbolic-ref","--short","HEAD"])}]);
    // Scripted process replies are fixture data owned by this Rust test. The
    // production adapter still launches each request and rereads real files.
    for (name, value) in [
        ("initial.json", initial),
        ("changed.json", changed),
        ("worktrees.json", worktrees),
    ] {
        fs::write(plan.root.join(name), value.to_string()).unwrap();
    }
    fs::write(&binary,"#!/bin/sh\ncase \"$1\" in\nphase) if test -f \"$RDM_ROOT/policy-changed\"; then cat \"$RDM_ROOT/changed.json\"; else cat \"$RDM_ROOT/initial.json\"; fi;;\nworktree) cat \"$RDM_ROOT/worktrees.json\";;\nmodel) touch \"$RDM_ROOT/policy-changed\"; printf '{\"step\":\"%s\",\"host\":\"codex\",\"tier\":\"medium\",\"model\":\"gpt-6-astra\",\"effort\":\"medium\"}\\n' \"$3\";;\n*) exit 1;;\nesac\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let codex = plan.root.join("codex");
    fs::write(&codex, "#!/bin/sh\ntouch unexpected-agent\nexit 99\n").unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec = phase_spec(&source);
    spec["operation"] = json!("code-review");
    spec["sourceDir"] = json!(source.root);
    spec["planRoot"] = json!(plan.root);
    spec["rdmBin"] = json!(binary);
    spec["project"] = json!("fixture");
    spec["session"] = json!("parent");
    spec["runDir"] = json!(run);
    spec["host"] = host();
    let error=bridge::invoke_request(json!({"module":bridge::repository().join(RUNTIME),"export":"runRuntime","args":[spec],"env":{"PATH":format!("{}:{}",plan.root.display(),std::env::var("PATH").unwrap())}}),|_,_|panic!()).unwrap_err();
    assert!(error.contains("changed during model resolution"), "{error}");
    assert!(!source.root.join("unexpected-agent").exists());
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(run.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["status"], "failed");
    assert!(
        !fs::read_to_string(run.join("journal.jsonl"))
            .unwrap()
            .contains("agent-started")
    );
}

#[test]
#[cfg(unix)]
fn process_timeout_reaps_descendants_before_late_effects() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let binary = fixture.root.join("fake-codex");
    fs::write(&binary,"#!/bin/sh\ncat >/dev/null\n(sleep 1; touch late-effect) &\necho $! > descendant-pid\nwait\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let error=bridge::invoke("scripts/lib/codex-process.mjs","runCodex",json!([{"bin":binary,"cwd":fixture.root,"prompt":"fixture","schema":{"type":"object","properties":{},"required":[],"additionalProperties":false},"model":"fixture-model","effort":"medium","timeoutMs":100}]),|_,_|panic!()).unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    assert!(fixture.root.join("descendant-pid").exists());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(!fixture.root.join("late-effect").exists());
}

fn approved_plan() -> Value {
    json!({"project":"fixture","slug":"approved-slice","status":"approved","body":"Implement a = 2.","implements":"rdm:phase/example/phase-3-runtime"})
}
fn rejects_plan_field(field: &str, value: Value) {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["planSlug"] = json!("approved-slice");
    let mut plan = approved_plan();
    plan[field] = value;
    let error = fixture
        .review(spec, |name, args| {
            assert_eq!(name, "rdm", "invalid plan must reject before judgment");
            assert_eq!(args[0][0], "plan");
            Ok(plan.clone())
        })
        .unwrap_err();
    assert!(
        error.contains("approved plan in the selected project"),
        "{error}"
    );
}
#[test]
fn code_review_rejects_unapproved_plan() {
    rejects_plan_field("status", json!("draft"));
}
#[test]
fn code_review_rejects_plan_from_wrong_project() {
    rejects_plan_field("project", json!("other"));
}
#[test]
fn code_review_rejects_wrong_plan_slug() {
    rejects_plan_field("slug", json!("other"));
}
#[test]
fn code_review_rejects_empty_approved_plan() {
    rejects_plan_field("body", json!("  "));
}
#[test]
fn code_review_rejects_plan_implementing_another_item() {
    let fixture = Fixture::new();
    let mut spec = phase_spec(&fixture);
    spec["planSlug"] = json!("approved-slice");
    let branch = git(&fixture.root, &["symbolic-ref", "--short", "HEAD"]);
    let mut plan = approved_plan();
    plan["implements"] = json!("rdm:phase/other/phase-3-runtime");
    let error = fixture
        .review(spec, |name, args| {
            assert_eq!(name, "rdm", "unrelated plan must reject before judgment");
            Ok(match args[0][0].as_str().unwrap() {
                "phase" => phase_item(),
                "plan" => plan.clone(),
                "worktree" => json!([{"item":"example","path":fixture.root,"branch":branch}]),
                other => panic!("unexpected command {other}"),
            })
        })
        .unwrap_err();
    assert!(error.contains("does not implement"), "{error}");
}
#[test]
fn code_review_rejects_approved_plan_drift() {
    let fixture = Fixture::new();
    let mut spec = fixture.spec();
    spec["planSlug"] = json!("approved-slice");
    let mut plan = approved_plan();
    let error = fixture
        .review(spec, |name, args| {
            if name == "rdm" {
                return Ok(plan.clone());
            }
            plan["body"] = json!("Changed scope while reviewing");
            reply(name, args)
        })
        .unwrap_err();
    assert!(error.contains("Approved plan changed"), "{error}");
}
#[test]
fn plan_review_phase_context_pins_source_range_and_checkout() {
    let fixture = Fixture::new();
    let plan_dir = tempfile::tempdir().unwrap();
    let plan = plan_dir.path().canonicalize().unwrap().join("plan.md");
    fs::write(&plan, "Implement a = 2.").unwrap();
    let branch = git(&fixture.root, &["symbolic-ref", "--short", "HEAD"]);
    let mut prompts = Vec::new();
    let result=bridge::invoke(RUNTIME,"reviewPlan",json!([fixture.ctx(),{"planFile":plan,"item":{"type":"phase","roadmap":"example","phase":"phase-3-runtime"},"base":fixture.base,"head":fixture.head,"reviewers":["coherence"]},deps(),{}]),|name,args|{
        if name=="rdm" {return Ok(if args[0][0]=="phase" {phase_item()}else{json!([{"item":"example","path":fixture.root,"branch":branch}])});}
        prompts.push(args[0].as_str().unwrap().to_owned());reply(name,args)
    }).unwrap();
    assert_eq!(result["coverage"]["complete"], true);
    assert_eq!(prompts.len(), 1);
    let prompt = &prompts[0];
    assert!(prompt.contains("review source"), "{prompt}");
    assert!(prompt.contains(fixture.root.to_str().unwrap()), "{prompt}");
    assert!(prompt.contains(&fixture.base), "{prompt}");
    assert!(prompt.contains(&fixture.head), "{prompt}");
    assert!(prompt.contains("phase-3-runtime"), "{prompt}");
    assert!(prompt.contains(&branch), "{prompt}");
}

#[test]
fn plan_review_rejects_source_revisions_without_item() {
    let fixture = Fixture::new();
    let plan_dir = tempfile::tempdir().unwrap();
    let plan = plan_dir.path().canonicalize().unwrap().join("plan.md");
    fs::write(&plan, "Implement a = 2.").unwrap();
    let error = bridge::invoke(
        RUNTIME,
        "reviewPlan",
        json!([fixture.ctx(), {"planFile": plan, "base": fixture.base, "head": fixture.head}, deps(), {}]),
        |_, _| panic!("an unbound source pin must fail before judgment"),
    )
    .unwrap_err();
    assert!(error.contains("require an explicit item"), "{error}");
}
